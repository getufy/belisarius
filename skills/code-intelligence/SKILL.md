---
name: code-intelligence
description: Use Belisarius MCP tools (hybrid search, SCIP call graphs, hotspots, test gaps) for code discovery on the current project. Activate when the user wants to find where code lives, trace a symbol's callers or callees, identify churn × complexity hotspots, locate untested high-risk code, or check architectural rules — before falling back to grep/find/Read.
---

# Belisarius code-intelligence

Belisarius indexes the current project with hybrid semantic + BM25 search and a SCIP call graph. Reach for it instead of grep when the question is about *meaning* or *relationships*, not literal text.

## Discovery order

1. **Project orientation** — start with `mcp__belisarius__belisarius_brief` for a one-shot summary. Don't re-read it unless the user changes projects.
2. **"Where is X?"** — `belisarius_search_code` for intent-based queries; `belisarius_search_symbols` for exact symbol names.
3. **"Who calls X / what does X reach?"** — `belisarius_symbol` (one-hop), `belisarius_impact` (transitive callers), `belisarius_flow` (transitive callees).
4. **"What's risky?"** — `belisarius_hotspots` (churn × complexity), `belisarius_test_gaps` (untested high-complexity code).
5. **"What constraints does the project enforce?"** — `belisarius_context_list` for runbooks and schemas the maintainers registered, `belisarius_rules_check` for the architectural gate.
6. **Read files only after the index has narrowed scope to 1-3 candidates.** Do not Read more than you need; the brief and symbol views are already optimized for context budget.

## What it won't do

- It doesn't reindex automatically on file change. If the user has just edited code, suggest `belisarius_reindex` (or `belisarius reindex .` from the shell) before trusting the next query.
- It can't answer about files that aren't indexed yet — if a search returns nothing surprising, check `belisarius_index_status` to confirm the chunk count is non-zero.
- The SCIP graph is built from the language indexers configured in `.belisarius/scip/`; languages without an indexer (e.g. some scripting langs) will have search hits but no call-graph edges.

## When NOT to use these tools

- Single-file edits the user has already located → just Edit.
- Tasks where the user has given a specific file path → Read it directly.
- Reading the latest git state → use `git log` / `git diff`, not the brief (the brief is a snapshot).
