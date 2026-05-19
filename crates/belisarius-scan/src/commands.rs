//! Workspace command discovery — surface "how do I run / build / test this?"
//! in one place.
//!
//! Looks at the conventional locations where teams record runnable
//! commands and stitches the findings into a single typed report:
//!
//!   * `package.json` `scripts` (npm / pnpm / yarn — including workspaces)
//!   * `Justfile` recipes
//!   * `Makefile` targets (excluding `.PHONY`, `.SUFFIXES`, …)
//!   * `Cargo.toml` (inferred standard commands — `cargo build/test/run`,
//!     plus any `[alias]` entries from `.cargo/config.toml`)
//!   * `pyproject.toml` `[project.scripts]` / `[tool.poetry.scripts]` /
//!     `[tool.poe.tasks]`
//!   * `.github/workflows/*.yml` workflow names + triggers
//!
//! Everything is best-effort: missing files yield empty buckets, malformed
//! files are skipped rather than aborting the discovery. The output is
//! tagged with a heuristic `purpose` (`run | build | test | lint | …`) so
//! agents can pick the right command without reading prose.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedCommand {
    pub name: String,
    pub command: String,
    /// The file we read this from, relative to the project root.
    pub source: String,
    /// Coarse classification — `run`, `build`, `test`, `lint`, `format`,
    /// `release`, `deploy`, `ci`, or `other` when nothing matches.
    pub purpose: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceCommands {
    pub package_scripts: Vec<NamedCommand>,
    pub cargo: Vec<NamedCommand>,
    pub just: Vec<NamedCommand>,
    pub make: Vec<NamedCommand>,
    pub python: Vec<NamedCommand>,
    pub workflows: Vec<NamedCommand>,
    /// Quick-answer pickers — best guesses for the canonical run/build/test
    /// commands across the entire project. Empty when we can't tell.
    pub suggested: Suggested,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Suggested {
    pub run: Option<String>,
    pub build: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
    pub format: Option<String>,
}

pub fn discover(project_root: &Path) -> Result<WorkspaceCommands> {
    let mut out = WorkspaceCommands::default();

    collect_package_jsons(project_root, &mut out.package_scripts);
    collect_cargo(project_root, &mut out.cargo);
    collect_just(project_root, &mut out.just);
    collect_make(project_root, &mut out.make);
    collect_python(project_root, &mut out.python);
    collect_workflows(project_root, &mut out.workflows);

    out.suggested = pick_suggested(&out);
    Ok(out)
}

fn collect_package_jsons(root: &Path, out: &mut Vec<NamedCommand>) {
    // Find every package.json in the tree, capped to a sensible depth so we
    // don't traverse node_modules. The walker already gitignores; we
    // additionally short-circuit on `node_modules` segments to be safe.
    let walker = ignore::WalkBuilder::new(root)
        .max_depth(Some(6))
        .standard_filters(true)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|s| s.to_str()) != Some("package.json") {
            continue;
        }
        if path.components().any(|c| c.as_os_str() == "node_modules") {
            continue;
        }
        let rel = relativize(root, path);
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let scripts = v.get("scripts").and_then(|s| s.as_object());
        let Some(scripts) = scripts else { continue };
        // Prefer `pnpm run X` when a pnpm-workspace.yaml exists at root,
        // otherwise default to `npm run X`. Yarn detection is rarer and
        // visually equivalent, so we don't try to distinguish.
        let runner = pick_runner(root, &rel);
        for (name, cmd_val) in scripts {
            let Some(cmd) = cmd_val.as_str() else {
                continue;
            };
            out.push(NamedCommand {
                name: name.clone(),
                command: format!("{runner} run {name}"),
                source: rel.clone(),
                purpose: classify(name, cmd),
            });
        }
    }
}

fn pick_runner(root: &Path, package_json_rel: &str) -> &'static str {
    // pnpm if pnpm-workspace.yaml exists at root, otherwise npm.
    let pnpm_marker = root.join("pnpm-workspace.yaml");
    let _ = package_json_rel;
    if pnpm_marker.exists() {
        "pnpm"
    } else {
        "npm"
    }
}

