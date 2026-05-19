# Ship-Faster Initiative — Implementation Log

A 3-workstream refactor that collapsed transport-specific duplication on the
backend, established a typed contract between Rust and TypeScript, and
introduced a real data-fetching layer on the frontend. This document is the
single source of truth for what changed and how to keep the new structure
healthy.

## Why this happened

Before the migration, three forces slowed iteration:

1. **Every capability was implemented twice.** `belisarius-cli/src/server.rs`
   (1,853 LOC) and `belisarius-cli/src/cmd_mcp.rs` (1,621 LOC) each packed
   routing + handlers + per-transport caches. Adding one capability meant
   touching both files and risked silent drift — the MCP analysis cache
   never invalidated by mtime, HTTP's was unbounded, and several MCP tools
   bypassed caching entirely while their HTTP twins used it.
2. **The frontend re-typed the backend by hand.** `web/src/api.ts` carried
   761 LOC of TypeScript interfaces mirroring Rust `#[derive(Serialize)]`
   structs. Drift was caught at runtime, not at compile time.
3. **Every tab repeated the same `useState + useEffect + fetch` boilerplate.**
   Twenty components copy-pasted the pattern; rapid tab switches leaked
   in-flight requests; one 668 KB bundle shipped on every page load.

## Workstream 1 — Service layer + route/tool modules

**Outcome:** every capability is implemented once. HTTP and MCP are thin
transports. Every previously-shared MCP tool is served by a registry; the
legacy `call_tool` match is empty.

### Layout

```
crates/belisarius-cli/src/
├── service/                 single-source-of-truth implementations
│   ├── mod.rs               module roster
│   ├── context.rs           AppContext: caches + fleet-aware path resolution
│   ├── error.rs             ServiceError (thiserror) + edge translations
│   ├── architecture.rs      mermaid / graph / summary / module
│   ├── brief.rs             markdown digest
│   ├── context_artifacts.rs list/get/search/index
│   ├── diagnostics.rs       status / run / list (with on-disk cache)
│   ├── fleet.rs             registry list/find/hotspots/test_gaps/diff
│   ├── function_detail.rs   per-function bundle (metrics + churn + tests + callers)
│   ├── pack.rs              token-budgeted snippet pack
│   ├── project.rs           scan/graph/analyze/functions/snippet/markers/file_dsm
│   │                        + hotspots/test_gaps/diff/commands/surface/components/rules
│   ├── quality.rs           composite quality score
│   ├── search.rs            hybrid semantic + BM25 + reindex
│   ├── state.rs             snapshot/drift + pin/unpin/list_pins (state_db)
│   └── symbols.rs           SCIP: status/search/refs/callers/file/trace/impact/flow/symbol_360
├── routes/                  axum routers per feature (thin HTTP wrappers)
│   └── (one .rs per service module above, where applicable)
├── mcp/
│   ├── mod.rs
│   └── registry.rs          ToolSpec + ToolRegistry + default_registry()
├── server.rs                serve(), AppState, AppError, the .merge() chain
└── cmd_mcp.rs               JSON-RPC over stdio + handle_request() + Server { ctx, registry }
```

| File | Before | After | Δ |
|---|---|---|---|
| `server.rs` | 1,853 | 196 | **−89%** |
| `cmd_mcp.rs` | 1,621 | 208 | **−87%** |

`server.rs` is now `serve()` + the `.merge()` chain + `AppState` + `AppError`
+ `health` + the shared `scan_markers` helper. Every other capability lives
in a feature module under `service/` and `routes/`. `cmd_mcp.rs` is JSON-RPC
framing + `Server { ctx, registry }` + a 4-arm `handle_request` match
(`initialize`, `tools/list`, `tools/call`, `ping`).

### Design positions

- **Free async fns over a god-struct.** `service::quality(ctx, args)` is
  trivially unit-testable; there's no `BelisariusService` trait.
