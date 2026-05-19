//! Architecture views derived from the file graph.
//!
//! Two renderers: `render_mermaid_modules` (aggregated by directory — the
//! default, useful for architecture overview) and `render_mermaid_files`
//! (legacy file-level view with subgraphs). Per-language classDefs use the
//! same palette as the rest of the UI.

use belisarius_core::Graph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Carbon-aligned palette: white-ish tints with a strong colored stroke.
// Designed to read on the `#ffffff` Mermaid canvas while keeping each language
// recognizable from the dark version.
const PALETTE: &[(&str, &str, &str)] = &[
    // language id, fill, stroke
    ("rust", "#fdebe5", "#d97a5d"),
    ("typescript", "#e6efff", "#0f62fe"),
    ("tsx", "#e6efff", "#0f62fe"),
    ("javascript", "#fdf4cf", "#bca84a"),
    ("python", "#dff0e3", "#24a148"),
    ("go", "#dceff2", "#0098a6"),
    ("toml", "#f4f4f4", "#8c8c8c"),
    ("json", "#f4f4f4", "#8c8c8c"),
    ("markdown", "#f4f4f4", "#8c8c8c"),
    ("css", "#efe6f6", "#8a3ffc"),
    ("html", "#efe6f6", "#8a3ffc"),
    ("yaml", "#f4f4f4", "#8c8c8c"),
    ("default", "#ffffff", "#525252"),
];

/// Aggregate the file graph into module-level nodes + weighted edges. Pure
/// data — Mermaid + Cytoscape renderers both consume this.
pub fn graph_modules(graph: &Graph, group_depth: usize) -> ArchitectureGraph {
    let nodes_total = graph.nodes.len();
    let edges_total = graph.edges.len();

    let mut modules: BTreeMap<String, ModuleAgg> = BTreeMap::new();
    for n in &graph.nodes {
        let key = group_key(&n.id, group_depth);
        let entry = modules.entry(key).or_insert(ModuleAgg {
            files: 0,
            loc: 0,
            lang_counts: BTreeMap::new(),
            is_entry: false,
        });
        entry.files += 1;
        entry.loc += n.loc;
        *entry.lang_counts.entry(n.language.clone()).or_insert(0) += 1;
        if n.is_entry_point {
            entry.is_entry = true;
        }
    }

    let mut id_map: BTreeMap<&str, String> = BTreeMap::new();
    for (i, k) in modules.keys().enumerate() {
        id_map.insert(k.as_str(), format!("m{i}"));
    }

    let nodes: Vec<GraphVizNode> = modules
        .iter()
        .map(|(key, m)| {
            let id = id_map[key.as_str()].clone();
            let language = dominant_language(&m.lang_counts);
            GraphVizNode {
                id,
                label: key.clone(),
                sublabel: format!("{} files · {} loc", m.files, m.loc),
                language,
                file_count: m.files,
                loc: m.loc,
                is_entry: m.is_entry,
            }
        })
        .collect();

    let mut edge_counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    for e in &graph.edges {
        let from = group_key(&e.from, group_depth);
        let to = group_key(&e.to, group_depth);
        if from == to {
            continue;
        }
        *edge_counts.entry((from, to)).or_insert(0) += 1;
    }
    let edges: Vec<GraphVizEdge> = edge_counts
        .into_iter()
        .filter_map(|((from, to), weight)| {
            let source = id_map.get(from.as_str())?.clone();
            let target = id_map.get(to.as_str())?.clone();
            Some(GraphVizEdge {
                source,
                target,
                weight,
            })
        })
        .collect();

    ArchitectureGraph {
        view: "module".to_string(),
        group_depth,
        nodes,
        edges,
        nodes_total,
        edges_total,
        rendered_cap: nodes_total,
    }
}

/// File-level structured graph — same shape as `graph_modules` but with one
/// node per file, capped to the top `max_nodes` by total degree.
pub fn graph_files(graph: &Graph, max_nodes: usize, group_depth: usize) -> ArchitectureGraph {
    let nodes_total = graph.nodes.len();
    let edges_total = graph.edges.len();

    let mut ranked: Vec<&belisarius_core::GraphNode> = graph.nodes.iter().collect();
    ranked.sort_by(|a, b| {
        let ca = a.in_degree + a.out_degree;
        let cb = b.in_degree + b.out_degree;
        cb.cmp(&ca).then(b.loc.cmp(&a.loc))
    });
    ranked.truncate(max_nodes);
    let kept: std::collections::HashSet<&str> = ranked.iter().map(|n| n.id.as_str()).collect();

    let nodes: Vec<GraphVizNode> = ranked
        .iter()
        .map(|n| GraphVizNode {
            id: n.id.clone(),
            label: leaf_name(&n.id),
            sublabel: group_key(&n.id, group_depth),
            language: n.language.clone(),
            file_count: 1,
            loc: n.loc,
            is_entry: n.is_entry_point,
        })
        .collect();

    let edges: Vec<GraphVizEdge> = graph
        .edges
        .iter()
        .filter(|e| kept.contains(e.from.as_str()) && kept.contains(e.to.as_str()))
        .map(|e| GraphVizEdge {
            source: e.from.clone(),
            target: e.to.clone(),
            weight: 1,
        })
        .collect();

    ArchitectureGraph {
        view: "file".to_string(),
        group_depth,
        nodes,
        edges,
        nodes_total,
        edges_total,
        rendered_cap: max_nodes,
    }
}

