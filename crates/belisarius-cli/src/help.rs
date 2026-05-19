//! Long-form help text constants. Pulling these out of clap attribute
//! macros keeps them readable, lintable, and easy to update in one place.
//!
//! Each constant follows a two-section template:
//!
//! ```text
//! <one-line summary repeated for symmetry with the clap `about`>
//!
//! When to use
//!   <prose>
//!
//! Example
//!   <bash example>
//! ```

pub const INIT_HELP: &str = "First-run bootstrap.

When to use
  Right after `git clone`, or any time `.belisarius/` looks broken.
  Idempotent — running it again on an initialized project is a no-op
  for state but re-prints the probe summary.

Example
  belisarius init .
  belisarius init my-project --skip-model    # offline / CI";

pub const DOCTOR_HELP: &str = "Environment health check.

When to use
  After install, when something looks off, or in CI before any
  agent-facing command runs. Probes every SCIP indexer, the search
  index, state.db, rules.toml, and fleet.toml. Exits non-zero when
  any probe needs attention.

Example
  belisarius doctor .
  belisarius doctor . --json    # for CI machinery";

pub const CHECK_HELP: &str = "Evaluate `.belisarius/rules.toml`.

When to use
  In CI, as a merge gate. Exit code is 1 when any rule is violated.
  JSON output is byte-identical to the `belisarius_rules_check` MCP
  tool — agents and CI see the same shape.

Example
  belisarius check .
  belisarius check . --json
  belisarius check . --no-fail    # preview without blocking";

pub const INDEX_HELP: &str = "Build SCIP symbol indexes (optionally also scan + search).

When to use
  After source files change in ways that affect symbols or imports,
  or for a fresh end-to-end index with `--all`. Default behavior
  (SCIP only) matches the legacy `belisarius index`.

Example
  belisarius index .                  # SCIP only, all detected langs
  belisarius index . --all            # scan + SCIP + search
  belisarius index . --lang rust      # only the Rust indexer
  belisarius index . --skip scip      # everything but SCIP
  belisarius index . --with-search    # SCIP + search, no scan";

pub const MCP_HELP: &str = "Speak MCP (Model Context Protocol) over stdio.

When to use
  Wired into Claude Code, Cursor, Claude Desktop, or any MCP client.
  Errors carry `code` / `kind` / `remediation` in JSON-RPC `error.data`;
  list-returning tools return `total_count` / `returned` / `truncated`
  so agents can detect pagination.

Example
  belisarius mcp                      # invoked by an MCP client";

/// Block printed by `belisarius help layout`.
pub const LAYOUT_HELP: &str = ".belisarius/ directory layout

  config.toml           project-level settings (cache caps, defaults)
  rules.toml            architectural constraints (cycles, dead code, cc)
  context_artifacts.json  registered context (schemas, runbooks, specs)
  fleet.toml            multi-project registry (apps + roots)
  state.db              SQLite — pins, snapshots, hot-fn fingerprints
  scip/                 per-language `.scip` + `merged.scip`
  search/               tantivy BM25 + vector chunks
  scip/merged.scip      what `Symbol` / `Impact` / `Flow` tools read
";

/// Block printed by `belisarius help json`.
pub const JSON_HELP: &str = "Canonical JSON shapes

Every list-returning tool wraps results with:
  total_count   total candidates before `limit`
  returned      items in this response
  truncated     true when `returned < total_count`
  next_offset   pass back as `offset` to page (when supported)

MCP tool errors carry structured `error` data:
  code          1001 bad_request · 1002 not_found · 1003 missing_index
                1004 feature_missing · 2001 internal
  kind          short string discriminant
  remediation   shell command to fix it (e.g. `belisarius index .`)
  which         present on missing_index: 'scip' | 'search' | …

JSON-RPC envelope errors use standard codes:
  -32700  parse error
  -32600  invalid request
  -32601  method not found
  -32603  internal
";
