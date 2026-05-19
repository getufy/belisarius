//! HTTP transport for context artifacts: list / get / search / index.

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
struct GetQuery {
    path: Option<String>,
    name: String,
}

#[derive(Deserialize)]
struct SearchQuery {
    path: Option<String>,
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/context", get(list))
        .route("/api/context/get", get(get_artifact))
        .route("/api/context/search", get(search))
        .route("/api/context/index", post(index))
}

async fn list(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::context_artifacts::ListArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::context_artifacts::list(&st.ctx, args).await?))
}

async fn get_artifact(
    State(st): State<AppState>,
    Query(q): Query<GetQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::context_artifacts::GetArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        name: q.name,
    };
    Ok(Json(service::context_artifacts::get(&st.ctx, args).await?))
}

async fn search(
    State(st): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::context_artifacts::SearchArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        query: q.q,
        limit: q.limit,
    };
    Ok(Json(
        service::context_artifacts::search(&st.ctx, args).await?,
    ))
}

async fn index(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::context_artifacts::ListArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(
        service::context_artifacts::index(&st.ctx, args).await?,
    ))
}