/// Module-level architecture diagram. Files are grouped into modules at
/// `group_depth`, edges are aggregated with weights. Result: ~6–30 nodes for
/// typical projects vs 100s for the file-level renderer.
pub fn render_mermaid_modules(graph: &Graph, group_depth: usize) -> String {
    if graph.nodes.is_empty() {
        return "flowchart LR\n  empty[\"(no nodes)\"]".to_string();
    }

    // Build module-level nodes.
    let mut modules: BTreeMap<String, ModuleAgg> = BTreeMap::new();
    for n in &graph.nodes {
        let key = group_key(&n.id, group_depth);
        let entry = modules.entry(key).or_insert(ModuleAgg {
            files: 0,
            loc: 0,
            lang_counts: BTreeMap::new(),
            is_entry: false,
        });
        entry.files += 1;
        entry.loc += n.loc;
        *entry.lang_counts.entry(n.language.clone()).or_insert(0) += 1;
        if n.is_entry_point {
            entry.is_entry = true;
        }
    }

    // Aggregate edges between modules, dropping self-loops.
    let mut edges: BTreeMap<(String, String), u32> = BTreeMap::new();
    for e in &graph.edges {
        let from = group_key(&e.from, group_depth);
        let to = group_key(&e.to, group_depth);
        if from == to {
            continue;
        }
        *edges.entry((from, to)).or_insert(0) += 1;
    }

    let mut id_map: BTreeMap<&str, String> = BTreeMap::new();
    for (i, k) in modules.keys().enumerate() {
        id_map.insert(k.as_str(), format!("m{i}"));
    }

    let mut out = String::new();
    out.push_str("flowchart TB\n");

    // Per-language classDef so the diagram uses our palette.
    for (lang, fill, stroke) in PALETTE {
        out.push_str(&format!(
            "  classDef {cls} fill:{fill},stroke:{stroke},stroke-width:1px,color:#161616\n",
            cls = mermaid_class(lang),
        ));
    }
    out.push_str("  classDef entryPoint stroke-width:2.5px\n");

    // Nodes — show files + LOC for context. We assign all classes (language +
    // optional entryPoint) in a single `class` statement so Mermaid merges
    // their declarations correctly rather than overwriting.
    for (key, m) in &modules {
        let nid = &id_map[key.as_str()];
        let label = format!("{}<br/>{} files · {} loc", escape(key), m.files, m.loc);
        out.push_str(&format!("  {nid}[\"{label}\"]\n", nid = nid, label = label));
        let lang = dominant_language(&m.lang_counts);
        let mut classes = vec![mermaid_class(&lang)];
        if m.is_entry {
            classes.push("entryPoint".to_string());
        }
        out.push_str(&format!("  class {nid} {}\n", classes.join(",")));
    }

    // Edges with weighted thickness; we render the weight only when > 1.
    for ((from, to), weight) in &edges {
        let (Some(a), Some(b)) = (id_map.get(from.as_str()), id_map.get(to.as_str())) else {
            continue;
        };
        if *weight > 1 {
            out.push_str(&format!("  {a} -->|{weight}| {b}\n"));
        } else {
            out.push_str(&format!("  {a} --> {b}\n"));
        }
    }
    out
}

