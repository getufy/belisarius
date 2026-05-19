//! Subprocess wrappers around per-language SCIP indexers.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 300;
const POLL_INTERVAL_MS: u64 = 100;

fn indexer_timeout() -> Duration {
    let secs = std::env::var("BELISARIUS_INDEXER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexerStatus {
    /// Indexer binary not found on PATH.
    NotInstalled,
    /// Project doesn't look like it belongs to this language.
    DoesNotApply,
    /// Ready to run.
    Ready,
}

pub trait Indexer: Send + Sync {
    /// Short language id (`rust`, `typescript`, `python`, `go`, …).
    fn language(&self) -> &'static str;
    /// Human-readable indexer name.
    fn name(&self) -> &'static str;
    /// Binary the indexer shells out to.
    fn binary(&self) -> &'static str;
    /// Quick probe: is the binary on PATH?
    fn is_installed(&self) -> bool {
        Command::new(self.binary())
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    /// Does this project look like it has files this indexer can handle?
    fn applies_to(&self, project: &Path) -> bool;
    /// Run the indexer, writing the `.scip` to `output`.
    fn run(&self, project: &Path, output: &Path) -> Result<()>;

    fn status(&self, project: &Path) -> IndexerStatus {
        if !self.is_installed() {
            return IndexerStatus::NotInstalled;
        }
        if !self.applies_to(project) {
            return IndexerStatus::DoesNotApply;
        }
        IndexerStatus::Ready
    }
}

/// Built-in indexer registry. Order is preserved for deterministic runs.
pub fn registry() -> Vec<Box<dyn Indexer>> {
    vec![
        Box::new(RustAnalyzerIndexer),
        Box::new(ScipTypescriptIndexer),
        Box::new(ScipPythonIndexer),
        Box::new(ScipGoIndexer),
    ]
}

pub fn by_language(lang: &str) -> Option<Box<dyn Indexer>> {
    registry().into_iter().find(|i| i.language() == lang)
}

fn run_logged(name: &str, mut cmd: Command, project: &Path) -> Result<()> {
    let timeout = indexer_timeout();
    tracing::info!(indexer = name, ?project, ?timeout, args = ?cmd, "running indexer");
    let started = std::time::Instant::now();

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {name}"))?;

    // Drain stderr in a worker thread so a full pipe buffer can't deadlock the
    // child. stdout is also drained, but we don't keep it.
    use std::io::Read;
    let stderr_handle = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });
    let _stdout_handle = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut sink = std::io::sink();
            let _ = std::io::copy(&mut s, &mut sink);
        })
    });

    let deadline = started + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("{name} timed out after {:?}", timeout);
                }
                std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            }
        }
    };
    let dur = started.elapsed();

    if !status.success() {
        let stderr = stderr_handle
            .and_then(|h| h.join().ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        bail!(
            "{name} exited {} after {:?}\nstderr:\n{}",
            status,
            dur,
            stderr.trim()
        );
    }
    tracing::info!(indexer = name, elapsed = ?dur, "indexer done");
    Ok(())
}

pub struct RustAnalyzerIndexer;

impl Indexer for RustAnalyzerIndexer {
    fn language(&self) -> &'static str {
        "rust"
    }
    fn name(&self) -> &'static str {
        "rust-analyzer"
    }
    fn binary(&self) -> &'static str {
        "rust-analyzer"
    }
    fn applies_to(&self, project: &Path) -> bool {
        project.join("Cargo.toml").exists() || walk_for_marker(project, "Cargo.toml", 3).is_some()
    }
    fn run(&self, project: &Path, output: &Path) -> Result<()> {
        // rust-analyzer scip <project> --output <path>
        let mut cmd = Command::new(self.binary());
        cmd.arg("scip").arg(project).arg("--output").arg(output);
        run_logged(self.name(), cmd, project)
    }
}

pub struct ScipTypescriptIndexer;

