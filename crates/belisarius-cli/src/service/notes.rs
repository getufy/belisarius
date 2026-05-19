//! Knowledge layer: `belisarius_remember`, `belisarius_recall`,
//! `belisarius_decisions`. Backed by the `notes` table in `state.db`.
//!
//! Embeddings are deferred: the BM25-style token-overlap recall in
//! `state_db::recall_notes` is the default. When the embedding model is
//! wired in (Phase 3 follow-up), `embedding` is computed here and stored
//! alongside the note.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;
use crate::state_db;

#[derive(Debug, Deserialize)]
pub struct RememberArgs {
    pub path: String,
    /// One of: decision | gotcha | todo | context | hypothesis.
    pub kind: String,
    pub content: String,
    /// project | file | function. Default: project.
    pub scope: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub symbol: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub ttl_days: Option<u32>,
}

pub async fn remember(ctx: &AppContext, args: RememberArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let scope = args.scope.unwrap_or_else(|| "project".into());
    let kind = args.kind.to_lowercase();
    if !matches!(
        kind.as_str(),
        "decision" | "gotcha" | "todo" | "context" | "hypothesis"
    ) {
        return Err(ServiceError::bad_request(format!(
            "kind must be one of decision|gotcha|todo|context|hypothesis, got {kind:?}"
        )));
    }
    if !matches!(scope.as_str(), "project" | "file" | "function") {
        return Err(ServiceError::bad_request(format!(
            "scope must be one of project|file|function, got {scope:?}"
        )));
    }

    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;

    // Try to compute an embedding. Failure (model not downloaded, feature
    // off, transient error) is non-fatal — we just store the note with
    // `embedding=NULL` and recall falls back to BM25.
    let embedding_vec =
        ctx.embedder()
            .and_then(|p| match p.embed(std::slice::from_ref(&args.content)) {
                Ok(mut vs) => vs.pop(),
                Err(e) => {
                    tracing::warn!(
                        target: "belisarius_cli::notes",
                        "embedding skipped (continuing with NULL): {e}"
                    );
                    None
                }
            });

    let id = state_db::insert_note(
        &conn,
        state_db::NoteDraft {
            kind: &kind,
            scope: &scope,
            file: args.file.as_deref(),
            line: args.line,
            symbol: args.symbol.as_deref(),
            content: &args.content,
            agent_id: args.agent_id.as_deref(),
            session_id: args.session_id.as_deref(),
            embedding: embedding_vec.as_deref(),
            ttl_days: args.ttl_days,
        },
    )
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("insert note: {e:#}")))?;

    Ok(json!({
        "note_id": id,
        "kind": kind,
        "scope": scope,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RecallArgs {
    pub path: String,
    pub query: String,
    pub scope: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

pub async fn recall(ctx: &AppContext, args: RecallArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    let limit = args.limit.unwrap_or(20);
    let scope = args.scope.as_deref();
    let kind = args.kind.as_deref();

    // Prefer dense recall when an embedding provider is available *and*
    // there are notes with embeddings to compare against. Fall back to the
    // token-overlap path otherwise — it's still useful for projects that
    // existed before embeddings were wired in.
    let (hits, mode) = if let Some(provider) = ctx.embedder() {
        match provider.embed(std::slice::from_ref(&args.query)) {
            Ok(mut vs) => {
                if let Some(qvec) = vs.pop() {
                    let dense = state_db::recall_notes_dense(&conn, &qvec, scope, kind, limit)
                        .map_err(|e| {
                            ServiceError::Internal(anyhow::anyhow!("recall dense: {e:#}"))
                        })?;
                    if dense.is_empty() {
                        let bm = state_db::recall_notes(&conn, &args.query, scope, kind, limit)
                            .map_err(|e| {
                                ServiceError::Internal(anyhow::anyhow!("recall: {e:#}"))
                            })?;
                        (bm, "bm25_fallback_empty_dense")
                    } else {
                        (dense, "dense")
                    }
                } else {
                    (
                        state_db::recall_notes(&conn, &args.query, scope, kind, limit).map_err(
                            |e| ServiceError::Internal(anyhow::anyhow!("recall: {e:#}")),
                        )?,
                        "bm25_fallback_no_query_vec",
                    )
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "belisarius_cli::notes",
                    "dense recall unavailable, falling back to BM25: {e}"
                );
                (
                    state_db::recall_notes(&conn, &args.query, scope, kind, limit)
                        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("recall: {e:#}")))?,
                    "bm25_fallback_embed_error",
                )
            }
        }
    } else {
        (
            state_db::recall_notes(&conn, &args.query, scope, kind, limit)
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("recall: {e:#}")))?,
            "bm25_no_provider",
        )
    };

    let returned = hits.len();
    Ok(json!({
        "query": args.query,
        "items": hits,
        "mode": mode,
        "total_count": returned, // recall is best-of-N, no separate total
        "returned": returned,
        "truncated": returned >= limit,
    }))
}

