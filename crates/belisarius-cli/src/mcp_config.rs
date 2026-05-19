//! `belisarius mcp config` — print the JSON snippet a Claude Code / Cursor /
//! Claude Desktop config file needs to wire Belisarius in as an MCP server.
//!
//! Previously the `Justfile` carried this responsibility (`just mcp-config`),
//! which assumed every user has `just`. Promoting it into the CLI itself
//! means a fresh `belisarius install-global` is enough to bootstrap an MCP
//! client — no Just / Make dependency.
//!
//! Three shapes are supported via `--client`:
//! - `claude-code` / `cursor`: nested under a top-level `"mcpServers"` key,
//!   the shape these clients expect in their settings.json.
//! - `claude-desktop`: identical to `claude-code` today, kept as a named
//!   option so we can tweak it later without churning user-facing flags.
//! - `generic` (default): bare snippet — useful for piping into another
//!   merger or for clients that read MCP entries in their own format.

use anyhow::Result;
use serde_json::{json, Value};

#[derive(clap::Args)]
pub struct ConfigArgs {
    /// Which client shape to emit. Defaults to `generic`.
    #[arg(long, value_enum, default_value_t = ClientKind::Generic)]
    pub client: ClientKind,
    /// Override the resolved `belisarius` binary path. Useful when generating
    /// configs ahead of an install. Default: `which belisarius` or
    /// `belisarius` if not on PATH.
    #[arg(long)]
    pub bin: Option<String>,
    /// Server entry key under `mcpServers`. Default: `belisarius`. Use this
    /// to disambiguate when running multiple Belisarius checkouts side-by-side.
    #[arg(long, default_value = "belisarius")]
    pub name: String,
    /// Emit a flat JSON value (the server entry only) without an outer envelope.
    /// Same as `--client generic`.
    #[arg(long)]
    pub bare: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ClientKind {
    Generic,
    ClaudeCode,
    Cursor,
    ClaudeDesktop,
}

pub fn run(args: ConfigArgs) -> Result<()> {
    let bin = args
        .bin
        .clone()
        .or_else(resolve_bin)
        .unwrap_or_else(|| "belisarius".to_string());
    let entry = json!({
        "command": bin,
        "args": ["mcp"],
    });
    let value = if args.bare || matches!(args.client, ClientKind::Generic) {
        json!({ &args.name: entry })
    } else {
        json!({ "mcpServers": { &args.name: entry } })
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Best-effort `which belisarius`. Falls back to `None` so the caller can
/// substitute a literal string.
fn resolve_bin() -> Option<String> {
    // `std::env::current_exe()` returns the running binary — exactly what a
    // client should call, even when there's no `belisarius` on PATH yet.
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Public helper so `mcp install` (and `init`) can reuse the same JSON shape
/// without duplicating the schema.
pub fn server_entry(bin: &str) -> Value {
    json!({ "command": bin, "args": ["mcp"] })
}