fn collect_cargo(root: &Path, out: &mut Vec<NamedCommand>) {
    let cargo_toml = root.join("Cargo.toml");
    if !cargo_toml.exists() {
        return;
    }
    let standards = [
        ("build", "cargo build", "build"),
        ("build --release", "cargo build --release", "build"),
        ("test", "cargo test --workspace", "test"),
        (
            "clippy",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "lint",
        ),
        ("fmt", "cargo fmt --all", "format"),
        ("run", "cargo run", "run"),
    ];
    for (name, cmd, purpose) in standards {
        out.push(NamedCommand {
            name: name.to_string(),
            command: cmd.to_string(),
            source: "Cargo.toml".to_string(),
            purpose: purpose.to_string(),
        });
    }
    // .cargo/config.toml `[alias]` table — user-defined shortcuts.
    let alias_path = root.join(".cargo").join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&alias_path) {
        if let Ok(v) = text.parse::<toml::Value>() {
            if let Some(aliases) = v.get("alias").and_then(|x| x.as_table()) {
                for (name, val) in aliases {
                    let cmd_str = match val {
                        toml::Value::String(s) => s.clone(),
                        toml::Value::Array(parts) => parts
                            .iter()
                            .filter_map(|p| p.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                            .join(" "),
                        _ => continue,
                    };
                    out.push(NamedCommand {
                        name: name.clone(),
                        command: format!("cargo {name}"),
                        source: ".cargo/config.toml".to_string(),
                        purpose: classify(name, &cmd_str),
                    });
                }
            }
        }
    }
}

fn collect_just(root: &Path, out: &mut Vec<NamedCommand>) {
    let path = root.join("Justfile");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        // Skip comments, settings, variables (`name := value`).
        if trimmed.starts_with('#') || trimmed.starts_with("set ") || trimmed.is_empty() {
            continue;
        }
        // A recipe looks like `name [args]:` at the start of a line (no
        // leading indent — those are the recipe body).
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let (head, rest) = match trimmed.split_once(':') {
            Some(x) => x,
            None => continue,
        };
        // `:=` is variable assignment, not a recipe.
        if rest.starts_with('=') {
            continue;
        }
        let name = head.split_whitespace().next().unwrap_or("").to_string();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        out.push(NamedCommand {
            name: name.clone(),
            command: format!("just {name}"),
            source: "Justfile".to_string(),
            purpose: classify(&name, &name),
        });
    }
}

fn collect_make(root: &Path, out: &mut Vec<NamedCommand>) {
    let path = root.join("Makefile");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        if line.starts_with('\t') || line.starts_with(' ') || line.starts_with('#') {
            continue;
        }
        let Some((head, _rest)) = line.split_once(':') else {
            continue;
        };
        let head = head.trim();
        // Skip pattern rules, special targets, and assignments.
        if head.contains('=') || head.starts_with('.') || head.is_empty() {
            continue;
        }
        let name = head.split_whitespace().next().unwrap_or("").to_string();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
        {
            continue;
        }
        out.push(NamedCommand {
            name: name.clone(),
            command: format!("make {name}"),
            source: "Makefile".to_string(),
            purpose: classify(&name, &name),
        });
    }
}

fn collect_python(root: &Path, out: &mut Vec<NamedCommand>) {
    let path = root.join("pyproject.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(v) = text.parse::<toml::Value>() else {
        return;
    };

    // PEP 621 `[project.scripts]` → `pip install` registers entry points.
    if let Some(scripts) = v
        .get("project")
        .and_then(|p| p.get("scripts"))
        .and_then(|s| s.as_table())
    {
        for (name, val) in scripts {
            let cmd = val.as_str().unwrap_or("").to_string();
            out.push(NamedCommand {
                name: name.clone(),
                command: name.clone(),
                source: format!("pyproject.toml [project.scripts] → {cmd}"),
                purpose: classify(name, &cmd),
            });
        }
    }

    // Poetry / Poe both nest under `[tool]`.
    for (table, prefix) in [("poetry", "poetry run"), ("poe", "poe")] {
        if let Some(scripts) = v
            .get("tool")
            .and_then(|t| t.get(table))
            .and_then(|p| {
                // Poe uses `tasks`, Poetry uses `scripts`.
                p.get("scripts").or_else(|| p.get("tasks"))
            })
            .and_then(|s| s.as_table())
        {
            for (name, val) in scripts {
                let cmd = match val {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Table(t) => t
                        .get("cmd")
                        .and_then(|c| c.as_str())
                        .map(String::from)
                        .unwrap_or_default(),
                    _ => continue,
                };
                out.push(NamedCommand {
                    name: name.clone(),
                    command: format!("{prefix} {name}"),
                    source: format!("pyproject.toml [tool.{table}.*]"),
                    purpose: classify(name, &cmd),
                });
            }
        }
    }
}

