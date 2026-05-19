//! HTTP transport for project-level capabilities. The set mirrors
//! `service::project` — scan / graph / analyze / functions / snippet /
//! markers / file_dsm / hotspots / test_gaps / diff / commands / surface /
//! components / rules.

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
struct FunctionsQuery {
    path: Option<String>,
    #[serde(default)]
    min_cc: Option<u32>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    sort_by: Option<String>,
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize)]
struct HotspotsQuery {
    path: Option<String>,
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct LimitQuery {
    path: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct SnippetQuery {
    path: String,
    file: String,
    line: u32,
    #[serde(default)]
    radius: Option<u32>,
}

#[derive(Deserialize)]
struct FileDsmQuery {
    path: Option<String>,
    file: String,
}

#[derive(Deserialize)]
struct DiffQuery {
    path: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    head: Option<String>,
    #[serde(default)]
    hotspot_window: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/scan", post(scan))
        .route("/api/graph", post(graph))
        .route("/api/analyze", post(analyze))
        .route("/api/functions", get(functions))
        .route("/api/snippet", get(snippet))
        .route("/api/markers", get(markers))
        .route("/api/file_dsm", get(file_dsm))
        .route("/api/hotspots", get(hotspots))
        .route("/api/test_gaps", get(test_gaps))
        .route("/api/diff", get(diff))
        .route("/api/commands", get(commands))
        .route("/api/surface", get(surface))
        .route("/api/components", get(components))
        .route("/api/rules", get(rules))
}

async fn scan(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::scan(&st.ctx, args).await?))
}

async fn graph(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::graph(&st.ctx, args).await?))
}

async fn analyze(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::analyze(&st.ctx, args).await?))
}

async fn functions(
    State(st): State<AppState>,
    Query(q): Query<FunctionsQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::FunctionsArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        min_cc: q.min_cc,
        limit: q.limit,
        sort_by: q.sort_by,
        file: q.file,
    };
    Ok(Json(service::project::functions(&st.ctx, args).await?))
}

async fn snippet(
    State(st): State<AppState>,
    Query(q): Query<SnippetQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::SnippetArgs {
        path: q.path,
        file: q.file,
        line: q.line,
        radius: q.radius,
    };
    Ok(Json(service::project::snippet(&st.ctx, args).await?))
}

async fn markers(
    State(st): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::LimitArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        limit: q.limit,
    };
    Ok(Json(service::project::markers(&st.ctx, args).await?))
}

async fn file_dsm(
    State(st): State<AppState>,
    Query(q): Query<FileDsmQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::FileDsmArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        file: q.file,
    };
    Ok(Json(service::project::file_dsm(&st.ctx, args).await?))
}

async fn hotspots(
    State(st): State<AppState>,
    Query(q): Query<HotspotsQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::HotspotsArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        days: q.days,
        limit: q.limit,
    };
    Ok(Json(service::project::hotspots(&st.ctx, args).await?))
}

async fn test_gaps(
    State(st): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::LimitArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        limit: q.limit,
    };
    Ok(Json(service::project::test_gaps(&st.ctx, args).await?))
}

async fn diff(
    State(st): State<AppState>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::DiffArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        base: q.base,
        head: q.head,
        hotspot_window: q.hotspot_window,
    };
    Ok(Json(service::project::diff(&st.ctx, args).await?))
}

async fn commands(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::commands(&st.ctx, args).await?))
}

async fn surface(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::surface(&st.ctx, args).await?))
}

async fn components(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::components(&st.ctx, args).await?))
}

async fn rules(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::project::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::project::rules_check(&st.ctx, args).await?))
}
