//! `belisarius watch` — file-system watcher that rebuilds the right pieces
//! of Belisarius's state when the project changes.
//!
//! Design goals:
//!   - Foreground process. User runs it in a terminal, sees live updates.
//!   - Cheap: `notify` + `notify-debouncer-mini` for the OS-level watch,
//!     blake3 hashing to skip no-op events (touch / chmod / save-with-no-
//!     content-change).
//!   - Composable: by default just prints the dirty set and rebuilds the
//!     search index. `--with-scip` opts into the slower per-language SCIP
//!     rebuild path.
//!   - Ignores `.git/`, `node_modules/`, `target/`, `.belisarius/`, and any
//!     extensionless / non-source paths.
//!
//! Stop with `Ctrl+C`. The watcher thread exits cleanly when stdin / signal
//! handling kills the process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEventKind};

use crate::state_db;

const DEBOUNCE_MS: u64 = 300;
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "swift", "c", "cc",
    "cpp", "h", "hpp", "rb", "php", "cs", "scala", "ex", "exs", "elm", "ml", "hs", "lua", "json",
    "yaml", "yml", "toml",
];
const IGNORED_DIR_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".belisarius",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".pytest_cache",
    ".venv",
    "venv",
    ".tox",
];

#[derive(clap::Args)]
pub struct WatchArgs {
    /// Project root to watch. Defaults to current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Debounce window. Multiple events arriving within this window are
    /// coalesced into one reindex pass. Default 300ms.
    #[arg(long, default_value_t = DEBOUNCE_MS)]
    pub debounce_ms: u64,
    /// Skip the hybrid-search reindex leg. Useful when the project hasn't
    /// run `belisarius search index` yet and you just want a dirty-set log.
    #[arg(long)]
    pub no_search: bool,
    /// Trigger SCIP per-language rebuild on changes. Off by default because
    /// SCIP indexers (rust-analyzer, scip-typescript, …) are slow.
    #[arg(long)]
    pub with_scip: bool,
}

pub async fn run(args: WatchArgs) -> Result<()> {
    let project = args
        .path
        .canonicalize()
        .with_context(|| format!("project path {}", args.path.display()))?;
    let belisarius_dir = project.join(".belisarius");
    std::fs::create_dir_all(&belisarius_dir).ok();

    println!("watch  {}", project.display());
    println!(
        "       debounce={}ms search={} scip={}",
        args.debounce_ms,
        if args.no_search { "off" } else { "on" },
        if args.with_scip { "on" } else { "off" }
    );
    println!("       (Ctrl+C to stop)");
    println!();

    // ── search-leg watcher (optional) ────────────────────────────────────
    // The belisarius_search crate already implements its own watcher; we
    // simply start it alongside ours when search reindex is wanted. Keeps
    // the handle alive for the lifetime of this process.
    let _search_watcher = if args.no_search {
        None
    } else {
        match belisarius_search::IndexHandle::open(&project) {
            Ok(handle) => belisarius_search::watch(handle, false)
                .map(Some)
                .unwrap_or_else(|e| {
                    eprintln!("search watcher unavailable: {e:#}");
                    None
                }),
            Err(e) => {
                eprintln!(
                    "search index not built (run `belisarius index --with-search .` first): {e:#}"
                );
                None
            }
        }
    };

    // ── our own watcher: scan-delta + (optional) SCIP rebuild ────────────
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(args.debounce_ms), move |res| {
        let _ = tx.send(res);
    })
    .context("creating debouncer")?;
    debouncer
        .watcher()
        .watch(&project, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", project.display()))?;

    // Drain the channel. Each batch is one "flush" of coalesced events.
    while let Ok(batch) = rx.recv() {
        let events = match batch {
            Ok(evs) => evs,
            Err(e) => {
                eprintln!("notify error: {e:#}");
                continue;
            }
        };

        // Filter to source files we care about, relative to the project.
        let mut dirty: HashSet<String> = HashSet::new();
        for ev in events {
            if !matches!(
                ev.kind,
                DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
            ) {
                continue;
            }
            let Ok(rel) = ev.path.strip_prefix(&project) else {
                continue;
            };
            if is_ignored(rel) {
                continue;
            }
            if !is_source(rel) {
                continue;
            }
            if let Some(s) = rel.to_str() {
                dirty.insert(s.to_string());
            }
        }
        if dirty.is_empty() {
            continue;
        }

        // Compute delta against state.db hashes. `full_scan=false` because
        // the candidate set here is the dirty notify paths, not the whole
        // tree — files outside the dirty set are stable and shouldn't be
        // false-positived as "removed".
        let Ok(conn) = state_db::open(&project) else {
            continue;
        };
        let candidate: Vec<String> = dirty.iter().cloned().collect();
        let (delta, new_hashes) =
            match state_db::compute_scan_delta(&conn, &project, &candidate, false) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("scan delta failed: {e:#}");
                    continue;
                }
            };
        if delta.is_empty() {
            // notify fired but content didn't change (touch / chmod).
            continue;
        }
        print_delta(&delta);

        // Persist new hashes so the next event batch sees a clean baseline.
        // We commit *before* SCIP rebuild because the hash table reflects
        // "what we've seen", not "what we've indexed" — SCIP failures
        // shouldn't make us reprocess the same files next time.
        if let Err(e) = state_db::commit_scan_delta(&conn, &delta, &new_hashes) {
            eprintln!("committing hashes: {e:#}");
        }

        // ── SCIP rebuild (opt-in) ────────────────────────────────────────
        if args.with_scip {
            run_scip_rebuild(&project, &delta).await;
        }
    }
    Ok(())
}

