//! `belisarius_next_action` — meta tool that scans drift, test gaps, hotspots,
//! and rules-check failures, scores each candidate by urgency × impact, and
//! returns a prioritized punch list with concrete shell commands the agent
//! can run next.
//!
//! Pure aggregation — no new analysis. Reuses `service::project::{hotspots,
//! test_gaps, rules_check}` and `state_db` (pins / drift) as data sources.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;
use crate::service::project::{
    hotspots as project_hotspots, rules_check, test_gaps, HotspotsArgs, LimitArgs, PathArgs,
};

#[derive(Debug, Deserialize)]
pub struct NextActionArgs {
    pub path: String,
    /// Cap on total actions returned. Default 8.
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
struct Action {
    title: String,
    why: String,
    command: String,
    score: f32,
    category: &'static str,
}

pub async fn next_action(ctx: &AppContext, args: NextActionArgs) -> Result<Value, ServiceError> {
    let limit = args.limit.unwrap_or(8);
    let path = args.path.clone();
    let mut candidates: Vec<Action> = Vec::new();

    // ── source 1: rules failures (highest urgency: explicit violation) ──
    if let Ok(report) = rules_check(ctx, PathArgs { path: path.clone() }).await {
        if let Some(arr) = report.get("violations").and_then(|v| v.as_array()) {
            for v in arr.iter().take(5) {
                let rule = v.get("rule").and_then(|x| x.as_str()).unwrap_or("?");
                let summary = v
                    .get("summary")
                    .and_then(|x| x.as_str())
                    .unwrap_or("rule violation");
                let file = v.get("file").and_then(|x| x.as_str()).unwrap_or("");
                candidates.push(Action {
                    title: format!("Fix `{rule}` violation: {summary}"),
                    why: "rules.toml flagged this. Rules failures are the cheapest signal — \
they're usually a clear yes/no fix."
                        .to_string(),
                    command: if file.is_empty() {
                        "belisarius check .".to_string()
                    } else {
                        format!("$EDITOR {file}")
                    },
                    score: 0.95,
                    category: "rules_violation",
                });
            }
        }
    }

    // ── source 2: untested hot files (high impact, low cost) ────────────
    let gaps = test_gaps(
        ctx,
        LimitArgs {
            path: path.clone(),
            limit: Some(5),
        },
    )
    .await;
    if let Ok(report) = gaps {
        if let Some(arr) = report.get("gaps").and_then(|v| v.as_array()) {
            for gap in arr.iter().take(5) {
                let source = gap.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                let cc = gap
                    .get("total_cyclomatic")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let fn_count = gap
                    .get("function_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                // Score scales with complexity — covering a cc=80 file beats cc=5.
                let score = (0.4 + (cc as f32 / 200.0).min(0.5)).min(0.9);
                candidates.push(Action {
                    title: format!(
                        "Add tests for `{source}` (cc={cc}, {fn_count} fns, untested)"
                    ),
                    why: "Top test gap by complexity — agents and humans both move faster \
when this file has a safety net."
                        .into(),
                    command: format!("belisarius mcp # then call belisarius_suggest_tests path={path:?} target={source:?}"),
                    score,
                    category: "test_gap",
                });
            }
        }
    }

    // ── source 3: hotspots (proxies for risk; ranked by churn × cc) ─────
    let hot = project_hotspots(
        ctx,
        HotspotsArgs {
            path: path.clone(),
            days: Some(90),
            limit: Some(5),
        },
    )
    .await;
    if let Ok(report) = hot {
        if let Some(arr) = report.get("hotspots").and_then(|v| v.as_array()) {
            for h in arr.iter().take(3) {
                let p = h.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let churn = h.get("churn").and_then(|v| v.as_u64()).unwrap_or(0);
                let cc = h.get("complexity").and_then(|v| v.as_u64()).unwrap_or(0);
                let score = (0.3 + (churn as f32 / 100.0).min(0.4)).min(0.85);
                candidates.push(Action {
                    title: format!("Review hotspot `{p}` ({churn} commits × cc {cc})"),
                    why: "High churn × complexity — risk concentrates here. Worth a \
`belisarius_describe` pass and possibly a split."
                        .into(),
                    command: format!(
                        "belisarius mcp # then call belisarius_describe path={path:?} target={p:?}"
                    ),
                    score,
                    category: "hotspot",
                });
            }
        }
    }

    // ── source 4: expiring pins (knowledge-layer reminders) ────────────
    let project_path = std::path::Path::new(&path);
    if let Ok(conn) = crate::state_db::open(project_path) {
        if let Ok(notes) = crate::state_db::list_notes(&conn, None, Some("todo"), None, 5) {
            for n in notes {
                candidates.push(Action {
                    title: format!("Resolve pinned todo: {}", truncate(&n.content, 70)),
                    why: "A note left by a previous session — closing it removes a long-\
running cognitive tax."
                        .into(),
                    command: format!(
                        "belisarius mcp # then call belisarius_recall path={path:?} query={:?}",
                        n.content
                            .split_whitespace()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    score: 0.5,
                    category: "pinned_todo",
                });
            }
        }
    }

    // Rank globally, then cap.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = candidates.len();
    candidates.truncate(limit);
    let returned = candidates.len();

    Ok(json!({
        "actions": candidates,
        "total_count": total,
        "returned": returned,
        "truncated": total > returned,
    }))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_next_action",
        description: "Prioritized 'what should I work on?' list. Aggregates rules failures, \
test gaps, hotspots, and pinned todos; scores each candidate; returns a punch list with a \
concrete shell command per action.\n\n\
When to use: starting a fresh session on a project, or at the end of one to set up the next.\n\
When not to use: deep diving on a specific file (use `belisarius_describe`); answering 'what \
did I do last time' (use `belisarius_recall` over notes).",
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "limit": { "type": "integer", "default": 8, "maximum": 30 }
            }
        }),
        handler: handle_next_action as ToolHandler,
    }]
}

fn handle_next_action(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: NextActionArgs = serde_json::from_value(args)?;
        next_action(&ctx, args).await
    })
}
