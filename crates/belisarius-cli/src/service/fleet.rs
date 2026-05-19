//! Fleet registry capabilities: list / info / find / hotspots / test_gaps /
//! surface_diff. These don't need the project caches in `AppContext` — they
//! query the global SQLite at `~/.belisarius/fleet.db` plus the TOML config
//! at `~/.belisarius/fleet.toml`. We still route through `AppContext` so the
//! MCP registry has one consistent handler signature.
//!
//! Behaviour change folded in: MCP's `tool_fleet_list` silently coerced any
//! load failure (including a corrupt `fleet.toml`) to an empty list via
//! `unwrap_or_default()`. The unified service surfaces the parse error like
//! HTTP did — a corrupt config now produces a real diagnostic instead of an
//! empty fleet that's hard to tell apart from "no apps registered".

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct ListArgs {}

#[derive(Debug, Deserialize)]
pub struct InfoArgs {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct FindArgs {
    pub pattern: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LimitArgs {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DiffArgs {
    pub from: String,
    pub to: String,
}

pub async fn list(_ctx: &AppContext, _args: ListArgs) -> Result<Value, ServiceError> {
    let cfg_path = crate::fleet::default_config_path();
    let cfg = crate::fleet::load(&cfg_path)
        .map_err(|e| ServiceError::bad_request(format!("loading fleet config: {e:#}")))?;
    Ok(json!({
        "config_path": cfg_path.display().to_string(),
        "apps": cfg.apps,
    }))
}

pub async fn info(_ctx: &AppContext, args: InfoArgs) -> Result<Value, ServiceError> {
    let cfg = crate::fleet::load(&crate::fleet::default_config_path())
        .map_err(|e| ServiceError::bad_request(format!("loading fleet: {e:#}")))?;
    let app = crate::fleet::find_app(&cfg, &args.name)
        .ok_or_else(|| ServiceError::not_found(format!("no app named {:?}", args.name)))?;
    serde_json::to_value(app).map_err(Into::into)
}

pub async fn find(_ctx: &AppContext, args: FindArgs) -> Result<Value, ServiceError> {
    let conn = crate::fleet_db::open(&crate::fleet_db::default_db_path())
        .map_err(|e| ServiceError::bad_request(format!("open db: {e:#}")))?;
    let rows = crate::fleet_db::find_surface(
        &conn,
        args.kind.as_deref(),
        Some(&args.pattern),
        args.limit.unwrap_or(200),
    )
    .map_err(|e| ServiceError::bad_request(format!("find: {e:#}")))?;
    Ok(json!({ "count": rows.len(), "results": rows }))
}

pub async fn hotspots(_ctx: &AppContext, args: LimitArgs) -> Result<Value, ServiceError> {
    let conn = crate::fleet_db::open(&crate::fleet_db::default_db_path())
        .map_err(|e| ServiceError::bad_request(format!("open db: {e:#}")))?;
    let rows = crate::fleet_db::top_hotspots(&conn, args.limit.unwrap_or(25))
        .map_err(|e| ServiceError::bad_request(format!("hotspots: {e:#}")))?;
    Ok(json!({ "count": rows.len(), "hotspots": rows }))
}

pub async fn test_gaps(_ctx: &AppContext, args: LimitArgs) -> Result<Value, ServiceError> {
    let conn = crate::fleet_db::open(&crate::fleet_db::default_db_path())
        .map_err(|e| ServiceError::bad_request(format!("open db: {e:#}")))?;
    let rows = crate::fleet_db::top_test_gaps(&conn, args.limit.unwrap_or(25))
        .map_err(|e| ServiceError::bad_request(format!("test_gaps: {e:#}")))?;
    Ok(json!({ "count": rows.len(), "gaps": rows }))
}

pub async fn surface_diff(_ctx: &AppContext, args: DiffArgs) -> Result<Value, ServiceError> {
    let conn = crate::fleet_db::open(&crate::fleet_db::default_db_path())
        .map_err(|e| ServiceError::bad_request(format!("open db: {e:#}")))?;
    let diff = crate::fleet_db::surface_diff(&conn, &args.from, &args.to)
        .map_err(|e| ServiceError::bad_request(format!("diff: {e:#}")))?;
    serde_json::to_value(diff).map_err(Into::into)
}

// ─── MCP tool registrations ──────────────────────────────────────────────

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_fleet_list",
            description: "List every project registered with `belisarius fleet add`. Returns config path + apps array.\n\n\
When to use: discovering which projects belong to the fleet before drilling in.\n\
When not to use: single-project analysis (every non-fleet tool accepts a `path` arg directly).",
            input_schema: json!({ "type": "object", "properties": {} }),
            handler: handle_list as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_fleet_find",
            description: "Find functions / classes / interfaces across the entire fleet's surface index. Pattern is a substring match.\n\n\
When to use: 'where in any of our apps does X exist?' — useful for cross-app symbol discovery.\n\
When not to use: single-project lookups (use `belisarius_search_symbols`); fuzzy semantic search across apps (no fleet-wide equivalent yet).",
            input_schema: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "kind": { "type": "string", "description": "Filter by symbol kind (e.g. `function`, `class`)." },
                    "limit": { "type": "integer", "default": 200 }
                }
            }),
            handler: handle_find as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_fleet_hotspots",
            description: "Top fleet-wide churn × complexity hotspots from the surface index.\n\n\
When to use: prioritizing across apps in a monorepo / multi-project setup.\n\
When not to use: single-project hotspot list (use `belisarius_hotspots`).",
            input_schema: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "default": 25 } }
            }),
            handler: handle_hotspots as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_fleet_test_gaps",
            description: "Top fleet-wide functions with no covering test, ranked by complexity.\n\n\
When to use: setting coverage priorities across apps.\n\
When not to use: single-project gap list (use `belisarius_test_gaps`).",
            input_schema: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "default": 25 } }
            }),
            handler: handle_test_gaps as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_fleet_diff",
            description: "Diff the public surface of two fleet apps. Returns added / removed / changed symbols between `from` and `to`.\n\n\
When to use: checking whether two apps drifted apart on a shared API.\n\
When not to use: single-app diffs across git refs (use `belisarius_diff` / `belisarius_explain`).",
            input_schema: json!({
                "type": "object",
                "required": ["from", "to"],
                "properties": {
                    "from": { "type": "string", "description": "Source app name." },
                    "to": { "type": "string", "description": "Target app name." }
                }
            }),
            handler: handle_diff as ToolHandler,
        },
    ]
}

