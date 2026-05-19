//! `ruff check --output-format=json` wrapper.

use anyhow::{Context, Result};
use belisarius_core::{Diagnostic, Scan, Severity};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

use super::{normalize_path, Tool};

pub struct RuffTool;

impl Tool for RuffTool {
    fn name(&self) -> &'static str {
        "ruff"
    }
    fn binary(&self) -> Option<&'static str> {
        Some("ruff")
    }
    fn applies_to(&self, scan: &Scan) -> bool {
        scan.files.iter().any(|f| f.language == "python")
    }
    fn run(&self, project: &Path, _scan: &Scan) -> Result<Vec<Diagnostic>> {
        let output = Command::new("ruff")
            .arg("check")
            .arg("--output-format=json")
            .arg("--no-cache")
            .arg(".")
            .current_dir(project)
            .output()
            .with_context(|| "spawning ruff")?;
        // Ruff exits 1 when issues are found; that's a successful run.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let hits: Vec<RuffHit> = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(hits
            .into_iter()
            .map(|h| Diagnostic {
                tool: "ruff".into(),
                rule_id: h.code.unwrap_or_else(|| "ruff".into()),
                severity: Severity::Warning,
                file: normalize_path(project, &h.filename),
                start_line: h.location.row,
                end_line: h.end_location.row,
                start_col: h.location.column,
                end_col: h.end_location.column,
                message: h.message,
                help: h.fix.and_then(|f| f.message),
                url: h.url,
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct RuffHit {
    #[serde(default)]
    code: Option<String>,
    filename: String,
    message: String,
    location: RuffPos,
    end_location: RuffPos,
    #[serde(default)]
    fix: Option<RuffFix>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct RuffPos {
    row: u32,
    column: u32,
}

#[derive(Deserialize)]
struct RuffFix {
    #[serde(default)]
    message: Option<String>,
}
