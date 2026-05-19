---
description: Top churn × complexity hotspots over the last 90 days of git history. The files most likely to harbor bugs.
---

Call `mcp__belisarius__belisarius_hotspots` for the current project. Show the top 15 files ranked by churn × complexity, plus each file's cyclomatic complexity and number of commits in the window.

If the user wants to drill into a specific hotspot, follow up with `mcp__belisarius__belisarius_file_dsm` (to see what depends on it) or `mcp__belisarius__belisarius_functions` (to see which functions inside it are heaviest).
