//! SCIP-backed symbol capabilities: status / search / refs / callers / file / trace.
//!
//! Before this module landed, HTTP cached the `SymbolStore` (mtime-keyed)
//! while MCP rebuilt it on every call — the unified service caches once and
//! both transports inherit the win. `belisarius_search_symbols` previously
//! omitted the `kind` field from its hits; the unified shape matches HTTP and
//! now always includes it.

use std::sync::Arc;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::{context::AppContext, error::ServiceError};

// ─── Response shapes ─────────────────────────────────────────────────────

/// `GET /api/symbols/status` response. The discriminated union mirrors the
/// existing JSON: `exists: true` carries index stats, `exists: false`
/// carries a `hint` telling the caller how to build the index.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
#[serde(untagged)]
pub enum SymbolsStatusResponse {
    Exists {
        exists: bool,
        path: String,
        // `u64` would land as `bigint` in TS; a Unix timestamp in seconds
        // fits comfortably in `f64` (and the frontend has always treated
        // it as `number`). Pin the TS shape explicitly.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        scip_mtime_unix: u64,
        documents: u32,
        symbols: u32,
    },
    Missing {
        exists: bool,
        path: String,
        hint: String,
    },
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolMatch {
    pub symbol: String,
    pub display_name: String,
    pub occurrences: u32,
    pub kind: i32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolsSearchResponse {
    pub matches: Vec<SymbolMatch>,
    /// Symbols in the current page (= `matches.len()`).
    #[serde(default)]
    pub returned: u32,
    /// `true` when `returned == limit` and more matches likely exist.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolOccurrence {
    pub path: String,
    pub start_line: i32,
    pub start_char: i32,
    pub end_line: i32,
    pub end_char: i32,
    pub is_definition: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct RefsByFile {
    pub file: String,
    pub refs: Vec<SymbolOccurrence>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct RefsResponse {
    pub symbol: String,
    pub files: u32,
    pub total: u32,
    pub groups: Vec<RefsByFile>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolsCallerEntry {
    pub symbol: String,
    pub display_name: String,
    pub call_sites: Vec<SymbolOccurrence>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CallersResponse {
    pub symbol: String,
    pub callers: Vec<SymbolsCallerEntry>,
    pub callers_count: u32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolDefinition {
    pub symbol: String,
    pub display_name: String,
    pub kind: i32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct SymbolFileResponse {
    pub path: String,
    pub definition_count: u32,
    pub incoming_refs: u32,
    pub outgoing_refs: u32,
    pub total_occurrences: u32,
    pub defines_truncated_to: u32,
    pub defines: Vec<SymbolDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct PathArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    pub path: String,
    #[serde(alias = "q")]
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SymbolRefArgs {
    pub path: String,
    pub sym: String,
}

#[derive(Debug, Deserialize)]
pub struct SymbolFileArgs {
    pub path: String,
    pub file: String,
}

pub async fn status(ctx: &AppContext, args: PathArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let scip = AppContext::scip_path_for(&project);
    let resp = match std::fs::metadata(&scip).and_then(|m| m.modified()) {
        Ok(mtime) => {
            let store = ctx.load_symbols(&project).await?;
            let secs = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            SymbolsStatusResponse::Exists {
                exists: true,
                path: scip.display().to_string(),
                scip_mtime_unix: secs,
                documents: store.document_count() as u32,
                symbols: store.symbol_count() as u32,
            }
        }
        Err(_) => SymbolsStatusResponse::Missing {
            exists: false,
            path: scip.display().to_string(),
            hint: format!("run `belisarius index {project}` to build the symbol index"),
        },
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn search(ctx: &AppContext, args: SearchArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    if args.query.trim().is_empty() {
        return serde_json::to_value(SymbolsSearchResponse {
            matches: vec![],
            returned: 0,
            truncated: false,
        })
        .map_err(Into::into);
    }
    let store = ctx.load_symbols(&project).await?;
    let limit = args.limit.unwrap_or(50);
    let hits = store.find_symbols(&args.query, limit);
    let matches: Vec<SymbolMatch> = hits
        .iter()
        .map(|h| SymbolMatch {
            symbol: h.symbol.to_string(),
            display_name: h.info.map(|i| i.display_name.clone()).unwrap_or_default(),
            occurrences: h.occurrences as u32,
            kind: h.info.map(|i| i.kind).unwrap_or(0),
        })
        .collect();
    let returned = matches.len() as u32;
    let truncated = (returned as usize) >= limit;
    serde_json::to_value(SymbolsSearchResponse {
        matches,
        returned,
        truncated,
    })
    .map_err(Into::into)
}

pub async fn refs(ctx: &AppContext, args: SymbolRefArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let occs = store.occurrences_of(&args.sym);
    let mut by_file: std::collections::BTreeMap<String, Vec<SymbolOccurrence>> = Default::default();
    for o in occs {
        let r = o.range();
        let hit = SymbolOccurrence {
            path: o.path().to_string(),
            start_line: r.start_line,
            start_char: r.start_char,
            end_line: r.end_line,
            end_char: r.end_char,
            is_definition: (o.occurrence.symbol_roles
                & belisarius_symbols::SymbolRole::Definition as i32)
                != 0,
        };
        by_file.entry(o.path().to_string()).or_default().push(hit);
    }
    let total = by_file.values().map(|v| v.len()).sum::<usize>() as u32;
    let groups: Vec<RefsByFile> = by_file
        .into_iter()
        .map(|(file, refs)| RefsByFile { file, refs })
        .collect();
    let resp = RefsResponse {
        symbol: args.sym,
        files: groups.len() as u32,
        total,
        groups,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn callers(ctx: &AppContext, args: SymbolRefArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let cs = store.callers_of(&args.sym);
    let callers: Vec<SymbolsCallerEntry> = cs
        .iter()
        .map(|c| SymbolsCallerEntry {
            symbol: c.symbol.clone(),
            display_name: c.info.map(|i| i.display_name.clone()).unwrap_or_default(),
            call_sites: c
                .call_sites
                .iter()
                .map(|o| {
                    let r = o.range();
                    SymbolOccurrence {
                        path: o.path().to_string(),
                        start_line: r.start_line,
                        start_char: r.start_char,
                        end_line: r.end_line,
                        end_char: r.end_char,
                        is_definition: false,
                    }
                })
                .collect(),
        })
        .collect();
    let resp = CallersResponse {
        symbol: args.sym,
        callers_count: callers.len() as u32,
        callers,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

pub async fn file(ctx: &AppContext, args: SymbolFileArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let summary = store.file_summary(&args.file).ok_or_else(|| {
        ServiceError::not_found(format!("no document at {} in this index", args.file))
    })?;
    let defines: Vec<SymbolDefinition> = summary
        .defines
        .iter()
        .take(200)
        .map(|s| SymbolDefinition {
            symbol: s.symbol.clone(),
            display_name: s.display_name.clone(),
            kind: s.kind,
        })
        .collect();
    let resp = SymbolFileResponse {
        path: summary.path.to_string(),
        definition_count: summary.definition_count as u32,
        incoming_refs: summary.incoming_refs as u32,
        outgoing_refs: summary.outgoing_refs as u32,
        total_occurrences: summary.total_occurrences as u32,
        defines_truncated_to: defines.len() as u32,
        defines,
    };
    serde_json::to_value(resp).map_err(Into::into)
}

#[derive(Debug, Deserialize)]
pub struct XrefArgs {
    pub path: String,
    /// Accept both `sym` (HTTP) and `symbol` (MCP).
    #[serde(alias = "sym")]
    pub symbol: String,
    /// Accept both `depth` (HTTP) and `max_depth` (MCP legacy).
    #[serde(default, alias = "max_depth")]
    pub depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SymbolArgs {
    pub path: String,
    #[serde(alias = "sym")]
    pub symbol: String,
}

pub async fn impact(ctx: &AppContext, args: XrefArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let depth = args.depth.unwrap_or(3);
    let report = store.impact_of(&args.symbol, depth);
    serde_json::to_value(report).map_err(Into::into)
}

pub async fn flow(ctx: &AppContext, args: XrefArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let depth = args.depth.unwrap_or(3);
    let report = store.flow_from(&args.symbol, depth);
    serde_json::to_value(report).map_err(Into::into)
}

pub async fn symbol_360(ctx: &AppContext, args: SymbolArgs) -> Result<Value, ServiceError> {
    let project = ctx.resolve_path(&args.path);
    let store = ctx.load_symbols(&project).await?;
    let view = store.symbol_360(&args.symbol);
    serde_json::to_value(view).map_err(Into::into)
}

// ─── MCP tool registrations ──────────────────────────────────────────────

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "belisarius_search_symbols",
            description: "Substring match over the SCIP symbol index. Returns symbol ids with display names, occurrence counts, and kind.\n\n\
When to use: turning a function or type name into a SCIP symbol id you can pass to `belisarius_symbol` / `belisarius_who_calls` / `belisarius_what_does_this_call`.\n\
When not to use: intent-based search (use `belisarius_search_code`); when you already have the symbol id.\n\n\
Requires `belisarius index <path>` first (SCIP, not the search index).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "q"],
                "properties": {
                    "path": { "type": "string", "description": "Project root (or fleet name)." },
                    "q": { "type": "string", "description": "Substring to match against symbol names." },
                    "limit": { "type": "integer", "default": 50 }
                }
            }),
            handler: handle_search as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_who_calls",
            description: "Transitive backward call graph — who reaches this symbol? Uses the SCIP index. Capped at 200 nodes.\n\nWhen to use: blast-radius analysis before refactoring or deleting a symbol. \"If I change this, what else moves?\"\nWhen not to use: forward exploration (use `belisarius_what_does_this_call`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "symbol"],
                "properties": {
                    "path": { "type": "string" },
                    "symbol": { "type": "string" },
                    "max_depth": { "type": "integer", "default": 3, "minimum": 1, "maximum": 8 }
                }
            }),
            handler: handle_impact as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_what_does_this_call",
            description: "Transitive forward call graph — what this symbol reaches. Uses the SCIP index. Capped at 200 nodes.\n\nWhen to use: understanding what a function depends on before reading its body in full. \"What do I need to know to follow this code?\"\nWhen not to use: backward exploration (use `belisarius_who_calls`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "symbol"],
                "properties": {
                    "path": { "type": "string" },
                    "symbol": { "type": "string" },
                    "max_depth": { "type": "integer", "default": 3, "minimum": 1, "maximum": 8 }
                }
            }),
            handler: handle_flow as ToolHandler,
        },
        ToolSpec {
            name: "belisarius_symbol",
            description: "One-shot 360° view of a symbol: definition + direct callers + direct callees + counts.\n\n\
When to use: the canonical way to look up a single symbol. Cheaper than calling `belisarius_who_calls` + `belisarius_what_does_this_call` + `belisarius_snippet` separately.\n\
When not to use: transitive blast radius beyond direct neighbors (use `belisarius_who_calls` / `belisarius_what_does_this_call` with `max_depth`).",
            input_schema: json!({
                "type": "object",
                "required": ["path", "symbol"],
                "properties": {
                    "path": { "type": "string" },
                    "symbol": { "type": "string" }
                }
            }),
            handler: handle_symbol as ToolHandler,
        },
    ]
}

fn handle_search(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SearchArgs = serde_json::from_value(args)?;
        search(&ctx, args).await
    })
}

fn handle_impact(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: XrefArgs = serde_json::from_value(args)?;
        impact(&ctx, args).await
    })
}

fn handle_flow(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: XrefArgs = serde_json::from_value(args)?;
        flow(&ctx, args).await
    })
}

fn handle_symbol(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SymbolArgs = serde_json::from_value(args)?;
        symbol_360(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::error::ServiceError;

    /// `status` should NEVER report a missing SCIP index as `MissingIndex` —
    /// it's the discovery probe; clients use it to find out whether they
    /// need to run `belisarius index`. A 412 here would defeat the purpose.
    #[tokio::test]
    async fn status_returns_exists_false_when_no_scip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = Arc::new(AppContext::new());
        let res = status(
            &ctx,
            PathArgs {
                path: tmp.path().to_string_lossy().into_owned(),
            },
        )
        .await
        .expect("status must not fail on missing index");
        assert_eq!(res.get("exists").and_then(|v| v.as_bool()), Some(false));
        assert!(res.get("hint").is_some());
    }

    /// Every other symbol tool must return `MissingIndex` (translated to HTTP
    /// 412) when the SCIP file is absent — gives callers a deterministic,
    /// actionable error instead of the cryptic "loading scip" anyhow chain
    /// the legacy code emitted.
    #[tokio::test]
    async fn search_reports_missing_index_when_no_scip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = Arc::new(AppContext::new());
        let err = search(
            &ctx,
            SearchArgs {
                path: tmp.path().to_string_lossy().into_owned(),
                query: "anything".into(),
                limit: None,
            },
        )
        .await
        .expect_err("should be MissingIndex without a SCIP file");
        assert!(matches!(err, ServiceError::MissingIndex { .. }));
    }

