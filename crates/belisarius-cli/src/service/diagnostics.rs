//! External lint/security tool aggregation — clippy, semgrep, ruff, eslint.
//!
//! Each run produces a `DiagnosticsReport` and writes it to
//! `.belisarius/diagnostics/report.json`. Subsequent reads return the cached
//! report unless any source file has been touched since (sentinel mtime
//! check). HTTP-only today — no MCP twin yet.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct PathArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RunArgs {
    pub path: String,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
pub struct ListArgs {
    pub path: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Walk the static tool registry and report which ones are installed +
/// applicable to this project. Cheap (no run).
pub async fn status(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let project_owned = project.clone();
    let statuses = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Value>> {
        let scan = belisarius_scan::scan(&project_owned)?;
        let mut out = Vec::new();
        for tool in belisarius_scan::diagnostics::registry() {
            out.push(json!({
                "name": tool.name(),
                "binary": tool.binary(),
                "installed": tool.is_installed(),
                "applied": tool.is_installed() && tool.applies_to(&scan),
            }));
        }
        Ok(out)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diag status join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diag status: {e:#}")))?;
    Ok(json!({ "tools": statuses }))
}

/// Run the diagnostics suite. Returns `{ cached: true, report }` when a
/// fresh-enough cache exists, else `{ cached: false, report }` after running.
pub async fn run(ctx: &AppContext, args: RunArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let cache_path = diag_cache_path(&path);
    let path_for_sentinel = path.clone();
    let sentinel = tokio::task::spawn_blocking(move || {
        crate::service::context::project_sentinel_mtime(&path_for_sentinel)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("sentinel join: {e}")))?;

    if !args.force {
        if let Some(cached) = read_diag_cache(&cache_path, sentinel) {
            return Ok(json!({ "cached": true, "report": cached }));
        }
    }

    let project = path.clone();
    let tools = args.tools.clone();
    let report = tokio::task::spawn_blocking(
        move || -> anyhow::Result<belisarius_core::DiagnosticsReport> {
            let scan = belisarius_scan::scan(&project)?;
            let only = tools.as_deref();
            belisarius_scan::diagnostics::run_all(Path::new(&project), &scan, only)
        },
    )
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diag run join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diag run: {e:#}")))?;
    let _ = write_diag_cache(&cache_path, &report);
    Ok(json!({ "cached": false, "report": report }))
}

/// Read the on-disk cache produced by an earlier `run`, applying client-side
/// filters. Returns `NotFound` when no cache exists — the UI surfaces
/// this as "POST /api/diagnostics/run first".
pub async fn list(ctx: &AppContext, args: ListArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let cache_path = diag_cache_path(&path);
    let report = read_diag_cache_any(&cache_path).ok_or_else(|| {
        ServiceError::not_found("no cached diagnostics — POST /api/diagnostics/run first")
    })?;
    let limit = args.limit.unwrap_or(500);
    let filtered: Vec<&belisarius_core::Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| args.tool.as_deref().is_none() || Some(d.tool.as_str()) == args.tool.as_deref())
        .filter(|d| {
            args.severity.as_deref().is_none_or(|s| {
                let want = s.to_lowercase();
                let have = format!("{:?}", d.severity).to_lowercase();
                want == have
            })
        })
        .filter(|d| args.file.as_deref().is_none_or(|f| d.file == f))
        .take(limit)
        .collect();
    Ok(json!({
        "total_cached": report.diagnostics.len(),
        "returned": filtered.len(),
        "diagnostics": filtered,
        "counts_by_tool": report.counts_by_tool,
        "counts_by_severity": report.counts_by_severity,
        "tools_ran": report.tools_ran,
    }))
}

// ─── On-disk cache helpers ────────────────────────────────────────────────

fn diag_cache_path(project: &str) -> PathBuf {
    Path::new(project)
        .join(".belisarius")
        .join("diagnostics")
        .join("report.json")
}

fn read_diag_cache(
    path: &Path,
    sentinel: SystemTime,
) -> Option<belisarius_core::DiagnosticsReport> {
    let cached_mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    if cached_mtime < sentinel {
        return None;
    }
    read_diag_cache_any(path)
}

fn read_diag_cache_any(path: &Path) -> Option<belisarius_core::DiagnosticsReport> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_diag_cache(
    path: &Path,
    report: &belisarius_core::DiagnosticsReport,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(report)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[allow(dead_code)]
fn _arc_ctx_marker(_: Arc<AppContext>) {}