- **One `AppContext`** holds `AnalysisCache` (LRU + mtime-invalidated —
  combines the best of both pre-migration caches), `SearchCache`, and
  `SymbolsCache`. Both transports wrap it in `Arc`. The cache cap defaults
  to 16 and is overridable via `BELISARIUS_CACHE_CAP`.
- **Path resolution is fleet-aware everywhere.** Pre-migration only MCP
  did this; the unified `AppContext::resolve_path` brings fleet names to
  HTTP for free.
- **`ServiceError` (thiserror)** with variants `BadRequest`, `NotFound`,
  `MissingIndex { which, hint }`, `Internal(#[from] anyhow::Error)`.
  Translation lives at the edge: `From<ServiceError> for AppError` (HTTP →
  `400 / 404 / 412 / 500`), and a parallel JSON-RPC mapping for MCP. The
  "run `belisarius index` first" hint lives in `MissingIndex` so both
  surfaces speak the same words.
- **`mcp::registry::ToolSpec`** = `{ name, description, input_schema, handler }`,
  where `handler: fn(Arc<AppContext>, Value) -> BoxFut<Result<Value, ServiceError>>`.
  Each feature module's `tool_specs()` returns a `Vec<ToolSpec>`;
  `default_registry()` stitches them together at startup.

### Behaviour fixes folded into the migration

| Endpoint / tool | Pre-migration | Post-migration |
|---|---|---|
| MCP analysis cache | Never invalidated; entries grew until process restart | LRU bounded; mtime invalidation |
| HTTP search mutex | `std::sync::Mutex` (risked tokio stalls under contention) | `tokio::sync::Mutex` |
| HTTP `quality`, `tool_quality` | One cached, the other cache-less | Both share the cached `load_analysis` path |
| `tool_scan` | Bypassed caching (intentional — scan args vary) | Still cache-less by design; documented |
| `tool_search_symbols` | Omitted `kind` field | Includes `kind` (matches HTTP) |
| `tool_impact / _flow / _symbol` | Reopened SCIP store per call | Reuse cached `SymbolStore` from `AppContext` |
| `tool_fleet_list` | Silently swallowed corrupt `fleet.toml` via `unwrap_or_default()` | Surfaces the parse error like HTTP |
| HTTP `tools/list` JSON shape | Slight ordering differences | Stable alphabetical |

## Workstream 2 — Rust→TS type bridge

**Outcome:** every shared wire type comes from one Rust source. The
TypeScript compiler catches backend drift at build time.

### How it works

`crates/belisarius-core` is the single source of truth for wire types.
With the optional `ts` feature enabled, `ts-rs` emits a `.ts` file per
struct/enum:

```toml
# crates/belisarius-core/Cargo.toml
[dependencies]
ts-rs = { version = "11", optional = true }

[features]
ts = ["dep:ts-rs"]
```

Each wire struct gets:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct Scan { ... }
```

`.cargo/config.toml` sets `TS_RS_EXPORT_DIR = { value = ".", relative = true }`
so paths resolve from the workspace root no matter which crate's tests fire.

### Refresh command

```sh
cargo test -p belisarius-core --features ts
```

22 `#[test]` blocks (one per exported type) write `web/src/types/generated/*.ts`.
The output is checked into git so consumers don't need the `ts` feature to
build the frontend.

### Generated types

**Core wire types (`belisarius-core`)** — `AnalysisReport`, `Diagnostic`,
`DiagnosticsReport`, `EdgeKind`, `FileMetrics`, `FileNode`, `FunctionInfo`,
`Graph`, `GraphCycle`, `GraphEdge`, `GraphNode`, `ImportEdge`,
`LanguageSummary`, `Quality`, `QualityAxes`, `QualityIssue`, `Scan`,
`Severity`, `SurfaceItem`, `SurfaceKind`, `SurfaceReport`, `ToolStatus`.

**Service-layer response shapes (`belisarius-cli`)** — `Brief`,
`QualityResponse` (a.k.a. `QualitySummary`), `FunctionsResponse`,
`SnippetResponse` (a.k.a. `Snippet`), `MarkersResponse`, `MarkerHit`,
`FunctionDetail` and its nested `Snippet`, `ChurnFacts`, `TestCoverage`,
`CallerSummary`, `CallerEntry`, `CallSite`.