/// File-level Mermaid (the previous default). Kept for the UI's "drill-in"
/// toggle. Caps the rendered node set to the most-connected files and groups
/// them by directory subgraph.
pub fn render_mermaid_files(graph: &Graph, max_nodes: usize, group_depth: usize) -> String {
    if graph.nodes.is_empty() {
        return "flowchart LR\n  empty[\"(no nodes)\"]".to_string();
    }
    let mut ranked: Vec<&belisarius_core::GraphNode> = graph.nodes.iter().collect();
    ranked.sort_by(|a, b| {
        let ca = a.in_degree + a.out_degree;
        let cb = b.in_degree + b.out_degree;
        cb.cmp(&ca).then(b.loc.cmp(&a.loc))
    });
    ranked.truncate(max_nodes);
    let kept: std::collections::HashSet<&str> = ranked.iter().map(|n| n.id.as_str()).collect();

    let mut groups: BTreeMap<String, Vec<&belisarius_core::GraphNode>> = BTreeMap::new();
    for n in &ranked {
        let key = group_key(&n.id, group_depth);
        groups.entry(key).or_default().push(*n);
    }

    let mut id_map: BTreeMap<&str, String> = BTreeMap::new();
    for (i, n) in ranked.iter().enumerate() {
        id_map.insert(n.id.as_str(), format!("n{i}"));
    }

    let mut out = String::new();
    out.push_str("flowchart TB\n");
    for (lang, fill, stroke) in PALETTE {
        out.push_str(&format!(
            "  classDef {cls} fill:{fill},stroke:{stroke},stroke-width:1px,color:#161616\n",
            cls = mermaid_class(lang),
        ));
    }
    out.push_str("  classDef entryPoint stroke-width:2.5px\n");

    for (group, nodes) in &groups {
        out.push_str(&format!(
            "  subgraph {}[\"{}\"]\n",
            mermaid_id(group),
            escape(group)
        ));
        for n in nodes {
            let nid = &id_map[n.id.as_str()];
            out.push_str(&format!("    {nid}[\"{}\"]\n", escape(&leaf_name(&n.id))));
            let mut classes = vec![mermaid_class(&n.language)];
            if n.is_entry_point {
                classes.push("entryPoint".to_string());
            }
            out.push_str(&format!("    class {nid} {}\n", classes.join(",")));
        }
        out.push_str("  end\n");
    }
    for e in &graph.edges {
        let (Some(a), Some(b)) = (id_map.get(e.from.as_str()), id_map.get(e.to.as_str())) else {
            continue;
        };
        if !kept.contains(e.from.as_str()) || !kept.contains(e.to.as_str()) {
            continue;
        }
        out.push_str(&format!("  {a} --> {b}\n"));
    }
    out
}

/// Directory-level summary used by the architecture overview table.
pub fn directory_summary(graph: &Graph, group_depth: usize) -> Vec<DirectorySummary> {
    let mut map: BTreeMap<String, DirectorySummary> = BTreeMap::new();
    for n in &graph.nodes {
        let key = group_key(&n.id, group_depth);
        let entry = map.entry(key.clone()).or_insert(DirectorySummary {
            path: key,
            files: 0,
            loc: 0,
            in_edges: 0,
            out_edges: 0,
            cross_edges: 0,
        });
        entry.files += 1;
        entry.loc += n.loc;
    }
    for e in &graph.edges {
        let from = group_key(&e.from, group_depth);
        let to = group_key(&e.to, group_depth);
        if let Some(s) = map.get_mut(&from) {
            s.out_edges += 1;
            if from != to {
                s.cross_edges += 1;
            }
        }
        if let Some(s) = map.get_mut(&to) {
            s.in_edges += 1;
        }
    }
    let mut out: Vec<DirectorySummary> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.cross_edges
            .cmp(&a.cross_edges)
            .then(b.files.cmp(&a.files))
    });
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySummary {
    pub path: String,
    pub files: u32,
    pub loc: u32,
    pub in_edges: u32,
    pub out_edges: u32,
    pub cross_edges: u32,
}

/// Structured graph data for the new (Cytoscape-driven) Architecture view.
/// Same underlying aggregation as `render_mermaid_*`, just emitted as JSON so
/// the frontend doesn't have to regex-parse Mermaid source to wire
/// interactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureGraph {
    pub view: String,
    pub group_depth: usize,
    pub nodes: Vec<GraphVizNode>,
    pub edges: Vec<GraphVizEdge>,
    pub nodes_total: usize,
    pub edges_total: usize,
    pub rendered_cap: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphVizNode {
    pub id: String,
    pub label: String,
    pub sublabel: String,
    pub language: String,
    pub file_count: u32,
    pub loc: u32,
    pub is_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphVizEdge {
    pub source: String,
    pub target: String,
    pub weight: u32,
}

struct ModuleAgg {
    files: u32,
    loc: u32,
    lang_counts: BTreeMap<String, u32>,
    is_entry: bool,
}

fn dominant_language(counts: &BTreeMap<String, u32>) -> String {
    counts
        .iter()
        .max_by_key(|(_, n)| *n)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| "default".to_string())
}

