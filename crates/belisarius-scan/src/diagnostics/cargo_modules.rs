//! `cargo modules structure` wrapper. Per Cargo.toml in the scan, capture the
//! module tree as text and surface it as informational Diagnostics — one per
//! crate, the message body is the tree.
//!
//! cargo-modules's stable output is human-readable text. We capture it
//! verbatim instead of parsing — the UI shows it in a `<pre>` block.

use anyhow::Result;
use belisarius_core::{Diagnostic, Scan, Severity};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Tool;

pub struct CargoModulesTool;

impl Tool for CargoModulesTool {
    fn name(&self) -> &'static str {
        "cargo-modules"
    }
    fn binary(&self) -> Option<&'static str> {
        Some("cargo")
    }
    fn is_installed(&self) -> bool {
        // `cargo modules --version` returns 0 when the subcommand exists.
        Command::new("cargo")
            .args(["modules", "--version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    fn applies_to(&self, scan: &Scan) -> bool {
        scan.files
            .iter()
            .any(|f| f.path.ends_with("Cargo.toml") || f.path == "Cargo.toml")
    }
    fn run(&self, project: &Path, scan: &Scan) -> Result<Vec<Diagnostic>> {
        // Collect every crate manifest in the scan.
        let manifests: Vec<PathBuf> = scan
            .files
            .iter()
            .filter(|f| f.path.ends_with("Cargo.toml") && !f.path.contains("target/"))
            .map(|f| project.join(&f.path))
            .collect();

        let mut out = Vec::new();
        for manifest in manifests {
            let pkg_name = read_package_name(&manifest);
            // Skip virtual workspaces (no [package] section).
            let Some(name) = pkg_name else { continue };
            let output = Command::new("cargo")
                .args(["modules", "structure", "--package", &name, "--no-fns"])
                .current_dir(project)
                .output();
            let Ok(o) = output else { continue };
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            if stdout.trim().is_empty() {
                continue;
            }
            let rel = manifest
                .strip_prefix(project)
                .unwrap_or(&manifest)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(Diagnostic {
                tool: "cargo-modules".into(),
                rule_id: format!("module-tree:{name}"),
                severity: Severity::Info,
                file: rel,
                start_line: 1,
                end_line: 1,
                start_col: 0,
                end_col: 0,
                message: stdout,
                help: None,
                url: None,
            });
        }
        Ok(out)
    }
}

fn read_package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    let pkg = value.get("package")?;
    pkg.get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
