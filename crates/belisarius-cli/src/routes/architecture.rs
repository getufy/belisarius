//! HTTP transport for the architecture capability — 4 routes
//! (mermaid / graph / summary / module).

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::server::{AppError, AppState};
use crate::service;

#[derive(Deserialize)]
struct MermaidQuery {
    path: Option<String>,
    #[serde(default)]
    max_nodes: Option<usize>,
    #[serde(default)]
    group_depth: Option<usize>,
    #[serde(default)]
    view: Option<String>,
}

#[derive(Deserialize)]
struct SummaryQuery {
    path: Option<String>,
    #[serde(default)]
    group_depth: Option<usize>,
}

#[derive(Deserialize)]
struct ModuleQuery {
    path: Option<String>,
    module: String,
    #[serde(default)]
    group_depth: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/architecture/mermaid", get(mermaid))
        .route("/api/architecture/graph", get(graph))
        .route("/api/architecture/summary", get(summary))
        .route("/api/architecture/module", get(module))
}

async fn mermaid(
    State(st): State<AppState>,
    Query(q): Query<MermaidQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::architecture::MermaidArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        max_nodes: q.max_nodes,
        group_depth: q.group_depth,
        view: q.view,
    };
    Ok(Json(service::architecture::mermaid(&st.ctx, args).await?))
}

async fn graph(
    State(st): State<AppState>,
    Query(q): Query<MermaidQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::architecture::MermaidArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        max_nodes: q.max_nodes,
        group_depth: q.group_depth,
        view: q.view,
    };
    Ok(Json(service::architecture::graph(&st.ctx, args).await?))
}

async fn summary(
    State(st): State<AppState>,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::architecture::SummaryArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        group_depth: q.group_depth,
    };
    Ok(Json(service::architecture::summary(&st.ctx, args).await?))
}

async fn module(
    State(st): State<AppState>,
    Query(q): Query<ModuleQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::architecture::ModuleArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        module: q.module,
        group_depth: q.group_depth,
    };
    Ok(Json(service::architecture::module(&st.ctx, args).await?))
}
