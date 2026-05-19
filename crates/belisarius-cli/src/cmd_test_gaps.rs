use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct TestGapsArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Print at most this many gap rows.
    #[arg(long, default_value_t = 25)]
    pub limit: usize,
    /// Emit the full report as JSON (mappings + gaps + summary).
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: TestGapsArgs) -> Result<()> {
    let report = belisarius_scan::analyze(&args.path)
        .with_context(|| format!("analyzing {}", args.path.display()))?;
    let inline = belisarius_scan::test_map::detect_inline_tests(&args.path, &report.scan);
    let map = belisarius_scan::test_map::compute(&report, &inline);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&map)?);
        return Ok(());
    }

    let s = &map.summary;
    println!(
        "test coverage (import-based): {}/{} source files covered ({:.1}%) · {} tests · {} gaps",
        s.covered_files, s.source_files, s.coverage_pct, s.test_files, s.gap_files
    );
    if map.gaps.is_empty() {
        println!("no untested source files — every file is reached by at least one test.");
        return Ok(());
    }
    println!(
        "\ntop {} untested files, ranked by total cyclomatic complexity:",
        args.limit.min(map.gaps.len())
    );
    println!(
        "{:>5}  {:>5}  {:>5}  {:<12}  file",
        "loc", "fns", "cc", "lang"
    );
    for g in map.gaps.iter().take(args.limit) {
        println!(
            "{:>5}  {:>5}  {:>5}  {:<12}  {}",
            g.loc, g.function_count, g.total_cyclomatic, g.language, g.source
        );
    }
    Ok(())
}
