//! External code-quality tool wrappers.
//!
//! Each tool is a subprocess (or in-process crate, in Tokei's case) that emits
//! a uniform `Diagnostic` shape. Missing tools soft-skip with `installed:
//! false` in their `ToolStatus` — they don't fail the report.

use anyhow::Result;
use belisarius_core::{Diagnostic, DiagnosticsReport, Scan, Severity, ToolStatus};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub mod cargo_modules;
pub mod clippy;
pub mod depcruise;
pub mod eslint;
pub mod ruff;
pub mod semgrep;
pub mod tokei;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    /// Name of the binary on PATH, when the tool is a subprocess.
    /// Returns `None` for in-process tools (Tokei).
    fn binary(&self) -> Option<&'static str>;
    fn is_installed(&self) -> bool {
        match self.binary() {
            None => true, // in-process, always available
            Some(b) => probe_binary(b),
        }
    }
    /// Does the project look like one this tool can analyze? Soft check.
    fn applies_to(&self, scan: &Scan) -> bool;
    fn run(&self, project: &Path, scan: &Scan) -> Result<Vec<Diagnostic>>;
}

pub fn registry() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(tokei::TokeiTool),
        Box::new(semgrep::SemgrepTool),
        Box::new(clippy::ClippyTool),
        Box::new(ruff::RuffTool),
        Box::new(eslint::EslintTool),
        Box::new(cargo_modules::CargoModulesTool),
        Box::new(depcruise::DependencyCruiserTool),
    ]
}

pub fn by_name(name: &str) -> Option<Box<dyn Tool>> {
    registry().into_iter().find(|t| t.name() == name)
}

/// Run every installed + applicable tool sequentially. Sequential rather than
/// parallel keeps stderr cleaner during interactive runs and avoids stampeding
/// the same project root with concurrent IO. Callers that want concurrency can
/// wrap each `tool.run()` themselves.
pub fn run_all(project: &Path, scan: &Scan, only: Option<&[String]>) -> Result<DiagnosticsReport> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut tools_ran: Vec<ToolStatus> = Vec::new();
    for tool in registry() {
        if let Some(filter) = only {
            if !filter.iter().any(|n| n == tool.name()) {
                continue;
            }
        }
        let installed = tool.is_installed();
        let applied = installed && tool.applies_to(scan);
        let mut status = ToolStatus {
            name: tool.name().to_string(),
            installed,
            applied,
            elapsed_ms: 0,
            count: 0,
            error: None,
        };
        if applied {
            let started = Instant::now();
            match tool.run(project, scan) {
                Ok(found) => {
                    status.count = found.len() as u32;
                    diagnostics.extend(found);
                }
                Err(e) => {
                    status.error = Some(format!("{e:#}"));
                }
            }
            status.elapsed_ms = started.elapsed().as_millis() as u64;
        }
        tools_ran.push(status);
    }
    Ok(build_report(diagnostics, tools_ran))
}

fn build_report(diagnostics: Vec<Diagnostic>, tools_ran: Vec<ToolStatus>) -> DiagnosticsReport {
    let mut counts_by_tool: BTreeMap<String, u32> = BTreeMap::new();
    let mut counts_by_severity: BTreeMap<String, u32> = BTreeMap::new();
    for d in &diagnostics {
        *counts_by_tool.entry(d.tool.clone()).or_default() += 1;
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        };
        *counts_by_severity.entry(sev.to_string()).or_default() += 1;
    }
    DiagnosticsReport {
        tools_ran,
        diagnostics,
        counts_by_tool,
        counts_by_severity,
    }
}

pub(crate) fn probe_binary(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Normalize a path produced by a tool (which may be absolute or relative to a
/// subproject root) to be relative to the scan root, with forward slashes.
pub(crate) fn normalize_path(project: &Path, raw: &str) -> String {
    let p = std::path::Path::new(raw);
    let stripped = if p.is_absolute() {
        p.strip_prefix(project).unwrap_or(p)
    } else {
        p
    };
    stripped.to_string_lossy().replace('\\', "/")
}
