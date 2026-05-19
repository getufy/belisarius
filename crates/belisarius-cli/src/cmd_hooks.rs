//! `belisarius hooks` — manage git hooks that keep architectural rules from
//! drifting between agent runs.
//!
//! `belisarius hooks install` drops a small POSIX shell hook at
//! `.git/hooks/<name>` that runs `belisarius check --no-fail` (or `--quiet`,
//! soon). The hook is intentionally minimal — no project bootstrapping, no
//! cargo invocations — so it stays fast enough for an interactive commit.
//!
//! Why not delegate to `pre-commit` (the Python framework)? That's another
//! global install agents have to learn. The git hook protocol is the same
//! everywhere, so writing the file directly is the simplest thing that
//! works for both humans and agents.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand)]
pub enum HooksCmd {
    /// Drop a git hook that runs `belisarius check` automatically.
    Install(InstallArgs),
    /// Remove a previously installed Belisarius git hook.
    Uninstall(UninstallArgs),
    /// Print where Belisarius hooks would live for this repo.
    Status(StatusArgs),
}

#[derive(clap::Args)]
pub struct InstallArgs {
    /// Project root (where `.git/` lives). Defaults to current directory.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Which hook to install. Default `pre-commit`. `pre-push` is slower but
    /// catches regressions before they leave the developer's machine.
    #[arg(long, value_enum, default_value_t = HookKind::PreCommit)]
    pub hook: HookKind,
    /// Block the commit/push when `belisarius check` fails. Default is
    /// non-blocking: prints the report and lets the commit through, so agents
    /// can iterate without a permission battle on every fix-up commit.
    #[arg(long)]
    pub blocking: bool,
    /// Overwrite an existing hook even if it wasn't written by us.
    #[arg(long)]
    pub force: bool,
    /// Emit JSON describing what changed (for agent consumption).
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct UninstallArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, value_enum, default_value_t = HookKind::PreCommit)]
    pub hook: HookKind,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum HookKind {
    PreCommit,
    PrePush,
}

impl HookKind {
    fn file_name(self) -> &'static str {
        match self {
            HookKind::PreCommit => "pre-commit",
            HookKind::PrePush => "pre-push",
        }
    }
}

/// A header we write into every hook so we can detect "did *we* write this?"
/// later, for safe uninstall and force-detection. Don't change without bumping
/// the marker.
const HOOK_MARKER: &str = "# belisarius:managed-hook v1";

pub async fn run(cmd: HooksCmd) -> Result<()> {
    match cmd {
        HooksCmd::Install(a) => install(a),
        HooksCmd::Uninstall(a) => uninstall(a),
        HooksCmd::Status(a) => status(a),
    }
}

#[derive(serde::Serialize)]
struct HookReport {
    hook: String,
    path: String,
    status: String,
    note: Option<String>,
}

fn install(args: InstallArgs) -> Result<()> {
    let hook_path = hook_path(&args.path, args.hook)?;
    let exists = hook_path.exists();
    let ours = exists && file_contains_marker(&hook_path)?;
    if exists && !ours && !args.force {
        return emit_report(
            HookReport {
                hook: args.hook.file_name().to_string(),
                path: hook_path.to_string_lossy().into_owned(),
                status: "conflict".to_string(),
                note: Some(
                    "an existing hook is in place that we didn't write; re-run with --force"
                        .to_string(),
                ),
            },
            args.json,
        );
    }

    let body = hook_body(args.blocking);
    std::fs::write(&hook_path, body.as_bytes())
        .with_context(|| format!("writing {}", hook_path.display()))?;
    make_executable(&hook_path)?;

    let status = if exists { "updated" } else { "wrote" };
    emit_report(
        HookReport {
            hook: args.hook.file_name().to_string(),
            path: hook_path.to_string_lossy().into_owned(),
            status: status.to_string(),
            note: Some(if args.blocking {
                "blocking on check failure".to_string()
            } else {
                "non-blocking — prints report and continues".to_string()
            }),
        },
        args.json,
    )
}

fn uninstall(args: UninstallArgs) -> Result<()> {
    let hook_path = hook_path(&args.path, args.hook)?;
    if !hook_path.exists() {
        return emit_report(
            HookReport {
                hook: args.hook.file_name().to_string(),
                path: hook_path.to_string_lossy().into_owned(),
                status: "absent".to_string(),
                note: None,
            },
            args.json,
        );
    }
    if !file_contains_marker(&hook_path)? {
        return emit_report(
            HookReport {
                hook: args.hook.file_name().to_string(),
                path: hook_path.to_string_lossy().into_owned(),
                status: "skipped".to_string(),
                note: Some(
                    "existing hook wasn't written by Belisarius; leaving it in place".to_string(),
                ),
            },
            args.json,
        );
    }
    std::fs::remove_file(&hook_path)
        .with_context(|| format!("removing {}", hook_path.display()))?;
    emit_report(
        HookReport {
            hook: args.hook.file_name().to_string(),
            path: hook_path.to_string_lossy().into_owned(),
            status: "removed".to_string(),
            note: None,
        },
        args.json,
    )
}

fn status(args: StatusArgs) -> Result<()> {
    let kinds = [HookKind::PreCommit, HookKind::PrePush];
    let mut rows = Vec::new();
    for k in kinds {
        let p = hook_path(&args.path, k)?;
        let status = if !p.exists() {
            "absent"
        } else if file_contains_marker(&p)? {
            "managed"
        } else {
            "external"
        };
        rows.push(HookReport {
            hook: k.file_name().to_string(),
            path: p.to_string_lossy().into_owned(),
            status: status.to_string(),
            note: None,
        });
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for r in &rows {
            println!("  {:<10}  {:<10}  {}", r.hook, r.status, r.path);
        }
    }
    Ok(())
}

