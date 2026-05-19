//! HTTP transport for the `search` service capability — 3 routes.

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
struct SearchQuery {
    path: Option<String>,
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
struct ReindexQuery {
    path: Option<String>,
    #[serde(default)]
    full: Option<bool>,
    #[serde(default)]
    bm25_only: Option<bool>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/search", get(query))
        .route("/api/search/status", get(status))
        .route("/api/search/reindex", post(reindex))
}

async fn query(
    State(st): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::search::QueryArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        query: q.q,
        limit: q.limit,
        lang: q.lang,
        kind: q.kind,
    };
    Ok(Json(service::search::query(&st.ctx, args).await?))
}

async fn status(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::search::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::search::status(&st.ctx, args).await?))
}

async fn reindex(
    State(st): State<AppState>,
    Query(q): Query<ReindexQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::search::ReindexArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        full: q.full,
        bm25_only: q.bm25_only,
    };
    Ok(Json(service::search::reindex(&st.ctx, args).await?))
}
