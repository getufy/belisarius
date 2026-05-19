use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct HotspotsArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Window for "recent" churn, in days.
    #[arg(long, default_value_t = 90)]
    pub days: u32,
    /// Print at most this many rows.
    #[arg(long, default_value_t = 25)]
    pub limit: usize,
    /// Emit the report as JSON.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: HotspotsArgs) -> Result<()> {
    let report = belisarius_scan::analyze(&args.path)
        .with_context(|| format!("analyzing {}", args.path.display()))?;
    let keep: Vec<String> = report.scan.files.iter().map(|f| f.path.clone()).collect();
    let git = belisarius_scan::git_stats::collect(&args.path, args.days, Some(&keep))
        .with_context(|| format!("git stats for {}", args.path.display()))?;
    let mut hotspots =
        belisarius_scan::git_stats::rank_hotspots(&git, &report.file_metrics, args.limit);
    let codeowners = belisarius_scan::codeowners::CodeownersFile::load(&args.path);
    belisarius_scan::git_stats::attach_owners(&mut hotspots, codeowners.as_ref());

    if args.json {
        println!("{}", serde_json::to_string_pretty(&hotspots)?);
        return Ok(());
    }

    if !hotspots.repo_present {
        println!(
            "no git repository under {} — nothing to rank",
            args.path.display()
        );
        return Ok(());
    }
    if hotspots.hotspots.is_empty() {
        println!("no file changes in the last {} days", args.days);
        return Ok(());
    }

    println!(
        "hotspots in last {} days  (churn × cyclomatic, log-damped)",
        hotspots.days_window
    );
    let has_owners = hotspots.hotspots.iter().any(|h| !h.owners.is_empty());
    if has_owners {
        println!(
            "{:>6}  {:>5}  {:>5}  {:>5}  {:<18}  {:<18}  {:<26}  file",
            "score", "churn", "total", "cc", "last commit by", "top in window", "owners"
        );
        for h in &hotspots.hotspots {
            let last = trim(h.last_author.as_deref().unwrap_or("—"), 18);
            let top = trim(h.top_author.as_deref().unwrap_or("—"), 18);
            let owners = if h.owners.is_empty() {
                "—".to_string()
            } else {
                h.owners.join(" ")
            };
            println!(
                "{:>6.0}  {:>5}  {:>5}  {:>5}  {:<18}  {:<18}  {:<26}  {}",
                h.score,
                h.churn,
                h.total_commits,
                h.complexity,
                last,
                top,
                trim(&owners, 26),
                h.path,
            );
        }
    } else {
        println!(
            "{:>6}  {:>5}  {:>5}  {:>5}  {:<22}  {:<22}  file",
            "score", "churn", "total", "cc", "last commit by", "top in window"
        );
        for h in &hotspots.hotspots {
            let last = trim(h.last_author.as_deref().unwrap_or("—"), 22);
            let top = trim(h.top_author.as_deref().unwrap_or("—"), 22);
            println!(
                "{:>6.0}  {:>5}  {:>5}  {:>5}  {:<22}  {:<22}  {}",
                h.score, h.churn, h.total_commits, h.complexity, last, top, h.path,
            );
        }
    }
    Ok(())
}

fn trim(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n - 1])
    }
}
