//! Distance from any entry-point to every other file in the graph.
//!
//! BFS from each entry. Within a single entry's BFS, the visited-set prevents
//! revisits, so we get the *shortest* path from that entry. We then take the
//! max across entries — a node reachable from multiple entries reports the
//! deepest of its shortest-paths.
//!
//! For a DAG this approximates Lakos depth; on cyclic graphs the visited-set
//! breaks the cycle and the answer is still bounded and stable.

use belisarius_core::Graph;
use std::collections::{HashMap, VecDeque};

/// Returns (max_depth, depth_per_node).
pub fn compute_depths(graph: &Graph) -> (u32, HashMap<String, u32>) {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &graph.edges {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }

    let entries: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.is_entry_point)
        .map(|n| n.id.as_str())
        .collect();

    let mut depths: HashMap<String, u32> = HashMap::with_capacity(graph.nodes.len());
    if entries.is_empty() {
        return (0, depths);
    }

    // BFS layered: distance to root entry-point. Take the max across entry-points
    // for nodes reachable from multiple entries.
    for entry in &entries {
        let mut visited: HashMap<&str, u32> = HashMap::new();
        let mut queue: VecDeque<(&str, u32)> = VecDeque::new();
        queue.push_back((entry, 0));
        visited.insert(entry, 0);
        while let Some((node, depth)) = queue.pop_front() {
            let prev = depths.entry(node.to_string()).or_insert(0);
            if *prev < depth {
                *prev = depth;
            }
            if let Some(children) = adj.get(node) {
                for &child in children {
                    if !visited.contains_key(child) {
                        visited.insert(child, depth + 1);
                        queue.push_back((child, depth + 1));
                    }
                }
            }
        }
    }

    let max_depth = depths.values().copied().max().unwrap_or(0);
    (max_depth, depths)
}

/// Mutate `graph.nodes` to fill `depth_from_entry`.
pub fn annotate(graph: &mut Graph) -> u32 {
    let (max_depth, depths) = compute_depths(graph);
    for n in &mut graph.nodes {
        n.depth_from_entry = depths.get(&n.id).copied().unwrap_or(0);
    }
    max_depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{Graph, GraphEdge, GraphNode};

    fn node(id: &str, entry: bool) -> GraphNode {
        GraphNode {
            id: id.into(),
            language: "rust".into(),
            loc: 0,
            in_degree: 0,
            out_degree: 0,
            is_entry_point: entry,
            depth_from_entry: 0,
        }
    }
    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            line: 0,
        }
    }

    #[test]
    fn linear_chain_depth_3() {
        let mut g = Graph {
            root: ".".into(),
            nodes: vec![
                node("a", true),
                node("b", false),
                node("c", false),
                node("d", false),
            ],
            edges: vec![edge("a", "b"), edge("b", "c"), edge("c", "d")],
            unresolved: 0,
        };
        let max = annotate(&mut g);
        assert_eq!(max, 3);
        let d = g.nodes.iter().find(|n| n.id == "d").unwrap();
        assert_eq!(d.depth_from_entry, 3);
    }

    #[test]
    fn diamond_picks_longest_path() {
        let mut g = Graph {
            root: ".".into(),
            nodes: vec![
                node("a", true),
                node("b", false),
                node("c", false),
                node("d", false),
            ],
            edges: vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "d"),
                edge("c", "d"),
            ],
            unresolved: 0,
        };
        let max = annotate(&mut g);
        assert_eq!(max, 2);
    }

    #[test]
    fn no_entry_points_zero_depth() {
        let mut g = Graph {
            root: ".".into(),
            nodes: vec![node("a", false), node("b", false)],
            edges: vec![edge("a", "b")],
            unresolved: 0,
        };
        let max = annotate(&mut g);
        assert_eq!(max, 0);
    }
}
