//! HTTP transport for the `symbols` service capability — 6 routes wrapping
//! the corresponding `service::symbols::*` functions.

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
struct SymbolsQuery {
    path: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    sym: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/symbols/status", get(status))
        .route("/api/symbols/search", get(search))
        .route("/api/symbols/refs", get(refs))
        .route("/api/symbols/callers", get(callers))
        .route("/api/symbols/file", get(file))
        .route("/api/impact", get(impact))
        .route("/api/flow", get(flow))
        .route("/api/symbol", get(symbol))
}

async fn status(
    State(st): State<AppState>,
    Query(q): Query<SymbolsQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::symbols::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::symbols::status(&st.ctx, args).await?))
}

async fn search(
    State(st): State<AppState>,
    Query(q): Query<SymbolsQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::symbols::SearchArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        query: q.q.unwrap_or_default(),
        limit: q.limit,
    };
    Ok(Json(service::symbols::search(&st.ctx, args).await?))
}

async fn refs(
    State(st): State<AppState>,
    Query(q): Query<SymbolsQuery>,
) -> Result<Json<Value>, AppError> {
    let sym = q
        .sym
        .ok_or_else(|| AppError::bad_request("missing `sym` query parameter"))?;
    let args = service::symbols::SymbolRefArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        sym,
    };
    Ok(Json(service::symbols::refs(&st.ctx, args).await?))
}

async fn callers(
    State(st): State<AppState>,
    Query(q): Query<SymbolsQuery>,
) -> Result<Json<Value>, AppError> {
    let sym = q
        .sym
        .ok_or_else(|| AppError::bad_request("missing `sym` query parameter"))?;
    let args = service::symbols::SymbolRefArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        sym,
    };
    Ok(Json(service::symbols::callers(&st.ctx, args).await?))
}

async fn file(
    State(st): State<AppState>,
    Query(q): Query<SymbolsQuery>,
) -> Result<Json<Value>, AppError> {
    let file = q
        .file
        .ok_or_else(|| AppError::bad_request("missing `file` query parameter"))?;
    let args = service::symbols::SymbolFileArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        file,
    };
    Ok(Json(service::symbols::file(&st.ctx, args).await?))
}

#[derive(Deserialize)]
struct SymbolQuery {
    path: Option<String>,
    sym: String,
}

#[derive(Deserialize)]
struct XrefQuery {
    path: Option<String>,
    sym: String,
    #[serde(default)]
    depth: Option<usize>,
}

async fn impact(
    State(st): State<AppState>,
    Query(q): Query<XrefQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::symbols::XrefArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        symbol: q.sym,
        depth: q.depth,
    };
    Ok(Json(service::symbols::impact(&st.ctx, args).await?))
}

async fn flow(
    State(st): State<AppState>,
    Query(q): Query<XrefQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::symbols::XrefArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        symbol: q.sym,
        depth: q.depth,
    };
    Ok(Json(service::symbols::flow(&st.ctx, args).await?))
}

async fn symbol(
    State(st): State<AppState>,
    Query(q): Query<SymbolQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::symbols::SymbolArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        symbol: q.sym,
    };
    Ok(Json(service::symbols::symbol_360(&st.ctx, args).await?))
}