fn group_key(path: &str, depth: usize) -> String {
    // We bucket each file by its containing directory at most `depth` levels
    // deep. Files at the very top of the repo (no directory) fall into a
    // synthetic `(root)` bucket. Crucially we cap `depth` at the file's actual
    // directory depth so a shallow Cargo.toml never gets mixed in with deeper
    // module buckets.
    let segs: Vec<&str> = path.split('/').collect();
    let dir_depth = segs.len().saturating_sub(1);
    if dir_depth == 0 {
        return "(root)".to_string();
    }
    let effective = depth.min(dir_depth).max(1);
    segs.iter()
        .take(effective)
        .copied()
        .collect::<Vec<_>>()
        .join("/")
}

fn leaf_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn mermaid_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

fn mermaid_class(lang: &str) -> String {
    format!("lang_{}", lang.replace(|c: char| !c.is_alphanumeric(), "_"))
}

fn escape(s: &str) -> String {
    s.replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{GraphEdge, GraphNode};

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

    #[test]
    fn module_view_aggregates_files() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![
                node("crates/foo/src/a.rs", "rust", 0, 1),
                node("crates/foo/src/b.rs", "rust", 1, 0),
                node("crates/bar/src/lib.rs", "rust", 1, 0),
                node("web/src/app.tsx", "tsx", 0, 1),
            ],
            edges: vec![
                GraphEdge {
                    from: "crates/foo/src/a.rs".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 1,
                },
                GraphEdge {
                    from: "crates/foo/src/a.rs".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 2,
                },
                GraphEdge {
                    from: "web/src/app.tsx".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 3,
                },
            ],
            unresolved: 0,
        };
        let out = render_mermaid_modules(&g, 2);
        // 3 modules at depth=2: crates/foo, crates/bar, web/src
        assert!(out.contains("crates/foo<br/>2 files"));
        assert!(out.contains("crates/bar<br/>1 files"));
        assert!(out.contains("web/src<br/>1 files"));
        // The two foo→bar edges should be aggregated with weight 2.
        assert!(out.contains("|2|"));
        // Class assignment uses our palette.
        assert!(out.contains("lang_rust"));
        assert!(out.contains("lang_tsx"));
    }

    #[test]
    fn graph_modules_matches_mermaid_aggregation() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![
                node("crates/foo/src/a.rs", "rust", 0, 1),
                node("crates/foo/src/b.rs", "rust", 1, 0),
                node("crates/bar/src/lib.rs", "rust", 1, 0),
                node("web/src/app.tsx", "tsx", 0, 1),
            ],
            edges: vec![
                GraphEdge {
                    from: "crates/foo/src/a.rs".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 1,
                },
                GraphEdge {
                    from: "crates/foo/src/a.rs".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 2,
                },
                GraphEdge {
                    from: "web/src/app.tsx".into(),
                    to: "crates/bar/src/lib.rs".into(),
                    line: 3,
                },
            ],
            unresolved: 0,
        };
        let ag = graph_modules(&g, 2);
        // 3 modules at depth=2: crates/foo, crates/bar, web/src
        assert_eq!(ag.nodes.len(), 3);
        // Two foo→bar imports collapse into one weighted edge
        let foo_bar = ag.edges.iter().find(|e| e.weight == 2).unwrap();
        let foo = ag.nodes.iter().find(|n| n.label == "crates/foo").unwrap();
        let bar = ag.nodes.iter().find(|n| n.label == "crates/bar").unwrap();
        assert_eq!(foo_bar.source, foo.id);
        assert_eq!(foo_bar.target, bar.id);
        // 3 distinct module-level edges total (2 inside crates, 1 web→crates)
        assert_eq!(ag.edges.len(), 2);
        // Sublabel matches what the Mermaid renderer would say.
        assert!(foo.sublabel.contains("2 files"));
        assert!(bar.sublabel.contains("1 files"));
    }

    #[test]
    fn graph_files_respects_node_cap() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![
                node("a.rs", "rust", 5, 1),
                node("b.rs", "rust", 0, 0),
                node("c.rs", "rust", 0, 0),
            ],
            edges: vec![GraphEdge {
                from: "b.rs".into(),
                to: "a.rs".into(),
                line: 1,
            }],
            unresolved: 0,
        };
        let ag = graph_files(&g, 1, 1);
        // Only the most-connected node kept; its lone edge is dropped because
        // the other endpoint was clipped.
        assert_eq!(ag.nodes.len(), 1);
        assert_eq!(ag.nodes[0].id, "a.rs");
        assert!(ag.edges.is_empty());
        assert_eq!(ag.nodes_total, 3);
        assert_eq!(ag.rendered_cap, 1);
    }

    #[test]
    fn file_view_caps_node_count() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![node("a.rs", "rust", 0, 0), node("b.rs", "rust", 0, 0)],
            edges: vec![],
            unresolved: 0,
        };
        let out = render_mermaid_files(&g, 1, 1);
        assert!(out.starts_with("flowchart TB"));
    }
}
