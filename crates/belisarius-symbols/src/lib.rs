//! SCIP index reader + symbol queries.
//!
//! Built on the SCIP protocol from <https://github.com/sourcegraph/scip>. Indexes
//! are produced by per-language indexers (rust-analyzer, scip-typescript, etc.)
//! and ingested here into a queryable in-memory `SymbolStore`.

use anyhow::{Context, Result};
use prost::Message;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub mod indexer;
pub mod trace;
pub use indexer::{by_language, registry, Indexer, IndexerStatus};

pub mod scip {
    include!(concat!(env!("OUT_DIR"), "/scip.rs"));
}

pub use scip::{Document, Index, Occurrence, Symbol, SymbolInformation, SymbolRole};

/// Concatenate multiple SCIP indexes into one. Documents and external_symbols are
/// appended verbatim; metadata is taken from the first non-empty index.
pub fn merge(indexes: impl IntoIterator<Item = Index>) -> Index {
    let mut merged = Index::default();
    for idx in indexes {
        if merged.metadata.is_none() {
            merged.metadata = idx.metadata.clone();
        }
        merged.documents.extend(idx.documents);
        merged.external_symbols.extend(idx.external_symbols);
    }
    merged
}

/// Write an `Index` back out as protobuf bytes.
pub fn write_index(index: &Index, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    let mut bytes = Vec::with_capacity(64 * 1024);
    index.encode(&mut bytes).context("encoding scip index")?;
    std::fs::write(path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read a `.scip` protobuf file from disk.
pub fn read_index(path: impl AsRef<Path>) -> Result<Index> {
    let path = path.as_ref();
    let bytes =
        std::fs::read(path).with_context(|| format!("reading scip index {}", path.display()))?;
    let index = Index::decode(&*bytes)
        .with_context(|| format!("decoding scip index {}", path.display()))?;
    Ok(index)
}

/// In-memory view of a SCIP index, indexed by symbol id for fast lookup.
pub struct SymbolStore {
    pub index: Index,
    /// symbol id → (document path, occurrence index)
    occurrences: HashMap<String, Vec<OccurrenceRef>>,
    /// symbol id → SymbolInformation (when the indexer emitted one)
    info: HashMap<String, SymbolInformation>,
}

#[derive(Debug, Clone, Copy)]
pub struct OccurrenceRef {
    pub document_idx: usize,
    pub occurrence_idx: usize,
}

impl SymbolStore {
    pub fn new(index: Index) -> Self {
        let mut occurrences: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();
        let mut info: HashMap<String, SymbolInformation> = HashMap::new();
        for (di, doc) in index.documents.iter().enumerate() {
            for (oi, occ) in doc.occurrences.iter().enumerate() {
                occurrences
                    .entry(occ.symbol.clone())
                    .or_default()
                    .push(OccurrenceRef {
                        document_idx: di,
                        occurrence_idx: oi,
                    });
            }
            for sym in &doc.symbols {
                info.insert(sym.symbol.clone(), sym.clone());
            }
        }
        // index.external_symbols (cross-doc metadata) — pick up any extra info too.
        for sym in &index.external_symbols {
            info.entry(sym.symbol.clone())
                .or_insert_with(|| sym.clone());
        }
        Self {
            index,
            occurrences,
            info,
        }
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(read_index(path)?))
    }

    pub fn document_count(&self) -> usize {
        self.index.documents.len()
    }

    pub fn symbol_count(&self) -> usize {
        self.occurrences.len()
    }

    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.index.documents.iter()
    }

    pub fn document_paths(&self) -> impl Iterator<Item = &str> {
        self.index
            .documents
            .iter()
            .map(|d| d.relative_path.as_str())
    }

    pub fn symbols(&self) -> impl Iterator<Item = &str> {
        self.occurrences.keys().map(|s| s.as_str())
    }

    pub fn info_for(&self, symbol: &str) -> Option<&SymbolInformation> {
        self.info.get(symbol)
    }

    /// All occurrences (definitions + references) of a symbol.
    pub fn occurrences_of<'a>(&'a self, symbol: &str) -> Vec<OccurrenceLocation<'a>> {
        let mut out = Vec::new();
        if let Some(refs) = self.occurrences.get(symbol) {
            for r in refs {
                let doc = &self.index.documents[r.document_idx];
                let occ = &doc.occurrences[r.occurrence_idx];
                out.push(OccurrenceLocation {
                    document: doc,
                    occurrence: occ,
                });
            }
        }
        out
    }

    /// References only (not the symbol's own definition site).
    pub fn refs_to<'a>(&'a self, symbol: &str) -> Vec<OccurrenceLocation<'a>> {
        self.occurrences_of(symbol)
            .into_iter()
            .filter(|o| !is_definition(o.occurrence))
            .collect()
    }

    /// Definition site(s) of a symbol.
    pub fn def_of<'a>(&'a self, symbol: &str) -> Vec<OccurrenceLocation<'a>> {
        self.occurrences_of(symbol)
            .into_iter()
            .filter(|o| is_definition(o.occurrence))
            .collect()
    }

    /// All symbols defined inside a document by relative path.
    pub fn defs_in_file<'a>(&'a self, path: &str) -> Vec<&'a SymbolInformation> {
        for doc in &self.index.documents {
            if doc.relative_path == path {
                let mut out = Vec::new();
                let defined: std::collections::HashSet<&str> = doc
                    .occurrences
                    .iter()
                    .filter(|o| is_definition(o))
                    .map(|o| o.symbol.as_str())
                    .collect();
                for sym in &doc.symbols {
                    if defined.contains(sym.symbol.as_str()) {
                        out.push(sym);
                    }
                }
                // Also include defined-but-info-less symbols by stitching from doc.symbols when present
                if out.is_empty() {
                    for sym in &doc.symbols {
                        out.push(sym);
                    }
                }
                return out;
            }
        }
        Vec::new()
    }

    /// Top symbols by occurrence count (rough proxy for "most-used in this index").
    pub fn top_symbols(&self, n: usize) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = self
            .occurrences
            .iter()
            .map(|(k, refs)| (k.clone(), refs.len()))
            .collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v.truncate(n);
        v
    }

    /// References grouped by the document path they appear in. Stable order (BTreeMap).
    pub fn refs_by_file<'a>(
        &'a self,
        symbol: &str,
    ) -> BTreeMap<String, Vec<OccurrenceLocation<'a>>> {
        let mut out: BTreeMap<String, Vec<OccurrenceLocation<'a>>> = BTreeMap::new();
        for loc in self.refs_to(symbol) {
            out.entry(loc.path().to_string()).or_default().push(loc);
        }
        out
    }

    /// For each reference to `symbol`, find the enclosing definition (the function /
    /// method / impl whose body contains the reference) and group by caller.
    ///
    /// Relies on `enclosing_range` being populated on definition occurrences. Most
    /// SCIP indexers (rust-analyzer, scip-typescript) emit this. If absent, no
    /// callers are returned for that document.
    pub fn callers_of<'a>(&'a self, symbol: &str) -> Vec<CallerSummary<'a>> {
        let mut by_caller: HashMap<String, Vec<OccurrenceLocation<'a>>> = HashMap::new();
        // Pre-index definitions with enclosing_range per document.
        let docs = &self.index.documents;
        for loc in self.refs_to(symbol) {
            let doc_idx = match docs.iter().position(|d| std::ptr::eq(d, loc.document)) {
                Some(i) => i,
                None => continue,
            };
            let doc = &docs[doc_idx];
            let ref_range = loc.range();
            let mut best: Option<(&str, Range, i64)> = None;
            for occ in &doc.occurrences {
                if !is_definition(occ) {
                    continue;
                }
                let Some(env) = parse_range(&occ.enclosing_range) else {
                    continue;
                };
                if !range_contains(&env, &ref_range) {
                    continue;
                }
                // Prefer the tightest enclosing range.
                let span = range_span(&env);
                let pick = match best {
                    None => true,
                    Some((_, _, prev_span)) => span < prev_span,
                };
                if pick {
                    best = Some((occ.symbol.as_str(), env, span));
                }
            }
            if let Some((caller_sym, _, _)) = best {
                by_caller
                    .entry(caller_sym.to_string())
                    .or_default()
                    .push(loc);
            }
        }
        let mut out: Vec<CallerSummary<'a>> = by_caller
            .into_iter()
            .map(|(sym, sites)| CallerSummary {
                info: self.info.get(&sym),
                symbol: sym,
                call_sites: sites,
            })
            .collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.call_sites.len()));
        out
    }

    /// Case-insensitive substring search over symbol ids and display names.
    /// Results ranked by occurrence count (most-used first).
    pub fn find_symbols(&self, query: &str, limit: usize) -> Vec<SymbolMatch<'_>> {
        let needle = query.to_lowercase();
        let mut hits: Vec<SymbolMatch<'_>> = Vec::new();
        for (sym, refs) in &self.occurrences {
            let info = self.info.get(sym);
            let display = info.map(|i| i.display_name.as_str()).unwrap_or("");
            let sym_l = sym.to_lowercase();
            let disp_l = display.to_lowercase();
            if !sym_l.contains(&needle) && !disp_l.contains(&needle) {
                continue;
            }
            hits.push(SymbolMatch {
                symbol: sym.as_str(),
                info,
                occurrences: refs.len(),
            });
        }
        hits.sort_by_key(|x| std::cmp::Reverse(x.occurrences));
        hits.truncate(limit);
        hits
    }

    /// Backward call traversal — every symbol that transitively reaches `symbol`
    /// via the caller graph, up to `max_depth`. Bounded by `MAX_IMPACT_NODES`
    /// to keep responses tractable on dense codebases.
    pub fn impact_of(&self, symbol: &str, max_depth: usize) -> ImpactReport {
        let mut report = ImpactReport {
            root: symbol.to_string(),
            nodes: Vec::new(),
            files: Vec::new(),
            truncated: false,
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen.insert(symbol.to_string());
        let mut frontier: Vec<String> = vec![symbol.to_string()];
        for depth in 1..=max_depth {
            let mut next: Vec<String> = Vec::new();
            for sym in &frontier {
                for caller in self.callers_of(sym) {
                    let csym = caller.symbol.clone();
                    if seen.contains(&csym) {
                        continue;
                    }
                    seen.insert(csym.clone());
                    let display = self
                        .info
                        .get(&csym)
                        .map(|i| i.display_name.clone())
                        .unwrap_or_default();
                    let files: Vec<String> = caller
                        .call_sites
                        .iter()
                        .map(|s| s.path().to_string())
                        .collect();
                    let unique_files = {
                        let mut v: Vec<String> = files
                            .into_iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        v.sort();
                        v
                    };
                    report.nodes.push(ImpactNode {
                        symbol: csym.clone(),
                        display_name: display,
                        depth,
                        callers_of: sym.clone(),
                        call_site_count: caller.call_sites.len(),
                        files: unique_files,
                    });
                    next.push(csym);
                    if report.nodes.len() >= MAX_IMPACT_NODES {
                        report.truncated = true;
                        break;
                    }
                }
                if report.nodes.len() >= MAX_IMPACT_NODES {
                    break;
                }
            }
            if report.nodes.len() >= MAX_IMPACT_NODES {
                break;
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        // Roll up unique files touched by anyone in the closure.
        let mut files = std::collections::BTreeSet::new();
        for n in &report.nodes {
            for f in &n.files {
                files.insert(f.clone());
            }
        }
        report.files = files.into_iter().collect();
        report
    }

    /// Forward call traversal — every symbol referenced inside the body of
    /// `symbol` (and recursively), up to `max_depth`.
    pub fn flow_from(&self, symbol: &str, max_depth: usize) -> FlowReport {
        let mut report = FlowReport {
            root: symbol.to_string(),
            nodes: Vec::new(),
            truncated: false,
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen.insert(symbol.to_string());
        let mut frontier: Vec<String> = vec![symbol.to_string()];
        for depth in 1..=max_depth {
            let mut next: Vec<String> = Vec::new();
            for sym in &frontier {
                for callee in self.callees_of(sym) {
                    if seen.contains(&callee) {
                        continue;
                    }
                    seen.insert(callee.clone());
                    let display = self
                        .info
                        .get(&callee)
                        .map(|i| i.display_name.clone())
                        .unwrap_or_default();
                    report.nodes.push(FlowNode {
                        symbol: callee.clone(),
                        display_name: display,
                        depth,
                        called_from: sym.clone(),
                    });
                    next.push(callee);
                    if report.nodes.len() >= MAX_IMPACT_NODES {
                        report.truncated = true;
                        break;
                    }
                }
                if report.nodes.len() >= MAX_IMPACT_NODES {
                    break;
                }
            }
            if report.nodes.len() >= MAX_IMPACT_NODES {
                break;
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        report
    }

    /// Symbols referenced inside the enclosing range of `sym`'s definition(s).
    /// Skips the symbol itself and skips definition-role occurrences (those
    /// only contribute to the function's own surface, not its calls).
    pub fn callees_of(&self, sym: &str) -> Vec<String> {
        let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for def in self.def_of(sym) {
            let Some(env) = def.enclosing_range() else {
                continue;
            };
            for occ in &def.document.occurrences {
                if is_definition(occ) {
                    continue;
                }
                if occ.symbol == sym {
                    continue;
                }
                let Some(r) = parse_range(&occ.range) else {
                    continue;
                };
                if range_contains(&env, &r) {
                    out.insert(occ.symbol.clone());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Consolidated def + direct callers + direct callees view of a symbol.
    /// Cheap one-shot call for "tell me everything about this symbol".
    pub fn symbol_360(&self, sym: &str) -> Symbol360 {
        let def_locs = self.def_of(sym);
        let occurrence_count = self.occurrences.get(sym).map(|v| v.len()).unwrap_or(0);
        let def_sites: Vec<DefSite> = def_locs
            .iter()
            .map(|l| DefSite {
                file: l.path().to_string(),
                range: l.range(),
            })
            .collect();
        let callers = self
            .callers_of(sym)
            .into_iter()
            .map(|c| CallerLite {
                symbol: c.symbol.clone(),
                display_name: c.info.map(|i| i.display_name.clone()).unwrap_or_default(),
                call_sites: c.call_sites.len(),
            })
            .collect();
        let callees = self
            .callees_of(sym)
            .into_iter()
            .map(|s| {
                let display = self
                    .info
                    .get(&s)
                    .map(|i| i.display_name.clone())
                    .unwrap_or_default();
                CalleeLite {
                    symbol: s,
                    display_name: display,
                }
            })
            .collect();
        let display_name = self
            .info
            .get(sym)
            .map(|i| i.display_name.clone())
            .unwrap_or_default();
        Symbol360 {
            symbol: sym.to_string(),
            display_name,
            occurrence_count,
            def_sites,
            callers,
            callees,
        }
    }

    /// Summarise activity in a single file: definitions, outgoing refs, incoming refs.
    pub fn file_summary(&self, path: &str) -> Option<FileSummary<'_>> {
        let doc = self
            .index
            .documents
            .iter()
            .find(|d| d.relative_path == path)?;
        // Symbols this file _defines_ (and which symbols outside the file reference them).
        let mut defines: Vec<&SymbolInformation> = doc.symbols.iter().collect();
        defines.sort_by(|a, b| a.symbol.cmp(&b.symbol));

        let defined_set: std::collections::HashSet<&str> = doc
            .occurrences
            .iter()
            .filter(|o| is_definition(o))
            .map(|o| o.symbol.as_str())
            .collect();

        let mut incoming_refs: usize = 0;
        for sym in &defined_set {
            if let Some(refs) = self.occurrences.get(*sym) {
                for r in refs {
                    let d = &self.index.documents[r.document_idx];
                    if d.relative_path != path {
                        incoming_refs += 1;
                    }
                }
            }
        }

        let outgoing_refs = doc.occurrences.iter().filter(|o| !is_definition(o)).count();

        Some(FileSummary {
            path: doc.relative_path.as_str(),
            defines,
            definition_count: defined_set.len(),
            outgoing_refs,
            incoming_refs,
            total_occurrences: doc.occurrences.len(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct OccurrenceLocation<'a> {
    pub document: &'a Document,
    pub occurrence: &'a Occurrence,
}

impl<'a> OccurrenceLocation<'a> {
    pub fn path(&self) -> &str {
        &self.document.relative_path
    }

    /// SCIP encodes ranges as flat int32 arrays: [start_line, start_char, end_line, end_char]
    /// or [start_line, start_char, end_char] when start_line == end_line.
    pub fn range(&self) -> Range {
        parse_range(&self.occurrence.range).unwrap_or_default()
    }

    /// Enclosing range (only populated by indexers for definition occurrences).
    pub fn enclosing_range(&self) -> Option<Range> {
        parse_range(&self.occurrence.enclosing_range)
    }
}

fn parse_range(arr: &[i32]) -> Option<Range> {
    match arr.len() {
        3 => Some(Range {
            start_line: arr[0],
            start_char: arr[1],
            end_line: arr[0],
            end_char: arr[2],
        }),
        4 => Some(Range {
            start_line: arr[0],
            start_char: arr[1],
            end_line: arr[2],
            end_char: arr[3],
        }),
        _ => None,
    }
}

fn range_contains(outer: &Range, inner: &Range) -> bool {
    if outer.start_line > inner.start_line || outer.end_line < inner.end_line {
        return false;
    }
    if outer.start_line == inner.start_line && outer.start_char > inner.start_char {
        return false;
    }
    if outer.end_line == inner.end_line && outer.end_char < inner.end_char {
        return false;
    }
    true
}

fn range_span(r: &Range) -> i64 {
    let lines = (r.end_line - r.start_line) as i64;
    let chars = (r.end_char - r.start_char) as i64;
    lines * 1_000 + chars
}

#[derive(Debug, Clone)]
pub struct CallerSummary<'a> {
    pub symbol: String,
    pub info: Option<&'a SymbolInformation>,
    pub call_sites: Vec<OccurrenceLocation<'a>>,
}

#[derive(Debug, Clone)]
pub struct SymbolMatch<'a> {
    pub symbol: &'a str,
    pub info: Option<&'a SymbolInformation>,
    pub occurrences: usize,
}

#[derive(Debug, Clone)]
pub struct FileSummary<'a> {
    pub path: &'a str,
    pub defines: Vec<&'a SymbolInformation>,
    pub definition_count: usize,
    pub outgoing_refs: usize,
    pub incoming_refs: usize,
    pub total_occurrences: usize,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct Range {
    pub start_line: i32,
    pub start_char: i32,
    pub end_line: i32,
    pub end_char: i32,
}

fn is_definition(occ: &Occurrence) -> bool {
    occ.symbol_roles & (SymbolRole::Definition as i32) != 0
}

/// Upper bound on nodes returned by `impact_of` / `flow_from`. Keeps responses
/// bounded on densely-connected codebases; truncation is signalled in the
/// returned report.
pub const MAX_IMPACT_NODES: usize = 200;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct ImpactReport {
    pub root: String,
    pub nodes: Vec<ImpactNode>,
    pub files: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct ImpactNode {
    pub symbol: String,
    pub display_name: String,
    pub depth: usize,
    /// The symbol whose body contains references to this caller's target.
    pub callers_of: String,
    pub call_site_count: usize,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct FlowReport {
    pub root: String,
    pub nodes: Vec<FlowNode>,
    pub truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct FlowNode {
    pub symbol: String,
    pub display_name: String,
    pub depth: usize,
    pub called_from: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct Symbol360 {
    pub symbol: String,
    pub display_name: String,
    pub occurrence_count: usize,
    pub def_sites: Vec<DefSite>,
    pub callers: Vec<CallerLite>,
    pub callees: Vec<CalleeLite>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct DefSite {
    pub file: String,
    pub range: Range,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CallerLite {
    pub symbol: String,
    pub display_name: String,
    pub call_sites: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, export_to = "web/src/types/generated/"))]
pub struct CalleeLite {
    pub symbol: String,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use scip::{Document, Index, Occurrence, SymbolInformation};

    fn occ(symbol: &str, range: Vec<i32>, def: bool, enclosing: Vec<i32>) -> Occurrence {
        Occurrence {
            range,
            symbol: symbol.into(),
            symbol_roles: if def {
                SymbolRole::Definition as i32
            } else {
                0
            },
            override_documentation: vec![],
            syntax_kind: 0,
            diagnostics: vec![],
            enclosing_range: enclosing,
        }
    }

    fn doc(path: &str, occurrences: Vec<Occurrence>, symbols: Vec<SymbolInformation>) -> Document {
        Document {
            language: "rust".into(),
            relative_path: path.into(),
            occurrences,
            symbols,
            text: String::new(),
            position_encoding: 0,
        }
    }

    fn sym(s: &str, display: &str) -> SymbolInformation {
        SymbolInformation {
            symbol: s.into(),
            documentation: vec![],
            relationships: vec![],
            kind: 0,
            display_name: display.into(),
            signature_documentation: None,
            enclosing_symbol: String::new(),
        }
    }

    fn make_store() -> SymbolStore {
        // Synthetic: file "lib.rs" defines fn foo (lines 1-10) and fn bar (lines 12-20).
        // foo calls helper at line 5; bar calls helper at line 14 (twice) and foo at line 16.
        let d = doc(
            "lib.rs",
            vec![
                occ("foo#", vec![1, 3, 1, 6], true, vec![1, 0, 10, 0]),
                occ("helper#", vec![5, 4, 5, 10], false, vec![]),
                occ("bar#", vec![12, 3, 12, 6], true, vec![12, 0, 20, 0]),
                occ("helper#", vec![14, 4, 14, 10], false, vec![]),
                occ("helper#", vec![14, 20, 14, 26], false, vec![]),
                occ("foo#", vec![16, 4, 16, 7], false, vec![]),
                // definition for helper lives in another doc:
            ],
            vec![sym("foo#", "foo"), sym("bar#", "bar")],
        );
        let helper_doc = doc(
            "util.rs",
            vec![occ("helper#", vec![1, 3, 1, 9], true, vec![1, 0, 5, 0])],
            vec![sym("helper#", "helper")],
        );
        let index = Index {
            metadata: None,
            documents: vec![d, helper_doc],
            external_symbols: vec![],
        };
        SymbolStore::new(index)
    }

    #[test]
    fn refs_to_skips_definitions() {
        let s = make_store();
        let refs = s.refs_to("helper#");
        assert_eq!(refs.len(), 3); // 1 in lib.rs at line 5, 2 in lib.rs at line 14
        assert!(refs.iter().all(|r| !is_definition(r.occurrence)));
    }

    #[test]
    fn callers_of_uses_enclosing_range() {
        let s = make_store();
        let callers = s.callers_of("helper#");
        // foo (1 call site) + bar (2 call sites)
        let mut got: Vec<(String, usize)> = callers
            .into_iter()
            .map(|c| (c.symbol, c.call_sites.len()))
            .collect();
        got.sort();
        assert_eq!(got, vec![("bar#".into(), 2), ("foo#".into(), 1)]);
    }

    #[test]
    fn refs_by_file_groups_correctly() {
        let s = make_store();
        let grouped = s.refs_by_file("foo#");
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped.get("lib.rs").unwrap().len(), 1);
    }

    #[test]
    fn find_symbols_matches_substring() {
        let s = make_store();
        let hits = s.find_symbols("foo", 10);
        assert!(hits.iter().any(|h| h.symbol == "foo#"));
        let hits2 = s.find_symbols("helper", 10);
        assert!(hits2.iter().any(|h| h.symbol == "helper#"));
    }

    #[test]
    fn file_summary_counts_defs_and_refs() {
        let s = make_store();
        let fs = s.file_summary("lib.rs").unwrap();
        assert_eq!(fs.definition_count, 2); // foo, bar
        assert_eq!(fs.outgoing_refs, 4); // helper x3 + foo x1
                                         // foo is referenced once from inside the same file → incoming_refs = 0
                                         // bar is never referenced → incoming_refs = 0
        assert_eq!(fs.incoming_refs, 0);
    }

    #[test]
    fn impact_of_walks_callers_transitively() {
        let s = make_store();
        // helper is called by foo and bar (both in lib.rs).
        let r = s.impact_of("helper#", 3);
        let symbols: std::collections::HashSet<String> =
            r.nodes.iter().map(|n| n.symbol.clone()).collect();
        assert!(symbols.contains("foo#"));
        assert!(symbols.contains("bar#"));
        // Both callers live in lib.rs, so files should include it.
        assert!(r.files.iter().any(|f| f == "lib.rs"));
    }

    #[test]
    fn flow_from_finds_inner_callees() {
        let s = make_store();
        // foo's body (lines 1-10) only references helper#.
        let r = s.flow_from("foo#", 2);
        assert!(r.nodes.iter().any(|n| n.symbol == "helper#"));
    }

    #[test]
    fn symbol_360_combines_views() {
        let s = make_store();
        let v = s.symbol_360("helper#");
        assert_eq!(v.symbol, "helper#");
        assert_eq!(v.def_sites.len(), 1);
        assert_eq!(v.def_sites[0].file, "util.rs");
        let caller_syms: std::collections::HashSet<&str> =
            v.callers.iter().map(|c| c.symbol.as_str()).collect();
        assert!(caller_syms.contains("foo#"));
        assert!(caller_syms.contains("bar#"));
    }

    #[test]
    fn range_containment() {
        let outer = Range {
            start_line: 0,
            start_char: 0,
            end_line: 10,
            end_char: 0,
        };
        let inside = Range {
            start_line: 5,
            start_char: 5,
            end_line: 5,
            end_char: 10,
        };
        let outside = Range {
            start_line: 11,
            start_char: 0,
            end_line: 12,
            end_char: 0,
        };
        assert!(range_contains(&outer, &inside));
        assert!(!range_contains(&outer, &outside));
    }
}
