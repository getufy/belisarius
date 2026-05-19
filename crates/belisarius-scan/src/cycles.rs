//! Strongly-connected-component detection on the resolved file graph.
//!
//! Tarjan's algorithm. Output is the list of SCCs of size > 1 (plus self-loops),
//! each ordered for stable JSON output.

use belisarius_core::{Graph, GraphCycle};
use std::collections::HashMap;

pub fn find_cycles(graph: &Graph) -> Vec<GraphCycle> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let id_of: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut self_loops: Vec<usize> = Vec::new();
    for e in &graph.edges {
        let (Some(&f), Some(&t)) = (id_of.get(e.from.as_str()), id_of.get(e.to.as_str())) else {
            continue;
        };
        if f == t {
            self_loops.push(f);
            continue;
        }
        adj[f].push(t);
    }

    let mut index_counter: usize = 0;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack: Vec<bool> = vec![false; n];
    let mut indices: Vec<Option<usize>> = vec![None; n];
    let mut lowlinks: Vec<usize> = vec![0; n];
    let mut sccs: Vec<Vec<usize>> = Vec::new();

    for v in 0..n {
        if indices[v].is_none() {
            strongconnect(
                v,
                &adj,
                &mut index_counter,
                &mut stack,
                &mut on_stack,
                &mut indices,
                &mut lowlinks,
                &mut sccs,
            );
        }
    }

    let mut out: Vec<GraphCycle> = sccs
        .into_iter()
        .filter(|s| s.len() > 1)
        .map(|s| {
            let mut nodes: Vec<String> = s.into_iter().map(|i| graph.nodes[i].id.clone()).collect();
            nodes.sort();
            let size = nodes.len() as u32;
            GraphCycle { nodes, size }
        })
        .collect();

    for s in self_loops {
        out.push(GraphCycle {
            nodes: vec![graph.nodes[s].id.clone()],
            size: 1,
        });
    }

    out.sort_by(|a, b| b.size.cmp(&a.size).then(a.nodes.cmp(&b.nodes)));
    out
}

#[allow(clippy::too_many_arguments)]
fn strongconnect(
    v: usize,
    adj: &[Vec<usize>],
    index_counter: &mut usize,
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    indices: &mut [Option<usize>],
    lowlinks: &mut [usize],
    sccs: &mut Vec<Vec<usize>>,
) {
    indices[v] = Some(*index_counter);
    lowlinks[v] = *index_counter;
    *index_counter += 1;
    stack.push(v);
    on_stack[v] = true;

    for &w in &adj[v] {
        if indices[w].is_none() {
            strongconnect(
                w,
                adj,
                index_counter,
                stack,
                on_stack,
                indices,
                lowlinks,
                sccs,
            );
            lowlinks[v] = lowlinks[v].min(lowlinks[w]);
        } else if on_stack[w] {
            lowlinks[v] = lowlinks[v].min(indices[w].unwrap());
        }
    }

    if Some(lowlinks[v]) == indices[v] {
        let mut comp = Vec::new();
        loop {
            let w = stack.pop().unwrap();
            on_stack[w] = false;
            comp.push(w);
            if w == v {
                break;
            }
        }
        sccs.push(comp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{Graph, GraphEdge, GraphNode};

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            language: "rust".into(),
            loc: 0,
            in_degree: 0,
            out_degree: 0,
            is_entry_point: false,
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
    fn no_cycles_returns_empty() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![node("a"), node("b"), node("c")],
            edges: vec![edge("a", "b"), edge("b", "c")],
            unresolved: 0,
        };
        assert!(find_cycles(&g).is_empty());
    }

    #[test]
    fn three_node_cycle() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![node("a"), node("b"), node("c")],
            edges: vec![edge("a", "b"), edge("b", "c"), edge("c", "a")],
            unresolved: 0,
        };
        let cycles = find_cycles(&g);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].size, 3);
        assert_eq!(cycles[0].nodes, vec!["a", "b", "c"]);
    }

    #[test]
    fn two_disjoint_cycles() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![node("a"), node("b"), node("c"), node("d")],
            edges: vec![
                edge("a", "b"),
                edge("b", "a"),
                edge("c", "d"),
                edge("d", "c"),
            ],
            unresolved: 0,
        };
        let cycles = find_cycles(&g);
        assert_eq!(cycles.len(), 2);
        assert!(cycles.iter().all(|c| c.size == 2));
    }

    #[test]
    fn self_loop_detected() {
        let g = Graph {
            root: ".".into(),
            nodes: vec![node("a")],
            edges: vec![edge("a", "a")],
            unresolved: 0,
        };
        let cycles = find_cycles(&g);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].size, 1);
    }
}
