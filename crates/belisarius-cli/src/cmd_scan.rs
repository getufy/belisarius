use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct ScanArgs {
    /// Path to scan.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Optional output file (defaults to stdout).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Emit the resolved file graph instead of the raw scan.
    #[arg(long)]
    pub graph: bool,
    /// Emit the full AnalysisReport (scan + graph + AST + cycles + quality).
    #[arg(long, conflicts_with = "graph")]
    pub with_ast: bool,
}

pub async fn run(args: ScanArgs) -> Result<()> {
    let json = if args.with_ast {
        let report = belisarius_scan::analyze(&args.path)
            .with_context(|| format!("analyzing {}", args.path.display()))?;
        serde_json::to_string_pretty(&report)?
    } else if args.graph {
        let scan = belisarius_scan::scan(&args.path)
            .with_context(|| format!("scanning {}", args.path.display()))?;
        let graph = belisarius_scan::build_graph(&scan);
        serde_json::to_string_pretty(&graph)?
    } else {
        let scan = belisarius_scan::scan(&args.path)
            .with_context(|| format!("scanning {}", args.path.display()))?;
        serde_json::to_string_pretty(&scan)?
    };
    match args.out {
        Some(path) => {
            std::fs::write(&path, &json)
                .with_context(|| format!("writing scan to {}", path.display()))?;
            tracing::info!(path = %path.display(), "scan written");
        }
        None => {
            println!("{json}");
        }
    }
    Ok(())
}
