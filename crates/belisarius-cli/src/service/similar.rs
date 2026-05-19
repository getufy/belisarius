//! `belisarius_similar` — find code semantically similar to a target file.
//!
//! Cheap MVP: read up to N bytes of the target file, feed it to the hybrid
//! search index as the query, then strip the target itself out of the
//! results. The AST-shape fingerprint variant (Phase 4 follow-up) layers on
//! top of this once we add a tree-sitter pass per file.
//!
//! Returns a ranked list with similarity scores from the search backend.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;

/// Cap the query body fed to the search index. Larger payloads dilute the
/// BM25 score and bloat embedding compute time. 4 KB is plenty for the
/// "what files look like this one" signal.
const QUERY_BYTES_CAP: usize = 4096;

#[derive(Debug, Deserialize)]
pub struct SimilarArgs {
    pub path: String,
    /// Relative file path under the project root.
    pub target: String,
    /// Max results. Default 10.
    pub limit: Option<usize>,
    /// Filter to a single language. Default: any.
    pub lang: Option<String>,
}

pub async fn similar(ctx: &AppContext, args: SimilarArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let target = args.target.trim();
    let resolved = Path::new(&project_root).join(target);
    if !resolved.is_file() {
        return Err(ServiceError::not_found(format!(
            "target file `{target}` not found under `{project_root}`"
        )));
    }
    let mut content = std::fs::read_to_string(&resolved)
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("read target: {e:#}")))?;
    if content.len() > QUERY_BYTES_CAP {
        content.truncate(QUERY_BYTES_CAP);
    }
    if content.trim().is_empty() {
        return Err(ServiceError::bad_request(format!(
            "target file `{target}` is empty"
        )));
    }

    // Reuse the search service so the ranking stays consistent with
    // `belisarius_search_code`.
    let limit = args.limit.unwrap_or(10);
    let probe_limit = (limit + 5).max(20); // overshoot so self-filtering still leaves `limit` matches
    let search_args = crate::service::search::QueryArgs {
        path: args.path.clone(),
        query: content,
        limit: Some(probe_limit),
        lang: args.lang.clone(),
        kind: None,
    };
    let raw = crate::service::search::query(ctx, search_args).await?;

    // The search response is `{ hits: [...], count, returned, truncated }`.
    let hits = raw
        .get("hits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut filtered: Vec<Value> = hits
        .into_iter()
        .filter(|h| {
            h.get("path")
                .and_then(|p| p.as_str())
                .map(|p| p != target)
                .unwrap_or(true)
        })
        .collect();
    filtered.truncate(limit);
    let returned = filtered.len();
    Ok(json!({
        "target": target,
        "matches": filtered,
        "total_count": returned,
        "returned": returned,
        "truncated": returned >= limit,
        "next_steps": [
            "`belisarius_describe` any match that looks like a duplicate candidate",
            "`belisarius_who_calls` to confirm whether two similar files are actually invoked from the same place",
        ],
    }))
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_similar",
        description: "Find code semantically similar to a target file. Backed by the hybrid \
search index — the target's content (up to 4 KB) is the query, the target itself is filtered \
out of the result.\n\n\
When to use: finding duplicate-ish code, locating siblings before refactoring shared logic, \
spotting drift between supposedly-parallel modules.\n\
When not to use: exact-string search (use `belisarius_search_code` with a literal); \
structural-only matching at the AST level (a planned follow-up adds AST fingerprints).",
        input_schema: json!({
            "type": "object",
            "required": ["path", "target"],
            "properties": {
                "path": { "type": "string" },
                "target": { "type": "string", "description": "Relative file path." },
                "limit": { "type": "integer", "default": 10, "maximum": 100 },
                "lang": { "type": "string" }
            }
        }),
        handler: handle_similar as ToolHandler,
    }]
}

fn handle_similar(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SimilarArgs = serde_json::from_value(args)?;
        similar(&ctx, args).await
    })
}