    /// Both transports route through the same registry handler — proven by
    /// asserting the registry-side and direct-call results are bit-identical
    /// on a project that *does* have a SCIP file. We don't have one in tests,
    /// so we settle for: registry returns the same kind of error as the
    /// direct call when the index is missing.
    #[tokio::test]
    async fn search_http_and_mcp_agree_on_missing_index(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = Arc::new(AppContext::new());
        let path = tmp.path().to_string_lossy().into_owned();
        let direct = search(
            &ctx,
            SearchArgs {
                path: path.clone(),
                query: "x".into(),
                limit: None,
            },
        )
        .await;
        let spec = tool_specs()
            .into_iter()
            .find(|s| s.name == "belisarius_search_symbols")
            .expect("tool spec registered");
        let via_registry =
            (spec.handler)(ctx.clone(), serde_json::json!({ "path": path, "q": "x" })).await;
        match (direct, via_registry) {
            (Err(ServiceError::MissingIndex { .. }), Err(ServiceError::MissingIndex { .. })) => {
                Ok(())
            }
            other => {
                let err: Box<dyn std::error::Error> =
                    Box::new(crate::cli_error::CliError::internal(format!(
                        "expected MissingIndex from both transports, got {other:?}"
                    )));
                Err(err)
            }
        }
    }
}
