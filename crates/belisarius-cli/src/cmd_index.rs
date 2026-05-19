use anyhow::{Context, Result};
use belisarius_symbols::{registry, IndexerStatus, SymbolStore};
use std::path::PathBuf;
use std::time::Instant;

#[derive(clap::Args)]
pub struct IndexArgs {
    /// Project root to index. Defaults to current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Comma-separated list of language ids to run (e.g., `rust,typescript`).
    /// If omitted, every detected indexer is run.
    #[arg(long, value_delimiter = ',')]
    pub lang: Vec<String>,
    /// Re-run indexers even if a cached `.scip` already exists.
    #[arg(long)]
    pub force: bool,
    /// Write the merged index to this path. Default: `<project>/.belisarius/scip/merged.scip`.
    #[arg(long)]
    pub out: Option<PathBuf>,

    // ── unified-index extensions ────────────────────────────────────────
    /// Also run a structural scan pass before indexing.
    #[arg(long)]
    pub with_scan: bool,
    /// Also rebuild the hybrid search index after SCIP.
    #[arg(long)]
    pub with_search: bool,
    /// Convenience: enable `--with-scan` and `--with-search`. Recommended for
    /// a fresh end-to-end index of a project.
    #[arg(long)]
    pub all: bool,
    /// Skip a stage. Comma-separated values: `scan`, `scip`, `search`.
    #[arg(long, value_delimiter = ',')]
    pub skip: Vec<String>,
    /// Reserved for future incremental indexing. Currently a no-op that
    /// logs a warning and falls back to a full rebuild.
    #[arg(long)]
    pub incremental: bool,
}

