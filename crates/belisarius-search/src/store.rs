//! Chunk + vector persistence.
//!
//! - `chunks.sqlite` holds chunk rows with a hash on each source file. WAL mode
//!   so the HTTP server can read while the indexer writes.
//! - `vectors.f16` is an append-only flat array of f16 vectors, indexed by
//!   `chunk_id` (the SQLite rowid). Row offset = `chunk_id * dim * 2 bytes`.
//!   This is mmap'd read-side for fast brute-force cosine.

use crate::chunker::{Chunk, ChunkKind};
use crate::embed::EMBEDDING_DIM;
use anyhow::{Context, Result};
use half::f16;
use memmap2::Mmap;
use rusqlite::{params, Connection};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: i64,
    pub file: String,
    pub lang: String,
    pub kind: ChunkKind,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub has_vector: bool,
}

pub struct ChunkStore {
    pub root: PathBuf,
    db: Connection,
}

impl ChunkStore {
    pub fn open(project_root: &Path) -> Result<Self> {
        let root = project_root.join(".belisarius").join("search");
        std::fs::create_dir_all(&root).context("create .belisarius/search")?;
        let db_path = root.join("chunks.sqlite");
        let db =
            Connection::open(&db_path).with_context(|| format!("open {}", db_path.display()))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                hash TEXT NOT NULL,
                indexed_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file TEXT NOT NULL,
                lang TEXT NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                content TEXT NOT NULL,
                has_vector INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file);
             CREATE INDEX IF NOT EXISTS idx_chunks_lang ON chunks(lang);
             CREATE INDEX IF NOT EXISTS idx_chunks_kind ON chunks(kind);
             CREATE TABLE IF NOT EXISTS meta (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL
             );",
        )?;
        Ok(Self { root, db })
    }

    pub fn vectors_path(&self) -> PathBuf {
        self.root.join("vectors.f16")
    }

    pub fn bm25_dir(&self) -> PathBuf {
        self.root.join("bm25")
    }

    pub fn status_path(&self) -> PathBuf {
        self.root.join("status.json")
    }

    pub fn chunk_count(&self) -> Result<usize> {
        let n: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn file_hash(&self, path: &str) -> Result<Option<String>> {
        let row: Option<String> = self
            .db
            .query_row(
                "SELECT hash FROM files WHERE path = ?1",
                params![path],
                |r| r.get(0),
            )
            .ok();
        Ok(row)
    }

    /// Insert chunks for a file in one transaction. Returns the assigned chunk ids
    /// (in input order).
    pub fn upsert_file(&mut self, file: &str, hash: &str, chunks: &[Chunk]) -> Result<Vec<i64>> {
        let tx = self.db.transaction()?;
        // Remove old rows for this file (also frees their vector rows in vectors.f16
        // — we use a tombstone strategy on the mmap side, see vector_writer).
        tx.execute("DELETE FROM chunks WHERE file = ?1", params![file])?;
        let mut ids = Vec::with_capacity(chunks.len());
        for c in chunks {
            tx.execute(
                "INSERT INTO chunks(file,lang,kind,name,start_line,end_line,content,has_vector)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,0)",
                params![
                    c.file,
                    c.lang,
                    kind_to_str(c.kind),
                    c.name,
                    c.start_line as i64,
                    c.end_line as i64,
                    c.content,
                ],
            )?;
            ids.push(tx.last_insert_rowid());
        }
        tx.execute(
            "INSERT INTO files(path,hash,indexed_at) VALUES(?1,?2,strftime('%s','now'))
             ON CONFLICT(path) DO UPDATE SET hash=excluded.hash, indexed_at=excluded.indexed_at",
            params![file, hash],
        )?;
        tx.commit()?;
        Ok(ids)
    }

    pub fn remove_file(&mut self, file: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM chunks WHERE file = ?1", params![file])?;
        tx.execute("DELETE FROM files WHERE path = ?1", params![file])?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_has_vector(&mut self, id: i64) -> Result<()> {
        self.db.execute(
            "UPDATE chunks SET has_vector = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_chunk(&self, id: i64) -> Result<Option<ChunkRow>> {
        let row = self
            .db
            .query_row(
                "SELECT id,file,lang,kind,name,start_line,end_line,content,has_vector
                 FROM chunks WHERE id = ?1",
                params![id],
                |r| {
                    Ok(ChunkRow {
                        id: r.get(0)?,
                        file: r.get(1)?,
                        lang: r.get(2)?,
                        kind: str_to_kind(&r.get::<_, String>(3)?),
                        name: r.get(4)?,
                        start_line: r.get::<_, i64>(5)? as u32,
                        end_line: r.get::<_, i64>(6)? as u32,
                        content: r.get(7)?,
                        has_vector: r.get::<_, i64>(8)? != 0,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    pub fn all_indexed_files(&self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<String>, _>>()?)
    }

    pub fn distinct_languages(&self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT DISTINCT lang FROM chunks")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<String>, _>>()?)
    }

    /// Set a metadata key (e.g. embedding model name).
    pub fn meta_set(&self, k: &str, v: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO meta(k,v) VALUES(?1,?2)
             ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    pub fn meta_get(&self, k: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row("SELECT v FROM meta WHERE k = ?1", params![k], |r| r.get(0))
            .ok())
    }
}

fn kind_to_str(k: ChunkKind) -> &'static str {
    match k {
        ChunkKind::Function => "function",
        ChunkKind::Window => "window",
        ChunkKind::Artifact => "artifact",
    }
}

fn str_to_kind(s: &str) -> ChunkKind {
    match s {
        "function" => ChunkKind::Function,
        "artifact" => ChunkKind::Artifact,
        _ => ChunkKind::Window,
    }
}

/// Append-only writer for vectors.f16. Each row is `EMBEDDING_DIM` f16 values
/// at offset `id * dim * 2`. We pre-extend the file with zeros for new ids and
/// overwrite their slot. Tombstones for deleted chunks stay as garbage rows —
/// reclaimed by a periodic compaction (not implemented in v1).
pub struct VectorWriter {
    file: File,
    dim: usize,
}

impl VectorWriter {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("open vectors file {}", path.display()))?;
        Ok(Self {
            file,
            dim: EMBEDDING_DIM,
        })
    }

    pub fn write_vector(&mut self, id: i64, vec: &[f32]) -> Result<()> {
        anyhow::ensure!(
            vec.len() == self.dim,
            "vector dim mismatch: got {} expected {}",
            vec.len(),
            self.dim
        );
        let offset = (id as u64) * (self.dim as u64) * 2;
        // Extend file if needed.
        let cur_len = self.file.metadata()?.len();
        let need = offset + (self.dim as u64) * 2;
        if cur_len < need {
            self.file.set_len(need)?;
        }
        let mut buf = vec![0u8; self.dim * 2];
        for (i, &x) in vec.iter().enumerate() {
            let h = f16::from_f32(x).to_le_bytes();
            buf[i * 2] = h[0];
            buf[i * 2 + 1] = h[1];
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&buf)?;
        Ok(())
    }
}

/// Read-side view over vectors.f16. mmap'd for fast brute-force cosine.
pub struct VectorReader {
    mmap: Mmap,
    dim: usize,
}

impl VectorReader {
    pub fn open(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let file = File::open(path)?;
        if file.metadata()?.len() == 0 {
            return Ok(None);
        }
        // Safety: file is opened read-only; we don't modify it while mmap is live.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Some(Self {
            mmap,
            dim: EMBEDDING_DIM,
        }))
    }

    /// Decode one row to f32 if it falls within the mapped region.
    pub fn read(&self, id: i64) -> Option<Vec<f32>> {
        let offset = (id as usize) * self.dim * 2;
        if offset + self.dim * 2 > self.mmap.len() {
            return None;
        }
        let slice = &self.mmap[offset..offset + self.dim * 2];
        let mut out = Vec::with_capacity(self.dim);
        for i in 0..self.dim {
            let bytes = [slice[i * 2], slice[i * 2 + 1]];
            out.push(f16::from_le_bytes(bytes).to_f32());
        }
        Some(out)
    }

    pub fn rows(&self) -> usize {
        self.mmap.len() / (self.dim * 2)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }
}

