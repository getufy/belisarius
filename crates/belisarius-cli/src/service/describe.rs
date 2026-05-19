//! `belisarius_describe` — single rich "explain this" returning the right
//! slice for the target type. Saves agents from chaining 5 tools to build
//! a mental model of a file or directory.
//!
//! Target resolution:
//!   - looks like a SCIP symbol (contains `^` or `#`) → delegates to
//!     `belisarius_symbol`-style 360° view (defer to existing tool — we
//!     return a hint string for now to keep the surface small).
//!   - filesystem directory → directory summary (files, languages, hotspots).
//!   - filesystem file → file summary (functions, metrics, tests, recent
//!     commits).

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;

#[derive(Debug, Deserialize)]
pub struct DescribeArgs {
    pub path: String,
    /// File path (relative to project), directory path, or SCIP symbol id.
    pub target: String,
}

pub async fn describe(ctx: &AppContext, args: DescribeArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let target = args.target.trim();

    // SCIP symbol IDs typically contain '^' (definition marker) or '#'.
    // Hand-off path: the caller should use belisarius_symbol directly.
    if target.contains('^') || target.contains('#') {
        return Ok(json!({
            "kind": "symbol",
            "target": target,
            "hint": "use `belisarius_symbol` with this `symbol` argument for the canonical 360° view",
        }));
    }

    let resolved = Path::new(&project_root).join(target);
    if resolved.is_dir() {
        return describe_dir(ctx, &project_root, target).await;
    }
    if resolved.is_file() {
        return describe_file(ctx, &project_root, target).await;
    }
    Err(ServiceError::not_found(format!(
        "target `{target}` not found under project root `{project_root}`"
    )))
}

async fn describe_file(
    ctx: &AppContext,
    project_root: &str,
    rel_path: &str,
) -> Result<Value, ServiceError> {
    let analysis = ctx.load_analysis(project_root).await?;

    let file_info = analysis.scan.files.iter().find(|f| f.path == rel_path);
    let file_metrics = analysis.file_metrics.iter().find(|m| m.path == rel_path);

    // Functions defined in this file, sorted by complexity descending.
    let mut functions: Vec<&belisarius_core::FunctionInfo> = analysis
        .functions
        .iter()
        .filter(|f| f.file == rel_path)
        .collect();
    functions.sort_by(|a, b| b.cyclomatic.cmp(&a.cyclomatic));
    let top_functions: Vec<Value> = functions
        .iter()
        .take(10)
        .map(|f| {
            json!({
                "name": f.name,
                "start_line": f.start_line,
                "cyclomatic": f.cyclomatic,
                "cognitive": f.cognitive,
                "loc": f.loc,
            })
        })
        .collect();

    // Test coverage: does any TestMapping point at this file as source?
    let scan_for_tests = analysis.scan.clone();
    let project_for_tests = std::path::PathBuf::from(project_root);
    let inline = tokio::task::spawn_blocking(move || {
        belisarius_scan::test_map::detect_inline_tests(&project_for_tests, &scan_for_tests)
    })
    .await
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("test_map join: {e}")))?;
    let test_map = belisarius_scan::test_map::compute(&analysis, &inline);
    let mapping = test_map.mappings.iter().find(|m| m.source == rel_path);
    let test_files: Vec<&str> = mapping
        .map(|m| m.tests.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    Ok(json!({
        "kind": "file",
        "target": rel_path,
        "language": file_info.map(|f| f.language.as_str()),
        "loc": file_info.map(|f| f.loc),
        "function_count": functions.len(),
        "metrics": file_metrics.map(|m| json!({
            "max_cyclomatic": m.max_cyclomatic,
            "total_cyclomatic": m.total_cyclomatic,
            "max_cognitive": m.max_cognitive,
            "longest_function_loc": m.longest_function_loc,
            "avg_cyclomatic": m.avg_cyclomatic,
        })),
        "top_functions": top_functions,
        "tests": {
            "covered": !test_files.is_empty(),
            "files": test_files,
        },
        "next_steps": [
            format!("`belisarius_symbol` for a specific function in {rel_path}"),
            format!("`belisarius_who_calls` to find callers"),
            "`belisarius_recent_changes` to see if this file has moved recently",
        ],
    }))
}

async fn describe_dir(
    ctx: &AppContext,
    project_root: &str,
    rel_dir: &str,
) -> Result<Value, ServiceError> {
    let analysis = ctx.load_analysis(project_root).await?;
    let prefix = if rel_dir.ends_with('/') || rel_dir.is_empty() {
        rel_dir.to_string()
    } else {
        format!("{rel_dir}/")
    };

    let mut files_in_dir: Vec<&belisarius_core::FileNode> = analysis
        .scan
        .files
        .iter()
        .filter(|f| f.path.starts_with(&prefix))
        .collect();
    files_in_dir.sort_by(|a, b| b.loc.cmp(&a.loc));

    // Language mix.
    let mut lang_counts: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    for f in &files_in_dir {
        *lang_counts.entry(f.language.as_str()).or_insert(0) += 1;
    }

    // Top-complexity files within the dir.
    let mut hot_files: Vec<&belisarius_core::FileMetrics> = analysis
        .file_metrics
        .iter()
        .filter(|m| m.path.starts_with(&prefix))
        .collect();
    hot_files.sort_by(|a, b| b.total_cyclomatic.cmp(&a.total_cyclomatic));
    let top_complex: Vec<Value> = hot_files
        .iter()
        .take(5)
        .map(|m| {
            json!({
                "path": m.path,
                "total_cyclomatic": m.total_cyclomatic,
                "function_count": m.function_count,
            })
        })
        .collect();

    Ok(json!({
        "kind": "dir",
        "target": rel_dir,
        "file_count": files_in_dir.len(),
        "total_loc": files_in_dir.iter().map(|f| f.loc as u64).sum::<u64>(),
        "language_mix": lang_counts,
        "top_complex_files": top_complex,
        "next_steps": [
            format!("`belisarius_describe` on a specific file under {rel_dir}"),
            "`belisarius_hotspots` scoped via path",
            "`belisarius_test_gaps` to find untested code in this dir",
        ],
    }))
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_describe",
        description: "Rich one-shot 'explain this' for a file, directory, or symbol. Returns \
the right slice automatically — file metrics, top functions, test coverage for files; language \
mix and complexity hotspots for directories; a hand-off pointer for SCIP symbols.\n\n\
When to use: building a mental model of a file or directory without chaining 5 tool calls.\n\
When not to use: blast-radius (`belisarius_who_calls`) or full call graphs (`belisarius_symbol`).",
        input_schema: json!({
            "type": "object",
            "required": ["path", "target"],
            "properties": {
                "path": { "type": "string", "description": "Project root." },
                "target": {
                    "type": "string",
                    "description": "Relative file path, directory path, or SCIP symbol id."
                }
            }
        }),
        handler: handle_describe as ToolHandler,
    }]
}

fn handle_describe(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: DescribeArgs = serde_json::from_value(args)?;
        describe(&ctx, args).await
    })
}
