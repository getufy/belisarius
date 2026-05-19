use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct CommandsArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Emit the full report as JSON instead of the human table.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: CommandsArgs) -> Result<()> {
    let report = belisarius_scan::commands::discover(&args.path)
        .with_context(|| format!("discovering commands under {}", args.path.display()))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let s = &report.suggested;
    println!("Suggested:");
    println!(
        "  run    {}",
        s.run.as_deref().unwrap_or("— (no dev/run script found)")
    );
    println!(
        "  build  {}",
        s.build.as_deref().unwrap_or("— (no build script found)")
    );
    println!(
        "  test   {}",
        s.test.as_deref().unwrap_or("— (no test script found)")
    );
    if let Some(l) = &s.lint {
        println!("  lint   {l}");
    }
    if let Some(f) = &s.format {
        println!("  format {f}");
    }

    let sections: Vec<(&str, &[belisarius_scan::commands::NamedCommand])> = vec![
        ("package.json scripts", &report.package_scripts),
        ("Cargo", &report.cargo),
        ("Justfile", &report.just),
        ("Makefile", &report.make),
        ("Python", &report.python),
        (".github/workflows", &report.workflows),
    ];
    for (label, rows) in sections {
        if rows.is_empty() {
            continue;
        }
        println!("\n{label}:");
        println!(
            "{:<22}  {:<10}  {:<36}  source",
            "name", "purpose", "command"
        );
        for r in rows {
            println!(
                "{:<22}  {:<10}  {:<36}  {}",
                trim(&r.name, 22),
                r.purpose,
                trim(&r.command, 36),
                r.source,
            );
        }
    }
    Ok(())
}

fn trim(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}