fn collect_workflows(root: &Path, out: &mut Vec<NamedCommand>) {
    let dir = root.join(".github").join("workflows");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "yml" | "yaml") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Grab the workflow `name:` (first top-level occurrence). We don't
        // pull in a yaml crate just for this — the format is conventional
        // enough that a regex is reliable.
        let workflow_name = text
            .lines()
            .find_map(|l| {
                let t = l.trim_start();
                t.strip_prefix("name:")
                    .map(|rest| rest.trim().trim_matches('"').to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| file_name.clone());
        // Pull a small purpose hint from the filename + workflow name.
        let purpose = classify(&workflow_name, &file_name);
        out.push(NamedCommand {
            name: workflow_name,
            command: format!(".github/workflows/{file_name}"),
            source: format!(".github/workflows/{file_name}"),
            purpose,
        });
    }
}

/// Coarse purpose categoriser. Looks at name + command together so a
/// script named `ship` running `npm publish` still classifies as `release`.
fn classify(name: &str, body: &str) -> String {
    let s = format!("{name} {body}").to_ascii_lowercase();
    let kinds: [(&str, &[&str]); 8] = [
        (
            "test",
            &["test", "spec", "pytest", "vitest", "jest", "cargo test"],
        ),
        (
            "lint",
            &["lint", "clippy", "eslint", "ruff", "flake8", "shellcheck"],
        ),
        ("format", &["fmt", "format", "prettier", "rustfmt", "black"]),
        (
            "build",
            &["build", "compile", "bundle", "tsc", "vite build"],
        ),
        ("dev", &["dev", "watch", "start", "serve"]),
        ("release", &["release", "publish", "ship", "deploy", "tag"]),
        ("ci", &["ci", "workflow"]),
        ("run", &["run", "cli"]),
    ];
    for (kind, needles) in kinds {
        if needles.iter().any(|n| s.contains(n)) {
            return kind.to_string();
        }
    }
    "other".to_string()
}

fn pick_suggested(c: &WorkspaceCommands) -> Suggested {
    let mut s = Suggested::default();
    let all: Vec<&NamedCommand> = c
        .package_scripts
        .iter()
        .chain(c.just.iter())
        .chain(c.make.iter())
        .chain(c.cargo.iter())
        .chain(c.python.iter())
        .collect();
    let first_with_purpose = |purpose: &str| {
        all.iter()
            .find(|cmd| cmd.purpose == purpose)
            .map(|c| c.command.clone())
    };
    s.run = first_with_purpose("dev").or_else(|| first_with_purpose("run"));
    s.build = first_with_purpose("build");
    s.test = first_with_purpose("test");
    s.lint = first_with_purpose("lint");
    s.format = first_with_purpose("format");
    s
}

fn relativize(root: &Path, abs: &Path) -> String {
    let rel: PathBuf = abs
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| abs.to_path_buf());
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn classifier_picks_right_buckets() {
        assert_eq!(classify("test", "vitest run"), "test");
        assert_eq!(classify("dev", "vite"), "dev");
        assert_eq!(classify("build", "tsc && vite build"), "build");
        assert_eq!(classify("clippy-all", "cargo clippy"), "lint");
        assert_eq!(classify("release", "npm publish"), "release");
        assert_eq!(classify("ship", "git tag && npm publish"), "release");
        assert_eq!(classify("nothing", "echo hi"), "other");
    }

    #[test]
    fn discover_finds_package_json_and_cargo() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("package.json"),
            r#"{ "scripts": { "dev": "vite", "build": "vite build", "test": "vitest" } }"#,
        )
        .unwrap();
        fs::write(
            root.join("Justfile"),
            "set shell := [\"bash\"]\n\nbuild:\n\tcargo build\n\ntest *args:\n\tcargo test {{args}}\n",
        )
        .unwrap();
        let cmds = discover(root).unwrap();
        let names: Vec<&str> = cmds
            .package_scripts
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"dev"));
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
        assert!(cmds.cargo.iter().any(|c| c.name == "build"));
        let just_names: Vec<&str> = cmds.just.iter().map(|c| c.name.as_str()).collect();
        assert!(just_names.contains(&"build"));
        assert!(just_names.contains(&"test"));
        assert_eq!(cmds.suggested.test.as_deref(), Some("npm run test"));
        assert!(cmds.suggested.run.is_some());
    }
}