#[derive(Debug, Deserialize)]
pub struct DecisionsArgs {
    pub path: String,
    pub scope: Option<String>,
    /// `7d` / `30d` / ISO 8601. Defaults to all-time.
    pub since: Option<String>,
    pub limit: Option<usize>,
}

pub async fn decisions(ctx: &AppContext, args: DecisionsArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    let limit = args.limit.unwrap_or(50);
    let since_iso = match args.since.as_deref() {
        Some(s) => Some(
            state_db::since_iso(s)
                .map_err(|e| ServiceError::bad_request(format!("invalid since: {e}")))?,
        ),
        None => None,
    };
    let hits = state_db::list_notes(
        &conn,
        args.scope.as_deref(),
        Some("decision"),
        since_iso.as_deref(),
        limit,
    )
    .map_err(|e| ServiceError::Internal(anyhow::anyhow!("list decisions: {e:#}")))?;
    let returned = hits.len();
    Ok(json!({
        "items": hits,
        "total_count": returned,
        "returned": returned,
        "truncated": returned >= limit,
    }))
}

// ─── link / sessions ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LinkArgs {
    pub path: String,
    pub from_id: i64,
    pub to_id: i64,
    /// `supports` | `contradicts` | `supersedes`.
    pub kind: String,
}

pub async fn link(ctx: &AppContext, args: LinkArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    state_db::insert_note_edge(&conn, args.from_id, args.to_id, &args.kind)
        .map_err(|e| ServiceError::bad_request(format!("{e:#}")))?;
    Ok(json!({
        "from_id": args.from_id,
        "to_id": args.to_id,
        "kind": args.kind,
    }))
}

#[derive(Debug, Deserialize)]
pub struct LinksArgs {
    pub path: String,
    pub note_id: i64,
    /// `out` (default), `in`, or `both`.
    pub direction: Option<String>,
}

