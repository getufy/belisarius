---
description: One-shot symbol 360° view — definition site, direct callers, direct callees, occurrence count.
argument-hint: <symbol-id-or-name>
---

Show the symbol view for: $ARGUMENTS

1. Resolve to a SCIP id with `mcp__belisarius__belisarius_search_symbols` if the input isn't already one.
2. Call `mcp__belisarius__belisarius_symbol` with the resolved id.

Format: defsite → direct callers (with call-site counts) → direct callees. Keep it tight; the user is using this to orient before reading code.