**35 generated `.ts` files total** plus a `web/src/types/generated/index.ts`
barrel. Refresh with `cargo test --features ts` (the `belisarius-cli` `ts`
feature transitively enables `belisarius-core/ts`).

`web/src/api.ts` consumes them via:

```ts
import type { AnalysisReport, Scan, ... } from "./types/generated";
export type { AnalysisReport, Scan, ... };
```

The hand-rolled versions are gone; api.ts went from 761 → 728 LOC and now
only carries the request/response shapes that aren't yet in `belisarius-core`
(server composites like `Brief`, `FunctionDetail`, `QualitySummary`, the
SymbolMatch wrappers, etc.).

### Bugs caught the day this landed

The strict, generated types caught three places where the hand-rolled
`Record<string, number>` was lying — backend `BTreeMap<String, u32>`
serializes as `{ [key]?: number }` (values can be absent during iteration):

- `DiagnosticsView.tsx:123` — `reduce((a, b) => a + b, 0)` over possibly-undefined values.
- `DiagnosticsView.tsx:195` / `SurfaceView.tsx:109` — sort comparator on possibly-undefined.
- `ScanView.tsx:485` — `language_summary` entry destructure without nullish check.

## Workstream 3 — Frontend data layer + code splitting

**Outcome:** the initial JS bundle dropped from 680 KB to **145 KB (−79%)**;
heavy tabs (Cytoscape, Treemap, Mermaid, etc.) lazy-load on first visit.
Twenty components stopped reimplementing the same fetch boilerplate; rapid
tab switches no longer leak in-flight requests.

### react-query

One `QueryClient` lives in `App.tsx`:

```ts
new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,          // tab flips inside 30s skip the network
      retry: false,               // Belisarius endpoints either work or hard-fail
      refetchOnWindowFocus: false,
    },
  },
})
```

`web/src/data/queries.ts` (~285 LOC) exposes one `useFoo()` hook per
endpoint — **25 hooks** spanning quality, brief, scan, graph, functions,
hotspots, test_gaps, markers, file_dsm, function_detail, surface, components,
commands, symbols (status/search/refs/callers), architecture
(mermaid/graph/module), impact/flow/symbol_360, diagnostics (status/run),
search (status), and context artifacts (list/get). Two `useMutation` helpers
back imperative actions (`useRunDiagnostics`, `useSearch...` actions in
`SearchView` / `ContextView`).

Components compress from the old pattern:

```ts
// Before — ~8 LOC of boilerplate per tab
const [data, setData] = useState<QualitySummary | null>(null);
const [err, setErr] = useState<string | null>(null);
useEffect(() => {
  setData(null);
  setErr(null);
  api.quality(path).then(setData).catch((e) => setErr(String(e)));
}, [path]);
```

```ts
// After — 1 LOC
const { data, error } = useQuality(path);
```

Free perks: cache hits on tab revisit, request dedup, automatic
cancellation, no stale data flashing into the new tab.

### Components on react-query (15 total)

`QualityView`, `MarkersView`, `HotspotsView`, `TestGapsView`,
`FunctionsView`, `SurfaceView`, `ComponentsView`, `CommandsView`,
`DiagnosticsView`, `ArchitectureView`, `DsmView`, `ImpactView`,
`SymbolsView`, `SearchView`, `ContextView`.

Not migrated (intentionally — already prop-only or imperative-only):
`LocTreemap`, `DeadFiles`, `CytoscapeGraph`.

### Code splitting

`web/src/routes/ScanView.tsx` wraps heavy tab components in `lazy()`:

```ts
const CytoscapeGraph = lazy(() =>
  import('../components/CytoscapeGraph').then((m) => ({ default: m.CytoscapeGraph })),
);
```

Eight tabs are now their own chunks. After `pnpm build`:

