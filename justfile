#!/usr/bin/env just --justfile
#
# Belisarius dev workflow. Install `just` with `brew install just` (macOS) or
# `cargo install just`. The recipes wrap commands that previously needed to
# be memorized from README.md + MIGRATION.md.
#
# Show the menu when invoked with no argument:
default:
    @just --list

# ─── Type bindings ────────────────────────────────────────────────────────

# Regenerate TypeScript bindings from `#[derive(TS)]` annotations across
# every crate that emits them. Output lands in `web/src/types/generated/`.
# Run this after editing any wire-type struct in `belisarius-core`,
# `belisarius-symbols`, or `belisarius-cli`.
types:
    cargo test -p belisarius-core --features ts
    cargo test -p belisarius-symbols --features ts
    cargo test -p belisarius-cli --features ts

# ─── Check / test / build (parity with what CI would run) ─────────────────

# Fast feedback loop: type-check Rust + TypeScript without running tests.
check:
    cargo check --workspace
    cd web && pnpm tsc -b

# Full test suite: Rust unit + integration tests, then TypeScript build,
# then the vitest frontend suite.
test:
    cargo test --workspace
    cd web && pnpm tsc -b
    cd web && pnpm test

# Strict Clippy across the workspace — every warning is an error.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Rust formatting check. Use `cargo fmt --all` to fix in place.
fmt-check:
    cargo fmt --all -- --check

# Security advisory scan against Cargo.lock. Requires `cargo install cargo-audit`.
# Run before each release; CI runs this in a separate job too.
audit:
    cargo audit

# Release build of both the binary and the web bundle.
build:
    cargo build --release
    cd web && pnpm build

# Architectural gate — run Belisarius's own rules check against this repo.
# Self-test: the binary we just built must pass its own `.belisarius/rules.toml`.
# Builds first to make sure we're gating with the freshest binary.
gate: build
    ./target/release/belisarius check .

# Verify the entire pipeline — what CI would run. Use before pushing.
ci:
    just fmt-check
    just clippy
    just test
    just build
    just gate

# ─── Dev loop ─────────────────────────────────────────────────────────────

# Build once in release mode, then serve the API on :7878 with the built
# web assets. The single-binary path — no separate frontend process.
serve:
    cargo build --release
    cd web && pnpm build
    ./target/release/belisarius serve --web-dir web/dist

# Two-terminal dev mode, terminal 1: the Rust API server on :7878 in
# release mode (re-runs needed if you change Rust code).
dev-server:
    cargo build --release
    ./target/release/belisarius serve --port 7878

# Two-terminal dev mode, terminal 2: the Vite dev server with HMR on :5173,
# proxying `/api/*` to :7878.
dev-web:
    cd web && pnpm dev

# ─── First-time setup ─────────────────────────────────────────────────────

# One-shot bootstrap: scan, create `.belisarius/`, fetch the embedding
# model, probe SCIP indexers, print next steps. Idempotent.
init *ARGS:
    cargo run --release -p belisarius-cli -- init {{ARGS}}

# Install web dependencies. Run after fresh checkout or `package.json` change.
install:
    cd web && pnpm install

# Install the `belisarius` binary into `~/.cargo/bin/`. After this you can
# invoke `belisarius` from anywhere — useful for MCP integration with
# Claude Code / Cursor / Claude Desktop. The web UI is baked into the
# binary so `belisarius serve` works standalone (no `--web-dir` needed).
# Re-run after major changes.
install-global:
    cd web && pnpm install && pnpm build
    cargo install --path crates/belisarius-cli --locked --features embed-web

# Print the MCP server snippet a Claude Code / Cursor / Claude Desktop
# config would need. Pipe into `pbcopy` on macOS to copy to clipboard.
mcp-config:
    @printf '"belisarius": {\n  "command": "%s",\n  "args": ["mcp"]\n}\n' "$(which belisarius 2>/dev/null || echo belisarius)"

# ─── Cleanup ──────────────────────────────────────────────────────────────

# Wipe Rust target and web build artifacts. Leaves `web/node_modules` alone.
clean:
    cargo clean
    rm -rf web/dist web/src/types/generated

# Nuclear option — wipes node_modules and `.belisarius/` artifacts too.
clean-all: clean
    rm -rf web/node_modules
    rm -rf .belisarius/scip .belisarius/search .belisarius/diagnostics
