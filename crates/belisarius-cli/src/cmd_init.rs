//! `belisarius init` — first-run bootstrap.
//!
//! Runs the five things every new project needs before the rest of
//! Belisarius is useful:
//!  1. Scan the project to detect language mix.
//!  2. Create `.belisarius/` and its expected subdirectories so subsequent
//!     commands (scip, search, diagnostics, snapshots) don't have to
//!     mkdir-or-fail.
//!  3. Pre-fetch the `bge-small-en-v1.5` embedding model (~33 MB) so the
//!     first hybrid-search call doesn't pay a cold-start download. Skip
//!     with `--skip-model` when offline.
//!  4. Probe each per-language SCIP indexer (`rust-analyzer`,
//!     `scip-typescript`, `scip-python`, `scip-go`) — report which are
//!     installed and applicable, hint at how to install the missing ones.
//!  5. Print a "what to run next" summary.
//!
//! The whole thing is idempotent; running `belisarius init` on an already-
//! initialized project re-runs the probes and prints the same summary
//! without disturbing existing state.

use anyhow::{Context, Result};
use belisarius_symbols::indexer::{registry, IndexerStatus};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, Default)]
pub struct InitReport {
    pub project: String,
    pub files: usize,
    pub total_loc: u64,
    pub languages: Vec<LangLoc>,
    pub dirs: Vec<String>,
    pub embedding_model: ModelStatus,
    pub indexers: Vec<IndexerEntry>,
    pub search_index_built: bool,
    pub agents_md: Option<AgentsResult>,
    pub hook: Option<HookResult>,
    pub next_steps: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct LangLoc {
    pub language: String,
    pub loc: u64,
}

#[derive(serde::Serialize, Default)]
pub struct ModelStatus {
    pub status: String,
    pub name: Option<String>,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct IndexerEntry {
    pub language: String,
    pub binary: String,
    pub status: String,
    pub install_hint: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AgentsResult {
    pub path: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct HookResult {
    pub path: String,
    pub status: String,
    pub error: Option<String>,
}

impl crate::output::Renderable for InitReport {}

#[derive(clap::Args)]
pub struct InitArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Don't pre-fetch the embedding model. Use when offline or on CI
    /// before a release where you'd rather fail fast than burn ~33 MB of
    /// bandwidth.
    #[arg(long)]
    pub skip_model: bool,
    /// After the bootstrap, also build the hybrid search index (BM25 +
    /// embeddings) so the first query works without an extra command.
    #[arg(long)]
    pub index: bool,
    /// Emit a JSON report (same shape an agent would consume from a
    /// hypothetical `belisarius_init` MCP tool). Suppresses the human stream.
    #[arg(long)]
    pub json: bool,
    /// Also drop a starter `AGENTS.md` describing how to drive this project
    /// with Belisarius. Idempotent — re-running refreshes the data lines.
    #[arg(long)]
    pub agents: bool,
    /// Override the filename written by `--agents`. Default: `AGENTS.md`.
    #[arg(long, default_value = "AGENTS.md")]
    pub agents_file: String,
    /// Also install a non-blocking git pre-commit hook running
    /// `belisarius check --no-fail`. Skips silently when not in a git repo.
    #[arg(long)]
    pub hooks: bool,
    /// Run every optional step (`--index`, `--agents`, `--hooks`) in one shot.
    #[arg(long)]
    pub all: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let project = canonicalize(&args.path);
    let quiet = args.json;
    let do_index = args.index || args.all;
    let do_agents = args.agents || args.all;
    let do_hooks = args.hooks || args.all;
    let mut report = InitReport {
        project: project.to_string_lossy().into_owned(),
        files: 0,
        total_loc: 0,
        languages: vec![],
        dirs: vec![],
        embedding_model: ModelStatus::default(),
        indexers: vec![],
        search_index_built: false,
        agents_md: None,
        hook: None,
        next_steps: vec![],
    };
    if !quiet {
        println!("Initializing Belisarius for {}", project.display());
        println!();
    }

    // 1. Scan
    let scan = belisarius_scan::scan(project.to_string_lossy().as_ref())
        .with_context(|| format!("scanning {}", project.display()))?;
    let total_loc: u64 = scan.language_summary.values().map(|s| s.loc as u64).sum();
    report.files = scan.files.len();
    report.total_loc = total_loc;
    if !quiet {
        println!(
            "  Project: {} files, {} LOC across {} language(s)",
            scan.files.len(),
            total_loc,
            scan.language_summary.len(),
        );
    }
    let langs_by_loc: Vec<(&String, u64)> = {
        let mut v: Vec<(&String, u64)> = scan
            .language_summary
            .iter()
            .map(|(k, s)| (k, s.loc as u64))
            .collect();
        v.sort_by_key(|(_, loc)| std::cmp::Reverse(*loc));
        v
    };
    for (lang, loc) in &langs_by_loc {
        report.languages.push(LangLoc {
            language: (*lang).clone(),
            loc: *loc,
        });
        if !quiet {
            println!("    {:<14} {} LOC", lang, loc);
        }
    }
    if !quiet {
        println!();
    }

    // 2. Create skeleton
    let dirs = create_skeleton(&project)?;
    report.dirs = dirs
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    if !quiet {
        println!("  Created/verified directories:");
        for d in &dirs {
            println!("    {}", d.display());
        }
        println!();
    }

    // 3. Embedding model
    if args.skip_model {
        report.embedding_model = ModelStatus {
            status: "skipped".into(),
            name: None,
            error: None,
        };
        if !quiet {
            println!("  Embedding model: skipped (--skip-model)");
            println!();
        }
    } else {
        match fetch_model() {
            Ok(name) => {
                report.embedding_model = ModelStatus {
                    status: "ready".into(),
                    name: Some(name.clone()),
                    error: None,
                };
                if !quiet {
                    println!(
                        "  Embedding model: {name} (cached locally — first search is now warm)"
                    );
                    println!();
                }
            }
            Err(e) => {
                report.embedding_model = ModelStatus {
                    status: "failed".into(),
                    name: None,
                    error: Some(format!("{e}")),
                };
                if !quiet {
                    println!("  Embedding model: failed — {e}");
                    println!("    Hybrid search will fall back to BM25-only.");
                    println!("    Re-run `belisarius init` once online to fetch it.");
                    println!();
                }
            }
        }
    }

    // 4. SCIP indexers
    let scip_results = probe_scip_indexers(&project, &scan.language_summary);
    let needed_installs = scip_results
        .iter()
        .filter(|p| matches!(p.status, IndexerStatus::NotInstalled) && p.applies)
        .count();
    let ready_count = scip_results
        .iter()
        .filter(|p| matches!(p.status, IndexerStatus::Ready))
        .count();
    if !quiet {
        println!(
            "  SCIP indexers ({} ready, {} would help but aren't installed):",
            ready_count, needed_installs,
        );
    }
    for p in &scip_results {
        // The status print blends `applies_to()` (which only looks at the
        // project root) with the scan's per-language file count. When the
        // root says "no" but the scan found files, that's usually a
        // monorepo where the indexer needs to run from a subdir — flag it
        // rather than dismissing it as n/a.
        let (status_label, status_machine) = match (p.status, p.applies) {
            (IndexerStatus::Ready, _) => ("  ready".to_string(), "ready"),
            (IndexerStatus::DoesNotApply, true) => (
                "    installed — run from the subdir containing the project file".to_string(),
                "subdir",
            ),
            (IndexerStatus::DoesNotApply, false) => {
                ("    n/a (no matching files)".to_string(), "n/a")
            }
            (IndexerStatus::NotInstalled, true) => (
                "    missing — but the project has files".to_string(),
                "missing",
            ),
            (IndexerStatus::NotInstalled, false) => (
                "    n/a (not installed, no files)".to_string(),
                "not-installed",
            ),
        };
        report.indexers.push(IndexerEntry {
            language: p.language.clone(),
            binary: p.binary.clone(),
            status: status_machine.to_string(),
            install_hint: install_hint(&p.language).map(str::to_string),
        });
        if !quiet {
            println!(
                "    {:<16} {:<26} {status_label}",
                p.language,
                format!("({})", p.binary),
            );
            if matches!(p.status, IndexerStatus::NotInstalled) && p.applies {
                if let Some(hint) = install_hint(&p.language) {
                    println!("                       install: {hint}");
                }
            }
        }
    }
    if !quiet {
        println!();
    }

    // 5. Optional initial index
    if do_index {
        if !quiet {
            println!("  Building search index…");
        }
        let project_str = project.to_string_lossy().to_string();
        let handle =
            belisarius_search::IndexHandle::open(&project).context("opening search index")?;
        let project_for_log = project_str.clone();
        tokio::task::spawn_blocking(move || {
            handle.reindex(belisarius_search::ReindexOptions {
                full: false,
                bm25_only: false,
            })
        })
        .await
        .context("indexer join")?
        .with_context(|| format!("indexing {project_for_log}"))?;
        report.search_index_built = true;
        if !quiet {
            println!("  Search index: built.");
            println!();
        }
    }

    // 6. Optional `AGENTS.md`.
    if do_agents {
        match write_agents_md(&project, &args.agents_file) {
            Ok((status, path)) => {
                if !quiet {
                    println!(
                        "  {} {} — {}",
                        agents_glyph(&status),
                        args.agents_file,
                        status
                    );
                    println!();
                }
                report.agents_md = Some(AgentsResult {
                    path,
                    status,
                    error: None,
                });
            }
            Err(e) => {
                let err = format!("{e:#}");
                if !quiet {
                    println!("  AGENTS.md: failed — {err}");
                    println!();
                }
                report.agents_md = Some(AgentsResult {
                    path: project
                        .join(&args.agents_file)
                        .to_string_lossy()
                        .into_owned(),
                    status: "error".into(),
                    error: Some(err),
                });
            }
        }
    }

    // 7. Optional git pre-commit hook.
    if do_hooks {
        match install_precommit_hook(&project) {
            Ok((status, path)) => {
                if !quiet {
                    println!("  {} pre-commit — {}", hook_glyph(&status), status);
                    println!();
                }
                report.hook = Some(HookResult {
                    path,
                    status,
                    error: None,
                });
            }
            Err(e) => {
                let err = format!("{e:#}");
                if !quiet {
                    println!("  pre-commit hook: skipped — {err}");
                    println!();
                }
                report.hook = Some(HookResult {
                    path: project
                        .join(".git/hooks/pre-commit")
                        .to_string_lossy()
                        .into_owned(),
                    status: "skipped".into(),
                    error: Some(err),
                });
            }
        }
    }

    // Summary / next steps
    let next_steps = build_next_steps(&args, do_index, do_agents, do_hooks);
    report.next_steps = next_steps.iter().map(|(c, _)| (*c).to_string()).collect();
    if !quiet {
        println!("Next steps:");
        for (cmd_line, note) in &next_steps {
            println!("  {cmd_line:<48}  # {note}");
        }
        println!();
        println!("Next: belisarius next   # state-aware recommendation");
    }

    if args.json {
        crate::output::emit(&report, true)?;
    }
    Ok(())
}

fn build_next_steps(
    args: &InitArgs,
    did_index: bool,
    did_agents: bool,
    did_hooks: bool,
) -> Vec<(String, &'static str)> {
    let p = args.path.display();
    let mut v: Vec<(String, &'static str)> = Vec::new();
    v.push((
        format!("belisarius brief {p}"),
        "one-shot markdown overview",
    ));
    v.push((format!("belisarius quality {p}"), "composite 0-100 score"));
    if !did_index {
        v.push((
            format!("belisarius search index {p}"),
            "build the hybrid search index",
        ));
    }
    if !did_agents {
        v.push((
            "belisarius agents init".to_string(),
            "drop an AGENTS.md guide for agents",
        ));
    }
    if !did_hooks {
        v.push((
            "belisarius hooks install".to_string(),
            "non-blocking pre-commit check",
        ));
    }
    v.push((
        "belisarius mcp install".to_string(),
        "wire up Claude Code / Cursor / Claude Desktop",
    ));
    v.push((
        "belisarius mcp tools".to_string(),
        "enumerate every MCP tool Belisarius exposes",
    ));
    v.push((
        "belisarius serve --watch".to_string(),
        "HTTP + UI, indexer follows edits",
    ));
    v
}

fn agents_glyph(status: &str) -> &'static str {
    match status {
        "wrote" | "updated" | "overwrote" => "✓",
        "skipped" => "·",
        _ => "?",
    }
}

fn hook_glyph(status: &str) -> &'static str {
    match status {
        "wrote" | "updated" => "✓",
        "unchanged" => "·",
        "skipped" => "!",
        _ => "?",
    }
}

/// Thin wrapper around `cmd_agents::render` that writes the result if there's
/// nothing already in the way. Same idempotency rules as `belisarius agents
/// init`: only refresh files we wrote ourselves.
fn write_agents_md(project: &Path, file_name: &str) -> Result<(String, String)> {
    const MARKER: &str = "<!-- belisarius:agents v1 -->";
    let out_path = project.join(file_name);
    let status = if out_path.exists() {
        let existing = std::fs::read_to_string(&out_path)?;
        if existing.contains(MARKER) {
            "updated"
        } else {
            return Ok((
                "skipped".to_string(),
                out_path.to_string_lossy().into_owned(),
            ));
        }
    } else {
        "wrote"
    };
    let content = crate::cmd_agents::render(project)?;
    std::fs::write(&out_path, content)?;
    Ok((status.to_string(), out_path.to_string_lossy().into_owned()))
}

/// Install a non-blocking `pre-commit` hook, or report why we couldn't.
fn install_precommit_hook(project: &Path) -> Result<(String, String)> {
    if !project.join(".git").exists() {
        anyhow::bail!("not a git repository");
    }
    let hooks_dir = project.join(".git/hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook = hooks_dir.join("pre-commit");
    const MARKER: &str = "# belisarius:managed-hook v1";
    if hook.exists() {
        let text = std::fs::read_to_string(&hook)?;
        if text.contains(MARKER) {
            return Ok(("unchanged".to_string(), hook.to_string_lossy().into_owned()));
        }
        anyhow::bail!("existing pre-commit hook not managed by Belisarius (use `belisarius hooks install --force`)");
    }
    let body = format!(
        "#!/usr/bin/env sh\n\
         {MARKER}\n\
         set -u\n\
         if ! command -v belisarius >/dev/null 2>&1; then\n\
           exit 0\n\
         fi\n\
         belisarius check . --no-fail\n\
         exit 0\n"
    );
    std::fs::write(&hook, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook, perms)?;
    }
    Ok(("wrote".to_string(), hook.to_string_lossy().into_owned()))
}

fn canonicalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Returns the directories created (or verified) so the caller can print
/// them. Always idempotent.
fn create_skeleton(project: &Path) -> Result<Vec<PathBuf>> {
    let base = project.join(".belisarius");
    let dirs = vec![
        base.clone(),
        base.join("scip"),
        base.join("search"),
        base.join("search/bm25"),
        base.join("diagnostics"),
    ];
    for d in &dirs {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }

    // Drop in a starter `context_artifacts.json` if nothing's there yet —
    // empty but valid so the registry loader doesn't 404 on a fresh project.
    let artifacts = base.join("context_artifacts.json");
    if !artifacts.exists() {
        std::fs::write(&artifacts, "{\n  \"artifacts\": []\n}\n")
            .with_context(|| format!("writing {}", artifacts.display()))?;
    }
    Ok(dirs)
}

fn fetch_model() -> Result<String> {
    use belisarius_search::EmbeddingError;
    match belisarius_search::embed::default_provider() {
        Ok(p) => Ok(p.model_name().to_string()),
        Err(EmbeddingError::Disabled) => {
            anyhow::bail!("compiled without `embed` feature")
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

struct ScipProbe {
    language: String,
    binary: String,
    status: IndexerStatus,
    /// Does the project have files this indexer would process?
    applies: bool,
}

fn probe_scip_indexers(
    project: &Path,
    language_summary: &BTreeMap<String, belisarius_core::LanguageSummary>,
) -> Vec<ScipProbe> {
    registry()
        .into_iter()
        .map(|i| {
            let status = i.status(project);
            // `applies` reflects "would this indexer help at all?" — i.e.
            // does the project have files in this language? Independent of
            // `applies_to(project_root)`, which only checks the root and
            // misses monorepo subdirs. The print layer combines this with
            // the status to give a nuanced message.
            let applies = language_summary
                .get(i.language())
                .map(|s| s.files > 0)
                .unwrap_or(false);
            ScipProbe {
                language: i.language().to_string(),
                binary: i.binary().to_string(),
                status,
                applies,
            }
        })
        .collect()
}

fn install_hint(language: &str) -> Option<&'static str> {
    match language {
        "rust" => Some("rustup component add rust-analyzer"),
        "typescript" => Some("npm i -g @sourcegraph/scip-typescript"),
        "python" => Some("pipx install scip-python  (or pip install --user scip-python)"),
        "go" => Some("go install github.com/sourcegraph/scip-go/cmd/scip-go@latest"),
        _ => None,
    }
}
