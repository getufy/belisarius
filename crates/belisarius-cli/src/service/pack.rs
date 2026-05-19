//! `pack` — token-budgeted snippet pack for an LLM context window. Selects
//! the most useful chunks (hot functions, fan-in centers, recent churn) and
//! truncates to a budget.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct Args {
    pub path: String,
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    #[serde(default)]
    pub focus: Option<String>,
}

pub async fn pack(ctx: &AppContext, args: Args) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let report = ctx.load_analysis(&project).await?;
    let budget = args.budget_tokens.unwrap_or(4000);
    let focus = args.focus;
    let project_owned = project.clone();
    let pack = tokio::task::spawn_blocking(move || {
        crate::pack::compose(&project_owned, &report, budget, focus.as_deref())
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("pack join: {e}")))?
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("pack: {e:#}")))?;
    serde_json::to_value(pack).map_err(Into::into)
}

pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: "belisarius_pack",
        description: "Token-budgeted snippet pack — selects the most informative code chunks for an LLM context. Returns markdown.\n\n\
When to use: building a focused context window for a follow-up reasoning step. Pass `focus` to anchor the selection.\n\
When not to use: discovery (use `belisarius_brief` or `belisarius_describe`); reading a single file (use `belisarius_snippet`).",
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "budget_tokens": { "type": "integer", "default": 4000 },
                "focus": { "type": "string", "description": "Optional focus (file path or function name) to anchor the pack." }
            }
        }),
        handler: tool_handler as ToolHandler,
    }
}

fn tool_handler(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: Args = serde_json::from_value(args)?;
        pack(&ctx, args).await
    })
}
