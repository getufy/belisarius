//! Per-project SQLite store at `.belisarius/state.db`. Backs two persistent
//! reasoning surfaces:
//!
//! - **Snapshots**: capture quality axes + hot-function fingerprints at a
//!   point in time. Drift compares the latest snapshot to an earlier one.
//! - **Pins**: persistent annotations agents leave for themselves ("I
//!   already investigated this; conclusion was X") so they don't relitigate
//!   findings each session.
//!
//! The DB is opt-in (only written when an agent explicitly calls
//! `belisarius_snapshot` or `belisarius_pin`) and lives under the
//! project's `.belisarius/` directory — gitignore it like other Belisarius
//! state.

use anyhow::{Context, Result};
use belisarius_core::{AnalysisReport, QualityAxes};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

const SCHEMA_VERSION: i32 = 4;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
    id INTEGER PRIMARY KEY,
    captured_at TEXT NOT NULL,
    score REAL,
    axis_complexity REAL,
    axis_acyclicity REAL,
    axis_dead_code REAL,
    axis_coupling REAL,
    cycles_count INTEGER NOT NULL,
    max_depth INTEGER NOT NULL,
    function_count INTEGER NOT NULL,
    file_count INTEGER NOT NULL,
    hot_functions_json TEXT NOT NULL  -- JSON array of {file,name,cc,cog}
);

CREATE TABLE IF NOT EXISTS pins (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    file TEXT,
    line INTEGER,
    note TEXT NOT NULL,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

-- v2: knowledge layer. `notes` is a typed, scoped, embeddable evolution of
-- the `pins` table. Pins remain as a `kind='context'` view for back-compat.
CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('decision','gotcha','todo','context','hypothesis')),
    scope TEXT NOT NULL,                -- project | file | function
    file TEXT,
    line INTEGER,
    symbol TEXT,
    content TEXT NOT NULL,
    agent_id TEXT,
    session_id TEXT,
    embedding BLOB,                     -- 384-dim f32, nullable when feature off
    refs_json TEXT,                     -- JSON array of related note ids
    created_at TEXT NOT NULL,
    ttl_days INTEGER
);

CREATE TABLE IF NOT EXISTS note_edges (
    from_id INTEGER NOT NULL,
    to_id INTEGER NOT NULL,
    kind TEXT NOT NULL,                 -- supports | contradicts | supersedes
    PRIMARY KEY (from_id, to_id, kind)
);

-- v3: per-file content hashes. The watch daemon uses this to skip
-- redundant reindex passes when `notify` fires on a file whose content
-- didn't actually change (touch / chmod / metadata events).
CREATE TABLE IF NOT EXISTS file_hashes (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    modified_at TEXT NOT NULL
);

-- v4: agent sessions. A session bundles all `belisarius_remember` calls
-- under one `session_id` so a later `belisarius_session_end` can summarise
-- what got persisted in that work block.
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    agent_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_snapshots_captured_at ON snapshots(captured_at);
CREATE INDEX IF NOT EXISTS idx_pins_scope ON pins(scope);
CREATE INDEX IF NOT EXISTS idx_notes_kind ON notes(kind);
CREATE INDEX IF NOT EXISTS idx_notes_scope ON notes(scope);
CREATE INDEX IF NOT EXISTS idx_notes_file ON notes(file);
CREATE INDEX IF NOT EXISTS idx_notes_session ON notes(session_id);
CREATE INDEX IF NOT EXISTS idx_note_edges_from ON note_edges(from_id);
CREATE INDEX IF NOT EXISTS idx_note_edges_to ON note_edges(to_id);
"#;

pub fn db_path(project_root: &Path) -> PathBuf {
    project_root.join(".belisarius").join("state.db")
}

