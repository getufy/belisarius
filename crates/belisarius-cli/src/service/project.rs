//! Project-level capabilities — the residual cluster after the named feature
//! modules (quality, symbols, search, fleet, state, brief, pack,
//! function_detail) had their own homes. Everything here either:
//!  - reads the cached `AnalysisReport` and projects/filters a view, or
//!  - walks the project tree (commands, snippet, markers, components).
//!
//! Grouping them in one file is the smallest stable split — they share
//! `AppContext` and overlapping helpers (canonical path, fleet resolution,
//! the analysis cache).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use belisarius_core::FunctionInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::server::MarkerHit;
use crate::service::{context::AppContext, error::ServiceError};

/// `GET /api/snippet` response. Mirrors the shape `service::project::snippet`
/// returns; lifted from an inline `json!` to a typed struct so `ts-rs` can
/// emit a matching TypeScript binding.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SnippetResponse {
    pub file: String,
    pub language: String,
    pub start_line: u32,
    pub end_line: u32,
    pub target_line: u32,
    pub total_lines: u32,
    pub snippet: String,
}

/// `GET /api/markers` response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct MarkersResponse {
    pub markers: Vec<MarkerHit>,
    /// Markers in the response (post-limit). Same as `markers.len()`.
    pub total: u32,
    /// `true` when the scan stopped at `limit` and more markers exist.
    /// Kept for backwards compatibility — prefer `truncated`.
    pub limited: bool,
    /// Standard pagination flag — alias of `limited` for uniformity with
    /// other list-returning tools.
    pub truncated: bool,
    /// Markers returned in this page (same as `total`).
    pub returned: u32,
}

