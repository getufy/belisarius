//! Progress UI helpers.
//!
//! `bar_for(n, json)` returns an `Option<ProgressBar>` that is:
//!   - `None` when `--json` is set (stdout must stay machine-clean), OR
//!   - `None` when stderr is not a terminal (piped / redirected / CI), OR
//!   - `Some(bar)` writing to stderr otherwise.
//!
//! Callers should hide / finish the bar before printing any final summary so
//! the terminal cursor lands cleanly. The helpers below are panic-free.

use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;

#[allow(dead_code)] // used by cmd_index / cmd_fleet; helper is module-public.
pub fn bar_for(total: u64, json: bool) -> Option<ProgressBar> {
    if json || !std::io::stderr().is_terminal() {
        return None;
    }
    let pb = ProgressBar::new(total);
    let style = ProgressStyle::with_template("{prefix:>10} [{bar:30}] {pos}/{len} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-");
    pb.set_style(style);
    pb.set_draw_target(indicatif::ProgressDrawTarget::stderr());
    Some(pb)
}

#[allow(dead_code)]
pub fn spinner(prefix: &'static str, json: bool) -> Option<ProgressBar> {
    if json || !std::io::stderr().is_terminal() {
        return None;
    }
    let pb = ProgressBar::new_spinner();
    pb.set_prefix(prefix);
    pb.set_draw_target(indicatif::ProgressDrawTarget::stderr());
    pb.enable_steady_tick(std::time::Duration::from_millis(120));
    Some(pb)
}
