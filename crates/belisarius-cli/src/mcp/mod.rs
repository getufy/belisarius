//! MCP transport plumbing — a tool registry that lets each feature module own
//! its own tool's name, schema, and handler in the same file as its service
//! function. The legacy `call_tool` match in `cmd_mcp.rs` is consulted as a
//! fallback while migrations are still in flight; tools land in the registry
//! one feature at a time.

pub mod registry;
