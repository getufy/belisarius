//! HTTP transport for `brief`.

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
struct BriefQuery {
    path: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/brief", get(handler))
}

async fn handler(
    State(st): State<AppState>,
    Query(q): Query<BriefQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::brief::Args {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::brief::brief(&st.ctx, args).await?))
}