/// File content hash; cheap blake3 truncation.
pub fn hash_content(bytes: &[u8]) -> String {
    let h = blake3::hash(bytes);
    h.to_hex().as_str()[..32].to_string()
}

/// Read the source file from disk and return (hash, source).
pub fn read_and_hash(path: &Path) -> Result<(String, String)> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let hash = hash_content(&buf);
    let source = String::from_utf8_lossy(&buf).into_owned();
    Ok((hash, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_and_upsert() {
        let dir = tempdir().unwrap();
        let mut s = ChunkStore::open(dir.path()).unwrap();
        let chunks = vec![Chunk {
            file: "a.rs".into(),
            lang: "rust".into(),
            kind: ChunkKind::Function,
            name: "foo".into(),
            start_line: 1,
            end_line: 3,
            content: "fn foo() {}".into(),
        }];
        let ids = s.upsert_file("a.rs", "h1", &chunks).unwrap();
        assert_eq!(ids.len(), 1);
        let row = s.get_chunk(ids[0]).unwrap().unwrap();
        assert_eq!(row.name, "foo");
        assert!(!row.has_vector);
        s.mark_has_vector(ids[0]).unwrap();
        let row2 = s.get_chunk(ids[0]).unwrap().unwrap();
        assert!(row2.has_vector);
    }

    #[test]
    fn vector_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("v.f16");
        let mut w = VectorWriter::open(&path).unwrap();
        let v: Vec<f32> = (0..EMBEDDING_DIM).map(|i| i as f32 / 1000.0).collect();
        w.write_vector(0, &v).unwrap();
        w.write_vector(3, &v).unwrap();
        drop(w);
        let r = VectorReader::open(&path).unwrap().unwrap();
        let v0 = r.read(0).unwrap();
        // f16 has limited precision; compare with tolerance.
        for (a, b) in v.iter().zip(v0.iter()) {
            assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        }
        assert!(r.read(99).is_none());
    }
}
