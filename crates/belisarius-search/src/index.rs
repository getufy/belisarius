//! Index orchestrator: walk → diff → chunk → embed → upsert.
//!
//! The handle owns three things: the chunk store (SQLite + vectors.f16), the
//! tantivy BM25 index, and an optional embedding provider. A status file is
//! kept under `.belisarius/search/status.json` so the HTTP server can show
//! progress without grabbing any lock.

use crate::bm25::Bm25Index;
use crate::chunker::{chunk_file, Chunk};
use crate::embed::{default_provider, EmbeddingError, EmbeddingProvider};
use crate::store::{hash_content, ChunkStore, VectorReader, VectorWriter};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Tunables for a reindex run.
#[derive(Debug, Clone, Default)]
pub struct ReindexOptions {
    /// If true, drop existing chunks/vectors and re-embed everything.
    pub full: bool,
    /// If true, skip the dense leg (BM25 only). Useful for fast bring-up.
    pub bm25_only: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    Idle,
    Indexing,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    pub state: IndexState,
    pub processed: usize,
    pub total: usize,
    pub chunk_count: usize,
    pub model: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_error: Option<String>,
    pub bm25_only: bool,
}

impl Default for IndexStatus {
    fn default() -> Self {
        Self {
            state: IndexState::Idle,
            processed: 0,
            total: 0,
            chunk_count: 0,
            model: "none".into(),
            started_at: None,
            finished_at: None,
            last_error: None,
            bm25_only: false,
        }
    }
}

pub struct IndexHandle {
    pub project_root: PathBuf,
    pub store: Mutex<ChunkStore>,
    pub bm25: Bm25Index,
    pub status: Mutex<IndexStatus>,
    pub provider: Mutex<Option<Arc<dyn EmbeddingProvider>>>,
}

impl IndexHandle {
    pub fn open(project_root: &Path) -> Result<Arc<Self>> {
        let store = ChunkStore::open(project_root)?;
        let bm25 = Bm25Index::open_or_create(&store.bm25_dir())?;
        let mut status = load_status(&store.status_path()).unwrap_or_default();
        status.chunk_count = store.chunk_count().unwrap_or(0);
        if let Ok(Some(model)) = store.meta_get("embedding_model") {
            status.model = model;
        }
        Ok(Arc::new(Self {
            project_root: project_root.to_path_buf(),
            store: Mutex::new(store),
            bm25,
            status: Mutex::new(status),
            provider: Mutex::new(None),
        }))
    }

    pub fn status_snapshot(&self) -> IndexStatus {
        self.status.lock().expect("status mutex").clone()
    }

    fn set_status(&self, s: IndexStatus) {
        if let Ok(store) = self.store.lock() {
            let _ = save_status(&store.status_path(), &s);
        }
        *self.status.lock().expect("status mutex") = s;
    }

    /// Run a full or incremental reindex synchronously. Callers wanting
    /// background execution should spawn a tokio blocking task.
    pub fn reindex(self: &Arc<Self>, opts: ReindexOptions) -> Result<()> {
        let start = Instant::now();
        let mut status = self.status_snapshot();
        status.state = IndexState::Indexing;
        status.processed = 0;
        status.bm25_only = opts.bm25_only;
        status.started_at = Some(now_secs());
        status.last_error = None;
        self.set_status(status.clone());

        let result = self.reindex_inner(&opts, &mut status);
        match &result {
            Ok(_) => {
                status.state = IndexState::Idle;
                status.finished_at = Some(now_secs());
            }
            Err(e) => {
                status.state = IndexState::Error;
                status.last_error = Some(format!("{e:#}"));
                status.finished_at = Some(now_secs());
            }
        }
        if let Ok(store) = self.store.lock() {
            status.chunk_count = store.chunk_count().unwrap_or(0);
        }
        self.set_status(status);
        tracing::info!(target: "belisarius_search", "reindex took {:?}", start.elapsed());
        result
    }

