//! `belisarius check` — CI-friendly rules evaluation.
//!
//! Runs `.belisarius/rules.toml` against the project, prints a human or
//! JSON summary, and exits non-zero when any violation is found. The JSON
//! shape is byte-identical to the `belisarius_rules_check` MCP tool so a
//! CI job can read the same output a Claude Code agent would see.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use crate::service::context::AppContext;
use crate::service::project::{rules_check, PathArgs};

#[derive(clap::Args)]
pub struct CheckArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Emit JSON (same shape as the `belisarius_rules_check` MCP tool).
    #[arg(long)]
    pub json: bool,
    /// Print violations but exit zero. Use to preview rules without
    /// blocking a CI step.
    #[arg(long)]
    pub no_fail: bool,
}

pub async fn run(args: CheckArgs) -> Result<()> {
    let ctx = Arc::new(AppContext::new());
    let path = args.path.to_string_lossy().into_owned();
    let report = rules_check(&ctx, PathArgs { path })
        .await
        .map_err(|e| anyhow::anyhow!("rules check failed: {e}"))?;
    let violations = report
        .get("violations")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serializing rules report")?
        );
    } else {
        print_human(&report, violations);
    }

    if violations > 0 && !args.no_fail {
        std::process::exit(1);
    }
    Ok(())
}

fn print_human(report: &serde_json::Value, violations: usize) {
    let rules_present = report
        .get("rules_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !rules_present {
        println!("rules: no .belisarius/rules.toml — nothing to check");
        return;
    }
    let rules_path = report
        .get("rules_path")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    println!("rules: {rules_path}");

    if violations == 0 {
        println!("ok: no violations");
        return;
    }

    println!("fail: {violations} violation(s)");
    if let Some(arr) = report.get("violations").and_then(|v| v.as_array()) {
        for v in arr {
            let rule = v.get("rule").and_then(|x| x.as_str()).unwrap_or("?");
            let summary = v.get("summary").and_then(|x| x.as_str()).unwrap_or("");
            let file = v.get("file").and_then(|x| x.as_str()).unwrap_or("");
            let line = v.get("line").and_then(|x| x.as_u64()).unwrap_or(0);
            if !file.is_empty() {
                println!("  [{rule}] {file}:{line} — {summary}");
            } else {
                println!("  [{rule}] {summary}");
            }
        }
    }
    if let Some(counts) = report.get("counts_by_rule").and_then(|v| v.as_object()) {
        if !counts.is_empty() {
            println!();
            println!("by rule:");
            for (k, v) in counts {
                let n = v.as_u64().unwrap_or(0);
                println!("  {k:24} {n:>4}");
            }
        }
    }
}
