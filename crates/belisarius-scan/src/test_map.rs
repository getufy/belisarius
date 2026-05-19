//! Static test-coverage heuristic — no test runner required.
//!
//! Classifies every file as either "source" or "test" by name + path
//! conventions, then uses the resolved import graph to record which test
//! files reference which source files. The output exposes:
//!
//!   * `mappings` — for each covered source file, the list of tests that
//!     import it.
//!   * `gaps`     — source files with zero covering tests, sorted by
//!     cyclomatic complexity descending so the riskiest untested code
//!     surfaces first.
//!   * `summary`  — overall coverage proportion for quick triage.
//!
//! Import-only coverage misses tests that reach a target via mocks or
//! pure-runtime injection, so the numbers are a lower bound. For the
//! 80%-case (unit tests that `import './foo'`) it's accurate and cheap.

use belisarius_core::{AnalysisReport, Scan};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMapping {
    pub source: String,
    pub tests: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestGap {
    pub source: String,
    pub language: String,
    pub loc: u32,
    pub function_count: u32,
    pub total_cyclomatic: u32,
    pub max_cyclomatic: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMapSummary {
    pub source_files: usize,
    pub test_files: usize,
    pub covered_files: usize,
    pub gap_files: usize,
    pub coverage_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMap {
    pub mappings: Vec<TestMapping>,
    pub gaps: Vec<TestGap>,
    pub summary: TestMapSummary,
}

/// Path-and-name-based test classifier. Catches the conventions used by
/// the languages we already scan: Rust, TS/JS, Python, Go.
pub fn is_test_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let segments: Vec<&str> = lower.split('/').collect();
    let last = segments.last().copied().unwrap_or("");

    // Filename markers.
    if matches!(last, "conftest.py") {
        return true;
    }
    if last.starts_with("test_") && last.ends_with(".py") {
        return true;
    }
    let suffixes = [
        ".test.ts",
        ".test.tsx",
        ".test.js",
        ".test.jsx",
        ".spec.ts",
        ".spec.tsx",
        ".spec.js",
        ".spec.jsx",
        "_test.rs",
        "_test.py",
        "_test.go",
    ];
    if suffixes.iter().any(|s| last.ends_with(s)) {
        return true;
    }

    // Directory markers — any ancestor segment.
    let dirs = &segments[..segments.len().saturating_sub(1)];
    dirs.iter()
        .any(|s| matches!(*s, "tests" | "__tests__" | "spec"))
}

/// Files that test themselves via an inline test block — primarily Rust's
/// `#[cfg(test)] mod tests` convention, where the test code lives inside
/// the same file it covers. Returns the relative paths of such files,
/// matching `Scan.files[*].path`.
pub fn detect_inline_tests(project_root: &Path, scan: &Scan) -> HashSet<String> {
    let mut out = HashSet::new();
    for f in &scan.files {
        if !is_inline_test_language(&f.language) {
            continue;
        }
        let full = project_root.join(&f.path);
        let Ok(src) = std::fs::read_to_string(&full) else {
            continue;
        };
        if file_has_inline_tests(&src, &f.language) {
            out.insert(f.path.clone());
        }
    }
    out
}

fn is_inline_test_language(lang: &str) -> bool {
    matches!(lang, "rust" | "go")
}

fn file_has_inline_tests(src: &str, lang: &str) -> bool {
    match lang {
        "rust" => src.contains("#[cfg(test)]") || src.contains("#[test]"),
        // Go tests are conventionally in `_test.go` files, but a same-file
        // test function pattern (`func TestXxx(t *testing.T)`) can appear too.
        "go" => src.contains("*testing.T)"),
        _ => false,
    }
}

pub fn compute(report: &AnalysisReport, inline_tested: &HashSet<String>) -> TestMap {
    let mut is_test: BTreeMap<&str, bool> = BTreeMap::new();
    for f in &report.scan.files {
        is_test.insert(f.path.as_str(), is_test_file(&f.path));
    }

    // source_file → set of test files that import it (deduped & ordered).
    let mut covers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in &report.graph.edges {
        let from_is_test = is_test.get(e.from.as_str()).copied().unwrap_or(false);
        let to_is_test = is_test.get(e.to.as_str()).copied().unwrap_or(false);
        if from_is_test && !to_is_test {
            covers
                .entry(e.to.as_str())
                .or_default()
                .insert(e.from.as_str());
        }
    }
    // Inline tests cover the file they live in. We record the source itself
    // as the "test" so downstream tools can show that the coverage comes
    // from a self-test (vs. an external test file).
    for p in inline_tested {
        if let Some(node) = report.scan.files.iter().find(|f| &f.path == p) {
            let key = node.path.as_str();
            if !is_test.get(key).copied().unwrap_or(false) {
                covers.entry(key).or_default().insert(key);
            }
        }
    }

    let metrics: BTreeMap<&str, &belisarius_core::FileMetrics> = report
        .file_metrics
        .iter()
        .map(|m| (m.path.as_str(), m))
        .collect();

    let mut mappings: Vec<TestMapping> = covers
        .iter()
        .map(|(src, tests)| TestMapping {
            source: (*src).to_string(),
            tests: tests.iter().map(|t| (*t).to_string()).collect(),
        })
        .collect();
    mappings.sort_by(|a, b| a.source.cmp(&b.source));

    let mut gaps: Vec<TestGap> = report
        .scan
        .files
        .iter()
        .filter(|f| !is_test.get(f.path.as_str()).copied().unwrap_or(false))
        .filter(|f| !covers.contains_key(f.path.as_str()))
        .map(|f| {
            let m = metrics.get(f.path.as_str());
            TestGap {
                source: f.path.clone(),
                language: f.language.clone(),
                loc: f.loc,
                function_count: m.map(|x| x.function_count).unwrap_or(0),
                total_cyclomatic: m.map(|x| x.total_cyclomatic).unwrap_or(0),
                max_cyclomatic: m.map(|x| x.max_cyclomatic).unwrap_or(0),
            }
        })
        .collect();
    // Riskiest-untested first: complexity, then size as a tiebreaker.
    gaps.sort_by(|a, b| {
        b.total_cyclomatic
            .cmp(&a.total_cyclomatic)
            .then(b.loc.cmp(&a.loc))
            .then(a.source.cmp(&b.source))
    });

    let test_files = is_test.values().filter(|v| **v).count();
    let source_files = is_test.len() - test_files;
    let covered_files = covers.len();
    let gap_files = gaps.len();
    let coverage_pct = if source_files == 0 {
        0.0
    } else {
        (covered_files as f32 / source_files as f32) * 100.0
    };

    TestMap {
        mappings,
        gaps,
        summary: TestMapSummary {
            source_files,
            test_files,
            covered_files,
            gap_files,
            coverage_pct,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_catches_common_conventions() {
        assert!(is_test_file("src/foo.test.ts"));
        assert!(is_test_file("src/foo.spec.tsx"));
        assert!(is_test_file("__tests__/foo.ts"));
        assert!(is_test_file("tests/integration.rs"));
        assert!(is_test_file("src/bar/foo_test.rs"));
        assert!(is_test_file("pkg/handler_test.go"));
        assert!(is_test_file("api/test_users.py"));
        assert!(is_test_file("conftest.py"));
        assert!(!is_test_file("src/foo.ts"));
        assert!(!is_test_file("src/contest.py")); // not a test
        assert!(!is_test_file("src/main.rs"));
    }

    #[test]
    fn compute_classifies_gaps_and_coverage() {
        use belisarius_core::{
            AnalysisReport, FileMetrics, FileNode, Graph, GraphEdge, Quality, QualityAxes, Scan,
        };

        let files = vec![
            FileNode {
                path: "src/lib.rs".into(),
                language: "rust".into(),
                loc: 100,
                bytes: 1000,
            },
            FileNode {
                path: "src/util.rs".into(),
                language: "rust".into(),
                loc: 50,
                bytes: 500,
            },
            FileNode {
                path: "tests/lib_test.rs".into(),
                language: "rust".into(),
                loc: 20,
                bytes: 200,
            },
        ];
        let scan = Scan {
            root: ".".into(),
            scanned_at: time::OffsetDateTime::UNIX_EPOCH,
            files,
            edges: vec![],
            language_summary: Default::default(),
        };
        let graph = Graph {
            root: ".".into(),
            nodes: vec![],
            edges: vec![GraphEdge {
                from: "tests/lib_test.rs".into(),
                to: "src/lib.rs".into(),
                line: 1,
            }],
            unresolved: 0,
        };
        let metrics = vec![
            FileMetrics {
                path: "src/lib.rs".into(),
                function_count: 1,
                max_cyclomatic: 3,
                total_cyclomatic: 3,
                max_cognitive: 0,
                longest_function_loc: 10,
                avg_cyclomatic: 3.0,
            },
            FileMetrics {
                path: "src/util.rs".into(),
                function_count: 4,
                max_cyclomatic: 7,
                total_cyclomatic: 18,
                max_cognitive: 0,
                longest_function_loc: 20,
                avg_cyclomatic: 4.5,
            },
        ];
        let report = AnalysisReport {
            scan,
            graph,
            functions: vec![],
            file_metrics: metrics,
            cycles: vec![],
            max_depth: 0,
            quality: Quality {
                score: None,
                axes: QualityAxes {
                    complexity: None,
                    acyclicity: None,
                    dead_code: None,
                    coupling: None,
                },
                top_issues: vec![],
            },
        };

        let tm = compute(&report, &HashSet::new());
        assert_eq!(tm.summary.source_files, 2);
        assert_eq!(tm.summary.test_files, 1);
        assert_eq!(tm.summary.covered_files, 1);
        assert_eq!(tm.summary.gap_files, 1);
        assert!((tm.summary.coverage_pct - 50.0).abs() < 0.1);

        assert_eq!(tm.mappings.len(), 1);
        assert_eq!(tm.mappings[0].source, "src/lib.rs");
        assert_eq!(tm.mappings[0].tests, vec!["tests/lib_test.rs"]);

        assert_eq!(tm.gaps.len(), 1);
        assert_eq!(tm.gaps[0].source, "src/util.rs");
        assert_eq!(tm.gaps[0].total_cyclomatic, 18);
    }
}
