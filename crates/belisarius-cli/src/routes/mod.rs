//! Feature-module routers. Each `<feature>::router()` returns a `Router<AppState>`
//! that `server.rs` `.merge`es into the top-level router. As migrations
//! complete, the matching `.route()` line in `server.rs` is removed.

pub mod architecture;
pub mod brief;
pub mod context_artifacts;
pub mod diagnostics;
pub mod fleet;
pub mod function_detail;
pub mod pack;
pub mod project;
pub mod quality;
pub mod search;
pub mod state;
pub mod symbols;
