//! `semgrep --config auto --json` wrapper.

use anyhow::{Context, Result};
use belisarius_core::{Diagnostic, Scan, Severity};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

use super::{normalize_path, Tool};

pub struct SemgrepTool;

impl Tool for SemgrepTool {
    fn name(&self) -> &'static str {
        "semgrep"
    }
    fn binary(&self) -> Option<&'static str> {
        Some("semgrep")
    }
    fn applies_to(&self, _scan: &Scan) -> bool {
        true
    }
    fn run(&self, project: &Path, _scan: &Scan) -> Result<Vec<Diagnostic>> {
        let output = Command::new("semgrep")
            .arg("scan")
            .arg("--config")
            .arg("auto")
            .arg("--json")
            .arg("--no-git-ignore")
            .arg("--quiet")
            .arg("--metrics=off")
            .arg("--timeout")
            .arg("60")
            .arg(".")
            .current_dir(project)
            .output()
            .with_context(|| "spawning semgrep")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: SemgrepReport = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(parsed
            .results
            .into_iter()
            .map(|r| Diagnostic {
                tool: "semgrep".into(),
                rule_id: r.check_id,
                severity: map_severity(&r.extra.severity),
                file: normalize_path(project, &r.path),
                start_line: r.start.line,
                end_line: r.end.line,
                start_col: r.start.col,
                end_col: r.end.col,
                message: r.extra.message,
                help: r.extra.fix,
                url: r.extra.metadata.and_then(|m| m.source),
            })
            .collect())
    }
}

fn map_severity(s: &str) -> Severity {
    match s {
        "ERROR" => Severity::Error,
        "WARNING" => Severity::Warning,
        "INFO" => Severity::Info,
        _ => Severity::Hint,
    }
}

#[derive(Deserialize)]
struct SemgrepReport {
    #[serde(default)]
    results: Vec<SemgrepHit>,
}

#[derive(Deserialize)]
struct SemgrepHit {
    check_id: String,
    path: String,
    start: SemgrepPos,
    end: SemgrepPos,
    extra: SemgrepExtra,
}

#[derive(Deserialize)]
struct SemgrepPos {
    line: u32,
    col: u32,
}

#[derive(Deserialize)]
struct SemgrepExtra {
    message: String,
    severity: String,
    #[serde(default)]
    fix: Option<String>,
    #[serde(default)]
    metadata: Option<SemgrepMetadata>,
}

#[derive(Deserialize)]
struct SemgrepMetadata {
    #[serde(default)]
    source: Option<String>,
}
