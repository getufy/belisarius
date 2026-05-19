---
description: Transitive backward call graph — who reaches the given symbol? Blast-radius analysis before refactoring.
argument-hint: <symbol-id-or-name>
---

Compute the impact (inbound transitive callers) of: $ARGUMENTS

1. If the input looks like a SCIP symbol id (contains `/` or `.`), pass it directly to `mcp__belisarius__belisarius_impact`.
2. Otherwise resolve it first with `mcp__belisarius__belisarius_search_symbols` to disambiguate, then call `belisarius_impact` on the chosen symbol id.

Report:
- total caller count + files touched
- the top 10-15 callers grouped by file
- a one-line warning if the result is truncated (caps at 200 nodes)

Use depth=3 by default. Suggest depth=5 if the user wants deeper traversal.
