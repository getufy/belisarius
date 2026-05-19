//! Context artifacts registry.
//!
//! Users author a `.belisarius/context_artifacts.json` describing non-code
//! knowledge (schemas, runbooks, API specs) that should be discoverable
//! alongside code. Each artifact has paths (file or directory globs) and a
//! description that doubles as a behavioral hint for the agent.
//!
//! Artifacts are indexed into the same search index as code, distinguished by
//! `kind="artifact"` in `chunks.sqlite`.

use anyhow::{Context, Result};
use belisarius_search::{
    chunker::{Chunk, ChunkKind},
    index::IndexHandle,
    search::{search, SearchOptions},
    store::{hash_content, VectorWriter},
};
use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub description: String,
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ContextRegistry {
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

impl ContextRegistry {
    pub fn registry_path(project_root: &Path) -> PathBuf {
        project_root
            .join(".belisarius")
            .join("context_artifacts.json")
    }

    pub fn load(project_root: &Path) -> Result<Self> {
        let p = Self::registry_path(project_root);
        if !p.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read(&p).with_context(|| format!("read {}", p.display()))?;
        // Accept either {"artifacts":[...]} or a bare array for ergonomics.
        if let Ok(reg) = serde_json::from_slice::<ContextRegistry>(&raw) {
            return Ok(reg);
        }
        let arts: Vec<Artifact> =
            serde_json::from_slice(&raw).with_context(|| format!("parse {}", p.display()))?;
        Ok(Self { artifacts: arts })
    }

    pub fn find(&self, name: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.name == name)
    }

    /// Resolve an artifact's `paths` (glob-style) to concrete files under the
    /// project root and return their concatenated text plus per-file metadata.
    pub fn read_artifact(&self, project_root: &Path, name: &str) -> Result<ArtifactContent> {
        let art = self
            .find(name)
            .ok_or_else(|| anyhow::anyhow!("no artifact named {name}"))?;
        let mut gb = GlobSetBuilder::new();
        for p in &art.paths {
            gb.add(Glob::new(p).with_context(|| format!("bad glob {p}"))?);
        }
        let gs = gb.build()?;
        let mut files: Vec<ResolvedFile> = Vec::new();
        for entry in ignore::WalkBuilder::new(project_root)
            .standard_filters(true)
            .build()
            .filter_map(|r| r.ok())
        {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(project_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .into_owned();
            if gs.is_match(&rel) {
                let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                files.push(ResolvedFile { path: rel, content });
            }
        }
        Ok(ArtifactContent {
            artifact: art.clone(),
            files,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactContent {
    pub artifact: Artifact,
    pub files: Vec<ResolvedFile>,
}

/// Index every artifact's resolved files into the project's search index as
/// `ChunkKind::Artifact` chunks. Returns the number of chunks inserted.
pub fn index_registry(handle: &Arc<IndexHandle>) -> Result<usize> {
    let registry = ContextRegistry::load(&handle.project_root)?;
    if registry.artifacts.is_empty() {
        return Ok(0);
    }
    let mut total = 0usize;

    // We deliberately bypass the BM25 leg for v1 artifact indexing to keep
    // this crate dependency-light; the dense leg is sufficient for the
    // typical "find the right schema" query, and the BM25 leg already
    // catches keyword matches via the code path when artifacts are
    // mentioned in source.
    let vectors_path = {
        let s = handle.store.lock().expect("store mutex");
        s.vectors_path()
    };
    let mut vw = VectorWriter::open(&vectors_path)?;

    let provider = handle.provider.lock().expect("provider mutex").clone();

    for art in &registry.artifacts {
        let resolved = ContextRegistry {
            artifacts: vec![art.clone()],
        }
        .read_artifact(&handle.project_root, &art.name)?;
        for file in resolved.files {
            let hash = hash_content(file.content.as_bytes());
            let chunks = vec![Chunk {
                file: file.path.clone(),
                lang: "context".into(),
                kind: ChunkKind::Artifact,
                name: art.name.clone(),
                start_line: 1,
                end_line: file.content.lines().count().max(1) as u32,
                content: format!(
                    "[{name}] {desc}\n{when}\n---\n{body}",
                    name = art.name,
                    desc = art.description,
                    when = art.when.clone().unwrap_or_default(),
                    body = file.content
                ),
            }];
            let ids = {
                let mut s = handle.store.lock().expect("store mutex");
                s.upsert_file(&file.path, &hash, &chunks)?
            };
            if let Some(p) = &provider {
                let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
                if let Ok(vecs) = p.embed(&texts) {
                    for (id, v) in ids.iter().zip(vecs) {
                        vw.write_vector(*id, &v)?;
                        let mut s = handle.store.lock().expect("store mutex");
                        s.mark_has_vector(*id)?;
                    }
                }
            }
            total += chunks.len();
        }
    }
    Ok(total)
}

/// Semantic search restricted to context artifacts.
pub fn search_artifacts(
    handle: &IndexHandle,
    query: &str,
    limit: usize,
) -> Result<Vec<belisarius_search::SearchHit>> {
    let opts = SearchOptions {
        limit,
        lang: None,
        kind: Some("artifact".into()),
        candidates: 50,
    };
    search(handle, query, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_returns_default_when_missing() {
        let dir = tempdir().unwrap();
        let r = ContextRegistry::load(dir.path()).unwrap();
        assert!(r.artifacts.is_empty());
    }

    #[test]
    fn loads_bare_array_form() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".belisarius")).unwrap();
        std::fs::write(
            dir.path().join(".belisarius/context_artifacts.json"),
            r#"[{"name":"x","description":"d","paths":["**/*.md"]}]"#,
        )
        .unwrap();
        let r = ContextRegistry::load(dir.path()).unwrap();
        assert_eq!(r.artifacts.len(), 1);
        assert_eq!(r.artifacts[0].name, "x");
    }

    #[test]
    fn resolves_paths_to_files() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".belisarius")).unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        std::fs::write(
            dir.path().join(".belisarius/context_artifacts.json"),
            r#"[{"name":"readme","description":"the readme","paths":["README.md"]}]"#,
        )
        .unwrap();
        let r = ContextRegistry::load(dir.path()).unwrap();
        let c = r.read_artifact(dir.path(), "readme").unwrap();
        assert_eq!(c.files.len(), 1);
        assert_eq!(c.files[0].content, "hello");
    }
}