pub async fn links(ctx: &AppContext, args: LinksArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    let dir = args.direction.as_deref().unwrap_or("out");
    let (out, inc) = match dir {
        "out" => (
            state_db::list_outgoing_edges(&conn, args.note_id)
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?,
            Vec::new(),
        ),
        "in" => (
            Vec::new(),
            state_db::list_incoming_edges(&conn, args.note_id)
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?,
        ),
        "both" => (
            state_db::list_outgoing_edges(&conn, args.note_id)
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?,
            state_db::list_incoming_edges(&conn, args.note_id)
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?,
        ),
        other => {
            return Err(ServiceError::bad_request(format!(
                "direction must be out|in|both, got {other:?}"
            )));
        }
    };
    Ok(json!({
        "note_id": args.note_id,
        "direction": dir,
        "outgoing": out,
        "incoming": inc,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SessionStartArgs {
    pub path: String,
    pub name: Option<String>,
    pub agent_id: Option<String>,
}

pub async fn session_start(
    ctx: &AppContext,
    args: SessionStartArgs,
) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    // Generate a session id without pulling in `uuid` — we just need a
    // process-unique opaque string. Time + a tiny entropy fragment is enough.
    let id = format!(
        "sess-{}-{:x}",
        time::OffsetDateTime::now_utc().unix_timestamp(),
        rand_suffix()
    );
    let s = state_db::insert_session(&conn, &id, args.name.as_deref(), args.agent_id.as_deref())
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?;
    serde_json::to_value(s).map_err(ServiceError::from)
}

#[derive(Debug, Deserialize)]
pub struct SessionEndArgs {
    pub path: String,
    pub session_id: String,
}

pub async fn session_end(ctx: &AppContext, args: SessionEndArgs) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let conn = state_db::open(std::path::Path::new(&project_root))
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("state.db open: {e:#}")))?;
    let session = state_db::end_session(&conn, &args.session_id)
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?;
    let Some(s) = session else {
        return Err(ServiceError::not_found(format!(
            "session `{}` not found",
            args.session_id
        )));
    };
    let summary = state_db::session_summary(&conn, &args.session_id)
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("{e:#}")))?;
    Ok(json!({
        "session": s,
        "summary": summary,
    }))
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Microsecond fragment as a cheap, dependency-free entropy source.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    // Mix with the process id so two starts in the same nanosecond differ.
    nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (std::process::id() as u64)
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_remember",
            description: "Persist a note in the knowledge layer so a later session can recall \
it. Notes are typed: `decision` (we picked X over Y), `gotcha` (this fails when…), `todo` \
(follow-up needed), `context` (background to keep), `hypothesis` (unconfirmed).\n\n\
When to use: at the moment an agent commits to a non-obvious choice, hits a subtle bug, or \
learns something the next session would benefit from knowing.\n\
When not to use: ephemeral state inside one session (use the conversation); facts that the \
codebase already documents.",
            input_schema: json!({
                "type": "object",
                "required": ["path", "kind", "content"],
                "properties": {
                    "path": { "type": "string", "description": "Project root." },
                    "kind": { "type": "string", "enum": ["decision","gotcha","todo","context","hypothesis"] },
                    "content": { "type": "string" },
                    "scope": { "type": "string", "enum": ["project","file","function"], "default": "project" },
                    "file": { "type": "string" },
                    "line": { "type": "integer", "minimum": 1 },
                    "symbol": { "type": "string" },
                    "agent_id": { "type": "string" },
                    "session_id": { "type": "string" },
                    "ttl_days": { "type": "integer", "minimum": 1 }
                }
            }),
            handler: handle_remember as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_recall",
            description: "Hybrid recall over stored notes. Today: token-overlap ranking over \
note content (no embeddings yet). Filters by `scope`, `kind`. Returns notes with relevance \
scores.\n\n\
When to use: 'what did I learn about auth?' / 'have I already looked at this?'.\n\
When not to use: exact-id lookups (just SQL); cross-project queries (notes are project-local).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "query"],
                "properties": {
                    "path": { "type": "string" },
                    "query": { "type": "string" },
                    "scope": { "type": "string" },
                    "kind": { "type": "string" },
                    "limit": { "type": "integer", "default": 20, "maximum": 200 }
                }
            }),
            handler: handle_recall as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_decisions",
            description: "List notes of `kind='decision'`, newest first. Optional `since` \
window (`7d`/`30d`/ISO 8601) and `scope` filter.\n\n\
When to use: 'what was decided about X?' — recovering the rationale chain for an area of the \
project.\n\
When not to use: gotchas / todos / general notes (use `belisarius_recall` with `kind`).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "scope": { "type": "string" },
                    "since": { "type": "string" },
                    "limit": { "type": "integer", "default": 50, "maximum": 500 }
                }
            }),
            handler: handle_decisions as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_link",
            description: "Add a directed edge between two notes. Kinds: `supports` (note A reinforces note B), \
`contradicts` (A invalidates B), `supersedes` (A replaces B and B should be considered historical).\n\n\
When to use: building a decision chain — link a new `decision` note to the older one it supersedes; tie \
a `gotcha` to the `decision` it contradicts.\n\
When not to use: ad-hoc grouping without a relationship (use `refs` on `belisarius_remember` instead, once supported).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "from_id", "to_id", "kind"],
                "properties": {
                    "path": { "type": "string" },
                    "from_id": { "type": "integer" },
                    "to_id": { "type": "integer" },
                    "kind": { "type": "string", "enum": ["supports", "contradicts", "supersedes"] }
                }
            }),
            handler: handle_link as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_note_links",
            description: "List edges in or out of a note. `direction` is `out` (default), `in`, or `both`.\n\n\
When to use: traversing the decision graph from a known note — 'what supersedes this?' or 'what contradicts this?'.\n\
When not to use: discovering notes from scratch (use `belisarius_recall`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "note_id"],
                "properties": {
                    "path": { "type": "string" },
                    "note_id": { "type": "integer" },
                    "direction": { "type": "string", "enum": ["out", "in", "both"], "default": "out" }
                }
            }),
            handler: handle_links as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_session_start",
            description: "Open a session — every subsequent `belisarius_remember` call within this work block can pass the returned `session_id` and have its note auto-tagged. `belisarius_session_end` later summarizes everything written.\n\n\
When to use: at the top of a focused work block (a PR, an investigation, a sprint task) you want to retrace later.\n\
When not to use: short one-off lookups; long-lived 'always on' sessions (the summary becomes less useful as it accumulates).",
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "name": { "type": "string", "description": "Optional human-readable label." },
                    "agent_id": { "type": "string" }
                }
            }),
            handler: handle_session_start as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_session_end",
            description: "Close a session and return a summary: note counts by kind, plus the session record.\n\n\
When to use: wrapping a work block; future sessions can `belisarius_recall` with `session_id` to see exactly what landed in this one.\n\
When not to use: forgetting about a session — just stop using its id; ending() is the way to get an aggregate report.",
            input_schema: json!({
                "type": "object",
                "required": ["path", "session_id"],
                "properties": {
                    "path": { "type": "string" },
                    "session_id": { "type": "string" }
                }
            }),
            handler: handle_session_end as ToolHandler,
        },
    ]
}