    fn reindex_inner(&self, opts: &ReindexOptions, status: &mut IndexStatus) -> Result<()> {
        // Reset on full.
        if opts.full {
            let mut s = self.store.lock().expect("store mutex");
            for f in s.all_indexed_files()? {
                s.remove_file(&f)?;
            }
            // Wipe vectors and bm25.
            let _ = std::fs::remove_file(s.vectors_path());
            let _ = std::fs::remove_dir_all(s.bm25_dir());
            // Recreate bm25 dir (the Bm25Index handle stays valid until next open).
            std::fs::create_dir_all(s.bm25_dir())?;
        }

        // Walk files using the scan crate's gitignore-aware walker.
        let scan = belisarius_scan::scan(&self.project_root).context("scan() for search-index")?;
        let total = scan.files.len();
        status.total = total;
        self.set_status(status.clone());

        let provider = if opts.bm25_only {
            None
        } else {
            // Lazily build the provider; cache it on the handle so subsequent
            // reindexes don't pay the init cost again.
            let mut slot = self.provider.lock().expect("provider mutex");
            if slot.is_none() {
                match default_provider() {
                    Ok(p) => *slot = Some(Arc::from(p)),
                    Err(EmbeddingError::Disabled) => {
                        tracing::warn!(target: "belisarius_search", "embed feature disabled, BM25 only");
                    }
                    Err(e) => return Err(anyhow::anyhow!("provider init: {e}")),
                }
            }
            slot.clone()
        };

        // Model name persisted as meta for status reporting.
        if let Some(p) = &provider {
            if let Ok(s) = self.store.lock() {
                let _ = s.meta_set("embedding_model", p.model_name());
            }
            status.model = provider.as_ref().unwrap().model_name().to_string();
        } else {
            status.model = "bm25-only".into();
        }

        let mut writer = self.bm25.writer(50_000_000)?;
        let vectors_path = {
            let s = self.store.lock().expect("store mutex");
            s.vectors_path()
        };
        let mut vec_writer = VectorWriter::open(&vectors_path)?;

        let mut batch: Vec<(i64, Chunk)> = Vec::new();
        const BATCH: usize = 16;

        for (i, f) in scan.files.iter().enumerate() {
            status.processed = i;
            self.set_status(status.clone());

            let abs = self.project_root.join(&f.path);
            let raw = match std::fs::read(&abs) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let hash = hash_content(&raw);
            let existing = {
                let s = self.store.lock().expect("store mutex");
                s.file_hash(&f.path).unwrap_or(None)
            };
            if !opts.full && existing.as_deref() == Some(hash.as_str()) {
                continue;
            }
            let source = String::from_utf8_lossy(&raw);
            let chunks = chunk_file(&f.language, &f.path, &source);
            if chunks.is_empty() {
                continue;
            }

            // Persist chunks.
            let ids = {
                let mut s = self.store.lock().expect("store mutex");
                s.upsert_file(&f.path, &hash, &chunks)?
            };
            // BM25: delete-by-file then add fresh.
            self.bm25.delete_for_file(&mut writer, &f.path)?;
            for (id, c) in ids.iter().zip(chunks) {
                self.bm25.add(&mut writer, *id, &c)?;
                batch.push((*id, c));
            }

            if batch.len() >= BATCH {
                self.flush_dense(&mut vec_writer, provider.as_deref(), &mut batch)?;
                // tantivy's `IndexWriter::commit` flushes the segment but
                // the writer stays valid for more docs — no need to drop
                // and reopen. Re-opening here used to race the lock: the
                // RHS `writer(...)` was evaluated before the old writer's
                // `Drop` ran, so the new acquire saw the old lock and
                // bailed with `LockBusy`.
                self.bm25.commit(&mut writer)?;
            }
        }

        if !batch.is_empty() {
            self.flush_dense(&mut vec_writer, provider.as_deref(), &mut batch)?;
        }
        self.bm25.commit(&mut writer)?;
        status.processed = total;
        Ok(())
    }

    fn flush_dense(
        &self,
        vec_writer: &mut VectorWriter,
        provider: Option<&dyn EmbeddingProvider>,
        batch: &mut Vec<(i64, Chunk)>,
    ) -> Result<()> {
        if let Some(p) = provider {
            let texts: Vec<String> = batch.iter().map(|(_, c)| embed_text(c)).collect();
            let vecs = p
                .embed(&texts)
                .map_err(|e| anyhow::anyhow!("embed batch: {e}"))?;
            for ((id, _), v) in batch.iter().zip(vecs) {
                vec_writer.write_vector(*id, &v)?;
                let mut s = self.store.lock().expect("store mutex");
                s.mark_has_vector(*id)?;
            }
        }
        batch.clear();
        Ok(())
    }
}

/// Text fed to the embedder. We prepend the file+name so the dense leg gets a
/// little structural context for free.
fn embed_text(c: &Chunk) -> String {
    format!("{}::{}\n{}", c.file, c.name, c.content)
}

