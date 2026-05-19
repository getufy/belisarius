//! `belisarius_explain` — friendlier `diff` for a git ref or range.
//!
//! Wraps `service::project::diff` and adds:
//!   - a top-line summary (N files, N hotspots touched, N untested)
//!   - sensible default range (`HEAD~5..HEAD` when caller doesn't specify)
//!   - prioritized list of "what changed and why it might matter"
//!
//! Cheap by reuse — every input the agent supplies maps to a `DiffArgs`.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;
use crate::service::project::{diff, DiffArgs};

#[derive(Debug, Deserialize)]
pub struct ExplainArgs {
    pub path: String,
    /// Base ref. Defaults to `HEAD~5`.
    pub base: Option<String>,
    /// Head ref. Defaults to `HEAD`.
    pub head: Option<String>,
    /// Hotspot lookback window (commits). Defaults to 100.
    pub hotspot_window: Option<usize>,
}

pub async fn explain(ctx: &AppContext, args: ExplainArgs) -> Result<Value, ServiceError> {
    let base = args.base.unwrap_or_else(|| "HEAD~5".to_string());
    let head = args.head.unwrap_or_else(|| "HEAD".to_string());
    let diff_args = DiffArgs {
        path: args.path.clone(),
        base: Some(base.clone()),
        head: Some(head.clone()),
        hotspot_window: args.hotspot_window,
    };
    let diff_report = diff(ctx, diff_args).await?;

    // Pull out structural facts. The diff service returns a JSON object whose
    // exact shape we treat as opaque except for the fields we surface here.
    let changed_files: Vec<Value> = diff_report
        .get("files")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let total_files = changed_files.len();
    let mut hot_touched = 0usize;
    let mut untested = 0usize;
    let mut top_changes: Vec<Value> = Vec::new();

    for f in &changed_files {
        let is_hot = f
            .get("is_hotspot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_tests = f
            .get("has_tests")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_hot {
            hot_touched += 1;
        }
        if !has_tests {
            untested += 1;
        }
    }

    // Top changes: hot first, then untested-but-non-trivial, capped at 10.
    let mut prioritized: Vec<&Value> = changed_files.iter().collect();
    prioritized.sort_by(|a, b| {
        let ah = a
            .get("is_hotspot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let bh = b
            .get("is_hotspot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let at = a.get("has_tests").and_then(|v| v.as_bool()).unwrap_or(true);
        let bt = b.get("has_tests").and_then(|v| v.as_bool()).unwrap_or(true);
        bh.cmp(&ah).then_with(|| at.cmp(&bt))
    });
    for f in prioritized.iter().take(10) {
        top_changes.push((*f).clone());
    }

    Ok(json!({
        "base": base,
        "head": head,
        "summary": {
            "files_changed": total_files,
            "hotspots_touched": hot_touched,
            "untested_files_changed": untested,
        },
        "top_changes": top_changes,
        "raw": diff_report,
        "next_steps": [
            "`belisarius_describe` on any file with a surprising change",
            "`belisarius_who_calls` on symbols in the changed files to gauge blast radius",
            "`belisarius_test_gaps` to find which untested files need coverage",
        ],
    }))
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_explain",
        description: "Summarize the impact of a git commit or range. Wraps the raw `diff` \
data with: files-changed count, hotspots touched, untested files changed, and a top-10 list \
prioritized by hotness × test gap.\n\n\
When to use: 'what landed in this PR / since main / over the last N commits?' before reviewing.\n\
When not to use: line-level diffs (`git diff`); structural blast-radius (use `belisarius_who_calls`).",
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string", "description": "Project root." },
                "base": { "type": "string", "description": "Base ref. Default `HEAD~5`." },
                "head": { "type": "string", "description": "Head ref. Default `HEAD`." },
                "hotspot_window": {
                    "type": "integer",
                    "description": "Commit window for hotspot scoring. Default 100."
                }
            }
        }),
        handler: handle_explain as ToolHandler,
    }]
}

fn handle_explain(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: ExplainArgs = serde_json::from_value(args)?;
        explain(&ctx, args).await
    })
}
