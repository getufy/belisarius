//! HTTP transport entry point.
//!
//! `serve()` builds the router by merging every `routes::<feature>::router()`
//! defined under `crate::routes`. The only handler that still lives in this
//! file is the trivial `health` probe. Every capability — scan, graph,
//! analyze, symbols, search, fleet, brief, pack, quality, function_detail,
//! state (snapshot/drift/pins), project (functions/snippet/markers/file_dsm/
//! hotspots/test_gaps/diff/commands/surface/components/rules), architecture,
//! diagnostics, context artifacts — lives in `crate::service` and is served
//! through a thin route wrapper. The `Server::registry` in `cmd_mcp.rs`
//! consumes the same service functions through `ToolSpec` entries.
//!
//! `AppState` holds the shared `service::context::AppContext` (caches +
//! fleet-aware path resolution); every route module reads it through the
//! state extractor.
//!
//! `AppError` translates `ServiceError` at the HTTP edge — `MissingIndex`
//! becomes 412 Precondition Failed with the `run `belisarius index` first`
//! hint, `NotFound` becomes 404, `BadRequest` 400, `Internal` 500.

use anyhow::Result;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

use crate::service::context::AppContext;

#[derive(Clone)]
pub(crate) struct AppState {
    /// Shared service context — used by every `routes::*` module.
    pub(crate) ctx: Arc<AppContext>,
}

pub async fn serve(port: u16, web_dir: Option<PathBuf>) -> Result<()> {
    let state = AppState {
        ctx: Arc::new(AppContext::new()),
    };

    let mut app = Router::new()
        .route("/api/health", get(health))
        // Every other route lives in a feature module under `crate::routes`.
        .merge(crate::routes::architecture::router())
        .merge(crate::routes::brief::router())
        .merge(crate::routes::context_artifacts::router())
        .merge(crate::routes::diagnostics::router())
        .merge(crate::routes::fleet::router())
        .merge(crate::routes::function_detail::router())
        .merge(crate::routes::pack::router())
        .merge(crate::routes::project::router())
        .merge(crate::routes::quality::router())
        .merge(crate::routes::search::router())
        .merge(crate::routes::state::router())
        .merge(crate::routes::symbols::router())
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // Static-file fallback selection:
    //   1. Explicit `--web-dir` always wins (lets devs override during
    //      `web && pnpm dev`-style iteration even on an embedded build).
    //   2. With the `embed-web` cargo feature, the SPA is baked into the
    //      binary — `belisarius serve` ships its own UI with no extra args.
    //   3. Otherwise the server is API-only and prints a hint so people
    //      don't think the binary is broken.
    if let Some(dir) = web_dir {
        if dir.exists() {
            app = app.fallback_service(ServeDir::new(dir));
        } else {
            tracing::warn!(
                "web_dir {} does not exist; skipping static fallback",
                dir.display()
            );
        }
    } else {
        #[cfg(feature = "embed-web")]
        {
            app = app
                .route("/", get(crate::web_assets::serve_index))
                .fallback(crate::web_assets::serve_embedded);
            tracing::info!("serving embedded web UI at /");
        }
        #[cfg(not(feature = "embed-web"))]
        {
            tracing::info!(
                "no --web-dir given and built without the `embed-web` feature; \
                 API-only. Rebuild with `cargo install --features embed-web ...` \
                 (after `cd web && pnpm build`) for a single-binary UI.",
            );
        }
    }

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "belisarius server listening");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "belisarius" }))
}

pub(crate) struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::internal(format!("serialization failed: {e}"))
    }
}

impl From<crate::service::error::ServiceError> for AppError {
    fn from(e: crate::service::error::ServiceError) -> Self {
        use crate::service::error::ServiceError as S;
        match e {
            S::BadRequest(m) => AppError::bad_request(m),
            S::NotFound(m) => AppError::not_found(m),
            S::MissingIndex { which, hint } => AppError {
                status: StatusCode::PRECONDITION_FAILED,
                message: format!("{which} index missing: {hint}"),
            },
            S::Internal(err) => AppError::internal(format!("{err:#}")),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

// `scan_markers` and `MarkerHit` live here because they're shared by both
// `service::brief` and `service::project::markers`. Promoting them into a
// service module is a follow-up — the function is pure, no transport-coupled
// state involved.

#[derive(Debug, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct MarkerHit {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub text: String,
}

pub fn scan_markers(project: &str, limit: usize) -> anyhow::Result<Vec<MarkerHit>> {
    use ignore::WalkBuilder;
    use once_cell::sync::Lazy;
    use regex::Regex;
    static MARKER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX|NOTE)\b[:\s]*(.*)$").unwrap());
    let project_path = std::path::Path::new(project);
    let mut out: Vec<MarkerHit> = Vec::new();
    for entry in WalkBuilder::new(project)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .build()
        .flatten()
    {
        if !entry.path().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if belisarius_scan::languages::language_for_ext(&ext).is_none() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let rel = entry
            .path()
            .strip_prefix(project_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        for (idx, line) in text.lines().enumerate() {
            if out.len() >= limit {
                return Ok(out);
            }
            if let Some(caps) = MARKER_RE.captures(line) {
                let kind = caps
                    .get(1)
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_default();
                let body = caps
                    .get(2)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                out.push(MarkerHit {
                    file: rel.clone(),
                    line: (idx + 1) as u32,
                    kind,
                    text: body,
                });
            }
        }
    }
    Ok(out)
}
