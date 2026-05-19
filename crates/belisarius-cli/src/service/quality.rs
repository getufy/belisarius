//! `quality` — composite 0-100 code-quality score with axis breakdown.
//!
//! Implements both surfaces' previous behavior in one place:
//!  - HTTP's `quality_now` (cached analysis)
//!  - MCP's `tool_quality` (was cache-less, gains caching as a bonus)
//!
//! Both transports now share the fleet-aware path resolution that MCP had.

use belisarius_core::Quality;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::mcp::registry::{ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct QualityArgs {
    pub path: String,
}

/// `GET /api/quality` / `belisarius_quality` response shape.
///
/// Composite quality score with axis breakdown, plus the project counts an
/// agent typically wants in the same call so it doesn't have to re-fetch
/// the full analysis report for cycles / depth / fn count / file count.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct QualityResponse {
    pub quality: Quality,
    pub cycles_count: u32,
    pub max_depth: u32,
    pub function_count: u32,
    pub file_count: u32,
}

pub async fn quality(ctx: &AppContext, args: QualityArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let report = ctx.load_analysis(&project).await?;
    let resp = QualityResponse {
        quality: report.quality.clone(),
        cycles_count: report.cycles.len() as u32,
        max_depth: report.max_depth,
        function_count: report.functions.len() as u32,
        file_count: report.scan.files.len() as u32,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: "belisarius_quality",
        description: "Composite 0-100 quality score over four axes — complexity, acyclicity, dead code, fan balance — plus cycle / depth / function / file counts.\n\n\
When to use: gauging a project's overall structural health, or comparing two points in time (pair with `belisarius_snapshot` + `belisarius_drift`).\n\
When not to use: ranking individual files (use `belisarius_hotspots` / `belisarius_functions`); finding what to do (use `belisarius_next_action`).",
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string", "description": "Project root (or fleet name)." }
            }
        }),
        handler: tool_handler as ToolHandler,
    }
}

fn tool_handler(
    ctx: std::sync::Arc<AppContext>,
    args: Value,
) -> crate::mcp::registry::BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: QualityArgs = serde_json::from_value(args)?;
        quality(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Migration parity: the HTTP route handler and the MCP tool handler both
    /// route through `quality(&ctx, args)`, so their JSON output must be
    /// byte-identical. If a future change adds wrapping or post-processing in
    /// only one transport, this test breaks first.
    #[tokio::test]
    async fn http_and_mcp_produce_identical_json() {
        let ctx = Arc::new(AppContext::new());
        let args = QualityArgs {
            path: ".".to_string(),
        };
        let direct = quality(&ctx, args)
            .await
            .expect("service call must succeed");

        let spec = tool_spec();
        let mcp = (spec.handler)(ctx.clone(), serde_json::json!({ "path": "." }))
            .await
            .expect("MCP handler must succeed");

        assert_eq!(direct, mcp);
    }

    #[tokio::test]
    async fn response_has_expected_shape() {
        let ctx = Arc::new(AppContext::new());
        let v = quality(
            &ctx,
            QualityArgs {
                path: ".".to_string(),
            },
        )
        .await
        .expect("service call must succeed");
        for key in [
            "quality",
            "cycles_count",
            "max_depth",
            "function_count",
            "file_count",
        ] {
            assert!(v.get(key).is_some(), "missing key in response: {key}");
        }
    }
}
