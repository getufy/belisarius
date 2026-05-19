//! File-system watcher that keeps the search index fresh.
//!
//! Wraps `notify-debouncer-mini` to coalesce filesystem events into a small
//! batch, filter out everything the indexer doesn't care about, then call
//! `IndexHandle::reindex(ReindexOptions { full: false, .. })`. The
//! reindex itself is already incremental — files whose content hash hasn't
//! changed are skipped inside `reindex_inner`, so the practical cost of a
//! "spurious" notification (Vim saving via rename, IDE creating temp files)
//! is one walk over the project + cheap stat per file.
//!
//! Filters applied before triggering a reindex:
//!  1. The path must lie under the project root (notify can hand us paths
//!     under symlinked targets we don't actually care about).
//!  2. Anything inside `.belisarius/` is ignored — those are our own
//!     index artifacts; reacting to them would loop forever.
//!  3. Anything inside well-known build / dependency dirs (`target/`,
//!     `node_modules/`, `dist/`, `.git/`, `.venv/`, etc.) is ignored.
//!  4. The file extension must map to a tracked language. Files that
//!     `belisarius_scan` can't chunk produce no chunks anyway, so reacting
//!     to them just burns CPU on the walk.
//!
//! The watcher runs on a background thread so callers (CLI `watch`, HTTP
//! `serve --watch`) can keep their main thread free. Stop by dropping the
//! returned `WatcherHandle`.

use crate::index::{IndexHandle, ReindexOptions};
use anyhow::{Context, Result};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEventKind, Debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How long to wait after the last change event before triggering a
/// reindex. Tuned to coalesce a typical "save, format-on-save, second save"
/// sequence into one reindex.
const DEBOUNCE_MS: u64 = 500;

/// Build-output / vendor directories we never want to walk. Matched as a
/// component name anywhere in the path, not as a prefix — `crates/foo/target`
/// is ignored just like top-level `./target`.
const SKIP_DIRS: &[&str] = &[
    ".belisarius",
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    ".next",
    ".turbo",
    ".cache",
];

/// Returned to the caller; dropping it stops the watcher.
pub struct WatcherHandle {
    /// Holds the notify watcher alive. Dropping `_debouncer` ends notify's
    /// internal thread, then the channel closes, then our processor exits.
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    /// JoinHandle for the processor thread. Not awaited on drop — we let it
    /// run to completion (the channel close is the stop signal).
    _processor: Option<JoinHandle<()>>,
}

/// Start watching `project_root`. Returns immediately; the watcher runs on
/// its own thread and triggers `IndexHandle::reindex` whenever files
/// matching the filter change.
///
/// `bm25_only` mirrors the reindex option of the same name — pass `true`
/// during local dev to skip embedding entirely (fast, but loses semantic
/// search until a full reindex runs).
pub fn watch(index: Arc<IndexHandle>, bm25_only: bool) -> Result<WatcherHandle> {
    let project_root = index.project_root.clone();
    if !project_root.exists() {
        anyhow::bail!("project root does not exist: {}", project_root.display());
    }

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(DEBOUNCE_MS), move |res| {
        // `send` only fails when the receiver has hung up — that's the stop
        // signal; we silently drop the event in that case.
        let _ = tx.send(res);
    })
    .context("creating debouncer")?;
    debouncer
        .watcher()
        .watch(&project_root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", project_root.display()))?;

    let project_root_for_thread = project_root.clone();
    let processor = thread::Builder::new()
        .name("belisarius-watcher".into())
        .spawn(move || run(rx, index, project_root_for_thread, bm25_only))
        .context("spawning watcher thread")?;

    tracing::info!(
        target: "belisarius_search",
        "watching {} for changes (debounce {}ms)",
        project_root.display(),
        DEBOUNCE_MS,
    );

    Ok(WatcherHandle {
        _debouncer: debouncer,
        _processor: Some(processor),
    })
}

