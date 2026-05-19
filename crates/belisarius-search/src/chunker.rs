//! AST-driven chunker. Reuses `belisarius_scan::ast` for the six languages
//! Belisarius understands; falls back to a 60-line sliding window for
//! everything else.

use belisarius_core::FunctionInfo;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    /// AST-extracted function / method.
    Function,
    /// Line-window fallback for unsupported languages or parse failures.
    Window,
    /// Context artifact body chunk.
    Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub file: String,
    pub lang: String,
    pub kind: ChunkKind,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
}

/// Chunk a single file. Returns AST chunks for supported languages with
/// extractable functions, otherwise line-window chunks.
pub fn chunk_file(language: &str, rel_path: &str, source: &str) -> Vec<Chunk> {
    let fns = belisarius_scan::ast::extract_functions(language, rel_path, source).ok();
    if let Some(fns) = fns {
        if !fns.is_empty() {
            return ast_chunks(language, rel_path, source, &fns);
        }
    }
    window_chunks(language, rel_path, source)
}

/// Same, but reads the source from disk under `project_root`.
pub fn chunk_path(language: &str, project_root: &Path, rel_path: &str) -> Vec<Chunk> {
    let full = project_root.join(rel_path);
    let Ok(source) = std::fs::read_to_string(&full) else {
        return Vec::new();
    };
    chunk_file(language, rel_path, &source)
}

fn ast_chunks(language: &str, rel_path: &str, source: &str, fns: &[FunctionInfo]) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::with_capacity(fns.len());
    for f in fns {
        // FunctionInfo lines are 1-indexed inclusive.
        let s = f.start_line.saturating_sub(1) as usize;
        let e = (f.end_line as usize).min(lines.len());
        if e <= s {
            continue;
        }
        let body = lines[s..e].join("\n");
        out.push(Chunk {
            file: rel_path.to_string(),
            lang: language.to_string(),
            kind: ChunkKind::Function,
            name: f.name.clone(),
            start_line: f.start_line,
            end_line: f.end_line,
            content: body,
        });
    }
    out
}

const WINDOW: usize = 60;
const OVERLAP: usize = 10;

fn window_chunks(language: &str, rel_path: &str, source: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let end = (i + WINDOW).min(lines.len());
        let body = lines[i..end].join("\n");
        out.push(Chunk {
            file: rel_path.to_string(),
            lang: language.to_string(),
            kind: ChunkKind::Window,
            name: format!("L{}-{}", i + 1, end),
            start_line: (i + 1) as u32,
            end_line: end as u32,
            content: body,
        });
        if end == lines.len() {
            break;
        }
        i = end.saturating_sub(OVERLAP);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_functions_become_chunks() {
        let src = "fn one() -> i32 { 1 }\n\nfn two() -> i32 { 2 }\n";
        let chunks = chunk_file("rust", "x.rs", src);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].kind, ChunkKind::Function);
        assert_eq!(chunks[0].name, "one");
        assert_eq!(chunks[1].name, "two");
    }

    #[test]
    fn unsupported_language_falls_back_to_windows() {
        let src: String = (1..=150).map(|i| format!("line {i}\n")).collect();
        let chunks = chunk_file("toml", "x.toml", &src);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.kind == ChunkKind::Window));
        // 150 lines with 60-window/10-overlap → 3 windows: 1-60, 51-110, 101-150
        assert!(chunks.len() >= 3);
    }
}