| Chunk | Size | Gzip |
|---|---|---|
| `index.js` (initial) | 145 KB | 42 KB |
| `CytoscapeGraph` | 543 KB | 178 KB |
| `ArchitectureView` | 12 KB | 4 KB |
| `DiagnosticsView` / `SymbolsView` | 6 KB each | 2 KB |
| `ImpactView` / `DsmView` / `LocTreemap` / `SearchView` | 4–6 KB | 1.5–2 KB |

The 543 KB Cytoscape chunk loads only when the user opens the Graph tab.

## Verification

- `cargo build --workspace` clean.
- `cargo test --workspace` — **105 tests pass** (original 99 + 6 new
  parity/shape tests covering quality, symbols, search, fleet).
- `pnpm tsc -b` clean under TypeScript strict mode.
- `pnpm build` produces the split bundles.

## Maintenance — when you touch what

| You're doing… | Then… |
|---|---|
| Adding an HTTP route AND an MCP tool for the same capability | Add a `service::<feature>::foo()` fn, a `routes::<feature>` handler, and a `ToolSpec` from the feature module's `tool_specs()`. Register the spec in `mcp::registry::default_registry()`. |
| Adding an HTTP-only route | Same `service` fn, plus the `routes::<feature>` wrapper. Skip the tool spec. |
| Adding an MCP-only tool | Same `service` fn, plus a `ToolSpec`. Skip the route wrapper. |
| Changing a wire type in `belisarius-core` | Run `cargo test -p belisarius-core --features ts` to refresh `web/src/types/generated/`. Check the diff in. `pnpm tsc -b` catches frontend consumers that need updating. |
| Adding a new endpoint to the frontend | Add a `useFoo()` hook in `web/src/data/queries.ts` (use the existing hooks as templates). Components import the hook and forget about loading/error/cancellation state. |
| Adding a new tab | If it's heavy (graph, chart, code editor), wrap it in `lazy()` in `ScanView.tsx`. Otherwise import statically. |

## Deferred (out of scope this initiative)

- ~~**`ts-rs` on service-layer response shapes.**~~ Done — completed in two
  passes. First pass: `QualityResponse`, `FunctionsResponse`, `SnippetResponse`,
  `MarkersResponse`, `Brief`, and the `FunctionDetail` tree. Second pass:
  `SymbolsStatusResponse`, `SymbolMatch`, `SymbolsSearchResponse`,
  `SymbolOccurrence`, `RefsByFile`, `RefsResponse`, `SymbolsCallerEntry`
  (renamed from `CallerEntry` to avoid colliding with the function_detail
  nested type), `CallersResponse`, `SymbolDefinition`, `SymbolFileResponse`
  in `belisarius-cli`, plus `ImpactReport`, `ImpactNode`, `FlowReport`,
  `FlowNode`, `Symbol360`, `DefSite`, `CallerLite`, `CalleeLite`, `Range`
  promoted to ts-annotated types in `belisarius-symbols`. Every wire shape
  now flows through ts-rs — `api.ts` carries zero hand-rolled response
  types.
- ~~**Migrate residual HTTP-only handlers.**~~ Done in the closeout pass.
  `diagnostics_*` and `architecture_*` now live in `service::diagnostics` +
  `service::architecture` with thin route wrappers. `server.rs` is 196 LOC.
- ~~**`AbortSignal` propagation.**~~ Done. `api.ts::call()` accepts a
  trailing `signal?: AbortSignal` and forwards to `fetch`. Every `api.X(...)`
  method takes an optional trailing `signal`. Every hook in
  `web/src/data/queries.ts` destructures `signal` from react-query's
  `QueryFunctionContext` and threads it through, so unmounting a component
  or rapid path changes now abort the in-flight network request — not just
  the framework-level promise. `useDiagnostics` is the one intentional
  exception: a long-running diagnostics run shouldn't be aborted on tab
  flip; the user probably wants the result waiting for them.
