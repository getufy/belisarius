//! `AppContext` — the shared engine the HTTP and MCP transports both wrap.
//!
//! Holds the caches that used to live independently inside `server.rs`
//! (unbounded `HashMap`, sentinel-mtime invalidation) and `cmd_mcp.rs`
//! (bounded LRU, never invalidated). The unified cache here combines both:
//! bounded LRU with mtime invalidation. Path resolution is fleet-aware so a
//! fleet name like `belisarius` resolves to its registered project path on
//! both surfaces — HTTP previously did not do this; the change is intentional
//! per the migration plan.
//!
//! Cap is configurable via `BELISARIUS_CACHE_CAP` (default 16). The cache is
//! linear-scan on insert — fine at this size.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use tokio::sync::Mutex;

use belisarius_search::embed::{default_provider, EmbeddingProvider};
use belisarius_symbols::SymbolStore;

use crate::service::error::ServiceError;

const DEFAULT_CACHE_CAP: usize = 16;
const CACHE_CAP_ENV: &str = "BELISARIUS_CACHE_CAP";

/// One cached analysis report keyed on the canonical project path. The mtime
/// is the newest file mtime observed inside the project at the time we
/// inserted — the entry is reused only while no file beneath the project has
/// changed since.
struct AnalysisEntry {
    sentinel_mtime: SystemTime,
    report: Arc<belisarius_core::AnalysisReport>,
}

struct AnalysisCache {
    map: HashMap<PathBuf, AnalysisEntry>,
    order: VecDeque<PathBuf>,
    cap: usize,
}

impl AnalysisCache {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
            cap: cap.max(1),
        }
    }

    fn get(
        &mut self,
        key: &PathBuf,
        sentinel: SystemTime,
    ) -> Option<Arc<belisarius_core::AnalysisReport>> {
        let entry = self.map.get(key)?;
        if entry.sentinel_mtime != sentinel {
            return None;
        }
        let report = entry.report.clone();
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
        Some(report)
    }

    fn insert(
        &mut self,
        key: PathBuf,
        sentinel: SystemTime,
        report: Arc<belisarius_core::AnalysisReport>,
    ) {
        if self.map.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        } else if self.map.len() >= self.cap {
            if let Some(evict) = self.order.pop_front() {
                self.map.remove(&evict);
            }
        }
        self.map.insert(
            key.clone(),
            AnalysisEntry {
                sentinel_mtime: sentinel,
                report,
            },
        );
        self.order.push_back(key);
    }
}

/// Search-index handles are cheap to keep open and expensive to rebuild —
/// once we've opened one for a project we hang onto it for the life of the
/// process. No invalidation: search reindex is an explicit user action.
struct SearchCache {
    map: HashMap<PathBuf, Arc<belisarius_search::IndexHandle>>,
}

impl SearchCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

/// Per-project SCIP store, keyed on the canonical `.belisarius/scip/merged.scip`
/// path. Invalidated when the SCIP file's mtime changes (so re-indexing is
/// picked up without restarting the server).
struct SymbolsEntry {
    mtime: SystemTime,
    store: Arc<SymbolStore>,
}

struct SymbolsCache {
    map: HashMap<PathBuf, SymbolsEntry>,
}

impl SymbolsCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

/// Shared engine wrapped in `Arc` by both transports.
pub struct AppContext {
    analysis: Mutex<AnalysisCache>,
    search: Mutex<SearchCache>,
    symbols: Mutex<SymbolsCache>,
    /// Lazily-initialized embedding provider. `None` until first use; an
    /// `Err` is also cached so repeated tool calls don't keep retrying a
    /// failing init (e.g. the model download was blocked). Set via
    /// `embedder()` which runs the init on a blocking thread.
    embedder: OnceLock<Result<Arc<dyn EmbeddingProvider>, String>>,
}

impl AppContext {
    pub fn new() -> Self {
        let cap = std::env::var(CACHE_CAP_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_CACHE_CAP);
        Self {
            analysis: Mutex::new(AnalysisCache::new(cap)),
            search: Mutex::new(SearchCache::new()),
            symbols: Mutex::new(SymbolsCache::new()),
            embedder: OnceLock::new(),
        }
    }

    /// Return the embedding provider, initializing it on first call. The
    /// model download happens inside `default_provider` and can take a few
    /// seconds on a cold cache — callers should expect a one-time delay on
    /// the first `belisarius_remember` / `belisarius_recall` after start.
    ///
    /// When the `embed` feature is off or initialization fails, returns
    /// `None`. Callers fall back to BM25-style recall.
    pub fn embedder(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        let r = self.embedder.get_or_init(|| match default_provider() {
            Ok(p) => Ok(Arc::from(p)),
            Err(e) => Err(format!("{e}")),
        });
        r.as_ref().ok().cloned()
    }

    /// Canonical SCIP path inside a project. Public so `service::symbols` can
    /// report the exact `path` field in its status response.
    pub fn scip_path_for(project: &str) -> PathBuf {
        std::path::Path::new(project)
            .join(".belisarius")
            .join("scip")
            .join("merged.scip")
    }

    /// Resolve a fleet name or path to a project path. Names take precedence
    /// over paths — when both a registered fleet app `frontend` and a directory
    /// `./frontend` exist, the fleet entry wins. Unmatched strings pass
    /// through as-is.
    pub fn resolve_path(&self, target: &str) -> String {
        if let Ok(cfg) = crate::fleet::load(&crate::fleet::default_config_path()) {
            return crate::fleet::resolve_target(&cfg, target);
        }
        target.to_string()
    }

