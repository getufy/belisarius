//! `dependency-cruiser` wrapper. Runs once per directory with a `package.json`.
//! Surface rule violations as Diagnostics. When no rules config exists we still
//! parse the graph for cycle detection.

use anyhow::Result;
use belisarius_core::{Diagnostic, Scan, Severity};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{normalize_path, Tool};

pub struct DependencyCruiserTool;

impl Tool for DependencyCruiserTool {
    fn name(&self) -> &'static str {
        "dependency-cruiser"
    }
    fn binary(&self) -> Option<&'static str> {
        Some("npx")
    }
    fn applies_to(&self, scan: &Scan) -> bool {
        scan.files.iter().any(|f| f.path.ends_with("package.json"))
    }
    fn run(&self, project: &Path, scan: &Scan) -> Result<Vec<Diagnostic>> {
        // Each directory that hosts a package.json gets its own run so the
        // tool finds its config file in the expected place.
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
            let cmd = Command::new("npx")
                .args(["--no-install", "depcruise", "--output-type", "json", "src"])
                .current_dir(&dir)
                .output();
            let Ok(o) = cmd else { continue };
            let stdout = String::from_utf8_lossy(&o.stdout);
            let parsed: DepReport = match serde_json::from_str(&stdout) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Two categories: explicit rule violations + circular dependencies.
            for module in &parsed.modules {
                for dep in &module.dependencies {
                    for rule in &dep.rules {
                        let severity = map_severity(&rule.severity);
                        out.push(Diagnostic {
                            tool: "dependency-cruiser".into(),
                            rule_id: rule.name.clone(),
                            severity,
                            file: normalize_path(project, &module.source),
                            start_line: 1,
                            end_line: 1,
                            start_col: 0,
                            end_col: 0,
                            message: format!(
                                "imports {} which violates rule '{}'",
                                dep.resolved, rule.name
                            ),
                            help: None,
                            url: None,
                        });
                    }
                    if let Some(cyc) = &dep.circular {
                        out.push(Diagnostic {
                            tool: "dependency-cruiser".into(),
                            rule_id: "cycle".into(),
                            severity: Severity::Warning,
                            file: normalize_path(project, &module.source),
                            start_line: 1,
                            end_line: 1,
                            start_col: 0,
                            end_col: 0,
                            message: format!("cycle: {}", cyc.join(" → ")),
                            help: None,
                            url: None,
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}

fn map_severity(s: &str) -> Severity {
    match s {
        "error" => Severity::Error,
        "warn" | "warning" => Severity::Warning,
        "info" => Severity::Info,
        _ => Severity::Hint,
    }
}

#[derive(Deserialize, Default)]
struct DepReport {
    #[serde(default)]
    modules: Vec<DepModule>,
}

#[derive(Deserialize)]
struct DepModule {
    source: String,
    #[serde(default)]
    dependencies: Vec<DepEdge>,
}

#[derive(Deserialize)]
struct DepEdge {
    resolved: String,
    #[serde(default)]
    rules: Vec<DepRule>,
    #[serde(default)]
    circular: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DepRule {
    name: String,
    severity: String,
}
