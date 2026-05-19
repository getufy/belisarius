//! `belisarius_recent_changes` — delta query for agents that come back to a
//! project after a break. Returns which files moved in the recent window,
//! whether they're hotspots, and whether they have tests covering them.
//!
//! Cheap by design: one `git log` walk plus the already-cached analysis.
//! No SCIP, no embeddings.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;

#[derive(Debug, Deserialize)]
pub struct RecentChangesArgs {
    pub path: String,
    /// `7d`, `48h`, `30m`, or ISO 8601. Defaults to `7d`.
    pub since: Option<String>,
    /// Optional path prefix filter (e.g. `crates/belisarius-cli/`).
    pub scope: Option<String>,
    /// Maximum files to return. Default 50.
    pub limit: Option<usize>,
}

pub async fn recent_changes(
    ctx: &AppContext,
    args: RecentChangesArgs,
) -> Result<Value, ServiceError> {
    let path = ctx.resolve_path(&args.path);
    let since = args.since.unwrap_or_else(|| "7d".into());
    let days = parse_window_days(&since).map_err(ServiceError::bad_request)?;
    let limit = args.limit.unwrap_or(50);
    let scope = args.scope.unwrap_or_default();
    let analysis = ctx.load_analysis(&path).await?;

    // ── git churn over the window ────────────────────────────────────────
    let project_for_git = PathBuf::from(&path);
    let stats = tokio::task::spawn_blocking(move || {
        belisarius_scan::git_stats::collect(&project_for_git, days, None)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("git stats join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("git stats: {e:#}")))?;

    // ── hotspot ranking so we can flag `is_hotspot` ──────────────────────
    let hotspot_report =
        belisarius_scan::git_stats::rank_hotspots(&stats, &analysis.file_metrics, 40);
    let hot_paths: std::collections::HashSet<&str> = hotspot_report
        .hotspots
        .iter()
        .map(|h| h.path.as_str())
        .collect();

    // ── test map so we can flag `has_tests` ──────────────────────────────
    let project_for_tests = PathBuf::from(&path);
    let scan_for_tests = analysis.scan.clone();
    let inline = tokio::task::spawn_blocking(move || {
        belisarius_scan::test_map::detect_inline_tests(&project_for_tests, &scan_for_tests)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("test_map join: {e}")))?;
    let test_map = belisarius_scan::test_map::compute(&analysis, &inline);
    let tested_paths: std::collections::HashSet<&str> = test_map
        .mappings
        .iter()
        .map(|m| m.source.as_str())
        .collect();

    // ── join + filter ────────────────────────────────────────────────────
    let mut files: Vec<Value> = stats
        .files
        .iter()
        .filter(|f| scope.is_empty() || f.path.starts_with(&scope))
        .map(|f| {
            json!({
                "path": f.path,
                "commits": f.commits_in_window,
                "total_commits": f.total_commits,
                "last_edited": f.last_edited,
                "last_author": f.last_author,
                "is_hotspot": hot_paths.contains(f.path.as_str()),
                "has_tests": tested_paths.contains(f.path.as_str()),
            })
        })
        .collect();
    // Sort: most recent commits first, then by total churn descending.
    files.sort_by(|a, b| {
        let cb = b["commits"].as_u64().unwrap_or(0);
        let ca = a["commits"].as_u64().unwrap_or(0);
        cb.cmp(&ca)
    });
    let total_count = files.len();
    let truncated = total_count > limit;
    files.truncate(limit);
    let returned = files.len();

    Ok(json!({
        "since": since,
        "scope": if scope.is_empty() { Value::Null } else { Value::String(scope) },
        "days_window": days,
        "repo_present": stats.repo_present,
        "files": files,
        "total_count": total_count,
        "returned": returned,
        "truncated": truncated,
    }))
}

/// Convert a window string (`7d` / `48h` / `30m` / ISO 8601) into a u32 day
/// count usable by `git_stats::collect`. ISO strings are converted to "days
/// from now"; sub-day windows round up to 1.
fn parse_window_days(since: &str) -> Result<u32, String> {
    let trimmed = since.trim();
    if let Some(s) = trimmed.strip_suffix('d') {
        return s.parse().map_err(|_| format!("bad day suffix: {trimmed}"));
    }
    if let Some(s) = trimmed.strip_suffix('h') {
        let hrs: u32 = s
            .parse()
            .map_err(|_| format!("bad hour suffix: {trimmed}"))?;
        return Ok(hrs.div_ceil(24).max(1));
    }
    if let Some(s) = trimmed.strip_suffix('m') {
        let mins: u32 = s
            .parse()
            .map_err(|_| format!("bad minute suffix: {trimmed}"))?;
        return Ok((mins / (60 * 24)).max(1));
    }
    // Try ISO 8601: compute days from that timestamp to now.
    let ts = time::OffsetDateTime::parse(
        trimmed,
        &time::format_description::well_known::Iso8601::DEFAULT,
    )
    .map_err(|e| format!("bad ISO 8601: {e}"))?;
    let dur = time::OffsetDateTime::now_utc() - ts;
    let days = (dur.whole_days() as u32).max(1);
    Ok(days)
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_recent_changes",
        description:
            "List files modified in a recent time window with hotspot + test-coverage status. \
Cheap delta query — backed by `git log` plus the cached analysis, no SCIP needed.\n\n\
When to use: 'what's new since I last looked?' — agents returning to a project after a break. \
Pair with `belisarius_describe` for the files you care about.\n\
When not to use: structural / blast-radius questions (use `belisarius_who_calls`). \
Long historical surveys (use `belisarius_hotspots` with a larger window).",
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "since": {
                    "type": "string",
                    "description": "Window: `7d` / `48h` / `30m` / ISO 8601. Default `7d`."
                },
                "scope": {
                    "type": "string",
                    "description": "Optional path prefix filter (e.g. `crates/belisarius-cli/`)."
                },
                "limit": { "type": "integer", "default": 50, "minimum": 1, "maximum": 500 }
            }
        }),
        handler: handle_recent_changes as ToolHandler,
    }]
}

fn handle_recent_changes(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: RecentChangesArgs = serde_json::from_value(args)?;
        recent_changes(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_days_d_suffix() {
        assert_eq!(parse_window_days("7d").unwrap(), 7);
        assert_eq!(parse_window_days("1d").unwrap(), 1);
    }

    #[test]
    fn parse_window_days_h_rounds_up_to_at_least_one() {
        assert_eq!(parse_window_days("1h").unwrap(), 1);
        assert_eq!(parse_window_days("24h").unwrap(), 1);
        assert_eq!(parse_window_days("48h").unwrap(), 2);
    }

    #[test]
    fn parse_window_days_rejects_garbage() {
        assert!(parse_window_days("xyz").is_err());
        assert!(parse_window_days("7q").is_err());
    }
}
