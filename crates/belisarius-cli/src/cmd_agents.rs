//! `belisarius agents` — write/refresh an `AGENTS.md` at the project root.
//!
//! `AGENTS.md` has become the de-facto agent-instruction file across Codex,
//! Cursor, Claude Code, and similar tools (~30k repos and growing). It's a
//! single, agent-readable contract describing how to navigate the project,
//! what tools are available, what conventions to follow.
//!
//! Belisarius is uniquely positioned to write a *useful* AGENTS.md: it already
//! knows the language mix, the hot files, the test gaps, the cycle count, the
//! quality score, and every MCP tool it exposes. Stitching those into a
//! concise file means a new agent dropped into the repo can start producing
//! code in one turn instead of five.
//!
//! Idempotency: if `AGENTS.md` exists with our marker, we update it in place.
//! If it exists without our marker (user-authored), we leave it alone unless
//! `--force` or `--append` is passed.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand)]
pub enum AgentsCmd {
    /// Generate or refresh `AGENTS.md` tailored to this project.
    Init(InitArgs),
}

#[derive(clap::Args)]
pub struct InitArgs {
    /// Project root. Defaults to current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Output filename. Default: `AGENTS.md`. Pass `CLAUDE.md` for Claude Code
    /// or `.cursorrules` for Cursor — the content is the same; clients just
    /// look in different files.
    #[arg(long, default_value = "AGENTS.md")]
    pub out: String,
    /// Overwrite a user-authored file (one without our marker).
    #[arg(long)]
    pub force: bool,
    /// Print the generated content to stdout instead of writing it.
    #[arg(long)]
    pub stdout: bool,
    /// JSON report on what was written.
    #[arg(long)]
    pub json: bool,
}

/// Marker written into the file so we can safely refresh it later without
/// stomping a hand-written AGENTS.md. Bumped only on schema-breaking changes.
const MARKER: &str = "<!-- belisarius:agents v1 -->";

#[derive(serde::Serialize)]
struct AgentsReport {
    path: String,
    status: String,
    bytes: usize,
}

pub async fn run(cmd: AgentsCmd) -> Result<()> {
    match cmd {
        AgentsCmd::Init(a) => init(a),
    }
}

pub fn init(args: InitArgs) -> Result<()> {
    let project = args
        .path
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", args.path.display()))?;
    let content = render(&project)?;

    if args.stdout {
        print!("{content}");
        return Ok(());
    }

    let out_path = project.join(&args.out);
    let status = if out_path.exists() {
        let existing = std::fs::read_to_string(&out_path)
            .with_context(|| format!("reading {}", out_path.display()))?;
        if existing.contains(MARKER) {
            "updated"
        } else if args.force {
            "overwrote"
        } else {
            let r = AgentsReport {
                path: out_path.to_string_lossy().into_owned(),
                status: "skipped".to_string(),
                bytes: 0,
            };
            return emit(
                r,
                args.json,
                Some("existing file isn't managed by Belisarius; re-run with --force"),
            );
        }
    } else {
        "wrote"
    };
    std::fs::write(&out_path, content.as_bytes())
        .with_context(|| format!("writing {}", out_path.display()))?;
    emit(
        AgentsReport {
            path: out_path.to_string_lossy().into_owned(),
            status: status.to_string(),
            bytes: content.len(),
        },
        args.json,
        None,
    )
}

fn emit(r: AgentsReport, json: bool, note: Option<&str>) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        let mark = match r.status.as_str() {
            "wrote" | "updated" | "overwrote" => "✓",
            "skipped" => "!",
            _ => "·",
        };
        println!("  {mark} AGENTS.md  {} — {}", r.status, r.path);
        if let Some(n) = note {
            println!("      {n}");
        }
        if r.bytes > 0 {
            println!("      {} bytes", r.bytes);
        }
    }
    Ok(())
}