impl Indexer for ScipTypescriptIndexer {
    fn language(&self) -> &'static str {
        "typescript"
    }
    fn name(&self) -> &'static str {
        "scip-typescript"
    }
    fn binary(&self) -> &'static str {
        "scip-typescript"
    }
    fn is_installed(&self) -> bool {
        // Try the direct binary first, then `npx scip-typescript --help` as fallback.
        if Command::new("scip-typescript")
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }
        Command::new("npx")
            .args(["--no-install", "scip-typescript", "--help"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    fn applies_to(&self, project: &Path) -> bool {
        // Needs at least tsconfig.json (or jsconfig.json) and node_modules / package.json.
        let has_tsconfig =
            project.join("tsconfig.json").exists() || project.join("jsconfig.json").exists();
        let has_pkg = project.join("package.json").exists();
        has_tsconfig && has_pkg
    }
    fn run(&self, project: &Path, output: &Path) -> Result<()> {
        // scip-typescript writes to `index.scip` in cwd by default; we override with --output.
        let direct = Command::new("scip-typescript")
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let mut cmd = if direct {
            Command::new("scip-typescript")
        } else {
            let mut c = Command::new("npx");
            c.args(["--no-install", "scip-typescript"]);
            c
        };
        cmd.current_dir(project).args(["index", "--output"]).arg(
            output
                .canonicalize()
                .unwrap_or_else(|_| output.to_path_buf()),
        );
        run_logged(self.name(), cmd, project)
    }
}

pub struct ScipPythonIndexer;

impl Indexer for ScipPythonIndexer {
    fn language(&self) -> &'static str {
        "python"
    }
    fn name(&self) -> &'static str {
        "scip-python"
    }
    fn binary(&self) -> &'static str {
        "scip-python"
    }
    fn applies_to(&self, project: &Path) -> bool {
        project.join("pyproject.toml").exists()
            || project.join("setup.py").exists()
            || project.join("requirements.txt").exists()
            || walk_for_marker(project, "pyproject.toml", 2).is_some()
    }
    fn run(&self, project: &Path, output: &Path) -> Result<()> {
        let mut cmd = Command::new(self.binary());
        cmd.current_dir(project)
            .args([
                "index",
                "--project-name",
                "scanned",
                "--project-version",
                "0",
                "--output",
            ])
            .arg(
                output
                    .canonicalize()
                    .unwrap_or_else(|_| output.to_path_buf()),
            );
        run_logged(self.name(), cmd, project)
    }
}

pub struct ScipGoIndexer;

impl Indexer for ScipGoIndexer {
    fn language(&self) -> &'static str {
        "go"
    }
    fn name(&self) -> &'static str {
        "scip-go"
    }
    fn binary(&self) -> &'static str {
        "scip-go"
    }
    fn applies_to(&self, project: &Path) -> bool {
        project.join("go.mod").exists() || walk_for_marker(project, "go.mod", 3).is_some()
    }
    fn run(&self, project: &Path, output: &Path) -> Result<()> {
        let mut cmd = Command::new(self.binary());
        cmd.current_dir(project).args(["--output"]).arg(
            output
                .canonicalize()
                .unwrap_or_else(|_| output.to_path_buf()),
        );
        run_logged(self.name(), cmd, project)
    }
}

fn walk_for_marker(root: &Path, marker: &str, max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, marker: &str, depth: usize, max: usize) -> Option<PathBuf> {
        if depth > max {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(marker) {
                return Some(path);
            }
            if path.is_dir() {
                let skip = matches!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some(".git" | "node_modules" | "target" | "dist" | ".belisarius")
                );
                if !skip {
                    subdirs.push(path);
                }
            }
        }
        for s in subdirs {
            if let Some(found) = walk(&s, marker, depth + 1, max) {
                return Some(found);
            }
        }
        None
    }
    walk(root, marker, 0, max_depth)
}
