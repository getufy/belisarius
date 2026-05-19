# Contributing to Belisarius

Thank you for considering a contribution. Belisarius is a code intelligence indexer
(hybrid semantic + BM25 search, SCIP-backed call graphs, MCP tooling). Contributions
of all sizes are welcome — bug reports, docs fixes, new language support, performance
patches, or new analysis tools.

## Quick start

```bash
git clone git@github.com:getufy/belisarius.git
cd belisarius
cargo build --workspace
cargo test --workspace
```

Frontend (optional, for the web UI):

```bash
pnpm -C web install
pnpm -C web dev
```

## Project layout

```
crates/
  belisarius-core      shared types and data model
  belisarius-scan      walker, AST, complexity, churn, rules
  belisarius-symbols   SCIP integration, call graphs
  belisarius-search    BM25 + embeddings (tantivy + fastembed)
  belisarius-context   non-code artifacts (schemas, runbooks)
  belisarius-cli       CLI, MCP server, HTTP server, web bundling
web/                   Preact/Vite frontend
```

## Before you open a PR

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. If you touched the frontend: `pnpm -C web test && pnpm -C web build`
5. Keep the PR focused — one logical change per PR is easier to review than a sprawling one.

## Commit style

Conventional-ish commits are appreciated but not enforced. Example:

```
search: fix BM25 scoring when token count exceeds window

Closes #42.
```

## Reporting bugs

Open an issue using the **Bug report** template. Include:
- Belisarius version (`belisarius --version`)
- OS and toolchain (`rustc --version`)
- The smallest repro you can produce
- Expected vs actual behavior

## Reporting security issues

Please **do not** open a public issue. See [SECURITY.md](SECURITY.md) for the
private disclosure process.

## Code of conduct

By participating you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By submitting a contribution, you agree it will be licensed under the
[MIT License](LICENSE).
