//! Tool registry — replaces the giant 40-arm `call_tool` match in cmd_mcp.rs
//! with a `HashMap<&str, ToolSpec>` populated at startup. Each feature module
//! contributes one (or more) `ToolSpec`s in its own file.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::service::{context::AppContext, error::ServiceError};

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type ToolHandler = fn(Arc<AppContext>, Value) -> BoxFut<Result<Value, ServiceError>>;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub handler: ToolHandler,
}

pub struct ToolRegistry {
    map: HashMap<&'static str, ToolSpec>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn register(&mut self, spec: ToolSpec) {
        self.map.insert(spec.name, spec);
    }

    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.map.get(name)
    }

    /// JSON definitions for `tools/list` — uses the same shape as the legacy
    /// `tool_definitions()` so a client can't tell whether a tool is served
    /// by the registry or the legacy match.
    pub fn definitions(&self) -> Vec<Value> {
        let mut defs: Vec<Value> = self
            .map
            .values()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "inputSchema": s.input_schema,
                })
            })
            .collect();
        defs.sort_by(|a, b| {
            a.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
        });
        defs
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the default registry with every migrated tool registered. Add a line
/// here whenever a feature module ships its service function.
pub fn default_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(crate::service::brief::tool_spec());
    r.register(crate::service::quality::tool_spec());
    r.register(crate::service::pack::tool_spec());
    for spec in crate::service::symbols::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::search::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::fleet::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::state::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::project::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::context_artifacts::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::recent_changes::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::describe::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::explain::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::suggest_tests::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::notes::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::architecture::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::similar::tool_specs() {
        r.register(spec);
    }
    for spec in crate::service::next_action::tool_specs() {
        r.register(spec);
    }
    r
}
