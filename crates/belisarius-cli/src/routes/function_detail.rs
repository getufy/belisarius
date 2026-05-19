//! HTTP transport for `function_detail`.

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
struct FunctionDetailQuery {
    path: Option<String>,
    file: String,
    name: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/function", get(handler))
}

async fn handler(
    State(st): State<AppState>,
    Query(q): Query<FunctionDetailQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::function_detail::Args {
        path: q.path.unwrap_or_else(|| ".".into()),
        file: q.file,
        name: q.name,
    };
    Ok(Json(
        service::function_detail::function_detail(&st.ctx, args).await?,
    ))
}
