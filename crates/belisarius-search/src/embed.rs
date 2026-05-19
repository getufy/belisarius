//! Embedding provider trait + a fastembed-backed default implementation.
//!
//! The default model is `BAAI/bge-small-en-v1.5` (384-dim, ~33MB ONNX). It is
//! downloaded on first use under `~/.cache/belisarius/models/` and pinned via
//! the fastembed model registry. Override the cache dir with the
//! `BELISARIUS_MODEL_PATH` env var.

use std::path::PathBuf;

pub const EMBEDDING_DIM: usize = 384;

pub fn embedding_dim() -> usize {
    EMBEDDING_DIM
}

#[derive(Debug)]
pub enum EmbeddingError {
    Disabled,
    Init(String),
    Embed(String),
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingError::Disabled => {
                write!(f, "embeddings disabled (build without --features embed)")
            }
            EmbeddingError::Init(s) => write!(f, "embedding init failed: {s}"),
            EmbeddingError::Embed(s) => write!(f, "embedding inference failed: {s}"),
        }
    }
}

impl std::error::Error for EmbeddingError {}

pub trait EmbeddingProvider: Send + Sync {
    /// Compute one f32 vector per input string. Returned vectors are L2-normalized.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
    fn dim(&self) -> usize;
    fn model_name(&self) -> &str;
}

pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BELISARIUS_MODEL_PATH") {
        return PathBuf::from(p);
    }
    if let Some(home) = dirs_home() {
        return home.join(".cache").join("belisarius").join("models");
    }
    PathBuf::from(".belisarius-models")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(feature = "embed")]
mod fastembed_impl {
    use super::*;
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use std::sync::Mutex;

    pub struct FastembedProvider {
        inner: Mutex<TextEmbedding>,
        name: String,
    }

    impl FastembedProvider {
        pub fn new() -> Result<Self, EmbeddingError> {
            let dir = super::cache_dir();
            std::fs::create_dir_all(&dir)
                .map_err(|e| EmbeddingError::Init(format!("creating cache dir: {e}")))?;
            let opts = InitOptions::new(EmbeddingModel::BGESmallENV15)
                .with_cache_dir(dir)
                .with_show_download_progress(true);
            let model =
                TextEmbedding::try_new(opts).map_err(|e| EmbeddingError::Init(e.to_string()))?;
            Ok(Self {
                inner: Mutex::new(model),
                name: "BAAI/bge-small-en-v1.5".into(),
            })
        }
    }

    impl EmbeddingProvider for FastembedProvider {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let docs: Vec<String> = texts.to_vec();
            let g = self.inner.lock().expect("embedding mutex poisoned");
            let out = g
                .embed(docs, None)
                .map_err(|e| EmbeddingError::Embed(e.to_string()))?;
            Ok(out)
        }

        fn dim(&self) -> usize {
            EMBEDDING_DIM
        }

        fn model_name(&self) -> &str {
            &self.name
        }
    }
}

#[cfg(feature = "embed")]
pub use fastembed_impl::FastembedProvider;

/// Build the default embedding provider. Returns `Err` when the `embed` feature
/// is off — callers should fall back to BM25-only search.
pub fn default_provider() -> Result<Box<dyn EmbeddingProvider>, EmbeddingError> {
    #[cfg(feature = "embed")]
    {
        Ok(Box::new(FastembedProvider::new()?))
    }
    #[cfg(not(feature = "embed"))]
    {
        Err(EmbeddingError::Disabled)
    }
}
