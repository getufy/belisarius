//! Compose a 0–100 quality score from per-function and graph signals.
//!
//! Four axes: complexity, acyclicity, dead_code, coupling. Each axis returns
//! `None` when there isn't enough signal to score it (e.g. no functions for
//! complexity, no resolved files for the graph axes). The composite is a
//! weighted geometric mean over the scored axes only — so we can't hide a bad
//! axis behind three good ones, and "no data" doesn't masquerade as 100.

use belisarius_core::{
    FileMetrics, FunctionInfo, Graph, GraphCycle, Quality, QualityAxes, QualityIssue,
};

/// Eligible languages for the graph axes — must match the import-extractor
/// coverage in `graph.rs` so we don't penalize files we never tried to resolve.
const RESOLVED_LANGS: &[&str] = &["rust", "typescript", "javascript", "python", "go"];

// Complexity ramp: severity = 1.0 at or below OK, 0.0 at or above BAD, linear between.
const CC_OK: f32 = 10.0;
const CC_BAD: f32 = 30.0;
const COG_OK: f32 = 15.0;
const COG_BAD: f32 = 50.0;
const CC_THRESHOLD_FOR_ISSUES: u32 = 10;

// Coupling ramp: out-degree per file.
const COUPLING_OK: f32 = 15.0;
const COUPLING_BAD: f32 = 40.0;

// Weights for the geometric mean. Justified by what predicts maintenance cost:
//   complexity (0.40) — strongest empirical correlate of defects.
//   acyclicity (0.30) — cycles block incremental builds, testing, reasoning.
//   dead_code  (0.15) — hygienic; tolerable during evolution.
//   coupling   (0.15) — god-file risk; degrades gracefully.
const W_COMPLEXITY: f64 = 0.40;
const W_ACYCLICITY: f64 = 0.30;
const W_DEAD_CODE: f64 = 0.15;
const W_COUPLING: f64 = 0.15;

pub fn compose(functions: &[FunctionInfo], graph: &Graph, cycles: &[GraphCycle]) -> Quality {
    let complexity = complexity_axis(functions);
    let acyclicity = acyclicity_axis(cycles, graph);
    let dead_code = dead_code_axis(graph);
    let coupling = coupling_axis(graph);

    let axes = QualityAxes {
        complexity,
        acyclicity,
        dead_code,
        coupling,
    };

    // Weighted geometric mean over scored axes only. Floor each value at 1.0
    // so ln stays defined. If nothing scored, score is None.
    let parts = [
        (complexity, W_COMPLEXITY),
        (acyclicity, W_ACYCLICITY),
        (dead_code, W_DEAD_CODE),
        (coupling, W_COUPLING),
    ];
    let mut log_sum = 0.0_f64;
    let mut weight_sum = 0.0_f64;
    for (value, w) in parts {
        if let Some(v) = value {
            let floored = (v.max(1.0)) as f64;
            log_sum += w * floored.ln();
            weight_sum += w;
        }
    }
    let score = if weight_sum > 0.0 {
        Some((log_sum / weight_sum).exp() as f32)
    } else {
        None
    };

    let top_issues = top_issues(functions, cycles, graph);
    Quality {
        score,
        axes,
        top_issues,
    }
}

/// 1.0 at or below `ok`, 0.0 at or above `bad`, linear in between.
fn ramp(value: f32, ok: f32, bad: f32) -> f32 {
    if value <= ok {
        1.0
    } else if value >= bad {
        0.0
    } else {
        1.0 - (value - ok) / (bad - ok)
    }
}

fn complexity_axis(functions: &[FunctionInfo]) -> Option<f32> {
    if functions.is_empty() {
        return None;
    }
    let mut sum = 0.0_f32;
    for f in functions {
        let cc = ramp(f.cyclomatic as f32, CC_OK, CC_BAD);
        let cog = ramp(f.cognitive as f32, COG_OK, COG_BAD);
        sum += cc.min(cog);
    }
    Some(100.0 * sum / functions.len() as f32)
}

fn acyclicity_axis(cycles: &[GraphCycle], graph: &Graph) -> Option<f32> {
    let total = graph
        .nodes
        .iter()
        .filter(|n| RESOLVED_LANGS.contains(&n.language.as_str()))
        .count() as u32;
    if total == 0 {
        return None;
    }
    // log2(size+1) so a 2-node cycle ≈ 1.58, an 8-node ≈ 3.17, a 50-node ≈ 5.67.
    let weight: f32 = cycles
        .iter()
        .filter(|c| c.size > 1)
        .map(|c| ((c.size + 1) as f32).log2())
        .sum();
    // Normalize by sqrt(N) — 3 small cycles in a 5000-file repo is noise;
    // 3 small cycles in a 10-file repo is most of the project.
    let normalized = weight / ((total as f32).sqrt() + 1.0);
    Some((100.0 * (-normalized / 0.7).exp()).clamp(0.0, 100.0))
}

