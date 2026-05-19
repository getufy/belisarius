//! HTTP transport for the `quality` service capability — 3-line handler that
//! parses the query, calls into `service::quality`, and wraps the response.

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
struct PathQuery {
    path: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/quality", get(handler))
}

async fn handler(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let path = q.path.unwrap_or_else(|| ".".to_string());
    let args = service::quality::QualityArgs { path };
    let value = service::quality::quality(&st.ctx, args).await?;
    Ok(Json(value))
}
