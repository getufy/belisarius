//! `.belisarius/rules.toml` — architectural constraints an agent can verify
//! before recommending a change. Pure read-only: load the TOML, evaluate
//! against an `AnalysisReport` (plus optional `SurfaceReport`), return
//! violations.
//!
//! Rule kinds (v1):
//! - `layers` + `[[layer_forbid]]` — declare module layers (by glob) and
//!   forbid edges between specific pairs. Catches "infra depending on UI".
//! - `[[complexity_cap]]` — fail when functions under a glob exceed
//!   cyclomatic / cognitive thresholds.
//! - `[[surface_forbid]]` — forbid public-API items matching a name pattern
//!   from a glob of files (e.g., `pub internal_*` should not exist).
//! - `[dead_code]` — `max_files` ceiling on count of files with in-degree 0.

use anyhow::{Context, Result};
use belisarius_core::AnalysisReport;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use belisarius_core::SurfaceReport;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RulesConfig {
    #[serde(default)]
    pub layers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub layer_forbid: Vec<LayerForbid>,
    #[serde(default)]
    pub complexity_cap: Vec<ComplexityCap>,
    #[serde(default)]
    pub surface_forbid: Vec<SurfaceForbid>,
    #[serde(default)]
    pub dead_code: Option<DeadCodeCap>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayerForbid {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComplexityCap {
    pub glob: String,
    #[serde(default)]
    pub max_cyclomatic: Option<u32>,
    #[serde(default)]
    pub max_cognitive: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceForbid {
    pub glob: String,
    pub pattern: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeadCodeCap {
    pub max_files: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: &'static str,
    pub summary: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RulesReport {
    pub rules_present: bool,
    pub rules_path: Option<String>,
    pub violations: Vec<Violation>,
    pub counts_by_rule: BTreeMap<String, u32>,
}

pub fn load(project_root: &Path) -> Result<Option<(RulesConfig, std::path::PathBuf)>> {
    let path = project_root.join(".belisarius").join("rules.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: RulesConfig =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some((cfg, path)))
}

pub fn evaluate(
    project_root: &Path,
    report: &AnalysisReport,
    surface: Option<&SurfaceReport>,
) -> Result<RulesReport> {
    let loaded = load(project_root)?;
    let (cfg, path) = match loaded {
        Some(x) => x,
        None => {
            return Ok(RulesReport {
                rules_present: false,
                ..Default::default()
            })
        }
    };

    let mut violations = Vec::new();

    if !cfg.layer_forbid.is_empty() {
        violations.extend(check_layer_forbid(&cfg, report)?);
    }
    if !cfg.complexity_cap.is_empty() {
        violations.extend(check_complexity_cap(&cfg, report)?);
    }
    if !cfg.surface_forbid.is_empty() {
        if let Some(s) = surface {
            violations.extend(check_surface_forbid(&cfg, s)?);
        }
    }
    if let Some(dc) = &cfg.dead_code {
        violations.extend(check_dead_code(dc, report));
    }

    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for v in &violations {
        *counts.entry(v.rule.to_string()).or_default() += 1;
    }

    Ok(RulesReport {
        rules_present: true,
        rules_path: Some(path.display().to_string()),
        violations,
        counts_by_rule: counts,
    })
}

fn check_layer_forbid(cfg: &RulesConfig, report: &AnalysisReport) -> Result<Vec<Violation>> {
    // Build a glob per layer.
    let mut layer_globs: BTreeMap<String, GlobSet> = BTreeMap::new();
    for (name, patterns) in &cfg.layers {
        let mut b = GlobSetBuilder::new();
        for p in patterns {
            b.add(Glob::new(p).with_context(|| format!("bad glob `{p}`"))?);
        }
        layer_globs.insert(name.clone(), b.build()?);
    }
    let layer_of = |file: &str| -> Option<&str> {
        for (name, gs) in &layer_globs {
            if gs.is_match(file) {
                return Some(name.as_str());
            }
        }
        None
    };
    let forbid: std::collections::HashSet<(&str, &str)> = cfg
        .layer_forbid
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    let reason_for = |from: &str, to: &str| -> Option<&str> {
        cfg.layer_forbid
            .iter()
            .find(|r| r.from == from && r.to == to)
            .and_then(|r| r.reason.as_deref())
    };

    let mut out = Vec::new();
    for e in &report.graph.edges {
        let Some(from_layer) = layer_of(&e.from) else {
            continue;
        };
        let Some(to_layer) = layer_of(&e.to) else {
            continue;
        };
        if from_layer == to_layer {
            continue;
        }
        if forbid.contains(&(from_layer, to_layer)) {
            let why = reason_for(from_layer, to_layer)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("layer `{from_layer}` may not depend on `{to_layer}`"));
            out.push(Violation {
                rule: "layer_forbid",
                summary: format!("`{}` → `{}` ({} → {})", e.from, e.to, from_layer, to_layer),
                file: Some(e.from.clone()),
                line: if e.line == 0 { None } else { Some(e.line) },
                detail: Some(why),
            });
        }
    }
    Ok(out)
}

fn check_complexity_cap(cfg: &RulesConfig, report: &AnalysisReport) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for cap in &cfg.complexity_cap {
        let gs = Glob::new(&cap.glob)
            .with_context(|| format!("bad glob `{}`", cap.glob))?
            .compile_matcher();
        for f in &report.functions {
            if !gs.is_match(&f.file) {
                continue;
            }
            let mut breaches = Vec::new();
            if let Some(maxcc) = cap.max_cyclomatic {
                if f.cyclomatic > maxcc {
                    breaches.push(format!("cc {} > {}", f.cyclomatic, maxcc));
                }
            }
            if let Some(maxcog) = cap.max_cognitive {
                if f.cognitive > maxcog {
                    breaches.push(format!("cog {} > {}", f.cognitive, maxcog));
                }
            }
            if !breaches.is_empty() {
                out.push(Violation {
                    rule: "complexity_cap",
                    summary: format!("`{}::{}` — {}", f.file, f.name, breaches.join(", ")),
                    file: Some(f.file.clone()),
                    line: Some(f.start_line),
                    detail: Some(format!("under glob `{}`", cap.glob)),
                });
            }
        }
    }
    Ok(out)
}

fn check_surface_forbid(cfg: &RulesConfig, surface: &SurfaceReport) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for rule in &cfg.surface_forbid {
        let gs = Glob::new(&rule.glob)
            .with_context(|| format!("bad glob `{}`", rule.glob))?
            .compile_matcher();
        let pat = regex::Regex::new(&rule.pattern)
            .with_context(|| format!("bad regex `{}`", rule.pattern))?;
        for item in &surface.items {
            if !gs.is_match(&item.file) {
                continue;
            }
            if let Some(want_kind) = &rule.kind {
                let item_kind = format!("{:?}", item.kind).to_lowercase();
                if &item_kind != want_kind {
                    continue;
                }
            }
            if pat.is_match(&item.name) {
                out.push(Violation {
                    rule: "surface_forbid",
                    summary: format!("`{}::{}` matches forbidden pattern", item.file, item.name),
                    file: Some(item.file.clone()),
                    line: Some(item.line),
                    detail: Some(format!(
                        "pattern `{}` under glob `{}`",
                        rule.pattern, rule.glob
                    )),
                });
            }
        }
    }
    Ok(out)
}

fn check_dead_code(cap: &DeadCodeCap, report: &AnalysisReport) -> Vec<Violation> {
    // Manifests and prose can't import anything — they show as in_degree 0
    // even when they're load-bearing. Restrict the dead-file check to nodes
    // whose language is one of the import-graph-aware languages we resolve.
    let code_langs: std::collections::HashSet<&str> =
        ["rust", "typescript", "javascript", "python", "go"]
            .into_iter()
            .collect();
    let dead: Vec<&str> = report
        .graph
        .nodes
        .iter()
        .filter(|n| n.in_degree == 0 && !n.is_entry_point)
        .filter(|n| code_langs.contains(n.language.as_str()))
        .map(|n| n.id.as_str())
        .collect();
    if (dead.len() as u32) > cap.max_files {
        let sample: Vec<String> = dead.iter().take(8).map(|s| (*s).to_string()).collect();
        vec![Violation {
            rule: "dead_code",
            summary: format!("{} dead files (cap {})", dead.len(), cap.max_files),
            file: None,
            line: None,
            detail: Some(format!("sample: {}", sample.join(", "))),
        }]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{
        AnalysisReport, FileNode, FunctionInfo, Graph, GraphNode, LanguageSummary, Quality,
        QualityAxes, Scan,
    };
    use std::collections::BTreeMap;

    fn empty_scan() -> Scan {
        Scan {
            root: String::new(),
            scanned_at: time::OffsetDateTime::UNIX_EPOCH,
            files: Vec::new(),
            edges: Vec::new(),
            language_summary: BTreeMap::new(),
        }
    }

    fn empty_quality() -> Quality {
        Quality {
            score: None,
            axes: QualityAxes {
                complexity: None,
                acyclicity: None,
                dead_code: None,
                coupling: None,
            },
            top_issues: Vec::new(),
        }
    }

    fn report_with_functions(fns: Vec<FunctionInfo>) -> AnalysisReport {
        AnalysisReport {
            scan: empty_scan(),
            graph: Graph {
                root: String::new(),
                nodes: Vec::new(),
                edges: Vec::new(),
                unresolved: 0,
            },
            functions: fns,
            file_metrics: Vec::new(),
            cycles: Vec::new(),
            max_depth: 0,
            quality: empty_quality(),
        }
    }

    fn fn_info(file: &str, name: &str, cc: u32, cog: u32) -> FunctionInfo {
        FunctionInfo {
            file: file.into(),
            name: name.into(),
            start_line: 1,
            end_line: 10,
            loc: 10,
            params: 0,
            cyclomatic: cc,
            cognitive: cog,
            body_hash: "0".into(),
        }
    }

    fn node(id: &str, language: &str, in_degree: u32, is_entry: bool) -> GraphNode {
        GraphNode {
            id: id.into(),
            language: language.into(),
            loc: 10,
            in_degree,
            out_degree: 0,
            is_entry_point: is_entry,
            depth_from_entry: 0,
        }
    }

    fn report_with_nodes(nodes: Vec<GraphNode>) -> AnalysisReport {
        AnalysisReport {
            scan: empty_scan(),
            graph: Graph {
                root: String::new(),
                nodes,
                edges: Vec::new(),
                unresolved: 0,
            },
            functions: Vec::new(),
            file_metrics: Vec::new(),
            cycles: Vec::new(),
            max_depth: 0,
            quality: empty_quality(),
        }
    }

    // ── load ─────────────────────────────────────────────────────────────

    #[test]
    fn load_returns_none_when_rules_toml_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let out = load(tmp.path()).expect("no rules.toml is not an error");
        assert!(out.is_none());
    }

    #[test]
    fn load_parses_valid_rules_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".belisarius");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("rules.toml"),
            r#"
[[complexity_cap]]
glob = "**/*.rs"
max_cyclomatic = 30

[dead_code]
max_files = 5
"#,
        )
        .unwrap();
        let (cfg, path) = load(tmp.path()).unwrap().expect("must parse");
        assert_eq!(cfg.complexity_cap.len(), 1);
        assert_eq!(cfg.complexity_cap[0].max_cyclomatic, Some(30));
        assert_eq!(cfg.dead_code.as_ref().unwrap().max_files, 5);
        assert!(path.ends_with("rules.toml"));
    }

    #[test]
    fn load_errors_on_malformed_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".belisarius");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rules.toml"), "this is = not [valid] toml").unwrap();
        let err = load(tmp.path()).expect_err("malformed toml must fail");
        assert!(format!("{err:#}").contains("rules.toml"));
    }

    // ── evaluate envelope ────────────────────────────────────────────────

    #[test]
    fn evaluate_reports_rules_absent_when_no_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let report = report_with_functions(Vec::new());
        let out = evaluate(tmp.path(), &report, None).unwrap();
        assert!(!out.rules_present);
        assert!(out.violations.is_empty());
        assert!(out.rules_path.is_none());
    }

    // ── complexity_cap ───────────────────────────────────────────────────

    #[test]
    fn complexity_cap_flags_only_breaches() {
        let cfg = RulesConfig {
            layers: BTreeMap::new(),
            layer_forbid: vec![],
            complexity_cap: vec![ComplexityCap {
                glob: "src/**/*.rs".into(),
                max_cyclomatic: Some(10),
                max_cognitive: None,
            }],
            surface_forbid: vec![],
            dead_code: None,
        };
        let report = report_with_functions(vec![
            fn_info("src/a.rs", "ok", 5, 0),
            fn_info("src/b.rs", "hot", 20, 0),
            fn_info("other/c.rs", "ignored_by_glob", 50, 0),
        ]);
        let v = check_complexity_cap(&cfg, &report).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].summary.contains("hot"));
        assert!(v[0].summary.contains("cc 20"));
    }

    #[test]
    fn complexity_cap_handles_cognitive_independently() {
        let cfg = RulesConfig {
            layers: BTreeMap::new(),
            layer_forbid: vec![],
            complexity_cap: vec![ComplexityCap {
                glob: "**/*.rs".into(),
                max_cyclomatic: Some(100),
                max_cognitive: Some(15),
            }],
            surface_forbid: vec![],
            dead_code: None,
        };
        // cc fits, cognitive breaches.
        let report = report_with_functions(vec![fn_info("x.rs", "fn1", 5, 30)]);
        let v = check_complexity_cap(&cfg, &report).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].summary.contains("cog 30"));
    }

    #[test]
    fn complexity_cap_rejects_bad_glob() {
        let cfg = RulesConfig {
            layers: BTreeMap::new(),
            layer_forbid: vec![],
            complexity_cap: vec![ComplexityCap {
                glob: "[[[".into(),
                max_cyclomatic: Some(10),
                max_cognitive: None,
            }],
            surface_forbid: vec![],
            dead_code: None,
        };
        let report = report_with_functions(vec![]);
        let err = check_complexity_cap(&cfg, &report).expect_err("bad glob must fail");
        assert!(format!("{err:#}").contains("bad glob"));
    }

    // ── dead_code ───────────────────────────────────────────────────────

    #[test]
    fn dead_code_flags_orphan_source_when_over_cap() {
        let cap = DeadCodeCap { max_files: 1 };
        let report = report_with_nodes(vec![
            node("a.rs", "rust", 0, false),
            node("b.rs", "rust", 0, false),
            // Entry point — must NOT count even though in_degree=0.
            node("main.rs", "rust", 0, true),
            // Non-code language (toml manifest) — must NOT count.
            node("Cargo.toml", "toml", 0, false),
        ]);
        let v = check_dead_code(&cap, &report);
        assert_eq!(v.len(), 1);
        // Both a.rs and b.rs are dead; cap=1; 2 > 1 so we get the violation.
        assert!(v[0].summary.contains("2 dead files"));
        let detail = v[0].detail.as_ref().unwrap();
        assert!(detail.contains("a.rs"));
        assert!(detail.contains("b.rs"));
        assert!(!detail.contains("main.rs"));
        assert!(!detail.contains("Cargo.toml"));
    }

    #[test]
    fn dead_code_under_cap_yields_no_violation() {
        let cap = DeadCodeCap { max_files: 5 };
        let report = report_with_nodes(vec![
            node("a.rs", "rust", 0, false),
            node("b.rs", "rust", 0, false),
        ]);
        assert!(check_dead_code(&cap, &report).is_empty());
    }

    #[test]
    fn dead_code_ignores_nodes_with_incoming_edges() {
        let cap = DeadCodeCap { max_files: 0 };
        let report = report_with_nodes(vec![
            node("imported.rs", "rust", 3, false),
            node("also_used.rs", "rust", 1, false),
        ]);
        assert!(check_dead_code(&cap, &report).is_empty());
    }

    fn _ignore_unused_helpers() {
        // Suppress "unused" warnings when conditional compilation paths
        // don't reference these helpers.
        let _ = node("", "", 0, false);
        let _ = fn_info("", "", 0, 0);
        let _ = FileNode {
            path: String::new(),
            language: String::new(),
            loc: 0,
            bytes: 0,
        };
        let _ = LanguageSummary::default();
    }
}
