//! HTTP transport for the fleet capability — 6 routes (list / info / find /
//! hotspots / test_gaps / surface_diff).

use axum::{
    extract::{Path as AxPath, Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::server::{AppError, AppState};
use crate::service;

#[derive(Deserialize)]
struct FindQuery {
    pattern: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct DiffQuery {
    from: String,
    to: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Literal sub-routes register BEFORE the `:name` catch-all so axum
        // doesn't capture `find` / `hotspots` / etc. as an app name.
        .route("/api/fleet", get(list))
        .route("/api/fleet/find", get(find))
        .route("/api/fleet/hotspots", get(hotspots))
        .route("/api/fleet/test_gaps", get(test_gaps))
        .route("/api/fleet/surface_diff", get(surface_diff))
        .route("/api/fleet/:name", get(info))
}

async fn list(State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::fleet::list(&st.ctx, service::fleet::ListArgs {}).await?,
    ))
}

async fn info(
    State(st): State<AppState>,
    AxPath(name): AxPath<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::fleet::info(&st.ctx, service::fleet::InfoArgs { name }).await?,
    ))
}

async fn find(
    State(st): State<AppState>,
    Query(q): Query<FindQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::fleet::FindArgs {
        pattern: q.pattern,
        kind: q.kind,
        limit: q.limit,
    };
    Ok(Json(service::fleet::find(&st.ctx, args).await?))
}

async fn hotspots(
    State(st): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::fleet::LimitArgs { limit: q.limit };
    Ok(Json(service::fleet::hotspots(&st.ctx, args).await?))
}

async fn test_gaps(
    State(st): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::fleet::LimitArgs { limit: q.limit };
    Ok(Json(service::fleet::test_gaps(&st.ctx, args).await?))
}

async fn surface_diff(
    State(st): State<AppState>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::fleet::DiffArgs {
        from: q.from,
        to: q.to,
    };
    Ok(Json(service::fleet::surface_diff(&st.ctx, args).await?))
}
