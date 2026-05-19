//! `function_detail` — per-function bundle: metrics + source + churn +
//! tests + SCIP callers. Used internally by `pack` to assemble token-budgeted
//! snippet packs; also exposed as an HTTP endpoint for the web UI. The
//! standalone MCP tool was removed — `belisarius_symbol` is the canonical
//! 360° view for agents.

use serde::Deserialize;
use serde_json::Value;

use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct Args {
    pub path: String,
    pub file: String,
    pub name: String,
}

pub async fn function_detail(ctx: &AppContext, args: Args) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&project).await?;
    let project_owned = project.clone();
    let file = args.file.clone();
    let name = args.name.clone();
    let detail = tokio::task::spawn_blocking(move || {
        crate::function_detail::compose(&project_owned, &analysis, &file, &name)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("function_detail join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("function_detail: {e:#}")))?;
    serde_json::to_value(detail).map_err(Into::into)
}
