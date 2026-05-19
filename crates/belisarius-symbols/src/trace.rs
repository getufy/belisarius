//! N-hop symbol trace — walks the call graph in either direction using the
//! SCIP index already built by `belisarius index`.
//!
//! Callers direction: BFS over `callers_of(sym)` (already returns one hop).
//! Callees direction: for the symbol's definition site, find all non-definition
//! occurrences whose range falls inside its `enclosing_range`. That gives
//! one hop outward; recurse from there.

use crate::scip::Occurrence;
use crate::{OccurrenceLocation, Range, SymbolStore};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
pub struct TraceNode {
    pub symbol: String,
    pub display_name: String,
    pub depth: u32,
    /// Definition site of this symbol, when one exists in the index.
    pub def_file: Option<String>,
    pub def_line: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceEdge {
    pub from: String,
    pub to: String,
    pub call_sites: Vec<CallSite>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallSite {
    pub file: String,
    pub start_line: i32,
    pub end_line: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Callers,
    Callees,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceReport {
    pub root: String,
    pub direction: &'static str,
    pub hops: u32,
    pub nodes: Vec<TraceNode>,
    pub edges: Vec<TraceEdge>,
    pub truncated: bool,
}

pub fn trace(store: &SymbolStore, symbol: &str, direction: Direction, hops: u32) -> TraceReport {
    let max_hops = hops.clamp(1, 8);
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((symbol.to_string(), 0));
    visited.insert(symbol.to_string());

    let mut nodes: Vec<TraceNode> = Vec::new();
    let mut edges: Vec<TraceEdge> = Vec::new();
    let mut truncated = false;
    nodes.push(make_node(store, symbol, 0));

    // Soft cap on total nodes to keep responses bounded.
    let node_cap: usize = 200;

    while let Some((cur, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        let neighbors = match direction {
            Direction::Callers => callers_neighbors(store, &cur),
            Direction::Callees => callees_neighbors(store, &cur),
        };
        for (neighbor, sites) in neighbors {
            if neighbor == cur {
                continue;
            }
            let (from, to) = match direction {
                Direction::Callers => (neighbor.clone(), cur.clone()),
                Direction::Callees => (cur.clone(), neighbor.clone()),
            };
            edges.push(TraceEdge {
                from,
                to,
                call_sites: sites,
            });
            if !visited.contains(&neighbor) {
                visited.insert(neighbor.clone());
                nodes.push(make_node(store, &neighbor, depth + 1));
                if nodes.len() >= node_cap {
                    truncated = true;
                    break;
                }
                queue.push_back((neighbor, depth + 1));
            }
        }
        if truncated {
            break;
        }
    }

    TraceReport {
        root: symbol.to_string(),
        direction: match direction {
            Direction::Callers => "callers",
            Direction::Callees => "callees",
        },
        hops: max_hops,
        nodes,
        edges,
        truncated,
    }
}

fn callers_neighbors(store: &SymbolStore, symbol: &str) -> Vec<(String, Vec<CallSite>)> {
    store
        .callers_of(symbol)
        .into_iter()
        .map(|c| {
            let sites = c
                .call_sites
                .iter()
                .map(|o| {
                    let r = o.range();
                    CallSite {
                        file: o.path().to_string(),
                        start_line: r.start_line,
                        end_line: r.end_line,
                    }
                })
                .collect();
            (c.symbol, sites)
        })
        .collect()
}

fn callees_neighbors(store: &SymbolStore, symbol: &str) -> Vec<(String, Vec<CallSite>)> {
    use std::collections::HashMap;

    // For each definition of `symbol`, find non-definition occurrences in the
    // same document whose range falls inside the enclosing range. Those are
    // the symbols this function calls.
    let mut by_callee: HashMap<String, Vec<CallSite>> = HashMap::new();
    for def in store.def_of(symbol) {
        let env = match def.enclosing_range() {
            Some(r) => r,
            None => continue,
        };
        for occ in &def.document.occurrences {
            if is_def_occurrence(occ) {
                continue;
            }
            let r = parse_occ_range(&occ.range);
            if !range_contains(&env, &r) {
                continue;
            }
            let site = CallSite {
                file: def.path().to_string(),
                start_line: r.start_line,
                end_line: r.end_line,
            };
            by_callee.entry(occ.symbol.clone()).or_default().push(site);
        }
    }
    by_callee.into_iter().collect()
}

fn is_def_occurrence(occ: &Occurrence) -> bool {
    (occ.symbol_roles & crate::SymbolRole::Definition as i32) != 0
}

fn parse_occ_range(arr: &[i32]) -> Range {
    match arr.len() {
        3 => Range {
            start_line: arr[0],
            start_char: arr[1],
            end_line: arr[0],
            end_char: arr[2],
        },
        4 => Range {
            start_line: arr[0],
            start_char: arr[1],
            end_line: arr[2],
            end_char: arr[3],
        },
        _ => Range::default(),
    }
}

fn range_contains(outer: &Range, inner: &Range) -> bool {
    if inner.start_line < outer.start_line || inner.end_line > outer.end_line {
        return false;
    }
    if inner.start_line == outer.start_line && inner.start_char < outer.start_char {
        return false;
    }
    if inner.end_line == outer.end_line && inner.end_char > outer.end_char {
        return false;
    }
    true
}

fn make_node(store: &SymbolStore, symbol: &str, depth: u32) -> TraceNode {
    let info = store.info_for(symbol);
    let display_name = info.map(|i| i.display_name.clone()).unwrap_or_default();
    let def_iter: Vec<OccurrenceLocation<'_>> = store.def_of(symbol);
    let (def_file, def_line) = match def_iter.first() {
        Some(loc) => {
            let r = loc.range();
            (Some(loc.path().to_string()), Some(r.start_line))
        }
        None => (None, None),
    };
    TraceNode {
        symbol: symbol.to_string(),
        display_name,
        depth,
        def_file,
        def_line,
    }
}