fn handle_remember(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: RememberArgs = serde_json::from_value(args)?;
        remember(&ctx, args).await
    })
}

fn handle_recall(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: RecallArgs = serde_json::from_value(args)?;
        recall(&ctx, args).await
    })
}

fn handle_decisions(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: DecisionsArgs = serde_json::from_value(args)?;
        decisions(&ctx, args).await
    })
}

fn handle_link(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LinkArgs = serde_json::from_value(args)?;
        link(&ctx, args).await
    })
}

fn handle_links(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: LinksArgs = serde_json::from_value(args)?;
        links(&ctx, args).await
    })
}

fn handle_session_start(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SessionStartArgs = serde_json::from_value(args)?;
        session_start(&ctx, args).await
    })
}

fn handle_session_end(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SessionEndArgs = serde_json::from_value(args)?;
        session_end(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_and_project() -> (Arc<AppContext>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = Arc::new(AppContext::new());
        (ctx, tmp)
    }

    fn path_str(tmp: &tempfile::TempDir) -> String {
        tmp.path().to_string_lossy().into_owned()
    }

    // ── remember validation ──────────────────────────────────────────────

    #[tokio::test]
    async fn remember_rejects_invalid_kind() {
        let (ctx, tmp) = ctx_and_project();
        let err = remember(
            &ctx,
            RememberArgs {
                path: path_str(&tmp),
                kind: "loves".into(),
                content: "x".into(),
                scope: None,
                file: None,
                line: None,
                symbol: None,
                agent_id: None,
                session_id: None,
                ttl_days: None,
            },
        )
        .await
        .expect_err("kind=loves must fail");
        assert!(matches!(err, ServiceError::BadRequest(_)));
    }

    #[tokio::test]
    async fn remember_rejects_invalid_scope() {
        let (ctx, tmp) = ctx_and_project();
        let err = remember(
            &ctx,
            RememberArgs {
                path: path_str(&tmp),
                kind: "decision".into(),
                content: "x".into(),
                scope: Some("global".into()),
                file: None,
                line: None,
                symbol: None,
                agent_id: None,
                session_id: None,
                ttl_days: None,
            },
        )
        .await
        .expect_err("scope=global must fail");
        assert!(matches!(err, ServiceError::BadRequest(_)));
    }

    #[tokio::test]
    async fn remember_normalizes_kind_case() {
        let (ctx, tmp) = ctx_and_project();
        let resp = remember(
            &ctx,
            RememberArgs {
                path: path_str(&tmp),
                kind: "DECISION".into(),
                content: "x".into(),
                scope: None,
                file: None,
                line: None,
                symbol: None,
                agent_id: None,
                session_id: None,
                ttl_days: None,
            },
        )
        .await
        .expect("upper-case kind should be accepted after lower-casing");
        assert_eq!(resp["kind"], "decision");
    }

    // ── recall mode selection ────────────────────────────────────────────

    /// Response carries a `mode` field telling the caller which recall path
    /// ran. We don't pin the exact value (depends on whether the embedding
    /// model is cached), but the field must always be present.
    #[tokio::test]
    async fn recall_response_carries_mode() {
        let (ctx, tmp) = ctx_and_project();
        remember(
            &ctx,
            RememberArgs {
                path: path_str(&tmp),
                kind: "decision".into(),
                content: "blake3 truncated to 16 hex chars".into(),
                scope: None,
                file: None,
                line: None,
                symbol: None,
                agent_id: None,
                session_id: None,
                ttl_days: None,
            },
        )
        .await
        .expect("first remember should succeed");
        let resp = recall(
            &ctx,
            RecallArgs {
                path: path_str(&tmp),
                query: "blake3 hash truncation".into(),
                scope: None,
                kind: None,
                limit: None,
            },
        )
        .await
        .expect("recall should not error");
        assert!(
            resp.get("mode").is_some(),
            "recall must surface a `mode` field"
        );
    }

    // ── decisions filter ─────────────────────────────────────────────────

    #[tokio::test]
    async fn decisions_filters_by_kind() {
        let (ctx, tmp) = ctx_and_project();
        for (kind, content) in [
            ("decision", "use SQLite + WAL"),
            ("gotcha", "WAL needs fsync"),
            ("decision", "schema is additive-only"),
        ] {
            remember(
                &ctx,
                RememberArgs {
                    path: path_str(&tmp),
                    kind: kind.into(),
                    content: content.into(),
                    scope: None,
                    file: None,
                    line: None,
                    symbol: None,
                    agent_id: None,
                    session_id: None,
                    ttl_days: None,
                },
            )
            .await
            .unwrap();
        }
        let resp = decisions(
            &ctx,
            DecisionsArgs {
                path: path_str(&tmp),
                scope: None,
                since: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(resp["returned"], 2, "must filter to kind=decision only");
    }

    // ── link / links ────────────────────────────────────────────────────

    #[tokio::test]
    async fn link_rejects_invalid_kind() {
        let (ctx, tmp) = ctx_and_project();
        for _ in 0..2 {
            remember(
                &ctx,
                RememberArgs {
                    path: path_str(&tmp),
                    kind: "context".into(),
                    content: "x".into(),
                    scope: None,
                    file: None,
                    line: None,
                    symbol: None,
                    agent_id: None,
                    session_id: None,
                    ttl_days: None,
                },
            )
            .await
            .unwrap();
        }
        let err = link(
            &ctx,
            LinkArgs {
                path: path_str(&tmp),
                from_id: 1,
                to_id: 2,
                kind: "follows".into(),
            },
        )
        .await
        .expect_err("kind=follows must be rejected");
        assert!(matches!(err, ServiceError::BadRequest(_)));
    }

    #[tokio::test]
    async fn links_rejects_invalid_direction() {
        let (ctx, tmp) = ctx_and_project();
        let err = links(
            &ctx,
            LinksArgs {
                path: path_str(&tmp),
                note_id: 1,
                direction: Some("sideways".into()),
            },
        )
        .await
        .expect_err("direction=sideways must be rejected");
        assert!(matches!(err, ServiceError::BadRequest(_)));
    }

    // ── sessions ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn session_round_trip_summarizes_notes() {
        let (ctx, tmp) = ctx_and_project();
        let start = session_start(
            &ctx,
            SessionStartArgs {
                path: path_str(&tmp),
                name: Some("t".into()),
                agent_id: None,
            },
        )
        .await
        .unwrap();
        let sid = start["id"]
            .as_str()
            .expect("session_id must be a string")
            .to_string();
        for kind in ["decision", "decision", "gotcha"] {
            remember(
                &ctx,
                RememberArgs {
                    path: path_str(&tmp),
                    kind: kind.into(),
                    content: format!("note for {kind}"),
                    scope: None,
                    file: None,
                    line: None,
                    symbol: None,
                    agent_id: None,
                    session_id: Some(sid.clone()),
                    ttl_days: None,
                },
            )
            .await
            .unwrap();
        }
        let end = session_end(
            &ctx,
            SessionEndArgs {
                path: path_str(&tmp),
                session_id: sid,
            },
        )
        .await
        .unwrap();
        assert_eq!(end["summary"]["total_notes"], 3);
        assert_eq!(end["summary"]["by_kind"]["decision"], 2);
        assert_eq!(end["summary"]["by_kind"]["gotcha"], 1);
        assert!(
            end["session"]["ended_at"].is_string(),
            "ended_at must be set"
        );
    }

    #[tokio::test]
    async fn session_end_unknown_id_returns_not_found() {
        let (ctx, tmp) = ctx_and_project();
        let err = session_end(
            &ctx,
            SessionEndArgs {
                path: path_str(&tmp),
                session_id: "sess-bogus".into(),
            },
        )
        .await
        .expect_err("unknown session id must be NotFound");
        assert!(matches!(err, ServiceError::NotFound(_)));
    }
}