fn run(
    rx: mpsc::Receiver<DebounceEventResult>,
    index: Arc<IndexHandle>,
    project_root: PathBuf,
    bm25_only: bool,
) {
    while let Ok(events) = rx.recv() {
        let events = match events {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(target: "belisarius_search", "watcher error: {e:?}");
                continue;
            }
        };

        // Filter the batch. If nothing survives, don't bother reindexing.
        let relevant: Vec<PathBuf> = events
            .into_iter()
            .filter(|e| matches!(e.kind, DebouncedEventKind::Any))
            .map(|e| e.path)
            .filter(|p| is_relevant(&project_root, p))
            .collect();

        if relevant.is_empty() {
            continue;
        }

        tracing::info!(
            target: "belisarius_search",
            "{} change(s) — reindexing (incremental)",
            relevant.len(),
        );

        // Run the incremental reindex on this thread. The hash-skip path
        // inside `reindex_inner` makes this O(N file stats) when only a
        // handful of files actually changed.
        let opts = ReindexOptions {
            full: false,
            bm25_only,
        };
        if let Err(e) = index.reindex(opts) {
            tracing::error!(target: "belisarius_search", "watcher reindex failed: {e:#}");
        }
    }
    tracing::info!(target: "belisarius_search", "watcher channel closed, exiting");
}

/// Decide whether a path change should trigger a reindex. Cheap (no I/O).
fn is_relevant(project_root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(project_root) else {
        // notify can hand us paths outside the watched root on some
        // platforms (symlinks, network mounts); skip them.
        return false;
    };

    // Skip our own index artifacts + every well-known build dir.
    for comp in rel.components() {
        if let Some(name) = comp.as_os_str().to_str() {
            if SKIP_DIRS.contains(&name) {
                return false;
            }
        }
    }

    // Only react to files whose extension maps to a tracked language. The
    // chunker would return empty otherwise, so reacting to e.g. `.lock` or
    // `.md` changes just burns the walk.
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if ext.is_empty() {
        return false;
    }
    belisarius_scan::languages::language_for_ext(&ext).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_dirs_filter() {
        let root = Path::new("/proj");
        assert!(!is_relevant(
            root,
            Path::new("/proj/.belisarius/search/chunks.sqlite")
        ));
        assert!(!is_relevant(root, Path::new("/proj/target/debug/foo")));
        assert!(!is_relevant(root, Path::new("/proj/node_modules/x/y.js")));
        assert!(!is_relevant(root, Path::new("/proj/web/dist/index.js")));
        assert!(!is_relevant(
            root,
            Path::new("/proj/crates/foo/target/x.rs")
        ));
        assert!(!is_relevant(root, Path::new("/proj/.git/HEAD")));
    }

    #[test]
    fn tracked_extensions_pass() {
        let root = Path::new("/proj");
        assert!(is_relevant(root, Path::new("/proj/src/lib.rs")));
        assert!(is_relevant(root, Path::new("/proj/web/src/App.tsx")));
        assert!(is_relevant(root, Path::new("/proj/scripts/build.py")));
    }

    #[test]
    fn unknown_extensions_skip() {
        let root = Path::new("/proj");
        // Lock files aren't a tracked extension.
        assert!(!is_relevant(root, Path::new("/proj/Cargo.lock")));
        // Files with no `.ext` (e.g. dotfile-only name) skip.
        assert!(!is_relevant(root, Path::new("/proj/.editorconfig")));
    }

    /// Markdown / TOML / JSON are tracked by `belisarius_scan::languages` so
    /// agents searching for "where's the contributing guide" can hit hits in
    /// markdown bodies. They should trigger reindexes.
    #[test]
    fn docs_and_configs_are_tracked() {
        let root = Path::new("/proj");
        assert!(is_relevant(root, Path::new("/proj/README.md")));
        assert!(is_relevant(root, Path::new("/proj/Cargo.toml")));
        assert!(is_relevant(root, Path::new("/proj/package.json")));
    }

    #[test]
    fn outside_root_skipped() {
        let root = Path::new("/proj");
        assert!(!is_relevant(root, Path::new("/elsewhere/main.rs")));
    }
}
