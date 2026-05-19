---
description: Evaluate `.belisarius/rules.toml` against the project — architectural layer rules, complexity caps, forbidden dependencies.
---

Run `mcp__belisarius__belisarius_rules_check` against the current project.

If there are no violations, say so in one line. If there are violations:
- group by rule kind (layer_forbid, complexity_cap, etc.)
- show file:line for each
- propose one concrete fix for the first 2-3 violations, but stop short of editing — the user should decide whether each violation is real or whether the rule needs to be relaxed
