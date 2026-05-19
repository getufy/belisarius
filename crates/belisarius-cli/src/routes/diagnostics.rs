//! HTTP transport for diagnostics — 3 routes (status / run / list).

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::server::{AppError, AppState};
use crate::service;

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
struct RunPayload {
    path: String,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize)]
struct ListQuery {
    path: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/diagnostics/status", get(status))
        .route("/api/diagnostics/run", post(run))
        .route("/api/diagnostics", get(list))
}

async fn status(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::diagnostics::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::diagnostics::status(&st.ctx, args).await?))
}

async fn run(
    State(st): State<AppState>,
    Json(payload): Json<RunPayload>,
) -> Result<Json<Value>, AppError> {
    let args = service::diagnostics::RunArgs {
        path: payload.path,
        tools: payload.tools,
        force: payload.force,
    };
    Ok(Json(service::diagnostics::run(&st.ctx, args).await?))
}

async fn list(
    State(st): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::diagnostics::ListArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        tool: q.tool,
        severity: q.severity,
        file: q.file,
        limit: q.limit,
    };
    Ok(Json(service::diagnostics::list(&st.ctx, args).await?))
}
