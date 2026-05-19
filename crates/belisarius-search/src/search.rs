//! Reciprocal Rank Fusion of the dense + BM25 legs.

use crate::index::IndexHandle;
use crate::store::{ChunkStore, VectorReader};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub chunk_id: i64,
    pub file: String,
    pub lang: String,
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub snippet: String,
    pub score: f32,
    pub bm25_rank: Option<usize>,
    pub dense_rank: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub limit: usize,
    pub lang: Option<String>,
    pub kind: Option<String>,
    /// Number of candidates pulled from each leg before RRF.
    pub candidates: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            lang: None,
            kind: None,
            candidates: 50,
        }
    }
}

const RRF_K: f32 = 60.0;

pub fn search(handle: &IndexHandle, query: &str, opts: &SearchOptions) -> Result<Vec<SearchHit>> {
    // BM25 leg.
    let bm25_hits = handle
        .bm25
        .search(query, opts.candidates)
        .unwrap_or_default();
    let mut bm25_ranks: HashMap<i64, usize> = HashMap::new();
    for (i, (id, _)) in bm25_hits.iter().enumerate() {
        bm25_ranks.insert(*id, i);
    }

    // Dense leg (best-effort — silent if provider missing or vectors absent).
    let dense_ranks = dense_leg(handle, query, opts).unwrap_or_default();

    // Union of candidate ids, RRF score.
    let mut ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    ids.extend(bm25_ranks.keys().copied());
    ids.extend(dense_ranks.keys().copied());

    let mut scored: Vec<(i64, f32, Option<usize>, Option<usize>)> = ids
        .into_iter()
        .map(|id| {
            let b = bm25_ranks.get(&id).copied();
            let d = dense_ranks.get(&id).copied();
            let s = rrf_score(b) + rrf_score(d);
            (id, s, b, d)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Pull chunk rows + apply filters.
    let store = handle.store.lock().expect("store mutex");
    let mut out = Vec::with_capacity(opts.limit);
    for (id, score, b_rank, d_rank) in scored {
        if out.len() >= opts.limit {
            break;
        }
        let Some(row) = store.get_chunk(id)? else {
            continue;
        };
        if let Some(l) = &opts.lang {
            if row.lang != *l {
                continue;
            }
        }
        if let Some(k) = &opts.kind {
            let kstr = match row.kind {
                crate::chunker::ChunkKind::Function => "function",
                crate::chunker::ChunkKind::Window => "window",
                crate::chunker::ChunkKind::Artifact => "artifact",
            };
            if kstr != *k {
                continue;
            }
        }
        let snippet = snippet_from(&row.content, 12);
        out.push(SearchHit {
            chunk_id: row.id,
            file: row.file,
            lang: row.lang,
            kind: match row.kind {
                crate::chunker::ChunkKind::Function => "function".into(),
                crate::chunker::ChunkKind::Window => "window".into(),
                crate::chunker::ChunkKind::Artifact => "artifact".into(),
            },
            name: row.name,
            start_line: row.start_line,
            end_line: row.end_line,
            snippet,
            score,
            bm25_rank: b_rank,
            dense_rank: d_rank,
        });
    }
    Ok(out)
}

fn rrf_score(rank: Option<usize>) -> f32 {
    match rank {
        None => 0.0,
        Some(r) => 1.0 / (RRF_K + r as f32),
    }
}

fn dense_leg(
    handle: &IndexHandle,
    query: &str,
    opts: &SearchOptions,
) -> Result<HashMap<i64, usize>> {
    // Lazy-load the embedding provider on first search; otherwise a fresh
    // `belisarius search query` call would silently degrade to BM25-only.
    // We only attempt this if vectors exist on disk — no point spinning up
    // ONNX runtime to compare a query against zero rows.
    let store = ChunkStore::open(&handle.project_root)?;
    let Some(reader) = VectorReader::open(&store.vectors_path())? else {
        return Ok(HashMap::new());
    };
    let provider = {
        let mut g = handle.provider.lock().expect("provider mutex");
        if g.is_none() {
            match crate::embed::default_provider() {
                Ok(p) => *g = Some(Arc::from(p)),
                Err(crate::embed::EmbeddingError::Disabled) => return Ok(HashMap::new()),
                Err(e) => {
                    tracing::warn!(target: "belisarius_search", "dense leg disabled: {e}");
                    return Ok(HashMap::new());
                }
            }
        }
        g.clone()
    };
    let Some(provider) = provider else {
        return Ok(HashMap::new());
    };

    let qvec = provider
        .embed(&[query.to_string()])
        .map_err(|e| anyhow::anyhow!("embed query: {e}"))?;
    let Some(qv) = qvec.into_iter().next() else {
        return Ok(HashMap::new());
    };
    let qn = normalize(&qv);

    // Brute-force cosine over all rows. Vectors are already L2-normalized by
    // bge-small, but we re-normalize the query and any read row to be safe.
    let rows = reader.rows();
    let mut scored: Vec<(i64, f32)> = Vec::with_capacity(rows);
    for id in 0..(rows as i64) {
        let Some(v) = reader.read(id) else { continue };
        let n = normalize(&v);
        let mut dot = 0.0f32;
        for i in 0..n.len().min(qn.len()) {
            dot += n[i] * qn[i];
        }
        scored.push((id, dot));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = HashMap::new();
    for (i, (id, _)) in scored.into_iter().take(opts.candidates).enumerate() {
        out.insert(id, i);
    }
    Ok(out)
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag <= 1e-9 {
        return v.to_vec();
    }
    v.iter().map(|x| x / mag).collect()
}

fn snippet_from(content: &str, max_lines: usize) -> String {
    let mut out = String::new();
    for (i, line) in content.lines().enumerate() {
        if i >= max_lines {
            out.push_str("\n…");
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_math() {
        // Doc appearing at rank 0 in both legs > doc at rank 0 in only one leg.
        let both = rrf_score(Some(0)) + rrf_score(Some(0));
        let one = rrf_score(Some(0)) + rrf_score(None);
        assert!(both > one);
    }
}
