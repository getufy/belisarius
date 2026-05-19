//! Hybrid semantic + BM25 search over a project's chunks.
//!
//! Pre-existing inconsistency fixed: HTTP's `search` cache used
//! `std::sync::Mutex`; MCP used `tokio::sync::Mutex`. The shared
//! `AppContext::open_search` is the async version — search reindex is async
//! work and the HTTP-side sync mutex used to risk runtime stalls on
//! contention. `IndexHandle::open` itself is permissive: an unindexed
//! project returns an idle handle (chunk_count=0), not an error.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct QueryArgs {
    pub path: String,
    /// Accept both `query` (MCP) and `q` (HTTP) so request bodies are
    /// interchangeable across surfaces.
    #[serde(alias = "q")]
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PathArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ReindexArgs {
    pub path: String,
    #[serde(default)]
    pub full: Option<bool>,
    #[serde(default)]
    pub bm25_only: Option<bool>,
}

pub async fn query(ctx: &AppContext, args: QueryArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let handle = ctx.open_search(&project).await?;
    let limit = args.limit.unwrap_or(20);
    let opts = belisarius_search::SearchOptions {
        limit,
        lang: args.lang,
        kind: args.kind,
        candidates: 50,
    };
    let query_text = args.query.clone();
    let hits = tokio::task::spawn_blocking(move || {
        belisarius_search::search::search(&handle, &query_text, &opts)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("search join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("search: {e:#}")))?;
    let returned = hits.len();
    // Candidate pool is fixed at 50; if we filled the requested limit, more
    // ranked matches likely exist beyond the cutoff. Score-based, not exact.
    let truncated = returned >= limit;
    Ok(json!({
        "hits": hits,
        "count": returned,
        "returned": returned,
        "truncated": truncated,
    }))
}

pub async fn status(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let handle = ctx.open_search(&project).await?;
    serde_json::to_value(handle.status_snapshot()).map_err(Into::into)
}

pub async fn reindex(ctx: &AppContext, args: ReindexArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let handle = ctx.open_search(&project).await?;
    let opts = belisarius_search::index::ReindexOptions {
        full: args.full.unwrap_or(false),
        bm25_only: args.bm25_only.unwrap_or(false),
    };
    // Fire-and-forget background reindex — mirrors the original HTTP/MCP
    // behaviour: callers get the current snapshot immediately and poll
    // status/reindex progress via the handle's status.json on disk.
    let bg_handle = handle.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = bg_handle.reindex(opts) {
            tracing::error!("reindex error: {e:#}");
        }
    });
    serde_json::to_value(handle.status_snapshot()).map_err(Into::into)
}

// ─── MCP tool registrations ──────────────────────────────────────────────

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_search_code",
            description: "Hybrid semantic + BM25 search over project chunks. Returns ranked code spans with file + line range + score.\n\n\
When to use: finding code by intent (`where do we parse SCIP?`) without knowing exact names — the primary discovery tool before reading files.\n\
When not to use: exact-symbol lookups (use `belisarius_search_symbols`); finding a file similar to one you already have (use `belisarius_similar`).\n\n\
Requires `belisarius index --with-search <path>` first; check `belisarius_index_status` if you get zero hits.",
            input_schema: json!({
                "type": "object",
                "required": ["path", "query"],
                "properties": {
                    "path": { "type": "string" },
                    "query": { "type": "string", "description": "Natural-language or keyword query." },
                    "limit": { "type": "integer", "default": 20 },
                    "lang": { "type": "string", "description": "Filter by language (e.g. `rust`, `typescript`)." },
                    "kind": { "type": "string", "description": "Filter by chunk kind (e.g. `function`, `class`)." }
                }
            }),
            handler: handle_query as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_index_status",
            description: "Snapshot of the hybrid-search index: state, chunk count, embedding model.\n\n\
When to use: confirming the search index is built before calling `belisarius_search_code` / `belisarius_similar`; sanity-checking after `belisarius_reindex`.\n\
When not to use: SCIP / SCIP-symbol questions (different index — use `belisarius_doctor` for a unified view).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
            handler: handle_status as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_reindex",
            description: "Trigger a background reindex of the hybrid-search index. Returns the current status; the actual work happens off-thread.\n\n\
When to use: forcing a fresh build after large code changes; switching the embedding model.\n\
When not to use: incremental updates during active development (start `belisarius watch` instead — it reindexes on save automatically).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "full": { "type": "boolean", "default": false },
                    "bm25_only": { "type": "boolean", "default": false }
                }
            }),
            handler: handle_reindex as ToolHandler,
        },
    ]
}

fn handle_query(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: QueryArgs = serde_json::from_value(args)?;
        query(&ctx, args).await
    })
}

fn handle_status(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: PathArgs = serde_json::from_value(args)?;
        status(&ctx, args).await
    })
}

fn handle_reindex(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: ReindexArgs = serde_json::from_value(args)?;
        reindex(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh project should get an "idle" status without erroring — both
    /// HTTP and MCP relied on this behavior pre-migration. If we accidentally
    /// tighten `open_search` to fail on missing indexes, this test catches it
    /// before the UI starts showing "Precondition Failed" toasts.
    #[tokio::test]
    async fn status_returns_idle_for_unindexed_project() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = Arc::new(AppContext::new());
        let snapshot = status(
            &ctx,
            PathArgs {
                path: tmp.path().to_string_lossy().into_owned(),
            },
        )
        .await
        .expect("status should not fail on a never-indexed project");
        assert_eq!(snapshot.get("state").and_then(|v| v.as_str()), Some("idle"));
        assert_eq!(
            snapshot.get("chunk_count").and_then(|v| v.as_u64()),
            Some(0)
        );
    }

    /// Both transports route through the registry handler. They MUST produce
    /// byte-identical JSON for the same call, or the migration regressed.
    #[tokio::test]
    async fn http_and_mcp_status_agree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_string_lossy().into_owned();
        let ctx = Arc::new(AppContext::new());
        let direct = status(&ctx, PathArgs { path: path.clone() })
            .await
            .expect("direct status");
        let spec = tool_specs()
            .into_iter()
            .find(|s| s.name == "belisarius_index_status")
            .expect("tool registered");
        let via_mcp = (spec.handler)(ctx.clone(), serde_json::json!({ "path": path }))
            .await
            .expect("mcp status");
        assert_eq!(direct, via_mcp);
    }
}
