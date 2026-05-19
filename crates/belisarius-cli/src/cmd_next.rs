//! `belisarius next` — recommend the single most useful next action based on
//! the current project state.
//!
//! Decision tree (first match wins):
//!   1. `.belisarius/` missing                  → `belisarius init .`
//!   2. no rules.toml                           → `belisarius rules init`
//!   3. no merged SCIP index                    → `belisarius index .`
//!   4. no search index                         → `belisarius search index .`
//!   5. no AGENTS.md and no MCP wiring          → `belisarius mcp install`
//!   6. everything in place                     → `belisarius brief .`
//!
//! Output is single-line by default. With `--json` we emit the structured
//! recommendation plus the list of considered checks so agents can introspect
//! why a step was suggested.

use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct NextArgs {
    /// Project root. Defaults to current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Emit a JSON recommendation instead of a human-readable line.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
pub struct NextReport {
    pub project: String,
    pub command: String,
    pub reason: &'static str,
    pub checks: Vec<Check>,
}

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
}

impl crate::output::Renderable for NextReport {
    fn render_human(&self, w: &mut dyn std::io::Write) -> std::io::Result<()> {
        writeln!(w, "next: {}", self.command)?;
        writeln!(w, "  why: {}", self.reason)
    }
}

pub async fn run(args: NextArgs) -> Result<()> {
    let project = std::fs::canonicalize(&args.path).unwrap_or_else(|_| args.path.clone());
    let report = recommend(&project);
    crate::output::emit(&report, args.json)?;
    Ok(())
}

/// Pure, side-effect-free recommendation. Exposed so `init` / `doctor` can
/// print the same nudge without re-deriving it.
pub fn recommend(project: &Path) -> NextReport {
    let bel_dir = project.join(".belisarius");
    let bel_dir_exists = bel_dir.is_dir();
    let rules = bel_dir.join("rules.toml").is_file();
    let merged_scip = bel_dir.join("scip").join("merged.scip").is_file();
    let any_scip = bel_dir.join("scip").is_dir()
        && std::fs::read_dir(bel_dir.join("scip"))
            .ok()
            .map(|it| {
                it.flatten()
                    .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("scip"))
            })
            .unwrap_or(false);
    let search_index = bel_dir.join("search").is_dir()
        || bel_dir.join("search-index").is_dir()
        || bel_dir.join("hybrid").is_dir();
    let agents_md = project.join("AGENTS.md").is_file();

    let checks = vec![
        Check {
            name: ".belisarius dir",
            ok: bel_dir_exists,
        },
        Check {
            name: "rules.toml",
            ok: rules,
        },
        Check {
            name: "scip index",
            ok: merged_scip || any_scip,
        },
        Check {
            name: "search index",
            ok: search_index,
        },
        Check {
            name: "AGENTS.md",
            ok: agents_md,
        },
    ];

    let project_disp = project.display().to_string();
    let (command, reason) = if !bel_dir_exists {
        (
            format!("belisarius init {project_disp}"),
            "no .belisarius/ found — bootstrap first",
        )
    } else if !rules {
        (
            "belisarius rules init".to_string(),
            "no rules.toml — declare architectural constraints",
        )
    } else if !(merged_scip || any_scip) {
        (
            format!("belisarius index {project_disp}"),
            "no SCIP index — needed for symbol / call graph queries",
        )
    } else if !search_index {
        (
            format!("belisarius search index {project_disp}"),
            "no hybrid search index — needed for code search",
        )
    } else if !agents_md {
        (
            "belisarius agents init".to_string(),
            "no AGENTS.md — drop a guide so agents know how to drive this repo",
        )
    } else {
        (
            format!("belisarius brief {project_disp}"),
            "everything is in place — start with the brief",
        )
    };

    NextReport {
        project: project_disp,
        command,
        reason,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_dir_recommends_init() {
        let tmp = tempfile::tempdir().unwrap();
        let r = recommend(tmp.path());
        assert!(r.command.starts_with("belisarius init"));
        assert_eq!(r.reason, "no .belisarius/ found — bootstrap first");
    }

    #[test]
    fn init_done_no_rules_recommends_rules_init() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".belisarius")).unwrap();
        let r = recommend(tmp.path());
        assert!(r.command.contains("rules init"));
    }
}
