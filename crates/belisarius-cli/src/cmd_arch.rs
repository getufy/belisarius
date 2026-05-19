//! Helpers behind the `/api/architecture/*` and `/api/components` endpoints.
//!
//! Pure helpers — the server.rs handlers call into here.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `react-docgen` JSON output for every component we find in the project.
pub fn run_react_docgen(project: &Path) -> Result<Vec<ComponentDoc>> {
    // We invoke through npx so the locally installed version is picked up.
    // Skip silently if there are no .tsx files (saves a process spawn).
    let component_files = collect_component_files(project)?;
    if component_files.is_empty() {
        return Ok(Vec::new());
    }

    // react-docgen accepts a list of paths; pass them via stdin to avoid arg
    // length limits on huge projects.
    let mut cmd = Command::new("npx");
    cmd.args(["--no-install", "react-docgen", "--pretty", "false"])
        .args(&component_files)
        .current_dir(project);
    let output = cmd.output().with_context(|| "spawning npx react-docgen")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    // react-docgen returns `{ "file/path.tsx": [Component, …], … }` since v7.
    let map = match parsed.as_object() {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for (file, arr) in map {
        let Some(components) = arr.as_array() else {
            continue;
        };
        for comp in components {
            let name = comp
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("Anonymous")
                .to_string();
            let description = comp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let props_raw = comp.get("props").and_then(|v| v.as_object());
            let mut props = Vec::new();
            if let Some(map) = props_raw {
                for (pname, pdef) in map {
                    let ty = pdef
                        .get("tsType")
                        .or_else(|| pdef.get("flowType"))
                        .or_else(|| pdef.get("type"))
                        .and_then(|t| t.get("name").or_else(|| t.get("raw")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let required = pdef
                        .get("required")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let desc = pdef
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let default = pdef
                        .get("defaultValue")
                        .and_then(|v| v.get("value"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    props.push(ComponentProp {
                        name: pname.clone(),
                        ty,
                        required,
                        description: desc,
                        default,
                    });
                }
            }
            props.sort_by(|a, b| b.required.cmp(&a.required).then(a.name.cmp(&b.name)));
            out.push(ComponentDoc {
                file: file.clone(),
                name,
                description,
                props,
            });
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn collect_component_files(project: &Path) -> Result<Vec<String>> {
    use ignore::WalkBuilder;
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for entry in WalkBuilder::new(project)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .build()
        .flatten()
    {
        if !entry.path().is_file() {
            continue;
        }
        let p = entry.path();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "tsx" && ext != "jsx" {
            continue;
        }
        // Skip generated / built / vendored.
        let rel = p.strip_prefix(project).unwrap_or(p);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.starts_with("dist/")
            || rel_str.starts_with("build/")
            || rel_str.contains("/node_modules/")
        {
            continue;
        }
        if seen.insert(p.to_path_buf()) {
            out.push(rel_str);
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComponentDoc {
    pub file: String,
    pub name: String,
    pub description: String,
    pub props: Vec<ComponentProp>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComponentProp {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub required: bool,
    pub description: String,
    pub default: Option<String>,
}
