# Security Policy

## Reporting a vulnerability

Please report security vulnerabilities **privately**, not via public GitHub issues.

Preferred channel: GitHub Security Advisories on this repository
(<https://github.com/getufy/belisarius/security/advisories/new>). The maintainers
will respond within 7 days.

When reporting, please include:

- A description of the issue and its impact
- Steps to reproduce, ideally a minimal proof-of-concept
- Affected version(s) / commit hash
- Any suggested mitigation

## Scope

In scope:

- The Belisarius CLI binary
- The MCP server (`belisarius mcp`) and its tool handlers
- The local HTTP server (`belisarius serve`) and the bundled web UI
- The on-disk state under `.belisarius/`
- Path traversal, command injection, or arbitrary-file-read surfaces in indexing

Out of scope:

- Issues that require physical access to the user's machine
- Vulnerabilities in third-party dependencies (please report those upstream;
  we will still bump versions promptly when notified)
- Self-XSS that requires the user to paste content into the dev console

## Hardening notes (current posture)

- The MCP server communicates over **stdio only**; it does not open a network socket.
- The HTTP server binds to `127.0.0.1` only and is intended for local UI use.
- Belisarius reads source code via tree-sitter; it never `eval`s or executes
  indexed code.
- Subprocess invocations use argv arrays (no `shell=true` / shell interpolation).

## Disclosure

We follow coordinated disclosure. Once a fix is ready, we will publish a security
advisory describing the issue, affected versions, and the remediated version.