fn is_ignored(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        std::path::Component::Normal(name) => {
            let s = name.to_string_lossy();
            IGNORED_DIR_NAMES.iter().any(|i| *i == s.as_ref())
        }
        _ => false,
    })
}

fn is_source(rel: &Path) -> bool {
    let Some(ext) = rel.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    SOURCE_EXTENSIONS.contains(&ext)
}

fn print_delta(delta: &state_db::ScanDelta) {
    let ts = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_default();
    println!(
        "[{ts}] +{} ~{} -{}",
        delta.added.len(),
        delta.changed.len(),
        delta.removed.len()
    );
    for p in delta.added.iter().take(5) {
        println!("  + {p}");
    }
    for p in delta.changed.iter().take(5) {
        println!("  ~ {p}");
    }
    for p in delta.removed.iter().take(5) {
        println!("  - {p}");
    }
    let extra = delta.total().saturating_sub(
        delta.added.iter().take(5).count()
            + delta.changed.iter().take(5).count()
            + delta.removed.iter().take(5).count(),
    );
    if extra > 0 {
        println!("  … {extra} more");
    }
}

/// Triggers a per-language SCIP rebuild for the languages touched by this
/// delta. We don't have file-level SCIP indexing (the upstream indexers
/// rebuild a whole language at a time), so this collapses to "rerun SCIP
/// for any language present in the dirty set".
async fn run_scip_rebuild(project: &Path, delta: &state_db::ScanDelta) {
    let touched: HashSet<&str> = delta
        .added
        .iter()
        .chain(delta.changed.iter())
        .filter_map(|p| {
            Path::new(p)
                .extension()
                .and_then(|s| s.to_str())
                .and_then(ext_to_language)
        })
        .collect();
    if touched.is_empty() {
        return;
    }
    println!(
        "  scip rebuild: {}",
        touched.iter().copied().collect::<Vec<_>>().join(", ")
    );
    // Reuse the existing `belisarius index` per-language run path by spawning
    // a child process. Keeps the watcher resilient to indexer crashes and
    // sidesteps having to thread async into the indexer plumbing.
    let exe = std::env::current_exe().unwrap_or_else(|_| "belisarius".into());
    for lang in touched {
        let project_owned = project.to_path_buf();
        let exe_owned = exe.clone();
        let lang_owned = lang.to_string();
        let status = tokio::task::spawn_blocking(move || {
            std::process::Command::new(&exe_owned)
                .arg("index")
                .arg(&project_owned)
                .arg("--lang")
                .arg(&lang_owned)
                .arg("--force")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .status()
        })
        .await;
        match status {
            Ok(Ok(s)) if s.success() => println!("    ok: {lang}"),
            Ok(Ok(s)) => println!("    fail: {lang} (exit {s})"),
            Ok(Err(e)) => println!("    error spawning indexer for {lang}: {e}"),
            Err(e) => println!("    join error for {lang}: {e}"),
        }
    }
}

fn ext_to_language(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_node_modules() {
        assert!(is_ignored(Path::new("node_modules/foo/bar.js")));
        assert!(is_ignored(Path::new("crates/x/node_modules/y.ts")));
        assert!(!is_ignored(Path::new("crates/x/src/lib.rs")));
    }

    #[test]
    fn ignores_belisarius_state_dir() {
        assert!(is_ignored(Path::new(".belisarius/state.db")));
        assert!(is_ignored(Path::new(".belisarius/scip/merged.scip")));
    }

    #[test]
    fn ignores_target_and_dist() {
        assert!(is_ignored(Path::new("target/debug/deps/foo")));
        assert!(is_ignored(Path::new("web/dist/bundle.js")));
    }

    #[test]
    fn matches_known_source_extensions() {
        assert!(is_source(Path::new("foo.rs")));
        assert!(is_source(Path::new("foo.ts")));
        assert!(is_source(Path::new("path/to/foo.py")));
        assert!(!is_source(Path::new("foo.lock")));
        assert!(!is_source(Path::new("README")));
    }

    #[test]
    fn ext_to_language_maps_common_extensions() {
        assert_eq!(ext_to_language("rs"), Some("rust"));
        assert_eq!(ext_to_language("tsx"), Some("typescript"));
        assert_eq!(ext_to_language("py"), Some("python"));
        assert_eq!(ext_to_language("toml"), None);
    }
}
