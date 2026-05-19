//! `belisarius_function_detail` — the rich per-function bundle.
//!
//! When an agent asks about one function, this returns everything an agent
//! reasonably needs in a single hop: the source snippet, the file's metrics,
//! the tests that touch the file, the 90-day churn, and (when an index
//! exists) the symbol callers. Each optional piece degrades gracefully when
//! the underlying signal is unavailable (no .git, no SCIP index, etc.).

use anyhow::{Context, Result};
use belisarius_core::{AnalysisReport, FileMetrics, FunctionInfo};
use belisarius_scan::{
    git_stats::{self, GitFileStat},
    test_map::{self},
};
use belisarius_symbols::SymbolStore;
use serde::Serialize;
use std::path::Path;

#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct FunctionDetail {
    pub function: FunctionInfo,
    pub snippet: Snippet,
    pub file_metrics: Option<FileMetrics>,
    pub churn: Option<ChurnFacts>,
    pub tests: TestCoverage,
    pub callers: CallerSummary,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct Snippet {
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct ChurnFacts {
    pub commits_in_window: u32,
    pub total_commits: u32,
    pub last_edited: Option<String>,
    pub last_author: Option<String>,
    /// `git_stats::AuthorCount` lives in `belisarius-scan` and isn't
    /// itself ts-rs-annotated; spelling the shape inline keeps this crate
    /// from leaking a `ts` feature down to scan just for one structural
    /// type. Keep in sync with the Rust shape.
    #[cfg_attr(feature = "ts", ts(type = "Array<{ name: string; commits: number }>"))]
    pub top_authors: Vec<git_stats::AuthorCount>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct TestCoverage {
    pub covered: bool,
    pub tests: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CallerSummary {
    pub available: bool,
    pub reason: Option<String>,
    pub matched_symbol: Option<String>,
    pub callers: Vec<CallerEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CallerEntry {
    pub symbol: String,
    pub display_name: String,
    pub call_sites: Vec<CallSite>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CallSite {
    pub file: String,
    pub start_line: i32,
    pub start_char: i32,
    pub end_line: i32,
}

pub fn compose(
    project: &str,
    report: &AnalysisReport,
    file: &str,
    name: &str,
) -> Result<FunctionDetail> {
    let function = report
        .functions
        .iter()
        .find(|f| f.file == file && f.name == name)
        .cloned()
        .with_context(|| format!("function `{name}` not found in {file}"))?;

    let snippet = read_snippet(project, &function)?;

    let file_metrics = report.file_metrics.iter().find(|m| m.path == file).cloned();

    let churn = collect_churn(project, file).ok().flatten();

    let tests = compute_test_coverage(report, project, file);

    let callers = lookup_callers(project, &function);

    Ok(FunctionDetail {
        function,
        snippet,
        file_metrics,
        churn,
        tests,
        callers,
    })
}

fn read_snippet(project: &str, function: &FunctionInfo) -> Result<Snippet> {
    let project_canon =
        std::fs::canonicalize(project).with_context(|| format!("project path: {project}"))?;
    let full = project_canon.join(&function.file);
    let full_canon =
        std::fs::canonicalize(&full).with_context(|| format!("file: {}", full.display()))?;
    if !full_canon.starts_with(&project_canon) {
        anyhow::bail!("file escapes project root");
    }
    let text = std::fs::read_to_string(&full_canon)?;
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start_idx = function.start_line.saturating_sub(1) as usize;
    let end_idx = (function.end_line as usize).min(total);
    let body = if start_idx < end_idx {
        lines[start_idx..end_idx].join("\n")
    } else {
        String::new()
    };
    Ok(Snippet {
        start_line: function.start_line,
        end_line: function.end_line.min(total as u32),
        text: body,
    })
}

fn collect_churn(project: &str, file: &str) -> Result<Option<ChurnFacts>> {
    let project_path = Path::new(project);
    let stats = git_stats::collect(project_path, 90, Some(&[file.to_string()]))?;
    if !stats.repo_present {
        return Ok(None);
    }
    let stat: &GitFileStat = match stats.files.iter().find(|s| s.path == file) {
        Some(s) => s,
        None => return Ok(None),
    };
    Ok(Some(ChurnFacts {
        commits_in_window: stat.commits_in_window,
        total_commits: stat.total_commits,
        last_edited: stat.last_edited.map(|t| t.to_string()),
        last_author: stat.last_author.clone(),
        top_authors: stat.top_authors.clone(),
    }))
}

fn compute_test_coverage(report: &AnalysisReport, project: &str, file: &str) -> TestCoverage {
    let project_path = Path::new(project);
    let inline = test_map::detect_inline_tests(project_path, &report.scan);
    let map = test_map::compute(report, &inline);
    let mapping = map.mappings.into_iter().find(|m| m.source == file);
    match mapping {
        Some(m) => TestCoverage {
            covered: true,
            tests: m.tests,
        },
        None => TestCoverage {
            covered: false,
            tests: Vec::new(),
        },
    }
}

fn lookup_callers(project: &str, function: &FunctionInfo) -> CallerSummary {
    let scip_path = Path::new(project)
        .join(".belisarius")
        .join("scip")
        .join("merged.scip");
    if !scip_path.exists() {
        return CallerSummary {
            available: false,
            reason: Some(format!(
                "no SCIP index at {} — run `belisarius index {}` first",
                scip_path.display(),
                project
            )),
            matched_symbol: None,
            callers: Vec::new(),
        };
    }
    let store = match SymbolStore::from_path(&scip_path) {
        Ok(s) => s,
        Err(e) => {
            return CallerSummary {
                available: false,
                reason: Some(format!("scip load failed: {e:#}")),
                matched_symbol: None,
                callers: Vec::new(),
            }
        }
    };

    // Symbol search is substring; match against the function name and prefer
    // a definition whose path matches the function's file. This is best-effort:
    // overloads and same-name siblings in other files are filtered by path.
    let hits = store.find_symbols(&function.name, 50);
    let chosen = hits.iter().find(|h| {
        h.occurrences > 0
            && h.info
                .map(|i| i.display_name == function.name)
                .unwrap_or(false)
            && symbol_lives_in_file(&store, h.symbol, &function.file)
    });
    let chosen = match chosen {
        Some(c) => c,
        None => {
            return CallerSummary {
                available: true,
                reason: Some("no symbol matched this function in the index".into()),
                matched_symbol: None,
                callers: Vec::new(),
            }
        }
    };

    let cs = store.callers_of(chosen.symbol);
    let entries: Vec<CallerEntry> = cs
        .iter()
        .map(|c| CallerEntry {
            symbol: c.symbol.clone(),
            display_name: c.info.map(|i| i.display_name.clone()).unwrap_or_default(),
            call_sites: c
                .call_sites
                .iter()
                .map(|o| {
                    let r = o.range();
                    CallSite {
                        file: o.path().to_string(),
                        start_line: r.start_line,
                        start_char: r.start_char,
                        end_line: r.end_line,
                    }
                })
                .collect(),
        })
        .collect();

    CallerSummary {
        available: true,
        reason: None,
        matched_symbol: Some(chosen.symbol.to_string()),
        callers: entries,
    }
}

fn symbol_lives_in_file(store: &SymbolStore, symbol: &str, file: &str) -> bool {
    use belisarius_symbols::SymbolRole;
    store.occurrences_of(symbol).iter().any(|o| {
        let is_def = (o.occurrence.symbol_roles & SymbolRole::Definition as i32) != 0;
        is_def && o.path() == file
    })
}
