//! `brief` — one-shot markdown digest of a project: language mix, quality
//! axes, top hotspots, top test gaps, markers, entry points, smallest cycle,
//! and hot functions. The agent's canonical first look.
//!
//! Gathers signals defensively: a missing `.git`, a markers walk failure, or
//! a missing `codeowners` file should NEVER block the brief. The optional
//! signals fall through as `None` / empty without erroring the whole call.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct Args {
    pub path: String,
}

pub async fn brief(ctx: &AppContext, args: Args) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let analysis = ctx.load_analysis(&project).await?;

    let keep: Vec<String> = analysis.scan.files.iter().map(|f| f.path.clone()).collect();
    let project_for_git = PathBuf::from(&project);
    let project_for_owners = project_for_git.clone();
    let metrics = analysis.file_metrics.clone();
    let hotspots = tokio::task::spawn_blocking(move || {
        belisarius_scan::git_stats::collect(&project_for_git, 90, Some(&keep))
            .ok()
            .map(|gs| {
                let mut hs = belisarius_scan::git_stats::rank_hotspots(&gs, &metrics, 10);
                let co = belisarius_scan::codeowners::CodeownersFile::load(&project_for_owners);
                belisarius_scan::git_stats::attach_owners(&mut hs, co.as_ref());
                hs
            })
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("brief join (hotspots): {e}")))?;

    let scan = analysis.scan.clone();
    let project_for_inline = PathBuf::from(&project);
    let inline = tokio::task::spawn_blocking(move || {
        belisarius_scan::test_map::detect_inline_tests(&project_for_inline, &scan)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("brief join (tests): {e}")))?;
    let mut test_map = belisarius_scan::test_map::compute(&analysis, &inline);
    if test_map.gaps.len() > 10 {
        test_map.gaps.truncate(10);
    }

    let project_for_markers = project.clone();
    let markers = tokio::task::spawn_blocking(move || {
        // `scan_markers` still lives in server.rs — it's planned for a future
        // `service::project::markers` slice. Both transports already share it
        // through this path.
        crate::server::scan_markers(&project_for_markers, 500).unwrap_or_default()
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("brief join (markers): {e}")))?;

    let brief = crate::brief::compose(
        &project,
        &analysis,
        hotspots.as_ref(),
        Some(&test_map),
        &markers,
    );
    serde_json::to_value(brief).map_err(Into::into)
}

pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: "belisarius_brief",
        description: "One-shot ~1-2 KB markdown digest covering language mix, quality score with axes, top hotspots, top test gaps, markers, entry points, smallest cycle, and hot functions.\n\n\
When to use: as the first call on an unfamiliar project — the cheapest way to build a working mental model.\n\
When not to use: targeted lookups (use `belisarius_describe` for a single file, `belisarius_hotspots` for ranking, `belisarius_next_action` for a punch list).",
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": { "path": { "type": "string" } }
        }),
        handler: tool_handler as ToolHandler,
    }
}

fn tool_handler(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: Args = serde_json::from_value(args)?;
        brief(&ctx, args).await
    })
}
