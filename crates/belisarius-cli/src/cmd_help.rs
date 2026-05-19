//! `belisarius help layout` / `belisarius help json` — prose subcommands
//! that document the `.belisarius/` directory layout and the canonical
//! JSON / error shapes. Kept separate from clap's built-in `--help` so the
//! tables stay readable in a terminal.

use anyhow::Result;

#[derive(clap::Subcommand)]
pub enum HelpCmd {
    /// Print the `.belisarius/` directory layout.
    Layout,
    /// Print the canonical JSON / error shapes every tool emits.
    Json,
}

pub async fn run(cmd: HelpCmd) -> Result<()> {
    let text = match cmd {
        HelpCmd::Layout => crate::help::LAYOUT_HELP,
        HelpCmd::Json => crate::help::JSON_HELP,
    };
    print!("{text}");
    Ok(())
}
