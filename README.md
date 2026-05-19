# Belisarius

[![ci](https://github.com/getufy/belisarius/actions/workflows/ci.yml/badge.svg)](https://github.com/getufy/belisarius/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

An analysis engine that walks code, indexes its symbols, and computes
structural quality metrics — exposed natively as an MCP server for agents.

Inspired by [`sentrux/sentrux`](https://github.com/sentrux/sentrux).

## What it ships

- **Scan + graph** — gitignore-aware file walk, regex-based import edges for
  TS/JS/Rust/Python/Go (Rust `mod` declarations and Go block-import form
  included), resolver-aware in/out-degree + entry-point + Lakos depth.
  Multiline TS imports and dynamic `require()` are not detected by the regex
  pass.
- **AST metrics** — per-function name, line range, parameter count, cyclomatic
  + cognitive complexity via tree-sitter for **Rust, TS, TSX, JS, Python, Go**.
  Arrow functions and inline lambdas are not counted as separate functions in
  v1 — they roll up into their enclosing named function.
- **Code-quality score** — composite 0–100 over four axes (complexity,
  acyclicity, dead code, fan balance) + top-issues list.
- **Hybrid code search** — semantic (fastembed bge-small-en-v1.5, 384d) + BM25
  (tantivy with a code-aware tokenizer that splits camelCase / snake_case)
  fused via Reciprocal Rank Fusion. AST-aware chunking at function boundaries
  with a 60-line window fallback. Single-binary, no Docker.
- **SCIP symbols** — orchestrate per-language indexers (rust-analyzer,
  scip-typescript, scip-python, scip-go), merge the SCIP indexes, expose
  search / refs / callers / **impact (backward transitive callers)** /
  **flow (forward transitive callees)** / **360° symbol view**.
- **Context artifacts** — register non-code knowledge (schemas, runbooks,
  API specs) in `.belisarius/context_artifacts.json`; semantically indexed
  alongside source so an agent can discover the right reference at the
  right moment.
- **MCP server** — every capability exposed as an agent-native tool over
  stdio (the primary interface). The `initialize` response includes
  "search-before-reading" instructions steering agents to prefer
  `belisarius_search_code` over speculative file reads.
- **HTTP API + Web UI** — single-page Preact app with tabs for Overview,
  **Search**, Architecture, Graph (Cytoscape.js + dagre, right-click for
  blast-radius), Treemap, DSM, Dead, Functions, Components, Surface,
  Quality, Hotspots, Test gaps, **Impact**, Commands, Diagnostics, Markers,
  Symbols, **Context**.

## Layout

```
crates/
  belisarius-core/      shared types (Scan, Graph, FunctionInfo, Quality, AnalysisReport)
  belisarius-scan/      walker, regex imports, tree-sitter AST, cycles (Tarjan), depth, quality
  belisarius-symbols/   SCIP reader + indexer orchestration + impact/flow/symbol_360
  belisarius-search/    chunker, fastembed embedder, tantivy BM25, RRF fusion
  belisarius-context/   non-code knowledge registry (uses belisarius-search pipeline)
  belisarius-cli/       `belisarius` binary (scan / index / symbols / funcs / quality / mcp / serve / search / impact / flow / symbol / context)
web/                    Vite + Preact + Tailwind app (Cytoscape.js + dagre graph)
```

## Build

```sh
cargo build --release
cd web && pnpm install && pnpm run build && cd ..
```

> If `pnpm install` complains about ignored build scripts for `esbuild`, run once:
> `pnpm config set verify-deps-before-run false`
> The native binary is delivered by the platform-specific optional dep, so the
> postinstall is not required.

## Install globally + onboard in one command

For day-to-day MCP use, install the binary into `~/.cargo/bin/` once and
let Belisarius wire itself into every MCP client it can find.

```sh
# Install the binary (~/.cargo/bin/belisarius)
cargo install --path crates/belisarius-cli --locked
# or: just install-global

# Auto-wire into Claude Code, Claude Desktop, and Cursor (whichever you have)
belisarius mcp install

# Verify (Claude Code shown — equivalent for other clients)
claude mcp list   # belisarius: belisarius mcp - ✓ Connected
```

`belisarius mcp install` is idempotent: it edits each client's JSON config
atomically, preserves sibling MCP servers and unrelated keys, and refuses
to overwrite a conflicting entry without `--force`. Pass `--client
claude-code|claude-desktop|cursor` to target a specific one, or `--dry-run`
to preview the result without touching disk. Pair with `belisarius mcp
tools` to see every tool the server exposes, or `belisarius mcp config`
to print a copy-paste snippet for clients we don't yet auto-detect.

Alternatively, the repo ships a project-scoped `.mcp.json` — any MCP
client that respects project scope picks Belisarius up when you open this
directory, as long as `belisarius` is on `PATH`.

## First-run bootstrap

```sh
belisarius init               # default: current directory
belisarius init --all         # bootstrap + index + AGENTS.md + git hook
```

Scans the project, creates `.belisarius/` subdirectories, pre-fetches the
embedding model (~33 MB), probes which SCIP indexers are installed and
applicable, and prints a "what to run next" summary. Idempotent.

Flags worth knowing:

- `--skip-model` — don't fetch embeddings (use when offline; hybrid search
  falls back to BM25-only until you re-run).
- `--index` — also build the hybrid search index in the same call.
- `--agents` — drop an `AGENTS.md` describing how to drive this repo with
  Belisarius. Refreshes in-place on re-run; never stomps a hand-written file.
- `--hooks` — install a non-blocking pre-commit hook that runs
  `belisarius check --no-fail`. Safe on existing repos: the hook carries a
  marker so future runs detect ownership.
- `--all` — every optional step at once.
- `--json` — emit a machine-readable report instead of the human stream.
  Same shape an agent would consume.

Sub-commands for finer control:

```sh
belisarius agents init            # write/refresh AGENTS.md
belisarius hooks install          # add pre-commit hook
belisarius hooks status           # which hooks are ours vs. external
belisarius hooks uninstall        # remove only hooks we wrote
```

## Quickstart

```sh
# Scan + structural metrics
belisarius scan .                    # cheap: files + raw graph
belisarius scan . --with-ast         # heavyweight: full AnalysisReport
belisarius funcs .                   # ranked function table
belisarius quality .                 # 4-axis score + top issues

# Symbols (requires per-language indexer installed)
belisarius index .                   # build merged.scip
belisarius symbols search Registry
belisarius symbols refs 'rust-analyzer cargo … Registry#'
belisarius symbols callers '…'

# Serve API + UI
belisarius serve --web-dir web/dist  # http://127.0.0.1:7878

# Hybrid code search (bm25_only is fast and needs no model download)
belisarius search index .             # full hybrid (downloads ~33MB model on first run)
belisarius search index . --bm25-only # skip embeddings
belisarius search query "where do we parse SCIP" .
belisarius search status .            # JSON: state, chunks, model

# Transitive call graphs (require SCIP index from `belisarius index`)
belisarius impact 'rust-analyzer cargo … analyze#' . --depth 3
belisarius flow   'rust-analyzer cargo … analyze#' . --depth 3
belisarius symbol 'rust-analyzer cargo … analyze#' .

# Context artifacts (register schemas, runbooks, API specs)
belisarius context list .
belisarius context index .            # ingest into the search index
```

For UI dev with hot reload:

```sh
# terminal 1
belisarius serve --port 7878

# terminal 2
cd web && pnpm run dev    # http://localhost:5173 (proxies /api to :7878)
```

## CLI subcommands

| command                                     | what it does                                                  |
|---------------------------------------------|----------------------------------------------------------------|
| `scan <path> [--graph] [--with-ast]`        | structural JSON: files + imports, optionally full AnalysisReport |
| `index <path>`                              | run per-language SCIP indexers, write `merged.scip`           |
| `symbols inspect \| search \| refs \| callers \| file` | query a SCIP index                                  |
| `funcs <path> [--min-cc N] [--limit N]`     | rank functions by complexity                                  |
| `quality <path> [--json]`                   | composite 0–100 score with axis breakdown + top issues        |
| `mcp`                                       | speak MCP over stdio                                          |
| `serve [--port N]`                          | start the HTTP API + serve `web/dist`                         |

## Quality metric

The composite score is the weighted geometric mean of four axes (0–100 each):

| axis        | weight | what it measures                                              |
|-------------|--------|---------------------------------------------------------------|
| complexity  | 0.35   | % of functions with cyclomatic ≤ 10                           |
| acyclicity  | 0.25   | `100 − 20 × cycle_count`, floored at 0                        |
| dead_code   | 0.20   | `100 − 100 × min(1, 10·dead/total)` over resolvable files     |
| fan_balance | 0.20   | `100 × (1 − Gini(out_degree))` — even out-degree distribution |

Cyclomatic counts each branch/arm; cognitive follows SonarSource 2016 — a
match/switch contributes once even with many arms, but nesting increments
inner branch costs.

## MCP — use Belisarius as an agent-native tool

`belisarius mcp` speaks the Model Context Protocol over stdio so agents
(Claude Code, Cursor, Claude Desktop, etc.) can call every capability
natively — no copy-paste from `curl`. This is the primary interface.

Add this to your client's MCP server config:

```jsonc
{
  "mcpServers": {
    "belisarius": {
      "command": "/absolute/path/to/target/release/belisarius",
      "args": ["mcp"]
    }
  }
}
```

The tools exposed today: `belisarius_scan`, `belisarius_quality`,
`belisarius_functions`, `belisarius_hotspots`, `belisarius_test_gaps`,
`belisarius_surface`, `belisarius_file_dsm`, `belisarius_snippet`,
`belisarius_markers`, `belisarius_search_symbols`, `belisarius_components`,
`belisarius_commands`, `belisarius_diff`, plus the fleet tools
(`belisarius_fleet_list`, `belisarius_fleet_find`, `belisarius_fleet_hotspots`,
`belisarius_fleet_test_gaps`, `belisarius_fleet_diff`), plus the new
hybrid-search / xref / context tools: `belisarius_search_code`,
`belisarius_index_status`, `belisarius_reindex`, `belisarius_impact`,
`belisarius_flow`, `belisarius_symbol`, `belisarius_context_list`,
`belisarius_context_get`, `belisarius_context_search`. The `initialize`
response carries a `instructions` field embedding the **search-before-reading**
philosophy — agents should prefer `belisarius_search_code` over opening files
speculatively. Tool names and JSON schemas are returned by `tools/list`. Logs
go to stderr; stdout is JSON-RPC only.

To smoke-test from the shell:

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  | belisarius mcp
```

## HTTP API

### Scan + analysis

| method | path                                                         | purpose                                              |
|--------|--------------------------------------------------------------|------------------------------------------------------|
| GET    | `/api/health`                                                | liveness                                             |
| POST   | `/api/scan?path=...`                                         | thin scan JSON                                       |
| POST   | `/api/graph?path=...`                                        | resolved file graph                                  |
| POST   | `/api/analyze?path=...`                                      | full `AnalysisReport` (scan + AST + cycles + quality) |
| GET    | `/api/quality?path=...`                                      | composite score + axes + top issues (mtime-cached)   |
| GET    | `/api/functions?path=...&min_cc=&limit=&sort_by=&file=`      | ranked function list                                 |

### Symbols (SCIP)

| method | path                                                         | purpose                                              |
|--------|--------------------------------------------------------------|------------------------------------------------------|
| GET    | `/api/symbols/status?path=...`                               | does an index exist? counts                          |
| GET    | `/api/symbols/search?path=...&q=&limit=`                     | substring symbol search                              |
| GET    | `/api/symbols/refs?path=...&sym=`                            | references grouped by file                           |
| GET    | `/api/symbols/callers?path=...&sym=`                         | enclosing-def callers (needs `enclosing_range`)      |
| GET    | `/api/symbols/file?path=...&file=`                           | per-file defs + ref counts                           |
| GET    | `/api/impact?path=...&sym=&depth=`                           | transitive callers (blast radius), capped at 200 nodes |
| GET    | `/api/flow?path=...&sym=&depth=`                             | transitive callees (forward flow), same caps         |
| GET    | `/api/symbol?path=...&sym=`                                  | 360°: def + direct callers + direct callees + counts |

### Hybrid code search

| method | path                                                         | purpose                                              |
|--------|--------------------------------------------------------------|------------------------------------------------------|
| GET    | `/api/search?path=...&q=&limit=&lang=&kind=`                 | RRF-fused semantic + BM25 results                    |
| GET    | `/api/search/status?path=...`                                | indexer state, chunk count, embedding model          |
| POST   | `/api/search/reindex?path=...&full=&bm25_only=`              | (re)index — runs in a background task                |

### Context artifacts

| method | path                                                         | purpose                                              |
|--------|--------------------------------------------------------------|------------------------------------------------------|
| GET    | `/api/context?path=...`                                      | list registered artifacts                            |
| GET    | `/api/context/get?path=...&name=`                            | resolve an artifact's globs and read its files       |
| GET    | `/api/context/search?path=...&q=&limit=`                     | semantic search over artifact chunks                 |
| POST   | `/api/context/index?path=...`                                | ingest artifacts into the search index               |

The server caches both the SCIP store and the AST analysis by file mtime, so
repeat calls are <10 ms.

## Roadmap

Done:
- ✅ tree-sitter AST extraction (Rust, TS, TSX, JS, Python, Go)
- ✅ cyclomatic + cognitive complexity per function
- ✅ Tarjan SCC cycles
- ✅ Lakos-style depth from entry points
- ✅ 4-axis composite quality score
- ✅ SCIP-backed symbol search / refs / callers
- ✅ MCP server wrapping the same HTTP routes
- ✅ Git-history hotspots (churn × complexity)
- ✅ Per-function test-gap ranking
- ✅ DSM (dependency structure matrix) view
- ✅ Hybrid semantic + BM25 code search (fastembed bge-small + tantivy + RRF)
- ✅ Transitive call traversal: impact / flow / symbol 360°
- ✅ Context artifacts registry (non-code knowledge)
- ✅ Cytoscape.js + dagre graph explorer with right-click blast-radius

Open:
- `.belisarius/rules.toml` architectural constraint engine
- More tree-sitter languages (Java, Ruby, C#, …)
- HNSW vector index for projects > 30k chunks (currently brute-force cosine)
- Live file watching with incremental re-embedding