/// `GET /api/functions` response.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct FunctionsResponse {
    pub functions: Vec<FunctionInfo>,
    /// Total functions in the project (across all files, before filtering).
    pub total: u32,
    /// Functions in the current page (after filtering and limit).
    pub returned: u32,
    /// Functions matching the filters before `limit` was applied.
    pub total_count: u32,
    /// `true` when `total_count > returned` and more results were elided.
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
pub struct PathArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FunctionsArgs {
    pub path: String,
    #[serde(default)]
    pub min_cc: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HotspotsArgs {
    pub path: String,
    #[serde(default)]
    pub days: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LimitArgs {
    pub path: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SnippetArgs {
    pub path: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub radius: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct FileDsmArgs {
    pub path: String,
    pub file: String,
}

#[derive(Debug, Deserialize)]
pub struct DiffArgs {
    pub path: String,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub hotspot_window: Option<usize>,
}

// ─── scan / analyze / functions / surface / commands ─────────────────────

pub async fn scan(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let scan = tokio::task::spawn_blocking(move || belisarius_scan::scan(&path))
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("scan join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("scan: {e:#}")))?;
    serde_json::to_value(scan).map_err(Into::into)
}

pub async fn graph(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let graph = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let scan = belisarius_scan::scan(&path)?;
        Ok(belisarius_scan::build_graph(&scan))
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("graph join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("graph: {e:#}")))?;
    serde_json::to_value(graph).map_err(Into::into)
}

pub async fn analyze(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let report = ctx.load_analysis(&project).await?;
    serde_json::to_value(&*report).map_err(Into::into)
}

pub async fn functions(ctx: &AppContext, args: FunctionsArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let report = ctx.load_analysis(&project).await?;
    let min_cc = args.min_cc.unwrap_or(0);
    let limit = args.limit.unwrap_or(200);
    let sort = args.sort_by.unwrap_or_else(|| "cc".into());
    let mut fns: Vec<&belisarius_core::FunctionInfo> = report
        .functions
        .iter()
        .filter(|f| f.cyclomatic >= min_cc)
        .filter(|f| args.file.as_deref().is_none_or(|p| f.file == p))
        .collect();
    fns.sort_by(|a, b| match sort.as_str() {
        "cognitive" => b.cognitive.cmp(&a.cognitive),
        "loc" => b.loc.cmp(&a.loc),
        "params" => b.params.cmp(&a.params),
        _ => b.cyclomatic.cmp(&a.cyclomatic),
    });
    let total_count = fns.len() as u32;
    fns.truncate(limit);
    let returned = fns.len() as u32;
    let resp = FunctionsResponse {
        functions: fns.into_iter().cloned().collect(),
        total: report.functions.len() as u32,
        returned,
        total_count,
        truncated: total_count > returned,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn surface(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&project).await?;
    let scan = analysis.scan.clone();
    let project_owned = project.clone();
    let report = tokio::task::spawn_blocking(move || {
        belisarius_scan::surface::extract(Path::new(&project_owned), &scan)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("surface join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("surface: {e:#}")))?;
    serde_json::to_value(report).map_err(Into::into)
}

pub async fn commands(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let project = PathBuf::from(&path);
    let report = tokio::task::spawn_blocking(move || belisarius_scan::commands::discover(&project))
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("commands join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("commands: {e:#}")))?;
    serde_json::to_value(report).map_err(Into::into)
}

pub async fn components(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let project = PathBuf::from(&path);
    let comps = tokio::task::spawn_blocking(move || crate::cmd_arch::run_react_docgen(&project))
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("components join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("react-docgen: {e:#}")))?;
    Ok(json!({ "components": comps, "count": comps.len() }))
}

pub async fn rules_check(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&path).await?;
    let scan = analysis.scan.clone();
    let path_for_surface = path.clone();
    let surface = tokio::task::spawn_blocking(move || {
        belisarius_scan::surface::extract(Path::new(&path_for_surface), &scan).ok()
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("rules surface join: {e}")))?;
    let path_for_rules = path.clone();
    let report = tokio::task::spawn_blocking(move || {
        belisarius_scan::rules::evaluate(Path::new(&path_for_rules), &analysis, surface.as_ref())
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("rules join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("rules: {e:#}")))?;
    serde_json::to_value(report).map_err(Into::into)
}

// ─── hotspots / test_gaps / diff ─────────────────────────────────────────

pub async fn hotspots(ctx: &AppContext, args: HotspotsArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&path).await?;
    let days = args.days.unwrap_or(90);
    let limit = args.limit.unwrap_or(40);
    let keep: Vec<String> = analysis.scan.files.iter().map(|f| f.path.clone()).collect();
    let project_for_owners = PathBuf::from(&path);
    let project_for_git = project_for_owners.clone();
    let stats = tokio::task::spawn_blocking(move || {
        belisarius_scan::git_stats::collect(&project_for_git, days, Some(&keep))
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("hotspots join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("git: {e:#}")))?;
    let total_count = stats.files.len();
    let mut report =
        belisarius_scan::git_stats::rank_hotspots(&stats, &analysis.file_metrics, limit);
    let co = belisarius_scan::codeowners::CodeownersFile::load(&project_for_owners);
    belisarius_scan::git_stats::attach_owners(&mut report, co.as_ref());
    let returned = report.hotspots.len();
    let truncated = total_count > returned;
    Ok(json!({
        "repo_present": report.repo_present,
        "days_window": report.days_window,
        "hotspots": report.hotspots,
        "git_files_total": total_count,
        "total_count": total_count,
        "returned": returned,
        "truncated": truncated,
    }))
}

pub async fn test_gaps(ctx: &AppContext, args: LimitArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&path).await?;
    let limit = args.limit.unwrap_or(25);
    let project_owned = PathBuf::from(&path);
    let scan = analysis.scan.clone();
    let inline = tokio::task::spawn_blocking(move || {
        belisarius_scan::test_map::detect_inline_tests(&project_owned, &scan)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("test_gaps join: {e}")))?;
    let mut map = belisarius_scan::test_map::compute(&analysis, &inline);
    let total_count = map.gaps.len();
    let truncated = total_count > limit;
    if truncated {
        map.gaps.truncate(limit);
    }
    let returned = map.gaps.len();
    let mut value = serde_json::to_value(map).map_err(ServiceError::from)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("total_count".into(), serde_json::json!(total_count));
        obj.insert("returned".into(), serde_json::json!(returned));
        obj.insert("truncated".into(), serde_json::json!(truncated));
    }
    Ok(value)
}

pub async fn diff(ctx: &AppContext, args: DiffArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let base = args.base.unwrap_or_default();
    let head = args.head.unwrap_or_else(|| "HEAD".to_string());
    let window = args.hotspot_window.unwrap_or(100);
    let project = PathBuf::from(&path);

    let project_for_diff = project.clone();
    let diff = tokio::task::spawn_blocking(move || {
        belisarius_scan::diff::compute(&project_for_diff, &base, &head)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diff join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diff: {e:#}")))?;

    if !diff.repo_present {
        return Ok(json!({ "diff": diff, "overlay": null }));
    }

    let analysis = ctx.load_analysis(&path).await?;
    let scan = analysis.scan.clone();
    let project_for_surface = project.clone();
    let surface = tokio::task::spawn_blocking(move || {
        belisarius_scan::surface::extract(&project_for_surface, &scan).ok()
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diff surface join: {e}")))?;

    let keep: Vec<String> = analysis.scan.files.iter().map(|f| f.path.clone()).collect();
    let project_for_git = project.clone();
    let metrics = analysis.file_metrics.clone();
    let hotspots = tokio::task::spawn_blocking(move || {
        belisarius_scan::git_stats::collect(&project_for_git, 90, Some(&keep))
            .ok()
            .map(|gs| belisarius_scan::git_stats::rank_hotspots(&gs, &metrics, window))
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diff git join: {e}")))?;

    let scan2 = analysis.scan.clone();
    let project_for_inline = project.clone();
    let inline = tokio::task::spawn_blocking(move || {
        belisarius_scan::test_map::detect_inline_tests(&project_for_inline, &scan2)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("diff tests join: {e}")))?;

    let co = belisarius_scan::codeowners::CodeownersFile::load(&project);
    let overlay = belisarius_scan::diff::overlay(
        &analysis,
        &diff,
        surface.as_ref(),
        hotspots.as_ref(),
        &inline,
        co.as_ref(),
    );
    Ok(json!({ "diff": diff, "overlay": overlay }))
}

// ─── snippet / markers / file_dsm ────────────────────────────────────────

pub async fn snippet(ctx: &AppContext, args: SnippetArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let radius = args.radius.unwrap_or(25).min(500) as usize;
    let project_canon = std::fs::canonicalize(&path)
        .with_context(|| format!("project path: {path}"))
        .map_err(ServiceError::Internal)?;
    let full = project_canon.join(&args.file);
    let full_canon = std::fs::canonicalize(&full)
        .with_context(|| format!("file: {}", full.display()))
        .map_err(|e| ServiceError::not_found(format!("{e:#}")))?;
    if !full_canon.starts_with(&project_canon) {
        return Err(ServiceError::bad_request("file escapes project root"));
    }
    let text = std::fs::read_to_string(&full_canon)
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("read: {e}")))?;
    let lines: Vec<&str> = text.lines().collect();
    let target = args.line.max(1) as usize;
    let start = target.saturating_sub(radius + 1);
    let end = (target + radius).min(lines.len());
    let snippet = if start < end {
        lines[start..end].join("\n")
    } else {
        String::new()
    };
    let language = Path::new(&args.file)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .and_then(|ext| belisarius_scan::languages::language_for_ext(&ext).map(String::from))
        .unwrap_or_default();
    let resp = SnippetResponse {
        file: args.file,
        language,
        start_line: (start + 1) as u32,
        end_line: end as u32,
        target_line: target as u32,
        total_lines: lines.len() as u32,
        snippet,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn markers(ctx: &AppContext, args: LimitArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let limit = args.limit.unwrap_or(500);
    let hits = tokio::task::spawn_blocking(move || crate::server::scan_markers(&path, limit))
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("markers join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("markers: {e:#}")))?;
    let total = hits.len();
    let truncated = total >= limit;
    let resp = MarkersResponse {
        markers: hits,
        total: total as u32,
        limited: truncated,
        truncated,
        returned: total as u32,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn file_dsm(ctx: &AppContext, args: FileDsmArgs) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&path).await?;
    let scan = &analysis.scan;
    let node = scan
        .files
        .iter()
        .find(|f| f.path == args.file)
        .ok_or_else(|| {
            ServiceError::not_found(format!("file {} not present in the scan", args.file))
        })?;

    let lookup_containing = |edge_from: &str, line: u32| -> Option<String> {
        if line == 0 {
            return None;
        }
        analysis
            .functions
            .iter()
            .find(|f| f.file == edge_from && f.start_line <= line && line <= f.end_line)
            .map(|f| f.name.clone())
    };

    let outbound: Vec<Value> = analysis
        .graph
        .edges
        .iter()
        .filter(|e| e.from == args.file)
        .map(|e| {
            json!({
                "to": e.to,
                "line": e.line,
                "in_function": lookup_containing(&e.from, e.line),
            })
        })
        .collect();

    let inbound: Vec<Value> = analysis
        .graph
        .edges
        .iter()
        .filter(|e| e.to == args.file)
        .map(|e| {
            json!({
                "from": e.from,
                "line": e.line,
                "in_function": lookup_containing(&e.from, e.line),
            })
        })
        .collect();

    let resolved_lines: std::collections::HashSet<u32> = analysis
        .graph
        .edges
        .iter()
        .filter(|e| e.from == args.file)
        .map(|e| e.line)
        .collect();
    let externals: Vec<Value> = scan
        .edges
        .iter()
        .filter(|e| e.from == args.file && !resolved_lines.contains(&e.line))
        .map(|e| {
            json!({
                "spec": e.to,
                "line": e.line,
                "kind": e.kind,
                "in_function": lookup_containing(&e.from, e.line),
            })
        })
        .collect();

    let functions: Vec<Value> = analysis
        .functions
        .iter()
        .filter(|f| f.file == args.file)
        .map(|f| {
            json!({
                "name": f.name,
                "start_line": f.start_line,
                "end_line": f.end_line,
                "loc": f.loc,
                "params": f.params,
                "cyclomatic": f.cyclomatic,
                "cognitive": f.cognitive,
            })
        })
        .collect();

    Ok(json!({
        "file": args.file,
        "language": node.language,
        "loc": node.loc,
        "in_degree": node.loc,
        "out_degree": outbound.len(),
        "outbound": outbound,
        "inbound": inbound,
        "externals": externals,
        "functions": functions,
    }))
}

// ─── MCP tool registrations ──────────────────────────────────────────────

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_scan",
            description: "Walk a project's files and import graph. Returns LOC, language summary, and resolved-edge counts.\n\n\
When to use: you need just the structural data (no metrics, no functions) — cheaper than `belisarius_brief`.\n\
When not to use: starting fresh on a project (use `belisarius_brief` for the richer summary).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_scan as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_functions",
            description: "Functions ranked by complexity. Filterable by `min_cc`, `file`, `sort_by` (cc / cognitive / loc / params).\n\n\
When to use: finding the most complex functions in a file or project; auditing a single file by passing `file=...`.\n\
When not to use: per-function detail (use `belisarius_symbol` once you know the name); cross-project view (use `belisarius_fleet_*` variants).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "min_cc": { "type": "integer", "default": 0 },
                    "limit": { "type": "integer", "default": 200 },
                    "sort_by": { "type": "string", "enum": ["cc", "cognitive", "loc", "params"], "default": "cc" },
                    "file": { "type": "string" }
                }
            }),
            handler: handle_functions as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_hotspots",
            description: "Rank files by churn × complexity over a git history window.\n\n\
When to use: finding where risk is concentrated — files that change often AND are complex are the highest-leverage refactor targets.\n\
When not to use: untested high-risk files (use `belisarius_test_gaps`); a single git range (use `belisarius_explain`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "days": { "type": "integer", "default": 90 },
                    "limit": { "type": "integer", "default": 25 }
                }
            }),
            handler: handle_hotspots as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_test_gaps",
            description: "Source files / functions with no covering test, ranked by complexity.\n\n\
When to use: deciding what to cover first — top entries are the most painful to ship without tests.\n\
When not to use: stub generation (use `belisarius_suggest_tests` once you've picked a target); finding tests for a specific file (use `belisarius_describe` on that file).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "limit": { "type": "integer", "default": 25 }
                }
            }),
            handler: handle_test_gaps as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_diff",
            description: "Files changed between two git refs, overlayed with hotspots / tests / surface flags.\n\n\
When to use: low-level data for PR / range review when you want the raw JSON.\n\
When not to use: human-readable summary with priorities (use `belisarius_explain`, which wraps this).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "base": { "type": "string" },
                    "head": { "type": "string", "default": "HEAD" },
                    "hotspot_window": { "type": "integer", "default": 100 }
                }
            }),
            handler: handle_diff as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_commands",
            description: "Discover runnable commands: package.json scripts, Justfile, Makefile, workflows.\n\n\
When to use: figuring out how to run tests / build / lint in an unfamiliar project so a follow-up shell call works.\n\
When not to use: searching code (use `belisarius_search_code`); reading a specific file (use `belisarius_snippet`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_commands as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_surface",
            description: "Project public API surface: exported symbols, route definitions, schema types.\n\n\
When to use: 'what does this project expose to callers?' — useful for understanding library boundaries before reading internals.\n\
When not to use: per-symbol detail (use `belisarius_symbol`); whole-architecture view (use `belisarius_architecture`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_surface as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_file_dsm",
            description: "Dependency-structure matrix for a single file: inbound / outbound / external edges + functions.\n\n\
When to use: understanding a single file's place in the dependency graph before refactoring.\n\
When not to use: project-level view (use `belisarius_architecture`); call-level traces (use `belisarius_who_calls`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "file"],
                "properties": {
                    "path": { "type": "string" },
                    "file": { "type": "string" }
                }
            }),
            handler: handle_file_dsm as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_snippet",
            description: "Read a code snippet around a target line. `radius` lines on either side (default 25, max 500).\n\n\
When to use: pulling just enough surrounding code to ground an answer without reading the whole file.\n\
When not to use: full files (use `Read`); rich per-file context (use `belisarius_describe`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "file", "line"],
                "properties": {
                    "path": { "type": "string" },
                    "file": { "type": "string", "description": "Relative to the project root." },
                    "line": { "type": "integer", "minimum": 1 },
                    "radius": { "type": "integer", "default": 25, "maximum": 500 }
                }
            }),
            handler: handle_snippet as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_markers",
            description: "TODO / FIXME / HACK / XXX markers across the project's source files.\n\n\
When to use: enumerating outstanding work the codebase itself flags.\n\
When not to use: typed cross-session notes (use `belisarius_recall` / `belisarius_decisions`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "limit": { "type": "integer", "default": 500 }
                }
            }),
            handler: handle_markers as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_components",
            description: "Run react-docgen across the project to extract React component metadata.\n\n\
When to use: surveying React component props / displayNames in a TS/JS project.\n\
When not to use: non-React projects; backend code (returns nothing useful).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_components as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_rules_check",
            description: "Evaluate `.belisarius/rules.toml` against the project. Returns rule pass/fail with detail.\n\n\
When to use: CI gate, or sanity-checking that a refactor doesn't reintroduce architectural violations.\n\
When not to use: getting started with rules (run `belisarius init` to scaffold the file first); CLI use (run `belisarius check` instead — it exits non-zero).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_rules_check as ToolHandler,
        },
    ]
}

fn handle_scan(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        scan(&ctx, args).await
    })
}

fn handle_functions(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: FunctionsArgs = serde_json::from_value(args)?;
        functions(&ctx, args).await
    })
}

fn handle_hotspots(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: HotspotsArgs = serde_json::from_value(args)?;
        hotspots(&ctx, args).await
    })
}

fn handle_test_gaps(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LimitArgs = serde_json::from_value(args)?;
        test_gaps(&ctx, args).await
    })
}

