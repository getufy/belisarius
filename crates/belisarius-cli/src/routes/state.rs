//! HTTP transport for snapshot / drift / pins.

use axum::{
    extract::{Path as AxPath, Query, State},
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
struct DriftQuery {
    path: Option<String>,
    #[serde(default)]
    since: Option<String>,
}

#[derive(Deserialize)]
struct PinBody {
    path: String,
    scope: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    note: String,
    #[serde(default)]
    ttl_days: Option<u32>,
}

#[derive(Deserialize)]
struct PinsQuery {
    path: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
struct UnpinQuery {
    path: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/snapshot", post(snapshot))
        .route("/api/drift", get(drift))
        .route("/api/pins", get(list_pins))
        .route("/api/pins/create", post(pin))
        .route("/api/pins/:id", axum::routing::delete(unpin))
}

async fn snapshot(
    State(st): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::state::PathArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
    };
    Ok(Json(service::state::snapshot(&st.ctx, args).await?))
}

async fn drift(
    State(st): State<AppState>,
    Query(q): Query<DriftQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::state::DriftArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        since: q.since,
    };
    Ok(Json(service::state::drift(&st.ctx, args).await?))
}

async fn pin(
    State(st): State<AppState>,
    Json(body): Json<PinBody>,
) -> Result<Json<Value>, AppError> {
    let args = service::state::PinArgs {
        path: body.path,
        scope: body.scope,
        file: body.file,
        line: body.line,
        note: body.note,
        ttl_days: body.ttl_days,
    };
    Ok(Json(service::state::pin(&st.ctx, args).await?))
}

async fn list_pins(
    State(st): State<AppState>,
    Query(q): Query<PinsQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::state::ListPinsArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        scope: q.scope,
    };
    Ok(Json(service::state::list_pins(&st.ctx, args).await?))
}

async fn unpin(
    State(st): State<AppState>,
    AxPath(id): AxPath<i64>,
    Query(q): Query<UnpinQuery>,
) -> Result<Json<Value>, AppError> {
    let args = service::state::UnpinArgs {
        path: q.path.unwrap_or_else(|| ".".into()),
        id,
    };
    Ok(Json(service::state::unpin(&st.ctx, args).await?))
}
