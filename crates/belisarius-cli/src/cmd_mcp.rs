//! MCP server — expose every Belisarius capability as an agent-native tool.
//!
//! Speaks JSON-RPC 2.0 over stdio per the MCP 2024-11-05 spec. Hand-rolled
//! transport keeps the dep tree tight; the tool handlers reuse the same Rust
//! functions the HTTP server already calls (no loopback).
//!
//! Wiring into Claude Code / Cursor / Claude Desktop:
//! ```jsonc
//! {
//!   "mcpServers": {
//!     "belisarius": {
//!       "command": "belisarius",
//!       "args": ["mcp"]
//!     }
//!   }
//! }
//! ```

use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::service::error::ServiceError;

const SERVER_NAME: &str = "belisarius";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Sent as `instructions` in `initialize` responses. The "search before
/// reading" philosophy steers agents to call `belisarius_search_code` and
/// `belisarius_symbol` before opening files speculatively.
const SEARCH_FIRST_INSTRUCTIONS: &str = "Belisarius indexes this repo with hybrid semantic + BM25 search and SCIP-backed symbol graphs. \
Prefer discovery over speculative reads:\n\
 1. Use `belisarius_search_code` to find code by intent (\"where do we parse SCIP?\").\n\
 2. Use `belisarius_symbol` for one-shot def + callers + callees views.\n\
 3. Use `belisarius_impact` (backward) or `belisarius_flow` (forward) for blast-radius / call traces.\n\
 4. Read files only after search has narrowed the scope to 1–3 candidates.\n\
 5. Check `belisarius_context_list` for non-code knowledge (schemas, runbooks) the maintainers have registered.";

#[derive(clap::Args)]
pub struct McpArgs {}

/// Subcommands for `belisarius mcp`. `None` runs the stdio MCP server (legacy
/// behavior — what every client config already invokes via `args: ["mcp"]`).
#[derive(clap::Subcommand)]
pub enum McpCmd {
    /// Enumerate every registered MCP tool (name + description + input schema).
    Tools(crate::mcp_tools::ToolsArgs),
    /// Print the JSON snippet a client config (Claude Code / Cursor / Claude
    /// Desktop) needs to wire Belisarius in.
    Config(crate::mcp_config::ConfigArgs),
    /// Auto-install the Belisarius MCP entry into a client's config file.
    Install(crate::mcp_install::InstallArgs),
}

pub async fn run(cmd: Option<McpCmd>) -> Result<()> {
    match cmd {
        None => run_server().await,
        Some(McpCmd::Tools(a)) => crate::mcp_tools::run(a),
        Some(McpCmd::Config(a)) => crate::mcp_config::run(a),
        Some(McpCmd::Install(a)) => crate::mcp_install::run(a),
    }
}

async fn run_server() -> Result<()> {
    let server = Arc::new(Server {
        ctx: Arc::new(crate::service::context::AppContext::new()),
        registry: crate::mcp::registry::default_registry(),
    });

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));

    // Stdio MCP is one-client, one-process. Handle requests sequentially so
    // each response flushes before we read the next line — keeps ordering
    // predictable and avoids dropping in-flight tasks at EOF.
    while let Some(line) = reader.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                emit_error(&stdout, None, McpError::parse(format!("parse error: {e}"))).await;
                continue;
            }
        };
        handle_request(server.clone(), stdout.clone(), request).await;
    }
    Ok(())
}

struct Server {
    ctx: Arc<crate::service::context::AppContext>,
    registry: crate::mcp::registry::ToolRegistry,
}

// LruCache + default_cache_cap removed — the cache moved into
// `service::context::AppContext` (key: `BELISARIUS_CACHE_CAP`, default 16).

async fn handle_request(
    server: Arc<Server>,
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    request: Value,
) {
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // Notifications (no id, fire-and-forget) — we just consume them.
    if id.is_none() {
        return;
    }

    let result: Result<Value, McpError> = match method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
            "instructions": SEARCH_FIRST_INSTRUCTIONS,
        })),
        "tools/list" => Ok(json!({ "tools": server.registry.definitions() })),
        "tools/call" => call_tool(server, params).await,
        "ping" => Ok(json!({})),
        _ => Err(McpError::method_not_found(&method)),
    };

    let response = match result {
        Ok(value) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": value,
        }),
        Err(err) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": err.to_json(),
        }),
    };
    emit(&stdout, &response).await;
}

