use anyhow::{Context, Result};
use belisarius_search::index::{IndexHandle, ReindexOptions};
use belisarius_search::search::{search, SearchOptions};
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SearchCmd {
    /// Build or refresh the hybrid (BM25 + embeddings) search index for a project.
    Index(IndexArgs),
    /// Run a hybrid search query against an existing index.
    Query(QueryArgs),
    /// Show index status (chunk count, embedding model, last run).
    Status(StatusArgs),
    /// Pre-download the embedding model (handy on CI or air-gapped hosts).
    FetchModel,
    /// Watch the project tree and reindex changed files automatically.
    ///
    /// Runs in the foreground. The first call after launch performs an
    /// incremental reindex of any file whose content hash has changed since
    /// the last run; subsequent edits trigger a debounced reindex. Stop with
    /// Ctrl-C.
    Watch(WatchArgs),
}

#[derive(clap::Args)]
pub struct IndexArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Drop existing chunks/vectors and re-embed everything.
    #[arg(long)]
    pub full: bool,
    /// Skip the embedding leg; BM25 only.
    #[arg(long)]
    pub bm25_only: bool,
}

#[derive(clap::Args)]
pub struct QueryArgs {
    pub query: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    #[arg(long)]
    pub lang: Option<String>,
    #[arg(long)]
    pub kind: Option<String>,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(clap::Args)]
pub struct WatchArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Skip the embedding leg in incremental updates — handy when iterating
    /// fast on a project you mostly query via BM25.
    #[arg(long)]
    pub bm25_only: bool,
}

pub async fn run(cmd: SearchCmd) -> Result<()> {
    match cmd {
        SearchCmd::Index(args) => index_cmd(args).await,
        SearchCmd::Query(args) => query_cmd(args).await,
        SearchCmd::Status(args) => status_cmd(args).await,
        SearchCmd::FetchModel => fetch_model().await,
        SearchCmd::Watch(args) => watch_cmd(args).await,
    }
}

async fn watch_cmd(args: WatchArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path).context("opening search index")?;

    // Best-effort kick: incremental reindex on launch so the user sees a
    // green status immediately if the disk is already in sync with the
    // tree. If it fails (e.g. missing model), we still start the watcher —
    // future saves can succeed.
    let handle_for_kick = handle.clone();
    let bm25_only = args.bm25_only;
    let _ = tokio::task::spawn_blocking(move || {
        handle_for_kick.reindex(ReindexOptions {
            full: false,
            bm25_only,
        })
    })
    .await;

    let _watcher =
        belisarius_search::watch(handle.clone(), args.bm25_only).context("starting watcher")?;
    println!("watching {} — press Ctrl-C to stop", args.path.display());

    // Park forever; the watcher runs on its own thread. Ctrl-C ends the
    // process which drops `_watcher` and tears down notify cleanly.
    tokio::signal::ctrl_c()
        .await
        .context("waiting for Ctrl-C")?;
    println!("stopping watcher.");
    Ok(())
}

async fn index_cmd(args: IndexArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path).context("opening search index")?;
    let opts = ReindexOptions {
        full: args.full,
        bm25_only: args.bm25_only,
    };
    tokio::task::spawn_blocking(move || handle.reindex(opts))
        .await
        .context("indexer join")??;
    println!("indexed {}", args.path.display());
    Ok(())
}

async fn query_cmd(args: QueryArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path).context("opening search index")?;
    let opts = SearchOptions {
        limit: args.limit,
        lang: args.lang,
        kind: args.kind,
        candidates: 50,
    };
    let hits = tokio::task::spawn_blocking(move || search(&handle, &args.query, &opts))
        .await
        .context("search join")??;
    if hits.is_empty() {
        println!("no hits");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        println!(
            "{:>2}. {:<28}  {}:{}-{}  [{:.4}]  bm25={:?} dense={:?}",
            i + 1,
            truncate(&h.name, 28),
            h.file,
            h.start_line,
            h.end_line,
            h.score,
            h.bm25_rank,
            h.dense_rank,
        );
        for line in h.snippet.lines().take(3) {
            println!("      {line}");
        }
    }
    Ok(())
}

async fn status_cmd(args: StatusArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path).context("opening search index")?;
    let s = handle.status_snapshot();
    println!("{}", serde_json::to_string_pretty(&s)?);
    Ok(())
}

async fn fetch_model() -> Result<()> {
    match belisarius_search::embed::default_provider() {
        Ok(_) => {
            println!(
                "model ready under {}",
                belisarius_search::embed::cache_dir().display()
            );
            Ok(())
        }
        Err(belisarius_search::EmbeddingError::Disabled) => {
            anyhow::bail!("this build was compiled without the `embed` feature")
        }
        Err(e) => Err(anyhow::anyhow!("init: {e}")),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}