pub fn open(project_root: &Path) -> Result<Connection> {
    let path = db_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
    conn.execute_batch(SCHEMA)?;
    // Mirror SCHEMA_VERSION into PRAGMA user_version so external tools
    // (sqlite3 CLI, `belisarius doctor`) can read the version without
    // knowing the `meta` table layout.
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('schema_version', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![SCHEMA_VERSION.to_string()],
    )?;
    Ok(conn)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotFunctionFingerprint {
    pub file: String,
    pub name: String,
    pub cyclomatic: u32,
    pub cognitive: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotRow {
    pub id: i64,
    pub captured_at: String,
    pub score: Option<f32>,
    pub axes: QualityAxes,
    pub cycles_count: u32,
    pub max_depth: u32,
    pub function_count: u32,
    pub file_count: u32,
    pub hot_functions: Vec<HotFunctionFingerprint>,
}

pub fn write_snapshot(conn: &Connection, report: &AnalysisReport) -> Result<i64> {
    let mut fns: Vec<&belisarius_core::FunctionInfo> = report
        .functions
        .iter()
        .filter(|f| f.cyclomatic >= 8)
        .collect();
    fns.sort_by(|a, b| b.cyclomatic.cmp(&a.cyclomatic));
    fns.truncate(50);
    let fingerprints: Vec<HotFunctionFingerprint> = fns
        .iter()
        .map(|f| HotFunctionFingerprint {
            file: f.file.clone(),
            name: f.name.clone(),
            cyclomatic: f.cyclomatic,
            cognitive: f.cognitive,
        })
        .collect();
    let json = serde_json::to_string(&fingerprints)?;
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    conn.execute(
        "INSERT INTO snapshots
            (captured_at, score, axis_complexity, axis_acyclicity, axis_dead_code,
             axis_coupling, cycles_count, max_depth, function_count, file_count,
             hot_functions_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            now,
            report.quality.score,
            report.quality.axes.complexity,
            report.quality.axes.acyclicity,
            report.quality.axes.dead_code,
            report.quality.axes.coupling,
            report.cycles.len() as u32,
            report.max_depth,
            report.functions.len() as u32,
            report.scan.files.len() as u32,
            json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn latest_snapshot(conn: &Connection) -> Result<Option<SnapshotRow>> {
    let row = conn
        .query_row(
            "SELECT id, captured_at, score, axis_complexity, axis_acyclicity, axis_dead_code,
                    axis_coupling, cycles_count, max_depth, function_count, file_count,
                    hot_functions_json
             FROM snapshots ORDER BY captured_at DESC LIMIT 1",
            [],
            row_to_snapshot,
        )
        .ok();
    Ok(row)
}

pub fn snapshot_at_or_before(conn: &Connection, ts_iso: &str) -> Result<Option<SnapshotRow>> {
    let row = conn
        .query_row(
            "SELECT id, captured_at, score, axis_complexity, axis_acyclicity, axis_dead_code,
                    axis_coupling, cycles_count, max_depth, function_count, file_count,
                    hot_functions_json
             FROM snapshots WHERE captured_at <= ?1
             ORDER BY captured_at DESC LIMIT 1",
            params![ts_iso],
            row_to_snapshot,
        )
        .ok();
    Ok(row)
}

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotRow> {
    let json: String = row.get(11)?;
    let hot_functions: Vec<HotFunctionFingerprint> =
        serde_json::from_str(&json).unwrap_or_default();
    Ok(SnapshotRow {
        id: row.get(0)?,
        captured_at: row.get(1)?,
        score: row.get(2)?,
        axes: QualityAxes {
            complexity: row.get(3)?,
            acyclicity: row.get(4)?,
            dead_code: row.get(5)?,
            coupling: row.get(6)?,
        },
        cycles_count: row.get::<_, i64>(7)? as u32,
        max_depth: row.get::<_, i64>(8)? as u32,
        function_count: row.get::<_, i64>(9)? as u32,
        file_count: row.get::<_, i64>(10)? as u32,
        hot_functions,
    })
}

// ─── Pins ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Pin {
    pub id: i64,
    pub scope: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub note: String,
    pub expires_at: Option<String>,
    pub created_at: String,
}

pub fn insert_pin(
    conn: &Connection,
    scope: &str,
    file: Option<&str>,
    line: Option<u32>,
    note: &str,
    ttl_days: Option<u32>,
) -> Result<i64> {
    let now = OffsetDateTime::now_utc();
    let now_str = now.format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    let expires = match ttl_days {
        Some(d) => Some(
            (now + time::Duration::days(d as i64))
                .format(&time::format_description::well_known::Iso8601::DEFAULT)?,
        ),
        None => None,
    };
    conn.execute(
        "INSERT INTO pins (scope, file, line, note, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![scope, file, line, note, expires, now_str],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_pin(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM pins WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

pub fn list_pins(conn: &Connection, scope: Option<&str>) -> Result<Vec<Pin>> {
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    let (sql, args): (&str, Vec<rusqlite::types::Value>) = match scope {
        Some(s) => (
            "SELECT id, scope, file, line, note, expires_at, created_at
             FROM pins WHERE scope = ?1 AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY created_at DESC",
            vec![s.to_string().into(), now.into()],
        ),
        None => (
            "SELECT id, scope, file, line, note, expires_at, created_at
             FROM pins WHERE expires_at IS NULL OR expires_at > ?1
             ORDER BY created_at DESC",
            vec![now.into()],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok(Pin {
                id: r.get(0)?,
                scope: r.get(1)?,
                file: r.get(2)?,
                line: r.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                note: r.get(4)?,
                expires_at: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ─── Drift ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DriftReport {
    pub baseline: Option<SnapshotRow>,
    pub latest: Option<SnapshotRow>,
    pub score_delta: Option<f32>,
    pub axis_deltas: Option<QualityAxes>,
    pub new_hot_functions: Vec<HotFunctionFingerprint>,
    pub disappeared_hot_functions: Vec<HotFunctionFingerprint>,
    pub worsened_functions: Vec<FunctionDrift>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDrift {
    pub file: String,
    pub name: String,
    pub from_cc: u32,
    pub to_cc: u32,
    pub from_cog: u32,
    pub to_cog: u32,
}

pub fn compute_drift(baseline: &SnapshotRow, latest: &SnapshotRow) -> DriftReport {
    let score_delta = match (baseline.score, latest.score) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let axis_deltas = Some(QualityAxes {
        complexity: diff_axis(baseline.axes.complexity, latest.axes.complexity),
        acyclicity: diff_axis(baseline.axes.acyclicity, latest.axes.acyclicity),
        dead_code: diff_axis(baseline.axes.dead_code, latest.axes.dead_code),
        coupling: diff_axis(baseline.axes.coupling, latest.axes.coupling),
    });

    let key = |f: &HotFunctionFingerprint| (f.file.clone(), f.name.clone());
    let base_map: std::collections::HashMap<_, &HotFunctionFingerprint> =
        baseline.hot_functions.iter().map(|f| (key(f), f)).collect();
    let latest_map: std::collections::HashMap<_, &HotFunctionFingerprint> =
        latest.hot_functions.iter().map(|f| (key(f), f)).collect();

    let new_hot_functions: Vec<HotFunctionFingerprint> = latest
        .hot_functions
        .iter()
        .filter(|f| !base_map.contains_key(&key(f)))
        .cloned()
        .collect();
    let disappeared_hot_functions: Vec<HotFunctionFingerprint> = baseline
        .hot_functions
        .iter()
        .filter(|f| !latest_map.contains_key(&key(f)))
        .cloned()
        .collect();

    let worsened_functions: Vec<FunctionDrift> = latest
        .hot_functions
        .iter()
        .filter_map(|nf| {
            let prev = base_map.get(&key(nf))?;
            if nf.cyclomatic > prev.cyclomatic || nf.cognitive > prev.cognitive {
                Some(FunctionDrift {
                    file: nf.file.clone(),
                    name: nf.name.clone(),
                    from_cc: prev.cyclomatic,
                    to_cc: nf.cyclomatic,
                    from_cog: prev.cognitive,
                    to_cog: nf.cognitive,
                })
            } else {
                None
            }
        })
        .collect();

    DriftReport {
        baseline: Some(baseline.clone()),
        latest: Some(latest.clone()),
        score_delta,
        axis_deltas,
        new_hot_functions,
        disappeared_hot_functions,
        worsened_functions,
    }
}

fn diff_axis(from: Option<f32>, to: Option<f32>) -> Option<f32> {
    match (from, to) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    }
}

pub fn since_iso(since: &str) -> Result<String> {
    // Accept things like "7d", "1h", "30m", or a full ISO 8601 timestamp.
    let trimmed = since.trim();
    if let Some(stripped) = trimmed.strip_suffix('d') {
        let days: i64 = stripped.parse().context("bad days suffix")?;
        let ts = OffsetDateTime::now_utc() - time::Duration::days(days);
        return Ok(ts.format(&time::format_description::well_known::Iso8601::DEFAULT)?);
    }
    if let Some(stripped) = trimmed.strip_suffix('h') {
        let hrs: i64 = stripped.parse().context("bad hours suffix")?;
        let ts = OffsetDateTime::now_utc() - time::Duration::hours(hrs);
        return Ok(ts.format(&time::format_description::well_known::Iso8601::DEFAULT)?);
    }
    if let Some(stripped) = trimmed.strip_suffix('m') {
        let mins: i64 = stripped.parse().context("bad minutes suffix")?;
        let ts = OffsetDateTime::now_utc() - time::Duration::minutes(mins);
        return Ok(ts.format(&time::format_description::well_known::Iso8601::DEFAULT)?);
    }
    // Assume ISO 8601 timestamp.
    Ok(trimmed.to_string())
}

// ─── v2: notes (knowledge layer) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub kind: String,
    pub scope: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub symbol: Option<String>,
    pub content: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    /// Cosine-similarity score when this note came from a recall query. `None`
    /// for plain list operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub created_at: String,
    pub ttl_days: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct NoteDraft<'a> {
    pub kind: &'a str,
    pub scope: &'a str,
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub symbol: Option<&'a str>,
    pub content: &'a str,
    pub agent_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,
    pub ttl_days: Option<u32>,
}

pub fn insert_note(conn: &Connection, draft: NoteDraft<'_>) -> Result<i64> {
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    let embedding_blob = draft.embedding.map(embedding_to_bytes);
    conn.execute(
        "INSERT INTO notes(kind, scope, file, line, symbol, content, agent_id, session_id, embedding, refs_json, created_at, ttl_days)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11)",
        params![
            draft.kind,
            draft.scope,
            draft.file,
            draft.line,
            draft.symbol,
            draft.content,
            draft.agent_id,
            draft.session_id,
            embedding_blob,
            now,
            draft.ttl_days,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_notes(
    conn: &Connection,
    scope: Option<&str>,
    kind: Option<&str>,
    since: Option<&str>,
    limit: usize,
) -> Result<Vec<Note>> {
    let mut sql = String::from(
        "SELECT id, kind, scope, file, line, symbol, content, agent_id, session_id, created_at, ttl_days \
         FROM notes WHERE 1=1",
    );
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(s) = scope {
        sql.push_str(" AND scope = ?");
        params_vec.push(s.to_string().into());
    }
    if let Some(k) = kind {
        sql.push_str(" AND kind = ?");
        params_vec.push(k.to_string().into());
    }
    if let Some(ts) = since {
        sql.push_str(" AND created_at >= ?");
        params_vec.push(ts.to_string().into());
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    params_vec.push((limit as i64).into());

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params_refs), row_to_note)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Pure BM25-style recall when no embeddings are present: ranks by token
/// overlap between query and note content. Cheap enough for thousands of
/// notes; the dense path (cosine over `embedding`) takes over once the
/// embedding model is wired in.
pub fn recall_notes(
    conn: &Connection,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<Note>> {
    let all = list_notes(conn, scope, kind, None, 10_000)?;
    let q_tokens: std::collections::BTreeSet<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    if q_tokens.is_empty() {
        return Ok(all.into_iter().take(limit).collect());
    }
    let mut scored: Vec<(f32, Note)> = all
        .into_iter()
        .map(|n| {
            let tokens: std::collections::BTreeSet<String> = n
                .content
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect();
            let overlap = q_tokens.intersection(&tokens).count() as f32;
            let score = overlap / (q_tokens.len() as f32).max(1.0);
            (score, n)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored
        .into_iter()
        .take(limit)
        .map(|(s, mut n)| {
            n.score = Some(s);
            n
        })
        .collect())
}

/// Cosine-similarity recall over the `embedding` BLOB column. Returns notes
/// with non-NULL embeddings ranked by inner product against `query_vec`.
/// Falls through silently when no notes have embeddings (the caller should
/// run `recall_notes` instead).
///
/// Vectors are assumed L2-normalized — the fastembed provider returns
/// normalized output — so the inner product is the cosine similarity.
pub fn recall_notes_dense(
    conn: &Connection,
    query_vec: &[f32],
    scope: Option<&str>,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<Note>> {
    let mut sql = String::from(
        "SELECT id, kind, scope, file, line, symbol, content, agent_id, session_id, created_at, ttl_days, embedding \
         FROM notes WHERE embedding IS NOT NULL",
    );
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(s) = scope {
        sql.push_str(" AND scope = ?");
        params_vec.push(s.to_string().into());
    }
    if let Some(k) = kind {
        sql.push_str(" AND kind = ?");
        params_vec.push(k.to_string().into());
    }
    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect();
    let mut scored: Vec<(f32, Note)> = stmt
        .query_map(rusqlite::params_from_iter(params_refs), |r| {
            let blob: Vec<u8> = r.get(11)?;
            let note = Note {
                id: r.get(0)?,
                kind: r.get(1)?,
                scope: r.get(2)?,
                file: r.get(3)?,
                line: r.get(4)?,
                symbol: r.get(5)?,
                content: r.get(6)?,
                agent_id: r.get(7)?,
                session_id: r.get(8)?,
                score: None,
                created_at: r.get(9)?,
                ttl_days: r.get(10)?,
            };
            Ok((blob, note))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(blob, note)| {
            let v = bytes_to_embedding(&blob)?;
            let score = cosine(&v, query_vec)?;
            Some((score, note))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored
        .into_iter()
        .take(limit)
        .map(|(s, mut n)| {
            n.score = Some(s);
            n
        })
        .collect())
}

fn bytes_to_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(dot / (na.sqrt() * nb.sqrt()))
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        kind: row.get(1)?,
        scope: row.get(2)?,
        file: row.get(3)?,
        line: row.get(4)?,
        symbol: row.get(5)?,
        content: row.get(6)?,
        agent_id: row.get(7)?,
        session_id: row.get(8)?,
        score: None,
        created_at: row.get(9)?,
        ttl_days: row.get(10)?,
    })
}

fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod note_tests {
    use super::*;

    fn fresh_db() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        // Leak the tempdir so the connection's open file outlives this fn.
        let path = tmp.keep().join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn insert_and_list_round_trip() {
        let conn = fresh_db();
        let id = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "use rusqlite for state",
                agent_id: Some("test"),
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        assert!(id > 0);
        let notes = list_notes(&conn, None, None, None, 10).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, "decision");
        assert_eq!(notes[0].content, "use rusqlite for state");
    }

    #[test]
    fn list_filters_by_kind_and_scope() {
        let conn = fresh_db();
        for (kind, scope, content) in [
            ("decision", "project", "use rusqlite"),
            ("gotcha", "file", "watch out for SQLite WAL"),
            ("decision", "file", "we keep schema additive-only"),
        ] {
            insert_note(
                &conn,
                NoteDraft {
                    kind,
                    scope,
                    file: None,
                    line: None,
                    symbol: None,
                    content,
                    agent_id: None,
                    session_id: None,
                    embedding: None,
                    ttl_days: None,
                },
            )
            .unwrap();
        }
        let decisions = list_notes(&conn, None, Some("decision"), None, 10).unwrap();
        assert_eq!(decisions.len(), 2);
        let file_scoped = list_notes(&conn, Some("file"), None, None, 10).unwrap();
        assert_eq!(file_scoped.len(), 2);
    }

    #[test]
    fn recall_ranks_by_token_overlap_when_no_embedding() {
        let conn = fresh_db();
        for content in [
            "use rusqlite for state storage",
            "embedding vectors are stored as blobs",
            "the search index uses tantivy under the hood",
        ] {
            insert_note(
                &conn,
                NoteDraft {
                    kind: "context",
                    scope: "project",
                    file: None,
                    line: None,
                    symbol: None,
                    content,
                    agent_id: None,
                    session_id: None,
                    embedding: None,
                    ttl_days: None,
                },
            )
            .unwrap();
        }
        let hits = recall_notes(&conn, "rusqlite state", None, None, 3).unwrap();
        assert!(!hits.is_empty());
        // Best match must be the "rusqlite for state" note.
        assert!(hits[0].content.contains("rusqlite"));
        assert!(hits[0].score.unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn cosine_helper_handles_edge_cases() {
        // Identical unit vectors → 1.0.
        let a = vec![1.0, 0.0, 0.0];
        assert!((cosine(&a, &a).unwrap() - 1.0).abs() < 1e-6);
        // Orthogonal → 0.0.
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine(&a, &b).unwrap().abs() < 1e-6);
        // Length mismatch → None.
        assert!(cosine(&a, &[1.0, 2.0]).is_none());
        // Zero vector → None (avoid divide-by-zero).
        assert!(cosine(&a, &[0.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn dense_recall_ranks_by_cosine() {
        let conn = fresh_db();
        // Note A's embedding aligns with the query; B's is perpendicular.
        let a_vec: Vec<f32> = vec![1.0, 0.0, 0.0];
        let b_vec: Vec<f32> = vec![0.0, 1.0, 0.0];
        let a_id = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "aligned with query",
                agent_id: None,
                session_id: None,
                embedding: Some(&a_vec),
                ttl_days: None,
            },
        )
        .unwrap();
        let _b_id = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "orthogonal to query",
                agent_id: None,
                session_id: None,
                embedding: Some(&b_vec),
                ttl_days: None,
            },
        )
        .unwrap();
        let query: Vec<f32> = vec![1.0, 0.0, 0.0];
        let hits = recall_notes_dense(&conn, &query, None, None, 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, a_id, "aligned note must rank first");
        assert!(hits[0].score.unwrap_or(0.0) > hits[1].score.unwrap_or(0.0));
    }

    #[test]
    fn dense_recall_skips_notes_without_embedding() {
        let conn = fresh_db();
        // Note with embedding.
        let with_emb = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "has vector",
                agent_id: None,
                session_id: None,
                embedding: Some(&[1.0f32, 0.0, 0.0]),
                ttl_days: None,
            },
        )
        .unwrap();
        // Note without embedding (BM25-only). Must NOT appear in dense recall.
        insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "legacy",
                agent_id: None,
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        let hits = recall_notes_dense(&conn, &[1.0, 0.0, 0.0], None, None, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, with_emb);
    }
}

// ─── v3: file hashes (watch-daemon incremental detection) ────────────────

/// Result of comparing the current filesystem state to the last-known
/// hashes in `state.db`. The watch daemon turns this into reindex actions.
#[derive(Debug, Default, Clone)]
pub struct ScanDelta {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl ScanDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
    pub fn total(&self) -> usize {
        self.added.len() + self.changed.len() + self.removed.len()
    }
}

/// blake3-truncated content hash. 16 hex chars is plenty to dedup file
/// changes — `notify` already filters to the project root, so cross-file
/// collisions are not a real risk in this context.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let h = blake3::hash(bytes);
    let hex = h.to_hex();
    hex[..16].to_string()
}

pub fn upsert_file_hash(conn: &Connection, path: &str, hash: &str) -> Result<()> {
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    conn.execute(
        "INSERT INTO file_hashes(path, hash, modified_at) VALUES(?1, ?2, ?3) \
         ON CONFLICT(path) DO UPDATE SET hash=excluded.hash, modified_at=excluded.modified_at",
        params![path, hash, now],
    )?;
    Ok(())
}

pub fn delete_file_hash(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM file_hashes WHERE path = ?1", params![path])?;
    Ok(())
}

pub fn get_file_hash(conn: &Connection, path: &str) -> Result<Option<String>> {
    let row = conn
        .query_row(
            "SELECT hash FROM file_hashes WHERE path = ?1",
            params![path],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(row)
}

pub fn list_known_paths(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("SELECT path FROM file_hashes")?;
    let paths: rusqlite::Result<std::collections::HashSet<String>> =
        stmt.query_map([], |r| r.get::<_, String>(0))?.collect();
    Ok(paths?)
}

/// Compute the delta between the candidate set and the hashes recorded in
/// `state.db`. The candidate set is interpreted as **the set of files we
/// just looked at** — typically the dirty set from a watch flush.
///
///   - File in candidates that exists on disk → `added` if unknown,
///     `changed` if its hash differs, otherwise no-op.
///   - File in candidates that no longer exists on disk → `removed` (only
///     if we'd known about it previously).
///   - When `full_scan` is `true`, files in the DB that are **not** in the
///     candidate set are also marked `removed`. Use this from a full
///     re-scan pass (`belisarius scan` / `belisarius index --with-scan`)
///     where the candidate set is the full project tree. **Do not** use it
///     from a watch flush — you'll false-positive every unchanged file as
///     "removed".
///
/// Side effect: the caller is expected to call `commit_scan_delta` afterward
/// to persist the new hashes. This split lets the watcher decide whether to
/// commit (after a successful reindex) or roll back (after a failure).
pub fn compute_scan_delta(
    conn: &Connection,
    project_root: &Path,
    candidates: &[String],
    full_scan: bool,
) -> Result<(ScanDelta, std::collections::HashMap<String, String>)> {
    use std::collections::{HashMap, HashSet};
    let known: HashSet<String> = list_known_paths(conn)?;
    let candidate_set: HashSet<String> = candidates.iter().cloned().collect();
    let mut delta = ScanDelta::default();
    let mut new_hashes: HashMap<String, String> = HashMap::new();
    for rel in candidates {
        let abs = project_root.join(rel);
        match std::fs::read(&abs) {
            Ok(bytes) => {
                let hash = hash_bytes(&bytes);
                let prev = get_file_hash(conn, rel)?;
                match prev {
                    None => delta.added.push(rel.clone()),
                    Some(old) if old != hash => delta.changed.push(rel.clone()),
                    _ => {}
                }
                new_hashes.insert(rel.clone(), hash);
            }
            Err(_) => {
                // Path was in the candidate set but is gone from disk. If we
                // previously knew about it, this is a deletion event.
                if get_file_hash(conn, rel)?.is_some() {
                    delta.removed.push(rel.clone());
                }
            }
        }
    }
    if full_scan {
        for old in known.difference(&candidate_set) {
            delta.removed.push(old.clone());
        }
    }
    Ok((delta, new_hashes))
}

/// Persist the hash map and prune removed entries. Call after a successful
/// reindex pass to commit the watcher's view of the world.
pub fn commit_scan_delta(
    conn: &Connection,
    delta: &ScanDelta,
    new_hashes: &std::collections::HashMap<String, String>,
) -> Result<()> {
    for path in delta.added.iter().chain(delta.changed.iter()) {
        if let Some(h) = new_hashes.get(path) {
            upsert_file_hash(conn, path, h)?;
        }
    }
    for path in &delta.removed {
        delete_file_hash(conn, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod file_hash_tests {
    use super::*;
    use std::collections::HashMap;

    fn db_in_project(project: &Path) -> Connection {
        let path = project.join(".belisarius").join("state.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn hash_bytes_is_deterministic_and_short() {
        assert_eq!(hash_bytes(b"hi"), hash_bytes(b"hi"));
        assert_ne!(hash_bytes(b"hi"), hash_bytes(b"bye"));
        assert_eq!(hash_bytes(b"x").len(), 16);
    }

    #[test]
    fn first_scan_is_all_added() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();
        let conn = db_in_project(tmp.path());
        let (delta, _) =
            compute_scan_delta(&conn, tmp.path(), &["a.rs".into(), "b.rs".into()], true).unwrap();
        assert_eq!(delta.added.len(), 2);
        assert!(delta.changed.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn unchanged_files_drop_out_of_delta() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        let conn = db_in_project(tmp.path());
        let (delta1, hashes1) =
            compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        commit_scan_delta(&conn, &delta1, &hashes1).unwrap();
        let (delta2, _) = compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        assert!(delta2.is_empty());
    }

    #[test]
    fn content_change_lands_in_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.rs");
        std::fs::write(&path, "fn a() {}").unwrap();
        let conn = db_in_project(tmp.path());
        let (d1, h1) = compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        commit_scan_delta(&conn, &d1, &h1).unwrap();
        std::fs::write(&path, "fn a() { let x = 1; }").unwrap();
        let (d2, _) = compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        assert_eq!(d2.changed, vec!["a.rs".to_string()]);
    }

    #[test]
    fn full_scan_marks_disappeared_files_as_removed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        let conn = db_in_project(tmp.path());
        let (d1, h1) = compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        commit_scan_delta(&conn, &d1, &h1).unwrap();
        // Full-scan with empty candidate set: a.rs is implicitly gone.
        let (d2, _) = compute_scan_delta(&conn, tmp.path(), &[], true).unwrap();
        assert_eq!(d2.removed, vec!["a.rs".to_string()]);
    }

    #[test]
    fn watch_mode_only_marks_disappeared_candidates_as_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.rs");
        let b = tmp.path().join("b.rs");
        std::fs::write(&a, "fn a() {}").unwrap();
        std::fs::write(&b, "fn b() {}").unwrap();
        let conn = db_in_project(tmp.path());
        // Seed both files via a full scan.
        let (d1, h1) =
            compute_scan_delta(&conn, tmp.path(), &["a.rs".into(), "b.rs".into()], true).unwrap();
        commit_scan_delta(&conn, &d1, &h1).unwrap();
        // Delete b.rs, then trigger a watch-mode delta where only b.rs is
        // in the candidate set (a.rs untouched).
        std::fs::remove_file(&b).unwrap();
        let (d2, _) = compute_scan_delta(&conn, tmp.path(), &["b.rs".into()], false).unwrap();
        assert_eq!(d2.removed, vec!["b.rs".to_string()]);
        // CRUCIAL: a.rs is NOT marked removed even though it's not in the
        // candidate set — the watch case doesn't know about it.
        assert!(!d2.removed.contains(&"a.rs".to_string()));
    }

    #[test]
    fn commit_clears_removed_paths_from_table() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        let conn = db_in_project(tmp.path());
        let (d1, h1) = compute_scan_delta(&conn, tmp.path(), &["a.rs".into()], true).unwrap();
        commit_scan_delta(&conn, &d1, &h1).unwrap();
        let (d2, h2) = compute_scan_delta(&conn, tmp.path(), &[], true).unwrap();
        commit_scan_delta(&conn, &d2, &h2).unwrap();
        let known = list_known_paths(&conn).unwrap();
        assert!(known.is_empty());
        // Suppress unused HashMap import on the strict path.
        let _ = HashMap::<String, String>::new();
    }
}

// ─── v3.5: note edges (knowledge-graph links) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEdge {
    pub from_id: i64,
    pub to_id: i64,
    pub kind: String,
}

/// Valid edge kinds. Kept inside the layer so service handlers reuse the
/// same validator.
pub const EDGE_KINDS: &[&str] = &["supports", "contradicts", "supersedes"];

pub fn insert_note_edge(conn: &Connection, from_id: i64, to_id: i64, kind: &str) -> Result<()> {
    if !EDGE_KINDS.contains(&kind) {
        anyhow::bail!("invalid edge kind: {kind}");
    }
    conn.execute(
        "INSERT INTO note_edges(from_id, to_id, kind) VALUES(?1, ?2, ?3) \
         ON CONFLICT(from_id, to_id, kind) DO NOTHING",
        params![from_id, to_id, kind],
    )?;
    Ok(())
}

pub fn list_outgoing_edges(conn: &Connection, from_id: i64) -> Result<Vec<NoteEdge>> {
    let mut stmt =
        conn.prepare("SELECT from_id, to_id, kind FROM note_edges WHERE from_id = ?1")?;
    let rows: rusqlite::Result<Vec<NoteEdge>> = stmt
        .query_map(params![from_id], |r| {
            Ok(NoteEdge {
                from_id: r.get(0)?,
                to_id: r.get(1)?,
                kind: r.get(2)?,
            })
        })?
        .collect();
    Ok(rows?)
}

pub fn list_incoming_edges(conn: &Connection, to_id: i64) -> Result<Vec<NoteEdge>> {
    let mut stmt = conn.prepare("SELECT from_id, to_id, kind FROM note_edges WHERE to_id = ?1")?;
    let rows: rusqlite::Result<Vec<NoteEdge>> = stmt
        .query_map(params![to_id], |r| {
            Ok(NoteEdge {
                from_id: r.get(0)?,
                to_id: r.get(1)?,
                kind: r.get(2)?,
            })
        })?
        .collect();
    Ok(rows?)
}

// ─── v4: sessions ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub agent_id: Option<String>,
}

pub fn insert_session(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    agent_id: Option<&str>,
) -> Result<Session> {
    let started_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    conn.execute(
        "INSERT INTO sessions(id, name, started_at, agent_id) VALUES(?1, ?2, ?3, ?4)",
        params![id, name, started_at, agent_id],
    )?;
    Ok(Session {
        id: id.to_string(),
        name: name.map(str::to_string),
        started_at,
        ended_at: None,
        agent_id: agent_id.map(str::to_string),
    })
}

pub fn end_session(conn: &Connection, id: &str) -> Result<Option<Session>> {
    let ended_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)?;
    let rows = conn.execute(
        "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
        params![ended_at, id],
    )?;
    if rows == 0 {
        return Ok(None);
    }
    let session = conn
        .query_row(
            "SELECT id, name, started_at, ended_at, agent_id FROM sessions WHERE id = ?1",
            params![id],
            |r| {
                Ok(Session {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    started_at: r.get(2)?,
                    ended_at: r.get(3)?,
                    agent_id: r.get(4)?,
                })
            },
        )
        .ok();
    Ok(session)
}

/// Summary statistics for a session: note count per kind.
pub fn session_summary(conn: &Connection, session_id: &str) -> Result<serde_json::Value> {
    let mut stmt =
        conn.prepare("SELECT kind, COUNT(*) FROM notes WHERE session_id = ?1 GROUP BY kind")?;
    let counts: std::collections::BTreeMap<String, u32> = stmt
        .query_map(params![session_id], |r| {
            Ok::<(String, u32), rusqlite::Error>((r.get(0)?, r.get(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    let total: u32 = counts.values().sum();
    Ok(serde_json::json!({
        "session_id": session_id,
        "total_notes": total,
        "by_kind": counts,
    }))
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn fresh() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.keep().join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn insert_and_list_round_trip() {
        let conn = fresh();
        // Need real note ids; insert two notes first.
        let a = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "a",
                agent_id: None,
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        let b = insert_note(
            &conn,
            NoteDraft {
                kind: "decision",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "b",
                agent_id: None,
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        insert_note_edge(&conn, a, b, "supersedes").unwrap();
        let out = list_outgoing_edges(&conn, a).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_id, b);
        let inc = list_incoming_edges(&conn, b).unwrap();
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].from_id, a);
    }

    #[test]
    fn invalid_edge_kind_rejected() {
        let conn = fresh();
        let r = insert_note_edge(&conn, 1, 2, "loves");
        assert!(r.is_err());
    }

    #[test]
    fn duplicate_edges_are_idempotent() {
        let conn = fresh();
        let a = insert_note(
            &conn,
            NoteDraft {
                kind: "context",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "x",
                agent_id: None,
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        let b = insert_note(
            &conn,
            NoteDraft {
                kind: "context",
                scope: "project",
                file: None,
                line: None,
                symbol: None,
                content: "y",
                agent_id: None,
                session_id: None,
                embedding: None,
                ttl_days: None,
            },
        )
        .unwrap();
        insert_note_edge(&conn, a, b, "supports").unwrap();
        insert_note_edge(&conn, a, b, "supports").unwrap(); // no-op
        assert_eq!(list_outgoing_edges(&conn, a).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn fresh() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.keep().join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn start_end_round_trip() {
        let conn = fresh();
        let s = insert_session(&conn, "sess-1", Some("test"), Some("agent-x")).unwrap();
        assert_eq!(s.id, "sess-1");
        assert!(s.ended_at.is_none());
        let ended = end_session(&conn, "sess-1").unwrap().unwrap();
        assert!(ended.ended_at.is_some());
    }

    #[test]
    fn end_unknown_session_returns_none() {
        let conn = fresh();
        let r = end_session(&conn, "does-not-exist").unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn summary_counts_notes_per_kind() {
        let conn = fresh();
        insert_session(&conn, "sess-2", None, None).unwrap();
        for (kind, content) in [("decision", "a"), ("decision", "b"), ("gotcha", "c")] {
            insert_note(
                &conn,
                NoteDraft {
                    kind,
                    scope: "project",
                    file: None,
                    line: None,
                    symbol: None,
                    content,
                    agent_id: None,
                    session_id: Some("sess-2"),
                    embedding: None,
                    ttl_days: None,
                },
            )
            .unwrap();
        }
        let summary = session_summary(&conn, "sess-2").unwrap();
        assert_eq!(summary["total_notes"], 3);
        assert_eq!(summary["by_kind"]["decision"], 2);
        assert_eq!(summary["by_kind"]["gotcha"], 1);
    }
}