async fn emit(stdout: &Arc<Mutex<tokio::io::Stdout>>, value: &Value) {
    let mut buf = serde_json::to_vec(value).unwrap_or_default();
    buf.push(b'\n');
    let mut guard = stdout.lock().await;
    let _ = guard.write_all(&buf).await;
    let _ = guard.flush().await;
}

async fn emit_error(stdout: &Arc<Mutex<tokio::io::Stdout>>, id: Option<Value>, err: McpError) {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": err.to_json(),
    });
    emit(stdout, &payload).await;
}

/// JSON-RPC error envelope with optional structured `data` field. Stable
/// numeric codes:
///   -32700 parse error / -32600 invalid request / -32601 method not found
///   -32603 internal / 1xxx user-level (mapped from `ServiceError`) / 2xxx system
#[derive(Debug)]
struct McpError {
    code: i32,
    message: String,
    data: Option<Value>,
}

impl McpError {
    fn parse(msg: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: msg.into(),
            data: None,
        }
    }
    fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: msg.into(),
            data: None,
        }
    }
    fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method not found: {method}"),
            data: None,
        }
    }
    fn from_service(e: &ServiceError) -> Self {
        Self {
            code: e.code(),
            message: e.to_string(),
            data: Some(e.data()),
        }
    }
    fn to_json(&self) -> Value {
        let mut obj = json!({ "code": self.code, "message": &self.message });
        if let Some(data) = &self.data {
            obj["data"] = data.clone();
        }
        obj
    }
}

// `tool_definitions()` (static JSON manifest of ~430 LOC) deleted — every
// tool is now served by `server.registry.definitions()`, which generates
// the same JSON shape from the feature-module `ToolSpec` entries. Add new
// tools by registering a `ToolSpec` in `mcp::registry::default_registry()`.
#[allow(dead_code)]
fn _deleted_tool_definitions_marker() {}

async fn call_tool(server: Arc<Server>, params: Value) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_request("missing tool name"))?
        .to_string();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    let Some(spec) = server.registry.get(name.as_str()) else {
        return Err(McpError::invalid_request(format!("unknown tool: {name}")));
    };

    let outcome = (spec.handler)(server.ctx.clone(), args).await;
    Ok(match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }],
            "isError": false,
        }),
        Err(e) => {
            let err = McpError::from_service(&e);
            json!({
                "content": [{ "type": "text", "text": err.message }],
                "isError": true,
                "error": err.data.unwrap_or(Value::Null),
            })
        }
    })
}

// All MCP tool implementations now live in their respective `service::*`
// modules and are routed through the `mcp::registry` populated by
// `default_registry()`. The legacy match in `handle_request` is empty —
// every call falls through to the registry.
//
// Behaviour changes folded in during migration:
//   - tool_fleet_list previously swallowed a corrupt fleet.toml with
//     unwrap_or_default(); now surfaces the parse error like HTTP.
//   - tool_search_symbols previously omitted the `kind` field; the unified
//     shape includes it.
//   - tool_impact / tool_flow / tool_symbol previously reloaded SCIP per
//     call; now reuse the cached SymbolStore from AppContext.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_error_from_missing_index_carries_data() {
        let err = McpError::from_service(&ServiceError::missing_index(
            "scip",
            "run `belisarius index .`",
        ));
        assert_eq!(err.code, 1003);
        let json = err.to_json();
        let data = &json["data"];
        assert_eq!(data["code"], 1003);
        assert_eq!(data["kind"], "missing_index");
        assert_eq!(data["which"], "scip");
        assert_eq!(data["remediation"], "run `belisarius index .`");
    }

    #[test]
    fn mcp_error_parse_has_no_data_field() {
        let err = McpError::parse("bad json");
        let json = err.to_json();
        assert_eq!(json["code"], -32700);
        assert!(json.get("data").is_none());
    }

    #[test]
    fn mcp_error_method_not_found_uses_jsonrpc_code() {
        let err = McpError::method_not_found("tools/peek");
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("tools/peek"));
    }
}