fn handle_diff(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: DiffArgs = serde_json::from_value(args)?;
        diff(&ctx, args).await
    })
}

fn handle_commands(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        commands(&ctx, args).await
    })
}

fn handle_surface(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        surface(&ctx, args).await
    })
}

fn handle_file_dsm(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: FileDsmArgs = serde_json::from_value(args)?;
        file_dsm(&ctx, args).await
    })
}

fn handle_snippet(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SnippetArgs = serde_json::from_value(args)?;
        snippet(&ctx, args).await
    })
}

fn handle_markers(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LimitArgs = serde_json::from_value(args)?;
        markers(&ctx, args).await
    })
}

fn handle_components(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        components(&ctx, args).await
    })
}

fn handle_rules_check(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        rules_check(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small Rust project on disk: N source files, each with one
    /// function. The resulting analysis has exactly N functions and N+1
    /// files (the manifest + N sources), which makes pagination math
    /// trivially checkable.
    fn make_project(n_files: u32) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let mut lib = String::new();
        for i in 0..n_files {
            std::fs::write(
                src.join(format!("m{i}.rs")),
                format!("pub fn f{i}() {{ println!(\"{i}\"); }}\n"),
            )
            .unwrap();
            lib.push_str(&format!("pub mod m{i};\n"));
        }
        std::fs::write(src.join("lib.rs"), lib).unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        tmp
    }

    fn path_str(tmp: &tempfile::TempDir) -> String {
        tmp.path().to_string_lossy().into_owned()
    }

    // ── functions: pagination contract ──────────────────────────────────

    #[tokio::test]
    async fn functions_truncated_signals_total_count() {
        let tmp = make_project(10);
        let ctx = Arc::new(AppContext::new());
        let resp = functions(
            &ctx,
            FunctionsArgs {
                path: path_str(&tmp),
                min_cc: Some(0),
                limit: Some(3),
                file: None,
                sort_by: None,
            },
        )
        .await
        .expect("functions should succeed");
        assert_eq!(
            resp["returned"], 3,
            "must return exactly the requested limit"
        );
        // total_count counts the filtered set (min_cc=0 → everything).
        // 10 source files × 1 fn each = 10 candidates.
        assert!(
            resp["total_count"].as_u64().unwrap() >= 10,
            "total_count must reflect candidates before limit, got {}",
            resp["total_count"]
        );
        assert_eq!(resp["truncated"], true);
    }

    #[tokio::test]
    async fn functions_no_truncation_when_under_limit() {
        let tmp = make_project(3);
        let ctx = Arc::new(AppContext::new());
        let resp = functions(
            &ctx,
            FunctionsArgs {
                path: path_str(&tmp),
                min_cc: Some(0),
                limit: Some(20),
                file: None,
                sort_by: None,
            },
        )
        .await
        .unwrap();
        let returned = resp["returned"].as_u64().unwrap();
        let total = resp["total_count"].as_u64().unwrap();
        assert_eq!(
            returned, total,
            "returned must equal total when under limit"
        );
        assert_eq!(resp["truncated"], false);
    }

    // ── test_gaps: pagination contract ──────────────────────────────────

    #[tokio::test]
    async fn test_gaps_response_carries_truncation_fields() {
        let tmp = make_project(5);
        let ctx = Arc::new(AppContext::new());
        let resp = test_gaps(
            &ctx,
            LimitArgs {
                path: path_str(&tmp),
                limit: Some(2),
            },
        )
        .await
        .expect("test_gaps should succeed");
        // The exact total varies (test_map adds inline tests + library
        // detection), but the response MUST carry the three fields.
        assert!(resp.get("total_count").is_some());
        assert!(resp.get("returned").is_some());
        assert!(resp.get("truncated").is_some());
        let returned = resp["returned"].as_u64().unwrap();
        assert!(returned <= 2, "returned must respect limit, got {returned}");
    }

    // ── markers: legacy + new fields ────────────────────────────────────

    #[tokio::test]
    async fn markers_response_has_both_limited_and_truncated() {
        let tmp = make_project(1);
        // Drop a file with a TODO so the markers count is non-zero.
        std::fs::write(
            tmp.path().join("src").join("todo_note.rs"),
            "// TODO: x\npub fn g() {}\n",
        )
        .unwrap();
        let ctx = Arc::new(AppContext::new());
        let resp = markers(
            &ctx,
            LimitArgs {
                path: path_str(&tmp),
                limit: Some(500),
            },
        )
        .await
        .unwrap();
        // Back-compat: `limited` is still present.
        assert!(
            resp.get("limited").is_some(),
            "legacy `limited` field must stay"
        );
        // New contract: `truncated` mirrors it.
        assert_eq!(
            resp["limited"], resp["truncated"],
            "`truncated` must mirror `limited`"
        );
        // And `returned` matches the array length.
        let n_markers = resp["markers"].as_array().unwrap().len() as u64;
        assert_eq!(resp["returned"], n_markers);
    }

    // ── hotspots: graceful on no-git ────────────────────────────────────

    /// `hotspots` calls `git_stats::collect` internally. A fresh tempdir
    /// has no git repo, which the analyzer reports as `repo_present: false`
    /// rather than an error. The shape contract must hold either way.
    #[tokio::test]
    async fn hotspots_carries_total_count_even_without_git() {
        let tmp = make_project(2);
        let ctx = Arc::new(AppContext::new());
        let resp = hotspots(
            &ctx,
            HotspotsArgs {
                path: path_str(&tmp),
                days: Some(30),
                limit: Some(5),
            },
        )
        .await
        .expect("hotspots should succeed even without git");
        assert!(resp.get("total_count").is_some());
        assert!(resp.get("returned").is_some());
        assert!(resp.get("truncated").is_some());
        // No git → empty hotspot list → not truncated.
        assert_eq!(resp["returned"], 0);
        assert_eq!(resp["truncated"], false);
    }
}