pub async fn run(args: IndexArgs) -> Result<()> {
    if args.incremental {
        eprintln!(
            "warning: --incremental is reserved for future use; falling back to full rebuild"
        );
    }
    let project = args
        .path
        .canonicalize()
        .with_context(|| format!("project path {}", args.path.display()))?;
    let cache_dir = project.join(".belisarius").join("scip");
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("creating cache dir {}", cache_dir.display()))?;

    let want_scan = (args.with_scan || args.all) && !args.skip.iter().any(|s| s == "scan");
    let want_scip = !args.skip.iter().any(|s| s == "scip");
    let want_search = (args.with_search || args.all) && !args.skip.iter().any(|s| s == "search");

    // ── stage 1: structural scan ─────────────────────────────────────────
    if want_scan {
        println!("[1/3] scan");
        let report = belisarius_scan::analyze(&project)
            .with_context(|| format!("scanning {}", project.display()))?;
        let score = report.quality.score.map(|s| s.round() as i32);
        let score_str = score.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        println!(
            "      {} files · {} functions · quality {score_str}/100",
            report.scan.files.len(),
            report.functions.len(),
        );
    }

    // ── stage 2: SCIP indexing (the original cmd_index body) ─────────────
    if !want_scip {
        println!("[2/3] scip skipped");
        if want_search {
            run_search_stage(&project, args.force).await?;
        }
        return Ok(());
    } else if want_scan || want_search {
        println!("[2/3] scip");
    }

    let filter: Option<Vec<String>> = if args.lang.is_empty() {
        None
    } else {
        Some(args.lang.iter().map(|s| s.to_lowercase()).collect())
    };

    let indexers = registry();
    let mut produced: Vec<(String, PathBuf)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();

    println!("indexing {}", project.display());
    println!();
    println!("  {:<14} {:<14} status", "language", "indexer");
    println!("  {}", "-".repeat(70));

    for ix in &indexers {
        let lang = ix.language();
        if let Some(filter) = &filter {
            if !filter.iter().any(|l| l == lang) {
                continue;
            }
        }
        let status = ix.status(&project);
        let output_path = cache_dir.join(format!("{lang}.scip"));

        match status {
            IndexerStatus::NotInstalled => {
                println!("  {:<14} {:<14} not installed (skip)", lang, ix.name());
                skipped.push((lang.into(), "not installed".into()));
                continue;
            }
            IndexerStatus::DoesNotApply => {
                println!(
                    "  {:<14} {:<14} no matching project markers (skip)",
                    lang,
                    ix.name()
                );
                skipped.push((lang.into(), "no project markers".into()));
                continue;
            }
            IndexerStatus::Ready => {}
        }

        if output_path.exists() && !args.force {
            let size = std::fs::metadata(&output_path)
                .map(|m| m.len())
                .unwrap_or(0);
            println!(
                "  {:<14} {:<14} cached ({} KB) — use --force to rebuild",
                lang,
                ix.name(),
                size / 1024
            );
            produced.push((lang.into(), output_path));
            continue;
        }

        let started = Instant::now();
        print!("  {:<14} {:<14} running…", lang, ix.name());
        std::io::Write::flush(&mut std::io::stdout()).ok();
        match ix.run(&project, &output_path) {
            Ok(()) => {
                let dur = started.elapsed();
                let size = std::fs::metadata(&output_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                println!(
                    "\r  {:<14} {:<14} ok · {:.1}s · {} KB",
                    lang,
                    ix.name(),
                    dur.as_secs_f64(),
                    size / 1024
                );
                produced.push((lang.into(), output_path));
            }
            Err(e) => {
                println!(
                    "\r  {:<14} {:<14} FAILED: {}",
                    lang,
                    ix.name(),
                    trunc(&format!("{e:#}"), 80)
                );
                skipped.push((lang.into(), format!("failed: {e}")));
            }
        }
    }

    println!();
    if produced.is_empty() {
        println!("no indexes produced.");
        if want_search {
            run_search_stage(&project, args.force).await?;
        }
        return Ok(());
    }

    let merged_path = args
        .out
        .clone()
        .unwrap_or_else(|| cache_dir.join("merged.scip"));

    let mut indexes = Vec::new();
    let pb = crate::progress::bar_for(produced.len() as u64, false);
    if let Some(b) = &pb {
        b.set_prefix("merge");
        b.set_message("reading SCIP shards");
    }
    for (_, path) in &produced {
        let idx = belisarius_symbols::read_index(path)?;
        indexes.push(idx);
        if let Some(b) = &pb {
            b.inc(1);
        }
    }
    if let Some(b) = pb {
        b.finish_and_clear();
    }
    let merged = belisarius_symbols::merge(indexes);
    belisarius_symbols::write_index(&merged, &merged_path)?;

    let store = SymbolStore::new(merged);
    println!("merged → {}", merged_path.display());
    println!(
        "  {} documents · {} unique symbols",
        store.document_count(),
        store.symbol_count()
    );

    let top = store.top_symbols(5);
    if !top.is_empty() {
        println!("\ntop 5 symbols across the merged index:");
        for (sym, count) in top {
            let display = store
                .info_for(&sym)
                .map(|i| i.display_name.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            println!("  {:>5}  {}  {}", count, display, trunc(&sym, 80));
        }
    }

    // ── stage 3: hybrid search index ─────────────────────────────────────
    if want_search {
        run_search_stage(&project, args.force).await?;
    }
    Ok(())
}

/// Rebuild the hybrid search index (BM25 + embeddings). Routed through the
/// same `IndexHandle::reindex` the `belisarius search index` subcommand uses
/// so the two stay in sync.
async fn run_search_stage(project: &std::path::Path, force: bool) -> Result<()> {
    println!("[3/3] search index");
    let path = project.to_path_buf();
    let handle = belisarius_search::IndexHandle::open(&path)
        .with_context(|| format!("opening search index at {}", path.display()))?;
    let opts = belisarius_search::ReindexOptions {
        full: force,
        bm25_only: false,
    };
    tokio::task::spawn_blocking(move || handle.reindex(opts))
        .await
        .context("search indexer join")??;
    println!("      reindexed");
    Ok(())
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (n - 1)..])
    }
}
