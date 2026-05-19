use anyhow::{Context, Result};
use belisarius_core::QualityIssue;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct QualityArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Emit JSON instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: QualityArgs) -> Result<()> {
    let report = belisarius_scan::analyze(&args.path)
        .with_context(|| format!("analyzing {}", args.path.display()))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report.quality)?);
        return Ok(());
    }
    let q = &report.quality;
    println!("project       {}", report.scan.root);
    println!("files         {}", report.scan.files.len());
    println!("functions     {}", report.functions.len());
    println!("cycles        {}", report.cycles.len());
    println!("max depth     {}", report.max_depth);
    println!();
    println!("quality       {} / 100", fmt_axis(q.score));
    println!(
        "  complexity  {}    severity-weighted mean (worse of cc & cognitive)",
        fmt_axis(q.axes.complexity)
    );
    println!(
        "  acyclicity  {}    exp decay over Σ log2(cycle_size+1), normalized by √files",
        fmt_axis(q.axes.acyclicity)
    );
    println!(
        "  dead_code   {}    100 - 500·(dead/total), excluding lib/main/bin/tests/examples",
        fmt_axis(q.axes.dead_code)
    );
    println!(
        "  coupling    {}    mean severity over out-degree (ok ≤ 15, bad ≥ 40)",
        fmt_axis(q.axes.coupling)
    );
    if !q.top_issues.is_empty() {
        println!("\ntop issues:");
        for issue in &q.top_issues {
            match issue {
                QualityIssue::HotFunction {
                    file,
                    name,
                    start_line,
                    cyclomatic,
                    cognitive,
                } => {
                    println!(
                        "  cc={cyclomatic:>2} cog={cognitive:>2}  {name}  ({file}:{start_line})"
                    );
                }
                QualityIssue::Cycle { nodes } => {
                    println!("  cycle: {}", nodes.join(" → "));
                }
                QualityIssue::DeadFile { path } => {
                    println!("  dead:  {path}");
                }
            }
        }
    }
    Ok(())
}

fn fmt_axis(v: Option<f32>) -> String {
    match v {
        Some(x) => format!("{:>5.1}", x),
        None => "    —".to_string(),
    }
}
