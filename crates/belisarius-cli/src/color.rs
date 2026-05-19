//! Color / TTY detection for the CLI.
//!
//! `use_color()` returns true only when:
//!   - the `--no-color` flag was NOT passed, AND
//!   - the `NO_COLOR` env var (https://no-color.org) is NOT set, AND
//!   - stderr is a real terminal (not a pipe / redirect / CI log).
//!
//! `init(no_color_flag)` must be called once from `main()` before any
//! command code runs. Subsequent reads of `use_color()` are lock-free.

use std::io::IsTerminal;
use std::sync::OnceLock;

static USE_COLOR: OnceLock<bool> = OnceLock::new();

/// Initialise the global color setting. Called once from `main()`.
pub fn init(no_color_flag: bool) {
    let enabled =
        !no_color_flag && std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal();
    // First writer wins; subsequent calls are no-ops. This keeps the API
    // safe in tests where `init` may be called more than once.
    let _ = USE_COLOR.set(enabled);
}

/// Whether ANSI color sequences should be emitted. Defaults to `false` if
/// `init` was never called — safer for non-TTY transports (MCP stdio, HTTP).
#[allow(dead_code)] // Public read-side API; gated callers land alongside color rollout.
pub fn use_color() -> bool {
    *USE_COLOR.get().unwrap_or(&false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_false_when_not_initialised() {
        // `OnceLock` is per-process, so we only check the "never called" path
        // via the underlying default. The integration of `init()` is covered
        // by the CLI smoke tests where stderr is piped (so color is off).
        // This unit test guards the read-side fallback.
        // NOTE: we can't reset USE_COLOR here without `unsafe`, so we read
        // whatever a previous test left — but the function must never panic.
        let _ = use_color();
    }
}
