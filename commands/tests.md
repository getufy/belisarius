---
description: High-risk source files with no covering test, ranked by complexity. Use to prioritize where new tests pay off most.
---

Call `mcp__belisarius__belisarius_test_gaps` for the current project. Show the top 20 untested functions ranked by cyclomatic complexity, grouped by file.

Note that "untested" is heuristic — the cross-reference is between source functions and any function in a file matching the project's test-naming convention. False positives are possible; surface that caveat once if the user asks why a known-tested function appears.
