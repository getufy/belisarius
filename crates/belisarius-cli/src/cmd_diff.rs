use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct DiffArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Base ref. Empty (default) means HEAD's first parent — i.e. show what
    /// changed in the most recent commit.
    #[arg(long, default_value = "")]
    pub base: String,
    /// Head ref. Defaults to HEAD.
    #[arg(long, default_value = "HEAD")]
    pub head: String,
    /// Number of top hotspots to consider for the overlap. The overlay
    /// surfaces changed files that show up anywhere in this slice.
    #[arg(long, default_value_t = 100)]
    pub hotspot_window: usize,
    /// Emit the full diff + overlay as JSON.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: DiffArgs) -> Result<()> {
    let diff = belisarius_scan::diff::compute(&args.path, &args.base, &args.head)
        .with_context(|| format!("computing diff for {}", args.path.display()))?;

    if !diff.repo_present {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&diff)?);
        } else {
            println!(
                "no git repository under {} — nothing to diff",
                args.path.display()
            );
        }
        return Ok(());
    }

    // Run the analysis once and reuse it across overlay signals.
    let report = belisarius_scan::analyze(&args.path)
        .with_context(|| format!("analyzing {}", args.path.display()))?;
    let surface = belisarius_scan::surface::extract(&args.path, &report.scan).ok();
    let keep: Vec<String> = report.scan.files.iter().map(|f| f.path.clone()).collect();
    let hotspots_report = belisarius_scan::git_stats::collect(&args.path, 90, Some(&keep))
        .ok()
        .map(|gs| {
            belisarius_scan::git_stats::rank_hotspots(
                &gs,
                &report.file_metrics,
                args.hotspot_window,
            )
        });
    let inline = belisarius_scan::test_map::detect_inline_tests(&args.path, &report.scan);
    let co = belisarius_scan::codeowners::CodeownersFile::load(&args.path);

    let overlay = belisarius_scan::diff::overlay(
        &report,
        &diff,
        surface.as_ref(),
        hotspots_report.as_ref(),
        &inline,
        co.as_ref(),
    );

    if args.json {
        let payload = serde_json::json!({
            "diff": diff,
            "overlay": overlay,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let total_adds: u32 = diff.files.iter().map(|f| f.additions).sum();
    let total_dels: u32 = diff.files.iter().map(|f| f.deletions).sum();
    let base_label = if diff.base.is_empty() {
        "HEAD^"
    } else {
        diff.base.as_str()
    };
    println!(
        "diff {base} → {head}   {n} files · +{adds}/-{dels}",
        base = base_label,
        head = diff.head,
        n = diff.files.len(),
        adds = total_adds,
        dels = total_dels,
    );

    if !overlay.hotspot_overlap.is_empty() {
        println!(
            "\nchanged files that are also hotspots ({}):",
            overlay.hotspot_overlap.len()
        );
        for f in &overlay.hotspot_overlap {
            println!("  hot       {f}");
        }
    }
    if !overlay.untested_changes.is_empty() {
        println!(
            "\nchanged files with no covering test ({}):",
            overlay.untested_changes.len()
        );
        for f in &overlay.untested_changes {
            println!("  untested  {f}");
        }
    }
    if !overlay.surface_changes.is_empty() {
        println!(
            "\nchanged files that expose public surface ({}):",
            overlay.surface_changes.len()
        );
        for f in &overlay.surface_changes {
            println!("  surface   {f}");
        }
    }
    if !overlay.owners.is_empty() {
        println!("\ncodeowners touched: {}", overlay.owners.join(" "));
    }

    println!("\nall changed files:");
    println!("{:<10}  {:>5}  {:>5}  file", "status", "+", "-");
    for f in &diff.files {
        println!(
            "{:<10}  {:>5}  {:>5}  {}",
            status_label(f.status),
            f.additions,
            f.deletions,
            f.path,
        );
    }
    Ok(())
}

fn status_label(s: belisarius_scan::diff::DiffStatus) -> &'static str {
    use belisarius_scan::diff::DiffStatus;
    match s {
        DiffStatus::Added => "added",
        DiffStatus::Modified => "modified",
        DiffStatus::Deleted => "deleted",
        DiffStatus::Renamed => "renamed",
        DiffStatus::Other => "other",
    }
}