fn load_status(path: &Path) -> Option<IndexStatus> {
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn save_status(path: &Path, s: &IndexStatus) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let body = serde_json::to_vec_pretty(s)?;
    std::fs::write(path, body)?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Helper: open a read-side view of stored vectors.
pub fn open_vector_reader(project_root: &Path) -> Result<Option<VectorReader>> {
    let store = ChunkStore::open(project_root)?;
    VectorReader::open(&store.vectors_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::{Chunk, ChunkKind};

    // ── pure helpers ─────────────────────────────────────────────────────

    #[test]
    fn default_status_is_idle() {
        let s = IndexStatus::default();
        assert_eq!(s.state, IndexState::Idle);
        assert_eq!(s.processed, 0);
        assert_eq!(s.total, 0);
        assert_eq!(s.chunk_count, 0);
        assert!(s.started_at.is_none());
        assert!(s.finished_at.is_none());
        assert!(s.last_error.is_none());
        assert!(!s.bm25_only);
    }

    #[test]
    fn now_secs_is_positive_and_monotonic() {
        let a = now_secs();
        assert!(a > 0, "should be a positive Unix timestamp");
        let b = now_secs();
        assert!(b >= a, "consecutive calls must not go backward");
    }

    #[test]
    fn embed_text_prepends_file_and_name() {
        let c = Chunk {
            file: "src/foo.rs".into(),
            lang: "rust".into(),
            kind: ChunkKind::Function,
            name: "bar".into(),
            start_line: 1,
            end_line: 5,
            content: "fn bar() {}".into(),
        };
        let s = embed_text(&c);
        assert!(s.starts_with("src/foo.rs::bar\n"));
        assert!(s.contains("fn bar() {}"));
    }

    // ── status persistence ───────────────────────────────────────────────

    #[test]
    fn save_then_load_status_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("status.json");
        // save_status creates parents on its own; the test exercises that.
        let s = IndexStatus {
            state: IndexState::Indexing,
            processed: 7,
            total: 42,
            model: "bge-small".into(),
            bm25_only: true,
            ..Default::default()
        };
        save_status(&path, &s).unwrap();
        let loaded = load_status(&path).expect("file should load");
        assert_eq!(loaded.state, IndexState::Indexing);
        assert_eq!(loaded.processed, 7);
        assert_eq!(loaded.total, 42);
        assert_eq!(loaded.model, "bge-small");
        assert!(loaded.bm25_only);
    }

    #[test]
    fn load_status_missing_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_status(&tmp.path().join("no_such_file.json")).is_none());
    }

    #[test]
    fn load_status_malformed_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.json");
        std::fs::write(&path, "this is not json").unwrap();
        assert!(load_status(&path).is_none());
    }

    // ── handle bring-up ──────────────────────────────────────────────────

    #[test]
    fn open_on_empty_project_yields_idle_status() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = IndexHandle::open(tmp.path()).expect("open should succeed");
        let status = handle.status_snapshot();
        assert_eq!(status.state, IndexState::Idle);
        assert_eq!(status.chunk_count, 0);
    }

    #[test]
    fn status_snapshot_reflects_set_status() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = IndexHandle::open(tmp.path()).expect("open");
        let s = IndexStatus {
            state: IndexState::Error,
            last_error: Some("boom".into()),
            ..Default::default()
        };
        handle.set_status(s);
        let got = handle.status_snapshot();
        assert_eq!(got.state, IndexState::Error);
        assert_eq!(got.last_error.as_deref(), Some("boom"));
    }

    /// `set_status` writes through to disk so the next `open` on the same
    /// project sees the previous run's terminal state. Important when the
    /// MCP server restarts mid-reindex — agents must not see "idle" when
    /// the last attempt errored.
    #[test]
    fn set_status_persists_across_open() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let h = IndexHandle::open(tmp.path()).unwrap();
            let s = IndexStatus {
                state: IndexState::Error,
                last_error: Some("disk full".into()),
                ..Default::default()
            };
            h.set_status(s);
        }
        // Drop the first handle; reopen.
        let h2 = IndexHandle::open(tmp.path()).unwrap();
        let got = h2.status_snapshot();
        assert_eq!(got.state, IndexState::Error);
        assert_eq!(got.last_error.as_deref(), Some("disk full"));
    }

    // ── vector-reader helper ─────────────────────────────────────────────

    #[test]
    fn open_vector_reader_on_unindexed_project_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let reader = open_vector_reader(tmp.path()).expect("should not error");
        assert!(
            reader.is_none(),
            "no vectors yet → reader must be None, not an empty handle"
        );
    }
}