fn dead_code_axis(graph: &Graph) -> Option<f32> {
    let (mut total, mut dead) = (0u32, 0u32);
    for n in &graph.nodes {
        if !RESOLVED_LANGS.contains(&n.language.as_str()) {
            continue;
        }
        total += 1;
        if n.in_degree == 0 && !n.is_entry_point && !is_dead_code_exempt(&n.id) {
            dead += 1;
        }
    }
    if total == 0 {
        return None;
    }
    // 0% dead → 100, 20% dead → 0 (linear). Old curve saturated at 10%.
    let frac = dead as f32 / total as f32;
    Some((100.0 - 500.0 * frac).clamp(0.0, 100.0))
}

fn coupling_axis(graph: &Graph) -> Option<f32> {
    let outs: Vec<u32> = graph
        .nodes
        .iter()
        .filter(|n| RESOLVED_LANGS.contains(&n.language.as_str()))
        .map(|n| n.out_degree)
        .collect();
    if outs.is_empty() {
        return None;
    }
    let mean: f32 = outs
        .iter()
        .map(|&o| ramp(o as f32, COUPLING_OK, COUPLING_BAD))
        .sum::<f32>()
        / outs.len() as f32;
    Some(100.0 * mean)
}

/// Files with `in_degree == 0` that are legitimately roots, not dead code.
fn is_dead_code_exempt(path: &str) -> bool {
    let p = path.replace('\\', "/");
    p == "lib.rs"
        || p == "main.rs"
        || p.ends_with("/lib.rs")
        || p.ends_with("/main.rs")
        || p.ends_with("/mod.rs")
        || p.ends_with("/__init__.py")
        || p.contains("/src/bin/")
        || p.starts_with("tests/")
        || p.contains("/tests/")
        || p.starts_with("benches/")
        || p.contains("/benches/")
        || p.starts_with("examples/")
        || p.contains("/examples/")
        || p.ends_with(".test.ts")
        || p.ends_with(".test.tsx")
        || p.ends_with(".test.js")
        || p.ends_with(".test.jsx")
        || p.ends_with(".spec.ts")
        || p.ends_with(".spec.tsx")
        || p.ends_with(".spec.js")
        || p.ends_with(".spec.jsx")
        || p.ends_with("_test.go")
}

fn top_issues(
    functions: &[FunctionInfo],
    cycles: &[GraphCycle],
    graph: &Graph,
) -> Vec<QualityIssue> {
    let mut out: Vec<QualityIssue> = Vec::new();

    let mut sorted_fns: Vec<&FunctionInfo> = functions
        .iter()
        .filter(|f| f.cyclomatic > CC_THRESHOLD_FOR_ISSUES)
        .collect();
    sorted_fns.sort_by(|a, b| {
        b.cyclomatic
            .cmp(&a.cyclomatic)
            .then(b.cognitive.cmp(&a.cognitive))
    });
    for f in sorted_fns.into_iter().take(10) {
        out.push(QualityIssue::HotFunction {
            file: f.file.clone(),
            name: f.name.clone(),
            start_line: f.start_line,
            cyclomatic: f.cyclomatic,
            cognitive: f.cognitive,
        });
    }

    for c in cycles.iter().take(5) {
        out.push(QualityIssue::Cycle {
            nodes: c.nodes.clone(),
        });
    }

    let mut dead_count = 0;
    for n in &graph.nodes {
        if !RESOLVED_LANGS.contains(&n.language.as_str()) {
            continue;
        }
        if n.in_degree == 0 && !n.is_entry_point && !is_dead_code_exempt(&n.id) {
            out.push(QualityIssue::DeadFile { path: n.id.clone() });
            dead_count += 1;
            if dead_count >= 5 {
                break;
            }
        }
    }

    out
}

