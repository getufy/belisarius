//! Cross-app aggregate index for the Belisarius fleet.
//!
//! A tiny SQLite database (`~/.belisarius/fleet.db` by default, override via
//! `BELISARIUS_FLEET_DB`) that holds one row per public surface item and one
//! row per file-level hotspot, across every registered app. This is the data
//! layer behind the `belisarius fleet find / diff / hotspots` commands and
//! the corresponding HTTP + MCP endpoints.
//!
//! Schema is created lazily; everything is idempotent so re-running `fleet
//! sync` is safe. We REPLACE-on-conflict instead of versioning rows because
//! the cache is rebuilt per-app on every sync.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("BELISARIUS_FLEET_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".belisarius").join("fleet.db")
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    // Lighter durability for a derived cache that we can always rebuild.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    // v1: surface_items PK now includes `kind` — a function and a type with
    // the same name on the same line are distinct rows. Earlier schemas
    // collapsed them and tripped UNIQUE on sync.
    if version < 1 {
        conn.execute_batch("DROP TABLE IF EXISTS surface_items;")?;
    }
    // v2 just adds test_mapping (additive — no destructive change).
    // v3: hotspots gains an `owners` column. Derived data, safe to drop.
    if version < 3 {
        conn.execute_batch("DROP TABLE IF EXISTS hotspots;")?;
    }

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS apps (
            name        TEXT PRIMARY KEY,
            path        TEXT NOT NULL,
            last_synced TEXT
        );

        CREATE TABLE IF NOT EXISTS surface_items (
            app_name   TEXT NOT NULL,
            file       TEXT NOT NULL,
            language   TEXT NOT NULL,
            kind       TEXT NOT NULL,
            name       TEXT NOT NULL,
            signature  TEXT,
            line       INTEGER NOT NULL,
            method     TEXT,
            PRIMARY KEY (app_name, file, line, name, kind)
        );
        CREATE INDEX IF NOT EXISTS idx_surface_kind   ON surface_items(kind);
        CREATE INDEX IF NOT EXISTS idx_surface_name   ON surface_items(name COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_surface_method ON surface_items(method);

        CREATE TABLE IF NOT EXISTS hotspots (
            app_name      TEXT NOT NULL,
            file          TEXT NOT NULL,
            churn         INTEGER NOT NULL,
            total_commits INTEGER NOT NULL,
            complexity    INTEGER NOT NULL,
            score         REAL    NOT NULL,
            last_edited   TEXT,
            last_author   TEXT,
            top_author    TEXT,
            days_window   INTEGER NOT NULL,
            owners        TEXT,
            PRIMARY KEY (app_name, file)
        );
        CREATE INDEX IF NOT EXISTS idx_hotspots_score ON hotspots(score DESC);

        CREATE TABLE IF NOT EXISTS test_mapping (
            app_name    TEXT NOT NULL,
            source_file TEXT NOT NULL,
            test_file   TEXT NOT NULL,
            PRIMARY KEY (app_name, source_file, test_file)
        );
        CREATE INDEX IF NOT EXISTS idx_test_mapping_source ON test_mapping(source_file);

        CREATE TABLE IF NOT EXISTS file_complexity (
            app_name         TEXT NOT NULL,
            file             TEXT NOT NULL,
            language         TEXT NOT NULL,
            loc              INTEGER NOT NULL,
            function_count   INTEGER NOT NULL,
            total_cyclomatic INTEGER NOT NULL,
            max_cyclomatic   INTEGER NOT NULL,
            PRIMARY KEY (app_name, file)
        );
        CREATE INDEX IF NOT EXISTS idx_file_complexity_cc
            ON file_complexity(total_cyclomatic DESC);
        "#,
    )?;

    conn.pragma_update(None, "user_version", 3)?;
    Ok(())
}

pub fn upsert_app(conn: &Connection, name: &str, path: &str, last_synced: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO apps(name, path, last_synced)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET path = excluded.path, last_synced = excluded.last_synced",
        params![name, path, last_synced],
    )?;
    Ok(())
}

