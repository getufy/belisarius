//! `state_db`-backed capabilities — snapshot / drift / pin / unpin / list_pins.
//!
//! Grouped in one module because they all hit the same per-project SQLite at
//! `.belisarius/state.db`. The plan listed them as separate files; keeping
//! them together keeps the public surface easy to scan (one place for
//! everything snapshot-shaped).

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct PathArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct DriftArgs {
    pub path: String,
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PinArgs {
    pub path: String,
    pub scope: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    pub note: String,
    #[serde(default)]
    pub ttl_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UnpinArgs {
    pub path: String,
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListPinsArgs {
    pub path: String,
    #[serde(default)]
    pub scope: Option<String>,
}

pub async fn snapshot(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let report = ctx.load_analysis(&project).await?;
    let project_owned = project.clone();
    let id = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let conn = crate::state_db::open(Path::new(&project_owned))?;
        crate::state_db::write_snapshot(&conn, &report)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("snapshot join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("snapshot: {e:#}")))?;
    Ok(json!({ "snapshot_id": id }))
}

pub async fn drift(ctx: &AppContext, args: DriftArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let since = args.since.unwrap_or_else(|| "7d".to_string());
    let value = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let conn = crate::state_db::open(Path::new(&project))?;
        let latest = match crate::state_db::latest_snapshot(&conn)? {
            Some(s) => s,
            None => return Ok(json!({
                "reason": "no snapshots captured yet — capture one via /api/snapshot or belisarius_snapshot first",
                "drift": null,
            })),
        };
        let since_iso = crate::state_db::since_iso(&since)?;
        let baseline = crate::state_db::snapshot_at_or_before(&conn, &since_iso)?;
        let baseline = match baseline {
            Some(b) if b.id != latest.id => b,
            _ => return Ok(json!({
                "reason": format!("only one snapshot in or after window `{since}`"),
                "drift": null,
            })),
        };
        let drift = crate::state_db::compute_drift(&baseline, &latest);
        Ok(serde_json::to_value(drift)?)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("drift join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("drift: {e:#}")))?;
    Ok(value)
}

pub async fn pin(ctx: &AppContext, args: PinArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let id = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let conn = crate::state_db::open(Path::new(&project))?;
        crate::state_db::insert_pin(
            &conn,
            &args.scope,
            args.file.as_deref(),
            args.line,
            &args.note,
            args.ttl_days,
        )
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("pin join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("pin: {e:#}")))?;
    Ok(json!({ "id": id }))
}

pub async fn unpin(ctx: &AppContext, args: UnpinArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let id = args.id;
    let removed = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let conn = crate::state_db::open(Path::new(&project))?;
        crate::state_db::delete_pin(&conn, id)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("unpin join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("unpin: {e:#}")))?;
    Ok(json!({ "removed": removed }))
}

pub async fn list_pins(ctx: &AppContext, args: ListPinsArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let scope = args.scope;
    let pins = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = crate::state_db::open(Path::new(&project))?;
        crate::state_db::list_pins(&conn, scope.as_deref())
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("list_pins join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("list_pins: {e:#}")))?;
    Ok(json!({ "count": pins.len(), "pins": pins }))
}

// ─── MCP tool registrations ──────────────────────────────────────────────

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_snapshot",
            description: "Capture the project's quality axes + top hot-function fingerprints to `.belisarius/state.db`. Returns the snapshot id.\n\n\
When to use: before a refactor or sprint boundary — gives `belisarius_drift` something to compare against.\n\
When not to use: every session (snapshots accumulate); transient state (use `belisarius_remember` for typed notes).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_snapshot as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_drift",
            description: "Compare the latest snapshot to one captured at or before `since` (default `7d`). Returns score delta, axis deltas, and functions that crossed the hot threshold.\n\n\
When to use: 'did this change make things better or worse?' across an arbitrary time window.\n\
When not to use: per-commit blame (use `git log` + `belisarius_explain`); first-run projects that haven't called `belisarius_snapshot` yet.",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "since": { "type": "string", "default": "7d", "description": "Relative window (e.g. `7d`, `48h`) or an ISO 8601 timestamp." }
                }
            }),
            handler: handle_drift as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_pin",
            description: "Pin a persistent note keyed to a file/line or the whole project. Returns the pin id.\n\n\
When to use: legacy compatibility — kept so older clients keep working.\n\
When not to use: prefer `belisarius_remember` for new code. It supports typed notes (decision/gotcha/todo/context/hypothesis) and powers `belisarius_recall` / `belisarius_decisions`.",
            input_schema: json!({
                "type": "object",
                "required": ["path", "scope", "note"],
                "properties": {
                    "path": { "type": "string" },
                    "scope": { "type": "string", "enum": ["project", "file", "function"] },
                    "file": { "type": "string" },
                    "line": { "type": "integer", "minimum": 1 },
                    "note": { "type": "string" },
                    "ttl_days": { "type": "integer", "minimum": 1 }
                }
            }),
            handler: handle_pin as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_unpin",
            description: "Delete a pinned note by id. Returns whether anything was removed.\n\n\
When to use: cleaning up after a pin has served its purpose; correcting an erroneous pin.\n\
When not to use: removing a `belisarius_remember` note (separate table — there's no remove tool yet; pins and notes are independent storage).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "id"],
                "properties": {
                    "path": { "type": "string" },
                    "id": { "type": "integer" }
                }
            }),
            handler: handle_unpin as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_list_pins",
            description: "List active pins. Expired pins are filtered out. Optionally narrow by `scope`.\n\n\
When to use: enumerating pins keyed to a file before deciding what to act on.\n\
When not to use: typed notes — use `belisarius_recall` / `belisarius_decisions` which operate on the richer `notes` table.",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "scope": { "type": "string", "enum": ["project", "file", "function"] }
                }
            }),
            handler: handle_list_pins as ToolHandler,
        },
    ]
}

fn handle_snapshot(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        snapshot(&ctx, args).await
    })
}

fn handle_drift(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: DriftArgs = serde_json::from_value(args)?;
        drift(&ctx, args).await
    })
}

fn handle_pin(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PinArgs = serde_json::from_value(args)?;
        pin(&ctx, args).await
    })
}

fn handle_unpin(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: UnpinArgs = serde_json::from_value(args)?;
        unpin(&ctx, args).await
    })
}

fn handle_list_pins(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: ListPinsArgs = serde_json::from_value(args)?;
        list_pins(&ctx, args).await
    })
}
