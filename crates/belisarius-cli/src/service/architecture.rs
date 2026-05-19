//! Architecture views — Mermaid diagram, Cytoscape-shape graph, directory
//! summary, and per-module drill-down. HTTP-only today (no MCP twin) but
//! follows the same `service::*` shape so a future MCP tool can register a
//! `ToolSpec` without touching the route layer.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::service::{context::AppContext, error::ServiceError};

#[derive(Debug, Deserialize)]
pub struct MermaidArgs {
    pub path: String,
    #[serde(default)]
    pub max_nodes: Option<usize>,
    #[serde(default)]
    pub group_depth: Option<usize>,
    /// "module" (default) aggregates files into directory modules; "file"
    /// renders the per-file graph (legacy view).
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SummaryArgs {
    pub path: String,
    #[serde(default)]
    pub group_depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ModuleArgs {
    pub path: String,
    pub module: String,
    #[serde(default)]
    pub group_depth: Option<usize>,
}

pub async fn mermaid(ctx: &AppContext, args: MermaidArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let max_nodes = args.max_nodes.unwrap_or(60);
    let group_depth = args.group_depth.unwrap_or(2);
    let view = args.view.unwrap_or_else(|| "module".to_string());
    let analysis = ctx.load_analysis(&project).await?;
    let mermaid = match view.as_str() {
        "file" => belisarius_scan::architecture::render_mermaid_files(
            &analysis.graph,
            max_nodes,
            group_depth,
        ),
        _ => belisarius_scan::architecture::render_mermaid_modules(&analysis.graph, group_depth),
    };
    let dir_summary =
        belisarius_scan::architecture::directory_summary(&analysis.graph, group_depth);
    Ok(json!({
        "mermaid": mermaid,
        "view": view,
        "directory_summary": dir_summary,
        "nodes_total": analysis.graph.nodes.len(),
        "edges_total": analysis.graph.edges.len(),
        "rendered_cap": max_nodes,
        "group_depth": group_depth,
    }))
}

pub async fn graph(ctx: &AppContext, args: MermaidArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let max_nodes = args.max_nodes.unwrap_or(60);
    let group_depth = args.group_depth.unwrap_or(2);
    let view = args.view.unwrap_or_else(|| "module".to_string());
    let analysis = ctx.load_analysis(&project).await?;
    let graph = match view.as_str() {
        "file" => {
            belisarius_scan::architecture::graph_files(&analysis.graph, max_nodes, group_depth)
        }
        _ => belisarius_scan::architecture::graph_modules(&analysis.graph, group_depth),
    };
    let dir_summary =
        belisarius_scan::architecture::directory_summary(&analysis.graph, group_depth);
    Ok(json!({
        "view": graph.view,
        "group_depth": graph.group_depth,
        "nodes": graph.nodes,
        "edges": graph.edges,
        "nodes_total": graph.nodes_total,
        "edges_total": graph.edges_total,
        "rendered_cap": graph.rendered_cap,
        "directory_summary": dir_summary,
    }))
}

pub async fn summary(ctx: &AppContext, args: SummaryArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let group_depth = args.group_depth.unwrap_or(1);
    let analysis = ctx.load_analysis(&project).await?;
    let summary = belisarius_scan::architecture::directory_summary(&analysis.graph, group_depth);
    Ok(json!({ "directory_summary": summary }))
}

pub async fn module(ctx: &AppContext, args: ModuleArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let group_depth = args.group_depth.unwrap_or(2);
    let analysis = ctx.load_analysis(&project).await?;
    let target = args.module;

    // Helper: derive the same group key the renderer used.
    let key_of = |path: &str| -> String {
        let segs: Vec<&str> = path.split('/').collect();
        let dir_depth = segs.len().saturating_sub(1);
        if dir_depth == 0 {
            return "(root)".to_string();
        }
        let effective = group_depth.min(dir_depth).max(1);
        segs.iter()
            .take(effective)
            .copied()
            .collect::<Vec<_>>()
            .join("/")
    };

    let files: Vec<Value> = analysis
        .scan
        .files
        .iter()
        .filter(|f| key_of(&f.path) == target)
        .map(|f| {
            json!({
                "path": f.path,
                "language": f.language,
                "loc": f.loc,
            })
        })
        .collect();

    let mut outgoing: BTreeMap<String, u32> = BTreeMap::new();
    let mut incoming: BTreeMap<String, u32> = BTreeMap::new();
    for e in &analysis.graph.edges {
        let from = key_of(&e.from);
        let to = key_of(&e.to);
        if from == target && to != target {
            *outgoing.entry(to.clone()).or_insert(0) += 1;
        }
        if to == target && from != target {
            *incoming.entry(from.clone()).or_insert(0) += 1;
        }
    }
    let mut outgoing_vec: Vec<Value> = outgoing
        .into_iter()
        .map(|(path, weight)| json!({ "path": path, "weight": weight }))
        .collect();
    outgoing_vec.sort_by(|a, b| {
        b["weight"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["weight"].as_u64().unwrap_or(0))
    });
    let mut incoming_vec: Vec<Value> = incoming
        .into_iter()
        .map(|(path, weight)| json!({ "path": path, "weight": weight }))
        .collect();
    incoming_vec.sort_by(|a, b| {
        b["weight"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["weight"].as_u64().unwrap_or(0))
    });

    let total_loc: u32 = files
        .iter()
        .map(|f| f["loc"].as_u64().unwrap_or(0) as u32)
        .sum();
    Ok(json!({
        "module": target,
        "group_depth": group_depth,
        "file_count": files.len(),
        "total_loc": total_loc,
        "files": files,
        "outgoing": outgoing_vec,
        "incoming": incoming_vec,
    }))
}

// Suppress unused warning — Arc<AppContext> only matters once architecture
// gets an MCP tool spec. The functions above already take &AppContext, so
// the bindings will fit the existing ToolHandler signature when wired up.
#[allow(dead_code)]
fn _arc_ctx_marker(_: Arc<AppContext>) {}

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_architecture",
            description: "Top-level architecture view: directories grouped at a configurable \
depth, with fan-in / fan-out / file counts per group. The cheapest 'how is this codebase \
shaped?' answer.\n\n\
When to use: orienting on a new project, or sanity-checking that a refactor matches the \
intended layering.\n\
When not to use: per-file details (`belisarius_describe`); call-level graphs \
(`belisarius_who_calls`); diagram output (use `belisarius_architecture_mermaid`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Project root." },
                    "group_depth": {
                        "type": "integer",
                        "description": "Number of leading path segments used to define a module. Default 1 (top-level dirs).",
                        "minimum": 1,
                        "maximum": 5
                    }
                }
            }),
            handler: handle_summary as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_architecture_mermaid",
            description: "Render the module-level dependency graph as a Mermaid diagram. \
Returns a `mermaid` field containing the diagram source.\n\n\
When to use: producing a visual the agent can paste into a PR description or doc.\n\
When not to use: programmatic graph traversal — use `belisarius_architecture` for the JSON \
shape instead.",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "max_nodes": { "type": "integer", "default": 60 },
                    "group_depth": { "type": "integer", "default": 2 },
                    "view": {
                        "type": "string",
                        "enum": ["module", "file"],
                        "description": "Aggregation granularity. Default 'module'."
                    }
                }
            }),
            handler: handle_mermaid as ToolHandler,
        },
    ]
}

fn handle_summary(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SummaryArgs = serde_json::from_value(args)?;
        summary(&ctx, args).await
    })
}

fn handle_mermaid(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: MermaidArgs = serde_json::from_value(args)?;
        mermaid(&ctx, args).await
    })
}