    /// Look up or build the analysis report. Always wrapped in
    /// `spawn_blocking` because `belisarius_scan::analyze` walks the tree and
    /// runs tree-sitter — strictly CPU-bound work that must not block the
    /// tokio runtime.
    pub async fn load_analysis(
        &self,
        project: &str,
    ) -> Result<Arc<belisarius_core::AnalysisReport>, ServiceError> {
        let key: PathBuf =
            std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
        let project_for_sentinel = project.to_string();
        let sentinel =
            tokio::task::spawn_blocking(move || project_sentinel_mtime(&project_for_sentinel))
                .await
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("sentinel join: {e}")))?;

        if let Some(report) = self.analysis.lock().await.get(&key, sentinel) {
            return Ok(report);
        }

        let project_for_analyze = project.to_string();
        let report =
            tokio::task::spawn_blocking(move || belisarius_scan::analyze(&project_for_analyze))
                .await
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("analyze join: {e}")))?
                .map_err(|e| ServiceError::Internal(anyhow::anyhow!("analyze: {e:#}")))?;
        let report = Arc::new(report);
        self.analysis
            .lock()
            .await
            .insert(key, sentinel, report.clone());
        Ok(report)
    }

    /// Look up (or load) the SCIP symbol store for `project`. Returns
    /// `MissingIndex` when no `.belisarius/scip/merged.scip` exists — both
    /// transports get the same `run `belisarius index <project>` first` hint
    /// instead of HTTP and MCP diverging on phrasing.
    pub async fn load_symbols(&self, project: &str) -> Result<Arc<SymbolStore>, ServiceError> {
        let scip = Self::scip_path_for(project);
        let mtime = tokio::task::spawn_blocking({
            let scip = scip.clone();
            move || std::fs::metadata(&scip).and_then(|m| m.modified())
        })
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("symbols stat join: {e}")))?
        .map_err(|_| {
            ServiceError::missing_index(
                "symbol",
                format!(
                    "no SCIP symbol index at {scip_path}. \
                     Build it with `belisarius index {project}` \
                     (this is different from the hybrid search index built by \
                     `belisarius search index` — the SCIP one needs per-language \
                     tools: `rust-analyzer`, `scip-typescript`, `scip-python`, \
                     `scip-go`). Only the Symbols / Impact / Flow tabs need it; \
                     Quality / Hotspots / Test gaps / Search / Architecture all \
                     work without it.",
                    scip_path = scip.display(),
                    project = project,
                ),
            )
        })?;

        let key: PathBuf = scip.canonicalize().unwrap_or_else(|_| scip.clone());

        {
            let cache = self.symbols.lock().await;
            if let Some(entry) = cache.map.get(&key) {
                if entry.mtime == mtime {
                    return Ok(entry.store.clone());
                }
            }
        }

        let scip_for_load = scip.clone();
        let store = tokio::task::spawn_blocking(move || SymbolStore::from_path(&scip_for_load))
            .await
            .map_err(|e| ServiceError::Internal(anyhow::anyhow!("symbols load join: {e}")))?
            .map_err(|e| ServiceError::Internal(anyhow::anyhow!("loading scip: {e:#}")))?;
        let store = Arc::new(store);
        self.symbols.lock().await.map.insert(
            key,
            SymbolsEntry {
                mtime,
                store: store.clone(),
            },
        );
        Ok(store)
    }

    /// Open (or reuse) a search index handle for `project`.
    ///
    /// `IndexHandle::open` is intentionally permissive — it creates the index
    /// directory if it doesn't exist and returns an idle handle whose
    /// `status_snapshot()` reports `state: "idle", chunk_count: 0`. We mirror
    /// that: a fresh project that's never been indexed still gets a working
    /// handle. Callers see "unindexed" through the status, not through an
    /// error.
    pub async fn open_search(
        &self,
        project: &str,
    ) -> Result<Arc<belisarius_search::IndexHandle>, ServiceError> {
        let key: PathBuf =
            std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
        if let Some(h) = self.search.lock().await.map.get(&key) {
            return Ok(h.clone());
        }
        let key_for_open = key.clone();
        let handle = tokio::task::spawn_blocking(move || {
            belisarius_search::IndexHandle::open(&key_for_open)
        })
        .await
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("search join: {e}")))?
        .map_err(|e| ServiceError::Internal(anyhow::anyhow!("open search index: {e:#}")))?;
        self.search.lock().await.map.insert(key, handle.clone());
        Ok(handle)
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Newest file mtime under the project — used as the cache sentinel.
/// Identical to the helper that used to live in `server.rs` so cached
/// reports invalidate the moment any source file changes.
///
/// `pub(crate)` because `service::diagnostics` also uses it to gate the
/// on-disk diagnostics cache (re-run if any source file changed since the
/// cache was written).
pub(crate) fn project_sentinel_mtime(project: &str) -> SystemTime {
    use ignore::WalkBuilder;
    let mut newest = SystemTime::UNIX_EPOCH;
    for entry in WalkBuilder::new(project)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .build()
        .flatten()
    {
        if !entry.path().is_file() {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(m) = meta.modified() {
                if m > newest {
                    newest = m;
                }
            }
        }
    }
    newest
}
