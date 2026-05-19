//! Hybrid semantic + BM25 code search for Belisarius.
//!
//! Storage layout (relative to project root):
//!
//! ```text
//! .belisarius/search/
//!   chunks.sqlite       chunks + file hashes (WAL)
//!   vectors.f16         mmap'd flat array of f16 vectors, row i ↔ chunk_id i
//!   bm25/               tantivy index (CodeTokenizer)
//!   status.json         last index run status
//! ```
//!
//! The dense leg uses fastembed (BAAI/bge-small-en-v1.5, 384-dim) behind the
//! `embed` feature; without the feature, search degrades to a BM25-only path
//! (still useful, no model download required).

pub mod bm25;
pub mod chunker;
pub mod embed;
pub mod index;
pub mod search;
pub mod store;
pub mod watcher;

pub use chunker::{chunk_file, Chunk, ChunkKind};
pub use embed::{embedding_dim, EmbeddingError, EmbeddingProvider};
pub use index::{IndexHandle, IndexState, IndexStatus, ReindexOptions};
pub use search::{SearchHit, SearchOptions};
pub use store::{ChunkRow, ChunkStore};
pub use watcher::{watch, WatcherHandle};