- **CI/CD, lint enforcement, pre-commit hooks.** Originally scoped out at
  the planning phase. No automation currently catches regressions before
  merge — but the `justfile` (added at the repo root) now wraps every
  command CI would need to run: `just ci` runs `fmt-check + clippy + test
  + build`. Wiring it into GitHub Actions is a single workflow file away.
- **`CONTRIBUTING.md` / `ARCHITECTURE.md` / ADRs.** This document partly
  covers ARCHITECTURE.md territory; a separate CONTRIBUTING.md and a
  proper ADR series would still be valuable.

## Closeouts (post-initiative)

These weren't in the original W1–W3 plan but landed in the same session as
natural follow-ups.

### Live file watching with incremental re-embedding

`belisarius-search` already had an incremental reindex path — files whose
content hash hadn't changed got skipped inside `IndexHandle::reindex`. What
was missing was the watcher that triggered it. Now:

- `crates/belisarius-search/src/watcher.rs` — `notify-debouncer-mini`-backed
  recursive watcher. 500 ms debounce; coalesced batches.
- Filters out `.belisarius/`, `target/`, `node_modules/`, `dist/`, `.git/`,
  `.venv/`, `__pycache__/`, `.next/`, `.turbo/`, `.cache/` (and several
  other build/vendor dirs) at the path level — and only triggers on
  extensions `belisarius_scan::languages::language_for_ext` recognizes, so
  markdown/toml/json edits all qualify but `.lock` files and dotfiles
  with no `.ext` don't.
- `belisarius search watch [PATH]` — foreground watcher CLI.
- `belisarius serve --watch [--watch-path] [--watch-bm25-only]` — runs the
  watcher in the background of the HTTP server, so the UI stays current
  with whatever the developer is editing.
- The watcher lives in a `WatcherHandle` returned to the caller; dropping
  it cleanly shuts `notify` down and the processor thread exits via channel
  close.

This closes the SocratiCode parity item that mattered most for daily use —
agents asking semantic-search questions now hit an index that's at most
~500 ms stale instead of "as fresh as your last manual `belisarius search
index`."

### Zero-config first run (`belisarius init`)

The bootstrap command new users would otherwise have to assemble from the
README. `belisarius init [PATH]` does five things, idempotently:

1. Scans the project and reports the language mix sorted by LOC.
2. Creates `.belisarius/` plus its expected subdirectories (`scip/`,
   `search/`, `search/bm25/`, `diagnostics/`) so later commands don't
   have to mkdir-or-fail.
3. Pre-fetches the `bge-small-en-v1.5` embedding model (~33 MB) by
   forcing `belisarius_search::embed::default_provider()` to run. Skip
   with `--skip-model` when offline.
4. Probes the four per-language SCIP indexers (`rust-analyzer`,
   `scip-typescript`, `scip-python`, `scip-go`) and prints a status row
   per indexer that combines `is_installed()` + `applies_to(root)` with
   the scan's per-language file count. The combination produces a more
   honest message than either alone — a monorepo with `web/` full of TS
   files but no top-level `tsconfig.json` now reads as "installed — run
   from the subdir containing the project file" rather than the
   misleading "n/a (no matching files)". Missing indexers that the
   project actually needs get an install hint
   (e.g. `npm i -g @sourcegraph/scip-typescript`).
5. Optionally builds the hybrid search index in the same call when
   `--index` is passed. Default off (the model download alone is usually
   enough work on a first run).

Wraps a single recipe in the `justfile`: `just init` (forwards any args).

## Pointers

- First-run bootstrap: `belisarius init` (or `just init`).
- Dev commands: `justfile` at the repo root. `just` (no arg) lists every
  recipe; common ones are `just types`, `just check`, `just test`,
  `just ci`, `just dev-server` + `just dev-web`.
- Original plan: `~/.claude/plans/vivid-whistling-nygaard.md`
- Service-layer entry: `crates/belisarius-cli/src/service/mod.rs`
- MCP registry: `crates/belisarius-cli/src/mcp/registry.rs`
- Frontend data hooks: `web/src/data/queries.ts`
- Generated types: `web/src/types/generated/` (54 files)
