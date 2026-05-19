//! Service layer — capabilities implemented once and reused by every transport
//! (HTTP, MCP, CLI). Each feature module exposes plain `async fn`s taking
//! `&AppContext` plus a typed args struct and returning `Result<_, ServiceError>`.
//!
//! Migration is route-by-route; until every legacy handler in `server.rs` and
//! `cmd_mcp.rs` has moved here, both paths coexist.

pub mod architecture;
pub mod brief;
pub mod context;
pub mod context_artifacts;
pub mod describe;
pub mod diagnostics;
pub mod error;
pub mod explain;
pub mod fleet;
pub mod function_detail;
pub mod next_action;
pub mod notes;
pub mod pack;
pub mod project;
pub mod quality;
pub mod recent_changes;
pub mod search;
pub mod similar;
pub mod state;
pub mod suggest_tests;
pub mod symbols;
