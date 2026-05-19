//! `belisarius mcp tools` — enumerate every registered MCP tool without
//! actually speaking MCP. Useful for agents that want a one-shot capability
//! sweep, for docs generation, and for sanity-checking that a new feature
//! actually registered its `tool_spec()`.
//!
//! Two output modes:
//! - Human: aligned `name  description` table sorted by name.
//! - JSON: the exact `tools/list` payload an MCP `initialize` would receive,
//!   so an agent can prime its tool catalog from a single shell call.

use anyhow::Result;
use serde_json::Value;

use crate::mcp::registry::default_registry;

#[derive(clap::Args)]
pub struct ToolsArgs {
    /// Emit the same JSON payload an MCP `tools/list` call would return.
    /// Pipe into `jq` for filtering.
    #[arg(long)]
    pub json: bool,
    /// Print full input-schema JSON for each tool. Human mode only — JSON
    /// mode always includes schemas.
    #[arg(long)]
    pub schemas: bool,
    /// Filter tools whose name contains this substring (case-insensitive).
    #[arg(long)]
    pub filter: Option<String>,
}

pub fn run(args: ToolsArgs) -> Result<()> {
    let defs: Vec<Value> = default_registry().definitions();
    let filter = args.filter.as_deref().map(|s| s.to_lowercase());

    let filtered: Vec<&Value> = defs
        .iter()
        .filter(|t| match &filter {
            None => true,
            Some(q) => t
                .get("name")
                .and_then(|v| v.as_str())
                .map(|n| n.to_lowercase().contains(q))
                .unwrap_or(false),
        })
        .collect();

    if args.json {
        let arr = Value::Array(filtered.into_iter().cloned().collect());
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    let name_width = filtered
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
        .map(str::len)
        .max()
        .unwrap_or(0);

    println!("{} tools registered\n", filtered.len());
    for tool in &filtered {
        let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = tool
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");
        println!("  {:<width$}  {}", name, desc, width = name_width);
        if args.schemas {
            if let Some(schema) = tool.get("inputSchema") {
                let s = serde_json::to_string_pretty(schema).unwrap_or_default();
                for line in s.lines() {
                    println!("      {line}");
                }
                println!();
            }
        }
    }
    Ok(())
}
