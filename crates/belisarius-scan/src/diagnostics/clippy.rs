//! `cargo clippy --message-format=json` wrapper.

use anyhow::{Context, Result};
use belisarius_core::{Diagnostic, Scan, Severity};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

use super::{normalize_path, Tool};

pub struct ClippyTool;

impl Tool for ClippyTool {
    fn name(&self) -> &'static str {
        "clippy"
    }
    fn binary(&self) -> Option<&'static str> {
        Some("cargo")
    }
    fn is_installed(&self) -> bool {
        super::probe_binary("cargo")
    }
    fn applies_to(&self, scan: &Scan) -> bool {
        // Any Cargo.toml in the scan means we have something to lint.
        scan.files
            .iter()
            .any(|f| f.path.ends_with("Cargo.toml") || f.path == "Cargo.toml")
    }
    fn run(&self, project: &Path, _scan: &Scan) -> Result<Vec<Diagnostic>> {
        let mut cmd = Command::new("cargo");
        cmd.arg("clippy")
            .arg("--workspace")
            .arg("--message-format=json")
            .arg("--quiet")
            .arg("--all-targets")
            .arg("--no-deps")
            .current_dir(project);
        let output = cmd.output().with_context(|| "spawning cargo clippy")?;

        // Clippy writes diagnostics as line-delimited JSON to stdout, even when
        // it exits non-zero. We parse stdout regardless of exit code.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut out = Vec::new();
        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
                continue;
            };
            if msg.reason != "compiler-message" {
                continue;
            }
            let Some(m) = msg.message else { continue };
            // We only want lint diagnostics — skip pure errors like
            // "couldn't read Cargo.toml" (which lack spans).
            if m.spans.is_empty() {
                continue;
            }
            let severity = match m.level.as_str() {
                "error" | "error: internal compiler error" => Severity::Error,
                "warning" => Severity::Warning,
                "note" => Severity::Info,
                _ => Severity::Hint,
            };
            let span = match m.spans.iter().find(|s| s.is_primary) {
                Some(s) => s,
                None => &m.spans[0],
            };
            out.push(Diagnostic {
                tool: "clippy".into(),
                rule_id: m
                    .code
                    .as_ref()
                    .map(|c| c.code.clone())
                    .unwrap_or_else(|| "rustc".to_string()),
                severity,
                file: normalize_path(project, &span.file_name),
                start_line: span.line_start,
                end_line: span.line_end,
                start_col: span.column_start,
                end_col: span.column_end,
                message: m.message.clone(),
                help: m
                    .children
                    .iter()
                    .find(|c| c.level == "help")
                    .map(|c| c.message.clone()),
                url: m.code.as_ref().and_then(|c| c.explanation.clone()),
            });
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    #[serde(default)]
    message: Option<ClippyDiag>,
}

#[derive(Deserialize)]
struct ClippyDiag {
    message: String,
    level: String,
    #[serde(default)]
    code: Option<ClippyCode>,
    #[serde(default)]
    spans: Vec<ClippySpan>,
    #[serde(default)]
    children: Vec<ClippyChild>,
}

#[derive(Deserialize)]
struct ClippyCode {
    code: String,
    #[serde(default)]
    explanation: Option<String>,
}

#[derive(Deserialize)]
struct ClippySpan {
    file_name: String,
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
    #[serde(default)]
    is_primary: bool,
}

#[derive(Deserialize)]
struct ClippyChild {
    message: String,
    level: String,
}