fn handle_list(ctx: Arc<AppContext>, _args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move { list(&ctx, ListArgs {}).await })
}

fn handle_find(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: FindArgs = serde_json::from_value(args)?;
        find(&ctx, args).await
    })
}

fn handle_hotspots(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LimitArgs = serde_json::from_value(args).unwrap_or(LimitArgs { limit: None });
        hotspots(&ctx, args).await
    })
}

fn handle_test_gaps(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LimitArgs = serde_json::from_value(args).unwrap_or(LimitArgs { limit: None });
        test_gaps(&ctx, args).await
    })
}

fn handle_diff(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: DiffArgs = serde_json::from_value(args)?;
        surface_diff(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTP `/api/fleet` and MCP `belisarius_fleet_list` must produce the same
    /// JSON. The shared `service::fleet::list` is the single implementation;
    /// this test guards against a regression where someone wraps it differently
    /// on one transport.
    #[tokio::test]
    async fn list_http_and_mcp_agree() {
        // Use a tempdir as the fleet config home so we don't depend on the
        // developer's real fleet.toml.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("BELISARIUS_FLEET", tmp.path().join("fleet.toml"));

        let ctx = Arc::new(AppContext::new());
        let direct = list(&ctx, ListArgs {}).await.expect("direct list");
        let spec = tool_specs()
            .into_iter()
            .find(|s| s.name == "belisarius_fleet_list")
            .expect("tool registered");
        let via_mcp = (spec.handler)(ctx.clone(), serde_json::json!({}))
            .await
            .expect("mcp list");

        assert_eq!(direct, via_mcp);
        assert_eq!(
            direct
                .get("apps")
                .and_then(|v| v.as_array())
                .map(|v| v.len()),
            Some(0)
        );

        std::env::remove_var("BELISARIUS_FLEET");
    }
}
