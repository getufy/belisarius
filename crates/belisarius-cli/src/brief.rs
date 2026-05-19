//! `belisarius_brief` — the canonical first-look digest for a project.
//!
//! Pure assembler: takes references to data the rest of the engine already
//! produces (AnalysisReport, hotspots, test gaps, markers) and emits a
//! token-lean markdown summary. The same payload backs the MCP tool and the
//! Home dashboard tile in the web UI.

use belisarius_core::{AnalysisReport, QualityIssue};
use belisarius_scan::{git_stats::HotspotsReport, test_map::TestMap};
use serde::Serialize;
use std::fmt::Write;

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::server::MarkerHit;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct Brief {
    pub root: String,
    pub markdown: String,
    pub bytes: u64,
}

pub fn compose(
    project: &str,
    report: &AnalysisReport,
    hotspots: Option<&HotspotsReport>,
    test_map: Option<&TestMap>,
    markers: &[MarkerHit],
) -> Brief {
    let mut md = String::with_capacity(2048);

    let primary_lang = report
        .scan
        .language_summary
        .iter()
        .max_by_key(|(_, s)| s.loc)
        .map(|(name, _)| name.as_str())
        .unwrap_or("unknown");
    let total_loc: u64 = report
        .scan
        .language_summary
        .values()
        .map(|s| s.loc as u64)
        .sum();

    let _ = writeln!(md, "# {}", project_name(project));
    let _ = writeln!(
        md,
        "{} files · {} LOC · primary: {}",
        report.scan.files.len(),
        total_loc,
        primary_lang
    );

    // Language mix — top 4.
    let mut langs: Vec<_> = report.scan.language_summary.iter().collect();
    langs.sort_by(|a, b| b.1.loc.cmp(&a.1.loc));
    if langs.len() > 1 {
        let parts: Vec<String> = langs
            .iter()
            .take(4)
            .map(|(name, s)| format!("{} {}", name, s.loc))
            .collect();
        let _ = writeln!(md, "Languages: {}", parts.join(" · "));
    }
    md.push('\n');

    // Quality block.
    let _ = writeln!(md, "## Quality");
    if let Some(score) = report.quality.score {
        let _ = writeln!(md, "Score: **{:.0} / 100 ({})**", score, grade(score));
    } else {
        let _ = writeln!(
            md,
            "Score: n/a (not enough functions for a meaningful axis)"
        );
    }
    let axes = &report.quality.axes;
    let _ = writeln!(
        md,
        "- complexity: {} · acyclicity: {} · dead_code: {} · coupling: {}",
        fmt_axis(axes.complexity),
        fmt_axis(axes.acyclicity),
        fmt_axis(axes.dead_code),
        fmt_axis(axes.coupling),
    );
    let _ = writeln!(
        md,
        "- cycles: {} · max depth: {} · functions: {}",
        report.cycles.len(),
        report.max_depth,
        report.functions.len(),
    );
    md.push('\n');

    // Hotspots — top 3.
    if let Some(h) = hotspots {
        if h.repo_present && !h.hotspots.is_empty() {
            let _ = writeln!(md, "## Hotspots (top 3, {}-day window)", h.days_window);
            for sp in h.hotspots.iter().take(3) {
                let author = sp
                    .top_author
                    .as_deref()
                    .or(sp.last_author.as_deref())
                    .unwrap_or("?");
                let _ = writeln!(
                    md,
                    "- `{}` — churn {} × cc {} (score {:.1}, {})",
                    sp.path, sp.churn, sp.complexity, sp.score, author,
                );
            }
            md.push('\n');
        }
    }

    // Test gaps — top 3 untested files.
    if let Some(tm) = test_map {
        if !tm.gaps.is_empty() {
            let _ = writeln!(
                md,
                "## Test gaps ({:.0}% covered, {} gap files)",
                tm.summary.coverage_pct, tm.summary.gap_files,
            );
            for g in tm.gaps.iter().take(3) {
                let _ = writeln!(
                    md,
                    "- `{}` — cc {} · {} fns · {} LOC",
                    g.source, g.total_cyclomatic, g.function_count, g.loc,
                );
            }
            md.push('\n');
        }
    }

    // Markers grouped.
    if !markers.is_empty() {
        use std::collections::BTreeMap;
        let mut by_kind: BTreeMap<&str, u32> = BTreeMap::new();
        for m in markers {
            *by_kind.entry(m.kind.as_str()).or_default() += 1;
        }
        let parts: Vec<String> = by_kind.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let _ = writeln!(md, "## Markers ({})", markers.len());
        let _ = writeln!(md, "{}", parts.join(" · "));
        for m in markers.iter().take(3) {
            let text = truncate(&m.text, 70);
            let _ = writeln!(md, "- `{}:{}` {} — {}", m.file, m.line, m.kind, text);
        }
        md.push('\n');
    }

    // Entry points (Lakos depth 0).
    let entries: Vec<&str> = report
        .graph
        .nodes
        .iter()
        .filter(|n| n.is_entry_point)
        .map(|n| n.id.as_str())
        .take(8)
        .collect();
    if !entries.is_empty() {
        let _ = writeln!(md, "## Entry points");
        for e in &entries {
            let _ = writeln!(md, "- `{}`", e);
        }
        md.push('\n');
    }

    // Smallest cycle, if any.
    if let Some(smallest) = report.cycles.iter().min_by_key(|c| c.size) {
        let _ = writeln!(md, "## Smallest cycle ({} files)", smallest.size);
        for n in smallest.nodes.iter().take(6) {
            let _ = writeln!(md, "- `{}`", n);
        }
        if smallest.nodes.len() > 6 {
            let _ = writeln!(md, "- … +{} more", smallest.nodes.len() - 6);
        }
        md.push('\n');
    }

    // Top quality issues — surface anything that didn't already get its own section.
    let extra_issues: Vec<&QualityIssue> = report
        .quality
        .top_issues
        .iter()
        .filter(|i| matches!(i, QualityIssue::HotFunction { .. }))
        .take(3)
        .collect();
    if !extra_issues.is_empty() {
        let _ = writeln!(md, "## Hot functions");
        for i in extra_issues {
            if let QualityIssue::HotFunction {
                file,
                name,
                start_line,
                cyclomatic,
                cognitive,
            } = i
            {
                let _ = writeln!(
                    md,
                    "- `{}:{}` `{}` — cc {} · cog {}",
                    file, start_line, name, cyclomatic, cognitive,
                );
            }
        }
    }

    let bytes = md.len() as u64;
    Brief {
        root: project.to_string(),
        markdown: md,
        bytes,
    }
}

fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| path.to_string())
}

fn fmt_axis(v: Option<f32>) -> String {
    match v {
        Some(x) => format!("{:.0}", x),
        None => "n/a".into(),
    }
}

fn grade(score: f32) -> &'static str {
    match score {
        s if s >= 90.0 => "A",
        s if s >= 80.0 => "B",
        s if s >= 70.0 => "C",
        s if s >= 60.0 => "D",
        _ => "F",
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}
