---
description: Show a token-budgeted brief of the current project — language mix, quality score, hotspots, test gaps, entry points, hot functions.
---

Call the `mcp__belisarius__belisarius_brief` tool against the current project root. This is the right first move before drilling into specific files: it returns the ~1-2 KB summary the project's maintainer would want a new contributor to read first.

Project root: $ARGUMENTS (default to the current working directory).

Relay the brief verbatim — don't paraphrase. If the user asks a follow-up, use the more specific Belisarius tools (`belisarius_search_code`, `belisarius_symbol`, `belisarius_hotspots`, etc.) rather than re-running the brief.