/// Build the AGENTS.md body. Designed to fit comfortably in an agent's
/// context window — under 4 KB of prose plus a short data table.
pub fn render(project: &Path) -> Result<String> {
    let scan = belisarius_scan::scan(project.to_string_lossy().as_ref())
        .with_context(|| format!("scanning {}", project.display()))?;
    let total_loc: u64 = scan.language_summary.values().map(|s| s.loc as u64).sum();
    let mut langs: Vec<(&String, u64)> = scan
        .language_summary
        .iter()
        .map(|(k, s)| (k, s.loc as u64))
        .collect();
    langs.sort_by_key(|x| std::cmp::Reverse(x.1));

    let project_name = project
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(unnamed)".to_string());

    let mut out = String::new();
    writeln!(out, "{MARKER}").ok();
    writeln!(out, "# {project_name} — agent guide\n").ok();
    writeln!(
        out,
        "This file was generated by `belisarius agents init`. Re-run that command to refresh \
        the data lines; everything else is yours to edit.\n"
    )
    .ok();

    writeln!(out, "## Snapshot").ok();
    writeln!(
        out,
        "- {} files · {} LOC · {} languages\n- Primary language: {}",
        scan.files.len(),
        total_loc,
        scan.language_summary.len(),
        langs.first().map(|(k, _)| k.as_str()).unwrap_or("unknown"),
    )
    .ok();
    if langs.len() > 1 {
        let mix: Vec<String> = langs
            .iter()
            .take(5)
            .map(|(k, v)| format!("{k} ({v} LOC)"))
            .collect();
        writeln!(out, "- Mix: {}", mix.join(", ")).ok();
    }
    writeln!(out).ok();

    writeln!(out, "## Working with Belisarius").ok();
    writeln!(
        out,
        "Belisarius indexes this repo (file walk, AST, call graph, hybrid search) and exposes \
        every capability as an MCP tool. Prefer those over ad-hoc `grep` / file reads — they're \
        cached, structured, and return JSON. Useful starting points:\n"
    )
    .ok();
    writeln!(out, "- `belisarius_brief` — one-shot markdown overview of language mix, hotspots, test gaps, hot functions.").ok();
    writeln!(
        out,
        "- `belisarius_search_code` — hybrid semantic + BM25 over chunked source."
    )
    .ok();
    writeln!(
        out,
        "- `belisarius_search_symbols` — substring search across the SCIP symbol index."
    )
    .ok();
    writeln!(
        out,
        "- `belisarius_symbol` — def + direct callers + direct callees in one call."
    )
    .ok();
    writeln!(
        out,
        "- `belisarius_impact` / `belisarius_flow` — transitive caller / callee traversal."
    )
    .ok();
    writeln!(out, "- `belisarius_function_detail` — full bundle: source, complexity, churn, callers, covering tests.").ok();
    writeln!(out, "- `belisarius_diff <base>` — files changed since `base`, overlayed with hotspots and tests.").ok();
    writeln!(
        out,
        "- `belisarius_rules_check` — fail-fast verification of `.belisarius/rules.toml`."
    )
    .ok();
    writeln!(
        out,
        "- `belisarius_next_action` — what would move the quality score the most right now.\n"
    )
    .ok();
    let tool_count = crate::mcp::registry::default_registry().definitions().len();
    writeln!(
        out,
        "Run `belisarius mcp tools` for the full catalog of {tool_count} tools.\n"
    )
    .ok();

    writeln!(out, "## Conventions").ok();
    writeln!(
        out,
        "- Don't reach for `grep`/`cat` until search has narrowed scope to 1–3 files.\n\
         - Prefer editing existing files to creating new ones.\n\
         - When changing a public symbol, run `belisarius_impact` first to size the blast radius.\n\
         - After non-trivial edits, run `belisarius check` (and the project's own test suite)."
    )
    .ok();
    writeln!(out).ok();

    writeln!(out, "## Local commands").ok();
    if project.join("Cargo.toml").exists() {
        writeln!(
            out,
            "- `cargo build` / `cargo test` / `cargo clippy` (Rust workspace)."
        )
        .ok();
    }
    if project.join("package.json").exists() {
        writeln!(
            out,
            "- `pnpm install` / `pnpm test` / `pnpm run build` (Node project)."
        )
        .ok();
    }
    if project.join("Justfile").exists() {
        writeln!(
            out,
            "- `just` — see Justfile for project-defined workflows."
        )
        .ok();
    }
    if project.join("Makefile").exists() {
        writeln!(out, "- `make` — see Makefile for project targets.").ok();
    }
    writeln!(
        out,
        "- `belisarius doctor` — environment health (indexers, search index, rules)."
    )
    .ok();
    writeln!(
        out,
        "- `belisarius init` — re-bootstrap `.belisarius/` if anything looks stale.\n"
    )
    .ok();

    writeln!(out, "## Project layout").ok();
    let dirs = top_level_dirs(project);
    for d in dirs.iter().take(12) {
        writeln!(out, "- `{d}/`").ok();
    }
    writeln!(out).ok();

    writeln!(out, "_Last refreshed by `belisarius agents init`._").ok();
    Ok(out)
}

fn top_level_dirs(project: &Path) -> Vec<String> {
    let mut dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(project) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist"
            {
                continue;
            }
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(name);
            }
        }
    }
    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    //! `render` is the kernel — it stitches scan output, MCP tool catalog,
    //! and project layout into AGENTS.md. If it produces something that
    //! parses as Markdown with our marker, the file machinery on top is
    //! straight glue and the integration test in `cmd_init` covers it.
    use super::*;
    use tempfile::TempDir;

    fn project_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, body) in files {
            let p = dir.path().join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    #[test]
    fn render_emits_marker_and_required_sections() {
        let dir = project_with(&[
            ("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n"),
            ("src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
        ]);
        let out = render(dir.path()).unwrap();
        assert!(out.starts_with(MARKER));
        assert!(out.contains("## Snapshot"));
        assert!(out.contains("## Working with Belisarius"));
        assert!(out.contains("## Conventions"));
        assert!(out.contains("## Local commands"));
        assert!(out.contains("## Project layout"));
        assert!(out.contains("cargo build"));
    }

    #[test]
    fn render_detects_node_project() {
        let dir = project_with(&[
            ("package.json", "{\n  \"name\": \"x\"\n}\n"),
            ("index.js", "console.log(1)\n"),
        ]);
        let out = render(dir.path()).unwrap();
        assert!(out.contains("pnpm install"));
        assert!(!out.contains("cargo build"));
    }

    #[test]
    fn init_writes_then_refreshes_in_place() {
        let dir = project_with(&[("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n")]);
        init(InitArgs {
            path: dir.path().to_path_buf(),
            out: "AGENTS.md".into(),
            force: false,
            stdout: false,
            json: false,
        })
        .unwrap();
        let path = dir.path().join("AGENTS.md");
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains(MARKER));
        // Re-run: must update in place (file written again) and keep the marker.
        init(InitArgs {
            path: dir.path().to_path_buf(),
            out: "AGENTS.md".into(),
            force: false,
            stdout: false,
            json: false,
        })
        .unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert!(second.contains(MARKER));
    }

    #[test]
    fn init_skips_unmanaged_file_without_force() {
        let dir = project_with(&[("Cargo.toml", "[package]\nname=\"x\"\nversion=\"0.1.0\"\n")]);
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "# Hand-written\n").unwrap();
        init(InitArgs {
            path: dir.path().to_path_buf(),
            out: "AGENTS.md".into(),
            force: false,
            stdout: false,
            json: false,
        })
        .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "# Hand-written\n");
    }
}
