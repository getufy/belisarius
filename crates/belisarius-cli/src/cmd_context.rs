use anyhow::{Context, Result};
use belisarius_context::{search_artifacts, ContextRegistry};
use belisarius_search::index::IndexHandle;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ContextCmd {
    /// List artifacts in `.belisarius/context_artifacts.json`.
    List(ListArgs),
    /// Print one artifact's resolved files.
    Get(GetArgs),
    /// Semantic search over artifact content (requires search index).
    Search(SearchArgs),
    /// Index registered artifacts into the search index.
    Index(IndexArgs),
}

#[derive(clap::Args)]
pub struct ListArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(clap::Args)]
pub struct GetArgs {
    pub name: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(clap::Args)]
pub struct SearchArgs {
    pub query: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
}

#[derive(clap::Args)]
pub struct IndexArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

pub async fn run(cmd: ContextCmd) -> Result<()> {
    match cmd {
        ContextCmd::List(args) => list_cmd(args).await,
        ContextCmd::Get(args) => get_cmd(args).await,
        ContextCmd::Search(args) => search_cmd(args).await,
        ContextCmd::Index(args) => index_cmd(args).await,
    }
}

async fn list_cmd(args: ListArgs) -> Result<()> {
    let r = ContextRegistry::load(&args.path).context("load registry")?;
    if r.artifacts.is_empty() {
        println!("no artifacts (create .belisarius/context_artifacts.json)");
        return Ok(());
    }
    for a in &r.artifacts {
        println!("{:<24}  {}", a.name, a.description);
        for p in &a.paths {
            println!("  - {p}");
        }
    }
    Ok(())
}

async fn get_cmd(args: GetArgs) -> Result<()> {
    let r = ContextRegistry::load(&args.path)?;
    let c = r.read_artifact(&args.path, &args.name)?;
    println!("# {}", c.artifact.name);
    println!("> {}", c.artifact.description);
    for f in &c.files {
        println!("\n--- {}", f.path);
        println!("{}", f.content);
    }
    Ok(())
}

async fn search_cmd(args: SearchArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path)?;
    let hits =
        tokio::task::spawn_blocking(move || search_artifacts(&handle, &args.query, args.limit))
            .await
            .context("search join")??;
    println!("{}", serde_json::to_string_pretty(&hits)?);
    Ok(())
}

async fn index_cmd(args: IndexArgs) -> Result<()> {
    let handle = IndexHandle::open(&args.path)?;
    let n = tokio::task::spawn_blocking(move || belisarius_context::index_registry(&handle))
        .await
        .context("indexer join")??;
    println!("indexed {n} artifact chunks");
    Ok(())
}
