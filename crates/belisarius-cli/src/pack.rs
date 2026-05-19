//! `belisarius_pack` — token-budgeted context pack for agents.
//!
//! Greedy assembly: start with the brief (always included, always ranks
//! first), then fill the remaining budget with detail sections according to
//! `focus`. Each section carries its own token-cost label so an agent can
//! reason about what to keep or prune.
//!
//! Token estimate: ~3.5 chars/token for code-heavy sections, 4 chars/token
//! for prose. Both are conservative enough for budgeting without invoking
//! a real tokenizer.

use anyhow::Result;
use belisarius_core::AnalysisReport;
use belisarius_scan::{git_stats, test_map};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Write;
use std::path::PathBuf;

use crate::brief;
use crate::function_detail;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    None,
    Hot,
    Untested,
    RecentChanges,
    Architecture,
}

impl Focus {
    fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("").to_ascii_lowercase().as_str() {
            "hot" => Focus::Hot,
            "untested" => Focus::Untested,
            "recent_changes" | "recent" => Focus::RecentChanges,
            "architecture" | "arch" => Focus::Architecture,
            _ => Focus::None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PackSection {
    pub title: String,
    pub estimated_tokens: usize,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pack {
    pub root: String,
    pub focus: String,
    pub budget_tokens: usize,
    pub estimated_tokens: usize,
    pub markdown: String,
    pub sections: Vec<PackSection>,
    pub truncated: bool,
}

pub fn compose(
    project: &str,
    report: &AnalysisReport,
    budget_tokens: usize,
    focus_str: Option<&str>,
) -> Result<Pack> {
    let focus = Focus::parse(focus_str);
    let budget = budget_tokens.max(500);

    // Section 1: brief — always included.
    let mut sections = Vec::new();
    let mut spent = 0usize;
    let mut truncated = false;

    let project_for_git = PathBuf::from(project);
    let project_for_owners = project_for_git.clone();
    let keep: Vec<String> = report.scan.files.iter().map(|f| f.path.clone()).collect();
    let metrics_clone = report.file_metrics.clone();
    let hotspots = git_stats::collect(&project_for_git, 90, Some(&keep))
        .ok()
        .map(|gs| {
            let mut hs = git_stats::rank_hotspots(&gs, &metrics_clone, 25);
            let co = belisarius_scan::codeowners::CodeownersFile::load(&project_for_owners);
            git_stats::attach_owners(&mut hs, co.as_ref());
            hs
        });

    let inline = test_map::detect_inline_tests(&PathBuf::from(project), &report.scan);
    let test_map_full = test_map::compute(report, &inline);

    let markers = crate::server::scan_markers(project, 200).unwrap_or_default();

    let brief = brief::compose(
        project,
        report,
        hotspots.as_ref(),
        Some(&test_map_full),
        &markers,
    );
    let brief_md = brief.markdown.clone();
    let brief_tokens = estimate_tokens_prose(&brief_md);
    sections.push(PackSection {
        title: "Brief".into(),
        estimated_tokens: brief_tokens,
        markdown: brief_md,
    });
    spent += brief_tokens;

    // Section 2+: focus-driven detail.
    let mut detail_targets: Vec<(String, String)> = Vec::new();
    let gap_files: HashSet<&str> = test_map_full
        .gaps
        .iter()
        .map(|g| g.source.as_str())
        .collect();
    let hot_files: HashSet<String> = hotspots
        .as_ref()
        .map(|h| h.hotspots.iter().map(|x| x.path.clone()).collect())
        .unwrap_or_default();

    match focus {
        Focus::None | Focus::Hot => {
            let mut fns = report.functions.iter().collect::<Vec<_>>();
            fns.sort_by_key(|x| std::cmp::Reverse(x.cyclomatic));
            for f in fns.iter().take(10) {
                detail_targets.push((f.file.clone(), f.name.clone()));
            }
        }
        Focus::Untested => {
            let mut fns: Vec<_> = report
                .functions
                .iter()
                .filter(|f| gap_files.contains(f.file.as_str()))
                .collect();
            fns.sort_by_key(|x| std::cmp::Reverse(x.cyclomatic));
            for f in fns.iter().take(10) {
                detail_targets.push((f.file.clone(), f.name.clone()));
            }
        }
        Focus::RecentChanges => {
            let mut fns: Vec<_> = report
                .functions
                .iter()
                .filter(|f| hot_files.contains(&f.file))
                .collect();
            fns.sort_by_key(|x| std::cmp::Reverse(x.cyclomatic));
            for f in fns.iter().take(10) {
                detail_targets.push((f.file.clone(), f.name.clone()));
            }
        }
        Focus::Architecture => {
            // No function targets — surface + cycles below carry the weight.
        }
    }

    // Greedy fill — try every target in order, skip over ones that don't
    // fit, and stop only when the remaining budget is too small for any
    // realistic section (~200 tokens). Skipping (not stopping) lets us
    // include three small helpers when one large function blew the budget.
    let mut any_skipped = false;
    for (file, name) in detail_targets {
        let remaining = budget.saturating_sub(spent);
        if remaining < 200 {
            any_skipped = true;
            break;
        }
        let detail = match function_detail::compose(project, report, &file, &name) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let md = render_function_detail_md(&detail);
        let tokens = estimate_tokens_code(&md);
        if tokens > remaining {
            any_skipped = true;
            continue;
        }
        sections.push(PackSection {
            title: format!("{} · {}", file, name),
            estimated_tokens: tokens,
            markdown: md,
        });
        spent += tokens;
    }
    if any_skipped {
        truncated = true;
    }

    // Architecture additions: surface counts + cycle list.
    if matches!(focus, Focus::Architecture | Focus::None) && spent < budget {
        let md = render_architecture_md(report);
        let tokens = estimate_tokens_prose(&md);
        if spent + tokens <= budget {
            sections.push(PackSection {
                title: "Architecture".into(),
                estimated_tokens: tokens,
                markdown: md,
            });
            spent += tokens;
        } else {
            truncated = true;
        }
    }

    // Concatenate.
    let mut full = String::with_capacity(spent * 4);
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            full.push_str("\n---\n");
        }
        full.push_str(&s.markdown);
    }

    Ok(Pack {
        root: project.to_string(),
        focus: focus_label(focus).to_string(),
        budget_tokens: budget,
        estimated_tokens: spent,
        markdown: full,
        sections,
        truncated,
    })
}

fn render_function_detail_md(d: &function_detail::FunctionDetail) -> String {
    let mut md = String::with_capacity(512);
    let f = &d.function;
    let _ = writeln!(
        md,
        "## `{}::{}` (cc {} · cog {} · {} LOC)",
        f.file, f.name, f.cyclomatic, f.cognitive, f.loc
    );
    let _ = writeln!(md, "Lines {}-{}", f.start_line, f.end_line);
    if let Some(ch) = &d.churn {
        let author = ch.last_author.as_deref().unwrap_or("?");
        let _ = writeln!(
            md,
            "Churn: {} commits / 90d · last {}",
            ch.commits_in_window, author
        );
    }
    if d.tests.covered {
        let _ = writeln!(md, "Tests: {}", d.tests.tests.join(", "));
    } else {
        let _ = writeln!(md, "Tests: none");
    }
    if d.callers.available && !d.callers.callers.is_empty() {
        let _ = writeln!(md, "Callers: {}", d.callers.callers.len());
    }
    md.push_str("```\n");
    md.push_str(&d.snippet.text);
    if !md.ends_with('\n') {
        md.push('\n');
    }
    md.push_str("```\n");
    md
}

fn render_architecture_md(report: &AnalysisReport) -> String {
    let mut md = String::with_capacity(512);
    let _ = writeln!(md, "## Architecture");
    let _ = writeln!(
        md,
        "{} files · {} resolved edges · max depth {}",
        report.graph.nodes.len(),
        report.graph.edges.len(),
        report.max_depth,
    );
    if !report.cycles.is_empty() {
        let _ = writeln!(md, "\n### Cycles ({})", report.cycles.len());
        for c in report.cycles.iter().take(5) {
            let _ = writeln!(md, "- ({} files) {}", c.size, c.nodes.join(" → "));
        }
        if report.cycles.len() > 5 {
            let _ = writeln!(md, "- … +{} more", report.cycles.len() - 5);
        }
    }
    let entries: Vec<&str> = report
        .graph
        .nodes
        .iter()
        .filter(|n| n.is_entry_point)
        .map(|n| n.id.as_str())
        .collect();
    if !entries.is_empty() {
        let _ = writeln!(md, "\n### Entry points ({})", entries.len());
        for e in entries.iter().take(12) {
            let _ = writeln!(md, "- `{}`", e);
        }
    }
    md
}

fn estimate_tokens_prose(s: &str) -> usize {
    s.len().div_ceil(4)
}

fn estimate_tokens_code(s: &str) -> usize {
    // Code packs denser than prose; assume ~3.5 chars/token.
    (s.len() * 2).div_ceil(7)
}

fn focus_label(f: Focus) -> &'static str {
    match f {
        Focus::None => "none",
        Focus::Hot => "hot",
        Focus::Untested => "untested",
        Focus::RecentChanges => "recent_changes",
        Focus::Architecture => "architecture",
    }
}
