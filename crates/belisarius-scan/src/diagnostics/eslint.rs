//! `npx eslint --format json` wrapper. Runs once per directory that contains a
//! `package.json` to respect that project's eslint config.

use anyhow::{Context, Result};
use belisarius_core::{Diagnostic, Scan, Severity};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{normalize_path, Tool};

pub struct EslintTool;

impl Tool for EslintTool {
    fn name(&self) -> &'static str {
        "eslint"
    }
    fn binary(&self) -> Option<&'static str> {
        // We invoke through `npx` so the locally installed version is picked
        // up. Probe `npx` for installability.
        Some("npx")
    }
    fn applies_to(&self, scan: &Scan) -> bool {
        scan.files.iter().any(|f| f.path.ends_with("package.json"))
    }
    fn run(&self, project: &Path, scan: &Scan) -> Result<Vec<Diagnostic>> {
        let dirs: HashSet<PathBuf> = scan
            .files
            .iter()
            .filter(|f| f.path.ends_with("package.json"))
            .map(|f| {
                let mut p = project.join(&f.path);
                p.pop();
                p
            })
            .collect();

        let mut out = Vec::new();
        for dir in dirs {
            let output = Command::new("npx")
                .arg("--no-install")
                .arg("eslint")
                .arg("--format")
                .arg("json")
                .arg(".")
                .current_dir(&dir)
                .output()
                .with_context(|| format!("spawning npx eslint in {}", dir.display()))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            // ESLint emits an array of file reports. It exits 1 when issues are
            // found, 2 on config errors. We only consume stdout.
            let parsed: Vec<EslintFile> = match serde_json::from_str(&stdout) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for f in parsed {
                for m in f.messages {
                    let severity = match m.severity {
                        2 => Severity::Error,
                        1 => Severity::Warning,
                        _ => Severity::Info,
                    };
                    out.push(Diagnostic {
                        tool: "eslint".into(),
                        rule_id: m.rule_id.unwrap_or_else(|| "eslint".into()),
                        severity,
                        file: normalize_path(project, &f.file_path),
                        start_line: m.line.unwrap_or(1),
                        end_line: m.end_line.unwrap_or_else(|| m.line.unwrap_or(1)),
                        start_col: m.column.unwrap_or(0),
                        end_col: m.end_column.unwrap_or(0),
                        message: m.message,
                        help: None,
                        url: None,
                    });
                }
            }
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct EslintFile {
    #[serde(rename = "filePath")]
    file_path: String,
    messages: Vec<EslintMessage>,
}

#[derive(Deserialize)]
struct EslintMessage {
    #[serde(rename = "ruleId", default)]
    rule_id: Option<String>,
    severity: u8,
    message: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(rename = "endLine", default)]
    end_line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
    #[serde(rename = "endColumn", default)]
    end_column: Option<u32>,
}
