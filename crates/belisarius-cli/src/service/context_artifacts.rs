//! Context artifacts — non-code knowledge (schemas, runbooks, API specs)
//! registered in `.belisarius/context_artifacts.json` and indexed alongside
//! source for semantic discovery.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct ListArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct GetArgs {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    pub path: String,
    #[serde(alias = "q")]
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn list(ctx: &AppContext, args: ListArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let project_path = PathBuf::from(&project);
    let registry = tokio::task::spawn_blocking(move || {
        belisarius_context::ContextRegistry::load(&project_path)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("context list join: {e}")))?
    .map_err(|e| ServiceError::bad_request(format!("load registry: {e:#}")))?;
    Ok(json!({ "artifacts": registry.artifacts, "count": registry.artifacts.len() }))
}

pub async fn get(ctx: &AppContext, args: GetArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let project_path = PathBuf::from(&project);
    let name = args.name;
    let content = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let registry = belisarius_context::ContextRegistry::load(&project_path)?;
        registry.read_artifact(&project_path, &name)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("context get join: {e}")))?
    .map_err(|e| ServiceError::not_found(format!("artifact: {e:#}")))?;
    serde_json::to_value(content).map_err(Into::into)
}

pub async fn search(ctx: &AppContext, args: SearchArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let handle = ctx.open_search(&project).await?;
    let limit = args.limit.unwrap_or(10);
    let query = args.query.clone();
    let hits = tokio::task::spawn_blocking(move || {
        belisarius_context::search_artifacts(&handle, &query, limit)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("context search join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("context search: {e:#}")))?;
    Ok(json!({ "hits": hits, "count": hits.len() }))
}

pub async fn index(ctx: &AppContext, args: ListArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let handle = ctx.open_search(&project).await?;
    let n = tokio::task::spawn_blocking(move || belisarius_context::index_registry(&handle))
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("context index join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("index registry: {e:#}")))?;
    Ok(json!({ "indexed_chunks": n }))
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_context_list",
            description: "List registered context artifacts (schemas, runbooks, API specs).\n\n\
When to use: discovering non-code knowledge a project's maintainers have registered for agents to consult.\n\
When not to use: searching code (use `belisarius_search_code`); ad-hoc files outside `.belisarius/context_artifacts.json`.",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_list as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_context_get",
            description: "Resolve an artifact's globs and read its files.\n\n\
When to use: pulling the full text of a registered artifact (schema, runbook) after `belisarius_context_list` told you it exists.\n\
When not to use: semantic search across artifacts (use `belisarius_context_search`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "name"],
                "properties": {
                    "path": { "type": "string" },
                    "name": { "type": "string", "description": "Artifact name as listed by belisarius_context_list." }
                }
            }),
            handler: handle_get as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_context_search",
            description: "Semantic search over indexed context artifact chunks.\n\n\
When to use: 'is there a runbook / schema covering X?' across all registered artifacts.\n\
When not to use: code search (use `belisarius_search_code`); fetching a known artifact in full (use `belisarius_context_get`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "query"],
                "properties": {
                    "path": { "type": "string" },
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "default": 10 }
                }
            }),
            handler: handle_search as ToolHandler,
        },
    ]
}

fn handle_list(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: ListArgs = serde_json::from_value(args)?;
        list(&ctx, args).await
    })
}

fn handle_get(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: GetArgs = serde_json::from_value(args)?;
        get(&ctx, args).await
    })
}

fn handle_search(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SearchArgs = serde_json::from_value(args)?;
        search(&ctx, args).await
    })
}
