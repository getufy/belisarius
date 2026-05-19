//! Embedded web UI served by `belisarius serve` when the `embed-web`
//! feature is enabled. The dist directory is captured at compile time via
//! `include_dir!`, so the resulting binary carries the SPA bundle and can
//! serve it without a `--web-dir` argument.
//!
//! Off by default — `cargo install` won't try to embed `web/dist` unless
//! you explicitly opt in via `--features embed-web`. The justfile's
//! `install-global` recipe builds the web bundle first and then turns the
//! feature on, so day-to-day users get the embedded UI for free.

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

/// The `web/dist` directory frozen at compile time. Paths inside this
/// const are relative to that root — `index.html`, `assets/index-xxx.js`,
/// etc.
static EMBEDDED_WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web/dist");

/// Axum fallback handler — runs for any request that didn't match an
/// `/api/*` route or the `/` index route. Pulls the path off the request
/// `Uri` rather than `extract::Path<String>` (which is for route pattern
/// params like `/x/:id` and would 500 on a true catch-all). Tries the
/// embedded asset first; falls back to `index.html` so SPA client-side
/// routes (`/scans`, `/fleet`) still hydrate.
pub async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = EMBEDDED_WEB.get_file(path) {
        return respond_with(file.path().to_string_lossy().as_ref(), file.contents());
    }
    // SPA fallback — anything that isn't a real asset lands on index.html.
    if let Some(index) = EMBEDDED_WEB.get_file("index.html") {
        return respond_with("index.html", index.contents());
    }
    (StatusCode::NOT_FOUND, "embedded web assets missing").into_response()
}

/// Same as `serve_embedded` but for the root path (`GET /`). Axum routes
/// `/` separately from the `/*` catch-all so we wire two handlers.
pub async fn serve_index() -> Response {
    if let Some(index) = EMBEDDED_WEB.get_file("index.html") {
        return respond_with("index.html", index.contents());
    }
    (StatusCode::NOT_FOUND, "embedded web assets missing").into_response()
}

fn respond_with(name: &str, bytes: &'static [u8]) -> Response {
    let content_type = mime_for(name);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        // Aggressive cache for hashed assets, no-cache for index.html so
        // SPA shell updates land immediately on refresh.
        .header(
            header::CACHE_CONTROL,
            if name.ends_with("index.html") {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            },
        )
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "map" => "application/json",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
