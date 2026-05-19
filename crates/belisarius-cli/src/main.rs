mod brief;
mod cli_error;
mod cmd_agents;
mod cmd_arch;
mod cmd_check;
mod cmd_commands;
mod cmd_context;
mod cmd_diag;
mod cmd_diff;
mod cmd_doctor;
mod cmd_fleet;
mod cmd_funcs;
mod cmd_help;
mod cmd_hooks;
mod cmd_hotspots;
mod cmd_index;
mod cmd_init;
mod cmd_mcp;
mod cmd_next;
mod cmd_quality;
mod cmd_scan;
mod cmd_search;
mod cmd_serve;
mod cmd_symbols;
mod cmd_test_gaps;
mod cmd_watch;
mod cmd_xref;
mod color;
mod fleet;
mod fleet_db;
mod function_detail;
mod help;
mod mcp;
mod mcp_config;
mod mcp_install;
mod mcp_tools;
mod output;
mod pack;
mod progress;
mod routes;
mod server;
mod service;
mod state_db;
#[cfg(feature = "embed-web")]
mod web_assets;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "belisarius", version, about = "Architectural analysis engine.")]
struct Cli {
    /// Disable ANSI color output. Also honoured: the `NO_COLOR` env var
    /// (https://no-color.org) and non-TTY stderr.
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// First-run bootstrap: scan, create dirs, pre-fetch the embedding model,
    /// probe SCIP indexers, print next steps.
    #[command(long_about = help::INIT_HELP, next_help_heading = "Indexing")]
    Init(cmd_init::InitArgs),
    /// Environment health check — probes every indexer, the search index,
    /// state.db, rules.toml, and fleet.toml. Exits non-zero on problems.
    #[command(long_about = help::DOCTOR_HELP, next_help_heading = "Indexing")]
    Doctor(cmd_doctor::DoctorArgs),
    /// Recommend the single most useful next action for the project.
    #[command(next_help_heading = "Indexing")]
    Next(cmd_next::NextArgs),
    /// Evaluate `.belisarius/rules.toml` and exit non-zero on violations.
    /// CI-friendly: identical JSON shape to the `belisarius_rules_check`
    /// MCP tool.
    #[command(long_about = help::CHECK_HELP, next_help_heading = "Quality")]
    Check(cmd_check::CheckArgs),
    /// Scan a project and emit a structural JSON report.
    #[command(next_help_heading = "Indexing")]
    Scan(cmd_scan::ScanArgs),
    /// Build a merged SCIP symbol index for a project.
    #[command(long_about = help::INDEX_HELP, next_help_heading = "Indexing")]
    Index(cmd_index::IndexArgs),
    /// Inspect / query SCIP symbol indexes.
    #[command(next_help_heading = "Indexing")]
    Symbols {
        #[command(subcommand)]
        cmd: cmd_symbols::SymbolsCmd,
    },
    /// List functions ranked by complexity.
    #[command(next_help_heading = "Quality")]
    Funcs(cmd_funcs::FuncsArgs),
    /// Compute a composite code-quality score (4 axes).
    #[command(next_help_heading = "Quality")]
    Quality(cmd_quality::QualityArgs),
    /// Run external lint/security tools (clippy, semgrep, ruff, eslint, tokei).
    #[command(next_help_heading = "Quality")]
    Diag(cmd_diag::DiagArgs),
    /// Rank files by churn × complexity over a git history window.
    #[command(next_help_heading = "Quality")]
    Hotspots(cmd_hotspots::HotspotsArgs),
    /// List source files with no covering test, ranked by complexity.
    #[command(next_help_heading = "Quality")]
    TestGaps(cmd_test_gaps::TestGapsArgs),
    /// Files changed between two git refs, overlayed with hotspots / tests / surface.
    #[command(next_help_heading = "Quality")]
    Diff(cmd_diff::DiffArgs),
    /// Discover runnable commands: package.json scripts, Justfile, Makefile, workflows.
    #[command(next_help_heading = "Quality")]
    Commands(cmd_commands::CommandsArgs),
    /// MCP stdio server (no args) + onboarding helpers: `tools`, `config`, `install`.
    #[command(long_about = help::MCP_HELP, next_help_heading = "Fleet")]
    Mcp {
        #[command(subcommand)]
        cmd: Option<cmd_mcp::McpCmd>,
    },
    /// Generate / refresh an `AGENTS.md` describing how to drive this project.
    #[command(next_help_heading = "Fleet")]
    Agents {
        #[command(subcommand)]
        cmd: cmd_agents::AgentsCmd,
    },
    /// Install / inspect git hooks that run `belisarius check` automatically.
    #[command(next_help_heading = "Fleet")]
    Hooks {
        #[command(subcommand)]
        cmd: cmd_hooks::HooksCmd,
    },
    /// Register and inspect a fleet of projects.
    #[command(next_help_heading = "Fleet")]
    Fleet(cmd_fleet::FleetArgs),
    /// Serve the HTTP API + web UI.
    #[command(next_help_heading = "Fleet")]
    Serve(cmd_serve::ServeArgs),
    /// Hybrid semantic + BM25 search (index, query, status, fetch-model).
    #[command(next_help_heading = "Context")]
    Search {
        #[command(subcommand)]
        cmd: cmd_search::SearchCmd,
    },
    /// Transitive backward call graph (who reaches this symbol).
    #[command(next_help_heading = "Context")]
    Impact(cmd_xref::ImpactArgs),
    /// Transitive forward call graph (what this symbol reaches).
    #[command(next_help_heading = "Context")]
    Flow(cmd_xref::FlowArgs),
    /// One-shot symbol 360° view: def + direct callers + direct callees.
    #[command(next_help_heading = "Context")]
    Symbol(cmd_xref::SymbolArgs),
    /// Watch the project tree and re-index changed files incrementally.
    /// Default: search index only. `--with-scip` opts into the slower SCIP
    /// rebuild path.
    #[command(next_help_heading = "Indexing")]
    Watch(cmd_watch::WatchArgs),
    /// Context artifacts registry: list / get / search / index.
    #[command(next_help_heading = "Context")]
    Context {
        #[command(subcommand)]
        cmd: cmd_context::ContextCmd,
    },
    /// Long-form prose docs: directory layout and JSON / error shapes.
    /// Named `docs` because clap reserves `help` for `--help` dispatch.
    #[command(next_help_heading = "Context")]
    Docs {
        #[command(subcommand)]
        cmd: cmd_help::HelpCmd,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for protocol output (MCP
    // JSON-RPC, `--json` flags, scan/quality reports, etc.).
    //
    // Suppress `tokei=warn` by default — it emits "Unknown MIME" lines for
    // every `<script type="module">` tag in HTML, which pollutes CI logs
    // and `belisarius check` output without conveying anything actionable.
    // Users who want the noise back can set `RUST_LOG=info,tokei=warn`.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tokei=error")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    color::init(cli.no_color);
    match cli.command {
        Command::Init(args) => cmd_init::run(args).await,
        Command::Doctor(args) => cmd_doctor::run(args).await,
        Command::Next(args) => cmd_next::run(args).await,
        Command::Check(args) => cmd_check::run(args).await,
        Command::Scan(args) => cmd_scan::run(args).await,
        Command::Index(args) => cmd_index::run(args).await,
        Command::Symbols { cmd } => cmd_symbols::run(cmd).await,
        Command::Funcs(args) => cmd_funcs::run(args).await,
        Command::Quality(args) => cmd_quality::run(args).await,
        Command::Diag(args) => cmd_diag::run(args).await,
        Command::Hotspots(args) => cmd_hotspots::run(args).await,
        Command::TestGaps(args) => cmd_test_gaps::run(args).await,
        Command::Diff(args) => cmd_diff::run(args).await,
        Command::Commands(args) => cmd_commands::run(args).await,
        Command::Mcp { cmd } => cmd_mcp::run(cmd).await,
        Command::Agents { cmd } => cmd_agents::run(cmd).await,
        Command::Hooks { cmd } => cmd_hooks::run(cmd).await,
        Command::Fleet(args) => cmd_fleet::run(args).await,
        Command::Serve(args) => cmd_serve::run(args).await,
        Command::Search { cmd } => cmd_search::run(cmd).await,
        Command::Impact(args) => cmd_xref::impact(args).await,
        Command::Flow(args) => cmd_xref::flow(args).await,
        Command::Symbol(args) => cmd_xref::symbol(args).await,
        Command::Context { cmd } => cmd_context::run(cmd).await,
        Command::Watch(args) => cmd_watch::run(args).await,
        Command::Docs { cmd } => cmd_help::run(cmd).await,
    }
}
