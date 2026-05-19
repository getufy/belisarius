//! HTTP transport for `pack`.

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
struct PackQuery {
    path: Option<String>,
    #[serde(default)]
    budget_tokens: Option<usize>,
    #[serde(default)]
    focus: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/pack", get(handler))
}

async fn handler(
    State(st): State<AppState>,
    Query(q): Query<PackQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::pack::Args {
        path: q.path.unwrap_or_else(|| ".".into()),
        budget_tokens: q.budget_tokens,
        focus: q.focus,
    };
    Ok(Json(service::pack::pack(&st.ctx, args).await?))
}
