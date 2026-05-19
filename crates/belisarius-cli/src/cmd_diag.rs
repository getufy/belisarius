use anyhow::{Context, Result};
use belisarius_core::Severity;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct DiagArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Restrict to a comma-separated list of tools (e.g. `clippy,semgrep`).
    #[arg(long)]
    pub tool: Option<String>,
    /// Filter by minimum severity: `error`, `warning`, `info`, `hint`.
    #[arg(long)]
    pub severity: Option<String>,
    /// Emit the full report as JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: DiagArgs) -> Result<()> {
    let scan = belisarius_scan::scan(&args.path)
        .with_context(|| format!("scanning {}", args.path.display()))?;
    let only: Option<Vec<String>> = args
        .tool
        .as_ref()
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect());
    let report = belisarius_scan::diagnostics::run_all(&args.path, &scan, only.as_deref())
        .with_context(|| "running diagnostics")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let min = args
        .severity
        .as_deref()
        .map(|s| match s.to_lowercase().as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            _ => Severity::Hint,
        });

    println!("tool       installed  applied  count   ms");
    for s in &report.tools_ran {
        let installed = if s.installed { "yes" } else { "no" };
        let applied = if s.applied { "yes" } else { "no" };
        println!(
            "  {:<8}  {:>9}  {:>7}  {:>5}  {:>4}",
            s.name, installed, applied, s.count, s.elapsed_ms
        );
        if let Some(err) = &s.error {
            println!("    ⚠ {}", err.lines().next().unwrap_or(err));
        }
    }

    println!();
    println!("by severity:");
    for (sev, n) in &report.counts_by_severity {
        println!("  {:<8}  {}", sev, n);
    }

    let filtered: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| match min {
            Some(Severity::Error) => matches!(d.severity, Severity::Error),
            Some(Severity::Warning) => {
                matches!(d.severity, Severity::Error | Severity::Warning)
            }
            Some(Severity::Info) => !matches!(d.severity, Severity::Hint),
            Some(Severity::Hint) | None => true,
        })
        .collect();

    println!("\ntop {} issues:", filtered.len().min(20));
    for d in filtered.iter().take(20) {
        let sev = match d.severity {
            Severity::Error => "ERR",
            Severity::Warning => "WRN",
            Severity::Info => "INF",
            Severity::Hint => "HNT",
        };
        let msg = d.message.lines().next().unwrap_or(&d.message);
        let msg = if msg.len() > 90 { &msg[..89] } else { msg };
        println!(
            "  {sev} {:<10} {:<32} {}:{}",
            d.tool,
            shorten(&d.rule_id, 32),
            d.file,
            d.start_line,
        );
        println!("      {msg}");
    }
    Ok(())
}

fn shorten(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (n - 1)..])
    }
}
