---
description: Hybrid semantic + BM25 search over indexed code chunks. Returns ranked spans with file:line ranges.
argument-hint: <natural-language or keyword query>
---

Search the project's code for: $ARGUMENTS

Use `mcp__belisarius__belisarius_search_code` with the query as written. If results look weak, try:
- adding the `lang:` filter for the most likely language
- broadening with synonyms
- falling back to `mcp__belisarius__belisarius_search_symbols` for exact symbol-name matches

Show the top 5-10 spans with file:line and a one-line excerpt each. Do not Read the files yet — that's the user's next move once they pick a candidate.
