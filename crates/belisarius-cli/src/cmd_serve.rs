use crate::server;
use anyhow::{Context, Result};
use belisarius_search::{IndexHandle, WatcherHandle};
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct ServeArgs {
    #[arg(long, default_value_t = 7878)]
    pub port: u16,
    /// Optional static directory to serve at `/` (e.g., web/dist).
    #[arg(long)]
    pub web_dir: Option<PathBuf>,
    /// Run a file watcher in the background that incrementally reindexes the
    /// project on change. Pair with `--watch-path` to target a specific
    /// project; defaults to the server's CWD.
    #[arg(long)]
    pub watch: bool,
    /// Project to watch when `--watch` is set. Defaults to the current
    /// working directory.
    #[arg(long, default_value = ".")]
    pub watch_path: PathBuf,
    /// Skip embeddings in the watcher's incremental updates — BM25 only.
    /// Faster, but semantic search drift accumulates until a full reindex.
    #[arg(long)]
    pub watch_bm25_only: bool,
}

pub async fn run(args: ServeArgs) -> Result<()> {
    // Watcher lives for the lifetime of `serve`. Dropping it (when serve
    // returns) tears the watcher down cleanly. We hold it in a binding so
    // it isn't dropped immediately as a temporary.
    let _watcher = if args.watch {
        Some(start_watcher(&args.watch_path, args.watch_bm25_only)?)
    } else {
        None
    };
    server::serve(args.port, args.web_dir).await
}

fn start_watcher(path: &Path, bm25_only: bool) -> Result<WatcherHandle> {
    let handle = IndexHandle::open(path).context("opening search index for watcher")?;
    let watcher = belisarius_search::watch(handle, bm25_only).context("starting watcher")?;
    tracing::info!(target: "belisarius_cli", "watching {} for changes", path.display());
    Ok(watcher)
}