pub fn replace_surface(
    conn: &mut Connection,
    app_name: &str,
    items: &[belisarius_core::SurfaceItem],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM surface_items WHERE app_name = ?1",
        params![app_name],
    )?;
    {
        // OR REPLACE so a single sync batch can carry duplicate logical rows
        // (e.g. two AST walks landing on the same item) without aborting the
        // whole transaction.
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO surface_items(app_name, file, language, kind, name, signature, line, method)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for it in items {
            let kind = kind_label(it.kind);
            stmt.execute(params![
                app_name,
                it.file,
                it.language,
                kind,
                it.name,
                it.signature,
                it.line,
                it.method,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn replace_hotspots(
    conn: &mut Connection,
    app_name: &str,
    days_window: u32,
    rows: &[belisarius_scan::git_stats::Hotspot],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM hotspots WHERE app_name = ?1",
        params![app_name],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO hotspots(app_name, file, churn, total_commits, complexity, score,
                                   last_edited, last_author, top_author, days_window, owners)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for h in rows {
            let last_edited = h.last_edited.as_ref().and_then(|t| {
                t.format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .ok()
            });
            // Store owners as a space-separated string — they're already
            // shell-safe (`@org/team` or `email@example.com`), and we treat
            // them as opaque tokens on read.
            let owners: Option<String> = if h.owners.is_empty() {
                None
            } else {
                Some(h.owners.join(" "))
            };
            stmt.execute(params![
                app_name,
                h.path,
                h.churn,
                h.total_commits,
                h.complexity,
                h.score,
                last_edited,
                h.last_author,
                h.top_author,
                days_window,
                owners,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn replace_test_mappings(
    conn: &mut Connection,
    app_name: &str,
    mappings: &[belisarius_scan::test_map::TestMapping],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM test_mapping WHERE app_name = ?1",
        params![app_name],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO test_mapping(app_name, source_file, test_file)
             VALUES (?1, ?2, ?3)",
        )?;
        for m in mappings {
            for t in &m.tests {
                stmt.execute(params![app_name, m.source, t])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Persist per-file complexity for every non-test source file. Test files
/// are skipped so they never appear as gaps in `top_test_gaps`.
pub fn replace_file_complexity(
    conn: &mut Connection,
    app_name: &str,
    files: &[belisarius_core::FileNode],
    metrics: &[belisarius_core::FileMetrics],
) -> Result<()> {
    let by_path: std::collections::HashMap<&str, &belisarius_core::FileMetrics> =
        metrics.iter().map(|m| (m.path.as_str(), m)).collect();
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM file_complexity WHERE app_name = ?1",
        params![app_name],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO file_complexity
               (app_name, file, language, loc, function_count, total_cyclomatic, max_cyclomatic)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for f in files {
            if belisarius_scan::test_map::is_test_file(&f.path) {
                continue;
            }
            let m = by_path.get(f.path.as_str());
            stmt.execute(params![
                app_name,
                f.path,
                f.language,
                f.loc,
                m.map(|x| x.function_count).unwrap_or(0),
                m.map(|x| x.total_cyclomatic).unwrap_or(0),
                m.map(|x| x.max_cyclomatic).unwrap_or(0),
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct TestGapRow {
    pub app: String,
    pub file: String,
    pub language: String,
    pub loc: u32,
    pub function_count: u32,
    pub total_cyclomatic: u32,
}

/// Untested source files across the fleet, ranked by complexity descending.
/// "Untested" = present in `file_complexity` but absent from `test_mapping`
/// (no test file imports it and no inline `#[cfg(test)]` self-test). Test
/// files themselves are excluded by the heuristic before insertion.
pub fn top_test_gaps(conn: &Connection, limit: usize) -> Result<Vec<TestGapRow>> {
    let mut stmt = conn.prepare(
        "SELECT fc.app_name, fc.file, fc.language, fc.loc, fc.function_count, fc.total_cyclomatic
           FROM file_complexity fc
          WHERE NOT EXISTS (
                  SELECT 1 FROM test_mapping tm
                   WHERE tm.app_name = fc.app_name AND tm.source_file = fc.file
                )
          ORDER BY fc.total_cyclomatic DESC, fc.loc DESC
          LIMIT ?",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok(TestGapRow {
            app: r.get(0)?,
            file: r.get(1)?,
            language: r.get(2)?,
            loc: r.get::<_, i64>(3)? as u32,
            function_count: r.get::<_, i64>(4)? as u32,
            total_cyclomatic: r.get::<_, i64>(5)? as u32,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[derive(Debug, serde::Serialize)]
pub struct SurfaceRow {
    pub app: String,
    pub file: String,
    pub language: String,
    pub kind: String,
    pub name: String,
    pub signature: Option<String>,
    pub line: u32,
    pub method: Option<String>,
}

/// Substring or LIKE-pattern search over surface items. `kind` filters by item
/// kind (function, type, http_route, …). When `pattern` contains `%` we treat
/// it as a SQL LIKE; otherwise we substring-match (case-insensitive).
pub fn find_surface(
    conn: &Connection,
    kind: Option<&str>,
    pattern: Option<&str>,
    limit: usize,
) -> Result<Vec<SurfaceRow>> {
    let mut sql = String::from(
        "SELECT app_name, file, language, kind, name, signature, line, method
           FROM surface_items
          WHERE 1=1",
    );
    let mut bound: Vec<String> = Vec::new();
    if let Some(k) = kind {
        sql.push_str(" AND kind = ?");
        bound.push(k.to_string());
    }
    if let Some(p) = pattern {
        if p.contains('%') {
            sql.push_str(" AND name LIKE ? COLLATE NOCASE");
            bound.push(p.to_string());
        } else {
            sql.push_str(" AND name LIKE ? COLLATE NOCASE");
            bound.push(format!("%{p}%"));
        }
    }
    sql.push_str(" ORDER BY kind, app_name, file LIMIT ?");
    bound.push(limit.to_string());

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        bound.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params.as_slice(), |r| {
        Ok(SurfaceRow {
            app: r.get(0)?,
            file: r.get(1)?,
            language: r.get(2)?,
            kind: r.get(3)?,
            name: r.get(4)?,
            signature: r.get(5)?,
            line: r.get::<_, i64>(6)? as u32,
            method: r.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[derive(Debug, serde::Serialize)]
pub struct HotspotRow {
    pub app: String,
    pub file: String,
    pub churn: u32,
    pub complexity: u32,
    pub score: f64,
    pub last_author: Option<String>,
    pub owners: Vec<String>,
}

pub fn top_hotspots(conn: &Connection, limit: usize) -> Result<Vec<HotspotRow>> {
    let mut stmt = conn.prepare(
        "SELECT app_name, file, churn, complexity, score, last_author, owners
           FROM hotspots
          ORDER BY score DESC
          LIMIT ?",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        let owners_raw: Option<String> = r.get(6)?;
        let owners: Vec<String> = owners_raw
            .map(|s| s.split_whitespace().map(|t| t.to_string()).collect())
            .unwrap_or_default();
        Ok(HotspotRow {
            app: r.get(0)?,
            file: r.get(1)?,
            churn: r.get::<_, i64>(2)? as u32,
            complexity: r.get::<_, i64>(3)? as u32,
            score: r.get(4)?,
            last_author: r.get(5)?,
            owners,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[derive(Debug, serde::Serialize)]
pub struct SurfaceDiffEntry {
    pub kind: String,
    pub name: String,
    pub method: Option<String>,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, serde::Serialize)]
pub struct SurfaceDiff {
    pub only_in_a: Vec<SurfaceDiffEntry>,
    pub only_in_b: Vec<SurfaceDiffEntry>,
    pub common_count: usize,
}

pub fn surface_diff(conn: &Connection, app_a: &str, app_b: &str) -> Result<SurfaceDiff> {
    fn fetch(conn: &Connection, app: &str) -> Result<Vec<SurfaceDiffEntry>> {
        let mut stmt = conn.prepare(
            "SELECT kind, name, method, file, line FROM surface_items WHERE app_name = ?1",
        )?;
        let rows = stmt.query_map(params![app], |r| {
            Ok(SurfaceDiffEntry {
                kind: r.get(0)?,
                name: r.get(1)?,
                method: r.get(2)?,
                file: r.get(3)?,
                line: r.get::<_, i64>(4)? as u32,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    let a = fetch(conn, app_a)?;
    let b = fetch(conn, app_b)?;
    let key = |e: &SurfaceDiffEntry| {
        format!(
            "{}|{}|{}",
            e.kind,
            e.name,
            e.method.clone().unwrap_or_default()
        )
    };
    let keys_b: std::collections::HashSet<String> = b.iter().map(key).collect();
    let keys_a: std::collections::HashSet<String> = a.iter().map(key).collect();
    let only_in_a: Vec<SurfaceDiffEntry> = a
        .into_iter()
        .filter(|e| !keys_b.contains(&key(e)))
        .collect();
    let only_in_b: Vec<SurfaceDiffEntry> = b
        .into_iter()
        .filter(|e| !keys_a.contains(&key(e)))
        .collect();
    let common_count = keys_a.intersection(&keys_b).count();
    Ok(SurfaceDiff {
        only_in_a,
        only_in_b,
        common_count,
    })
}

fn kind_label(k: belisarius_core::SurfaceKind) -> &'static str {
    use belisarius_core::SurfaceKind;
    match k {
        SurfaceKind::Function => "function",
        SurfaceKind::Type => "type",
        SurfaceKind::Module => "module",
        SurfaceKind::Constant => "constant",
        SurfaceKind::ReExport => "re_export",
        SurfaceKind::HttpRoute => "http_route",
        SurfaceKind::CliCommand => "cli_command",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{SurfaceItem, SurfaceKind};
    use tempfile::tempdir;

    fn item(name: &str, kind: SurfaceKind, method: Option<&str>) -> SurfaceItem {
        SurfaceItem {
            file: "src/main.rs".into(),
            language: "rust".into(),
            kind,
            name: name.into(),
            signature: None,
            line: 1,
            method: method.map(String::from),
        }
    }

    #[test]
    fn find_surface_filters_by_kind_and_pattern() {
        let dir = tempdir().unwrap();
        let mut conn = open(&dir.path().join("fleet.db")).unwrap();
        replace_surface(
            &mut conn,
            "alpha",
            &[
                item("create_user", SurfaceKind::Function, None),
                item("/api/users", SurfaceKind::HttpRoute, Some("POST")),
                item("UserModel", SurfaceKind::Type, None),
            ],
        )
        .unwrap();
        replace_surface(
            &mut conn,
            "bravo",
            &[item("/api/users", SurfaceKind::HttpRoute, Some("GET"))],
        )
        .unwrap();

        let routes = find_surface(&conn, Some("http_route"), Some("users"), 100).unwrap();
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().all(|r| r.kind == "http_route"));
        let users_methods: std::collections::HashSet<String> =
            routes.iter().filter_map(|r| r.method.clone()).collect();
        assert!(users_methods.contains("GET"));
        assert!(users_methods.contains("POST"));

        let fns = find_surface(&conn, Some("function"), Some("user"), 100).unwrap();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].app, "alpha");
    }

    #[test]
    fn surface_diff_classifies_unique_items() {
        let dir = tempdir().unwrap();
        let mut conn = open(&dir.path().join("fleet.db")).unwrap();
        replace_surface(
            &mut conn,
            "alpha",
            &[
                item("/api/users", SurfaceKind::HttpRoute, Some("GET")),
                item("/api/billing", SurfaceKind::HttpRoute, Some("POST")),
            ],
        )
        .unwrap();
        replace_surface(
            &mut conn,
            "bravo",
            &[
                item("/api/users", SurfaceKind::HttpRoute, Some("GET")),
                item("/api/admin", SurfaceKind::HttpRoute, Some("POST")),
            ],
        )
        .unwrap();
        let diff = surface_diff(&conn, "alpha", "bravo").unwrap();
        assert_eq!(diff.common_count, 1);
        assert_eq!(diff.only_in_a.len(), 1);
        assert_eq!(diff.only_in_a[0].name, "/api/billing");
        assert_eq!(diff.only_in_b.len(), 1);
        assert_eq!(diff.only_in_b[0].name, "/api/admin");
    }
}
