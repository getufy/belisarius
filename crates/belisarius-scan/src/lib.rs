//! Thin scan stub: gitignore-aware file walk + LOC counts + import edge extraction.
//!
//! v1 uses regex-based import detection for fast bootstrap. Swapping to tree-sitter
//! later only requires changes inside `graph.rs`; the `Scan` schema stays stable.

pub mod architecture;
pub mod ast;
pub mod codeowners;
pub mod commands;
pub mod complexity;
pub mod cycles;
pub mod depth;
pub mod diagnostics;
pub mod diff;
pub mod git_stats;
pub mod graph;
pub mod languages;
pub mod quality;
pub mod resolve;
pub mod rules;
pub mod surface;
pub mod test_map;
pub mod walker;

pub use resolve::build_graph;

use anyhow::Result;
use belisarius_core::{LanguageSummary, Scan};
use std::collections::BTreeMap;
use std::path::Path;
use time::OffsetDateTime;

pub fn scan(root: impl AsRef<Path>) -> Result<Scan> {
    let root = root.as_ref();
    let files = walker::walk(root)?;
    let edges = graph::edges_for(root, &files)?;
    let mut language_summary: BTreeMap<String, LanguageSummary> = BTreeMap::new();
    for f in &files {
        let entry = language_summary.entry(f.language.clone()).or_default();
        entry.files += 1;
        entry.loc += f.loc;
        entry.bytes += f.bytes;
    }
    Ok(Scan {
        root: root.display().to_string(),
        scanned_at: OffsetDateTime::now_utc(),
        files,
        edges,
        language_summary,
    })
}

/// Full pipeline: scan + graph + per-function AST + cycles + depth + composite
/// quality. The heavyweight call; `scan()` and `build_graph()` are still
/// available for cheap paths.
pub fn analyze(root: impl AsRef<Path>) -> Result<belisarius_core::AnalysisReport> {
    let root = root.as_ref();
    let scan = scan(root)?;
    let mut graph = build_graph(&scan);

    let mut functions: Vec<belisarius_core::FunctionInfo> = Vec::new();
    for f in &scan.files {
        let fns = ast::extract_functions_from_path(&f.language, root, &f.path)?;
        functions.extend(fns);
    }

    let file_metrics = quality::rollup_file_metrics(&functions);
    let cycles = cycles::find_cycles(&graph);
    let max_depth = depth::annotate(&mut graph);
    let q = quality::compose(&functions, &graph, &cycles);

    Ok(belisarius_core::AnalysisReport {
        scan,
        graph,
        functions,
        file_metrics,
        cycles,
        max_depth,
        quality: q,
    })
}