fn emit_report(r: HookReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        let mark = match r.status.as_str() {
            "wrote" | "updated" | "removed" => "✓",
            "absent" | "managed" | "external" => "·",
            "skipped" | "conflict" => "!",
            _ => "?",
        };
        println!("  {mark} {:<10} {} — {}", r.hook, r.status, r.path);
        if let Some(note) = &r.note {
            println!("      {note}");
        }
    }
    Ok(())
}

fn hook_path(project: &Path, kind: HookKind) -> Result<PathBuf> {
    let git = project.join(".git");
    if !git.exists() {
        anyhow::bail!(
            "no .git directory at {} — Belisarius hooks need a git repo",
            project.display()
        );
    }
    let hooks_dir = if git.is_dir() {
        git.join("hooks")
    } else {
        // Handle git worktrees / submodules where `.git` is a file
        // pointing at the real gitdir.
        let txt =
            std::fs::read_to_string(&git).with_context(|| format!("reading {}", git.display()))?;
        let real = txt
            .lines()
            .find_map(|l| l.strip_prefix("gitdir:").map(str::trim))
            .ok_or_else(|| anyhow::anyhow!("unexpected `.git` file at {}", git.display()))?;
        PathBuf::from(real).join("hooks")
    };
    std::fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("creating {}", hooks_dir.display()))?;
    Ok(hooks_dir.join(kind.file_name()))
}

fn file_contains_marker(p: &Path) -> Result<bool> {
    let txt = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    Ok(txt.contains(HOOK_MARKER))
}

fn hook_body(blocking: bool) -> String {
    let fail_clause = if blocking { "exit $status" } else { "exit 0" };
    format!(
        "#!/usr/bin/env sh\n\
         {HOOK_MARKER}\n\
         #\n\
         # Run `belisarius check` against the working tree. Edit at your own risk\n\
         # — `belisarius hooks install` will overwrite this file.\n\
         set -u\n\
         if ! command -v belisarius >/dev/null 2>&1; then\n\
           echo 'belisarius not on PATH; skipping architectural check' >&2\n\
           exit 0\n\
         fi\n\
         belisarius check . --no-fail\n\
         status=$?\n\
         {fail_clause}\n"
    )
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(p)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(p, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) -> Result<()> {
    // On Windows git executes hooks via the shell shim; no chmod required.
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Walk the install/uninstall/status state machine against a fake git
    //! repo. We never invoke git itself — just stub `.git/hooks/` so the
    //! marker logic can run.
    use super::*;
    use tempfile::TempDir;

    fn fake_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git/hooks")).unwrap();
        dir
    }

    fn install_args(path: &Path, blocking: bool, force: bool) -> InstallArgs {
        InstallArgs {
            path: path.to_path_buf(),
            hook: HookKind::PreCommit,
            blocking,
            force,
            json: false,
        }
    }

    #[test]
    fn install_writes_hook_with_marker_and_exec_bit() {
        let dir = fake_repo();
        install(install_args(dir.path(), false, false)).unwrap();
        let hook = dir.path().join(".git/hooks/pre-commit");
        let body = std::fs::read_to_string(&hook).unwrap();
        assert!(body.contains(HOOK_MARKER));
        assert!(body.contains("belisarius check"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "hook must be executable");
        }
    }

    #[test]
    fn install_is_idempotent_on_managed_hook() {
        let dir = fake_repo();
        install(install_args(dir.path(), false, false)).unwrap();
        // Second run must not error and must keep the marker intact.
        install(install_args(dir.path(), false, false)).unwrap();
        let body = std::fs::read_to_string(dir.path().join(".git/hooks/pre-commit")).unwrap();
        assert!(body.contains(HOOK_MARKER));
    }

    #[test]
    fn install_refuses_to_overwrite_user_hook_without_force() {
        let dir = fake_repo();
        let hook = dir.path().join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\n# hand-written by the user\n").unwrap();
        install(install_args(dir.path(), false, false)).unwrap();
        let body = std::fs::read_to_string(&hook).unwrap();
        assert!(!body.contains(HOOK_MARKER));
        assert!(body.contains("hand-written"));
    }

    #[test]
    fn install_force_overwrites_user_hook() {
        let dir = fake_repo();
        let hook = dir.path().join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho stale\n").unwrap();
        install(install_args(dir.path(), false, true)).unwrap();
        let body = std::fs::read_to_string(&hook).unwrap();
        assert!(body.contains(HOOK_MARKER));
    }

    #[test]
    fn uninstall_removes_only_managed_hooks() {
        let dir = fake_repo();
        install(install_args(dir.path(), false, false)).unwrap();
        uninstall(UninstallArgs {
            path: dir.path().to_path_buf(),
            hook: HookKind::PreCommit,
            json: false,
        })
        .unwrap();
        assert!(!dir.path().join(".git/hooks/pre-commit").exists());
    }

    #[test]
    fn uninstall_leaves_external_hook_alone() {
        let dir = fake_repo();
        let hook = dir.path().join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho external\n").unwrap();
        uninstall(UninstallArgs {
            path: dir.path().to_path_buf(),
            hook: HookKind::PreCommit,
            json: false,
        })
        .unwrap();
        assert!(hook.exists(), "external hook must be preserved");
    }

    #[test]
    fn blocking_flag_changes_hook_body() {
        let dir = fake_repo();
        install(install_args(dir.path(), true, false)).unwrap();
        let body = std::fs::read_to_string(dir.path().join(".git/hooks/pre-commit")).unwrap();
        // Blocking hook propagates `$status`; non-blocking forces exit 0.
        assert!(body.contains("exit $status"));
    }
}