pub fn rollup_file_metrics(functions: &[FunctionInfo]) -> Vec<FileMetrics> {
    let mut by_file: std::collections::BTreeMap<String, Vec<&FunctionInfo>> = Default::default();
    for f in functions {
        by_file.entry(f.file.clone()).or_default().push(f);
    }
    by_file
        .into_iter()
        .map(|(path, fns)| {
            let count = fns.len() as u32;
            let max_cc = fns.iter().map(|f| f.cyclomatic).max().unwrap_or(0);
            let total_cc: u32 = fns.iter().map(|f| f.cyclomatic).sum();
            let max_cog = fns.iter().map(|f| f.cognitive).max().unwrap_or(0);
            let longest = fns.iter().map(|f| f.loc).max().unwrap_or(0);
            let avg = if count > 0 {
                total_cc as f32 / count as f32
            } else {
                0.0
            };
            FileMetrics {
                path,
                function_count: count,
                max_cyclomatic: max_cc,
                total_cyclomatic: total_cc,
                max_cognitive: max_cog,
                longest_function_loc: longest,
                avg_cyclomatic: avg,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{Graph, GraphNode};

    fn fnstub(file: &str, name: &str, cc: u32) -> FunctionInfo {
        FunctionInfo {
            file: file.into(),
            name: name.into(),
            start_line: 1,
            end_line: 10,
            loc: 10,
            params: 0,
            cyclomatic: cc,
            cognitive: cc,
            body_hash: "x".into(),
        }
    }

    fn fnstub_cog(file: &str, name: &str, cc: u32, cog: u32) -> FunctionInfo {
        let mut f = fnstub(file, name, cc);
        f.cognitive = cog;
        f
    }

    fn empty_graph() -> Graph {
        Graph {
            root: ".".into(),
            nodes: vec![],
            edges: vec![],
            unresolved: 0,
        }
    }

    fn node(id: &str, lang: &str, in_deg: u32, out_deg: u32) -> GraphNode {
        GraphNode {
            id: id.into(),
            language: lang.into(),
            loc: 10,
            in_degree: in_deg,
            out_degree: out_deg,
            is_entry_point: false,
            depth_from_entry: 0,
        }
    }

    fn graph_with(nodes: Vec<GraphNode>) -> Graph {
        Graph {
            root: ".".into(),
            nodes,
            edges: vec![],
            unresolved: 0,
        }
    }

    #[test]
    fn complexity_axis_all_simple() {
        let fns = vec![
            fnstub("a.rs", "a", 1),
            fnstub("a.rs", "b", 5),
            fnstub("a.rs", "c", 8),
        ];
        assert_eq!(complexity_axis(&fns), Some(100.0));
    }

    #[test]
    fn complexity_axis_softens_mild_overshoot() {
        // CC=12 sits just above OK=10 — old binary axis treated it as 0, now ~0.9.
        let fns = vec![
            fnstub("a", "a", 5),
            fnstub("a", "b", 12),
            fnstub("a", "c", 3),
        ];
        let v = complexity_axis(&fns).unwrap();
        assert!(v > 90.0 && v < 100.0, "got {v}");
    }

    #[test]
    fn complexity_axis_uses_cognitive() {
        // Low CC but high cognitive complexity should still drag the axis.
        let fns = vec![fnstub_cog("a", "a", 2, 60), fnstub_cog("a", "b", 2, 2)];
        let v = complexity_axis(&fns).unwrap();
        assert!(v < 60.0, "got {v} — high cognitive should drag axis");
    }

    #[test]
    fn complexity_axis_unscored_when_empty() {
        assert_eq!(complexity_axis(&[]), None);
    }

    #[test]
    fn acyclicity_axis_unscored_when_empty_graph() {
        assert_eq!(acyclicity_axis(&[], &empty_graph()), None);
    }

    #[test]
    fn acyclicity_axis_clean_graph_perfect() {
        let g = graph_with(vec![node("a.rs", "rust", 0, 0), node("b.rs", "rust", 0, 0)]);
        assert_eq!(acyclicity_axis(&[], &g), Some(100.0));
    }

    #[test]
    fn acyclicity_size_scales() {
        let cycles = vec![
            GraphCycle {
                nodes: vec!["a".into(), "b".into()],
                size: 2,
            },
            GraphCycle {
                nodes: vec!["c".into(), "d".into()],
                size: 2,
            },
            GraphCycle {
                nodes: vec!["e".into(), "f".into()],
                size: 2,
            },
        ];
        let small: Vec<GraphNode> = (0..10)
            .map(|i| node(&format!("f{i}.rs"), "rust", 1, 1))
            .collect();
        let big: Vec<GraphNode> = (0..5000)
            .map(|i| node(&format!("f{i}.rs"), "rust", 1, 1))
            .collect();
        let s = acyclicity_axis(&cycles, &graph_with(small)).unwrap();
        let b = acyclicity_axis(&cycles, &graph_with(big)).unwrap();
        assert!(
            b > s + 20.0,
            "big repo should score much higher: small={s}, big={b}"
        );
        assert!(
            b > 90.0,
            "3 cycles in 5000 files should be near-perfect, got {b}"
        );
    }

    #[test]
    fn acyclicity_big_cycle_costs_more_per_cycle() {
        let nodes: Vec<GraphNode> = (0..100)
            .map(|i| node(&format!("f{i}.rs"), "rust", 1, 1))
            .collect();
        let g = graph_with(nodes);
        let one_big = vec![GraphCycle {
            nodes: vec!["x".into(); 20],
            size: 20,
        }];
        let one_small = vec![GraphCycle {
            nodes: vec!["a".into(), "b".into()],
            size: 2,
        }];
        let big = acyclicity_axis(&one_big, &g).unwrap();
        let small = acyclicity_axis(&one_small, &g).unwrap();
        assert!(
            big < small,
            "20-node cycle should hurt more than 2-node: big={big}, small={small}"
        );
    }

    #[test]
    fn dead_code_exempts_lib_main_tests() {
        // All "dead" by the in_degree==0 rule, but all exempt.
        let g = graph_with(vec![
            node("crates/foo/src/lib.rs", "rust", 0, 0),
            node("crates/foo/src/main.rs", "rust", 0, 0),
            node("crates/foo/src/bin/tool.rs", "rust", 0, 0),
            node("tests/integration.rs", "rust", 0, 0),
            node("examples/demo.rs", "rust", 0, 0),
            node("crates/foo/src/foo/mod.rs", "rust", 0, 0),
            node("pkg/__init__.py", "python", 0, 0),
            node("src/foo.test.ts", "typescript", 0, 0),
            node("internal/foo_test.go", "go", 0, 0),
            node("crates/foo/src/real.rs", "rust", 1, 0),
        ]);
        assert_eq!(dead_code_axis(&g), Some(100.0));
    }

    #[test]
    fn dead_code_axis_unscored_when_no_resolved() {
        let g = graph_with(vec![node("foo.md", "markdown", 0, 0)]);
        assert_eq!(dead_code_axis(&g), None);
    }

    #[test]
    fn dead_code_axis_gentler_curve() {
        // 10% dead — old curve gave 0, new curve gives 50.
        let mut nodes: Vec<GraphNode> = (0..10)
            .map(|i| node(&format!("live{i}.rs"), "rust", 1, 0))
            .collect();
        nodes.push(node("dead.rs", "rust", 0, 0));
        let g = graph_with(nodes);
        let v = dead_code_axis(&g).unwrap();
        // 1 dead of 11 ≈ 9.09% → 100 - 500*0.0909 ≈ 54.5
        assert!((v - 54.5).abs() < 2.0, "got {v}");
    }

    #[test]
    fn coupling_axis_god_file() {
        let mut nodes: Vec<GraphNode> = (0..10)
            .map(|i| node(&format!("leaf{i}.rs"), "rust", 1, 2))
            .collect();
        nodes.push(node("god.rs", "rust", 1, 50));
        let g = graph_with(nodes);
        let v = coupling_axis(&g).unwrap();
        // 10 leaves at 1.0, 1 god at 0.0 → mean ≈ 10/11 ≈ 90.9.
        assert!(v < 95.0 && v > 85.0, "got {v}");
    }

    #[test]
    fn coupling_axis_unscored_when_no_resolved() {
        let g = graph_with(vec![node("foo.md", "markdown", 0, 0)]);
        assert_eq!(coupling_axis(&g), None);
    }

    #[test]
    fn compose_unscored_when_empty() {
        let q = compose(&[], &empty_graph(), &[]);
        assert!(q.score.is_none(), "score = {:?}", q.score);
        assert!(q.axes.complexity.is_none());
        assert!(q.axes.acyclicity.is_none());
        assert!(q.axes.dead_code.is_none());
        assert!(q.axes.coupling.is_none());
    }

    #[test]
    fn compose_partial_renormalizes() {
        // Functions but no graph: composite is the complexity axis alone.
        let fns = vec![fnstub("a", "a", 1)];
        let q = compose(&fns, &empty_graph(), &[]);
        assert!(q.score.is_some());
        assert!((q.score.unwrap() - 100.0).abs() < 0.5);
    }
}
