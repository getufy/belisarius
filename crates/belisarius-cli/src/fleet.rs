//! Fleet registry — Belisarius shifts from "one repo at a time" to "host
//! hundreds of apps as a fleet".
//!
//! The registry is a single TOML file at `~/.belisarius/fleet.toml` by default
//! (override with `--fleet <path>` or `BELISARIUS_FLEET` env). Each entry
//! carries a name, path, optional description + tags, and a cached summary
//! refreshed by `belisarius fleet sync`.

use anyhow::{anyhow, Context, Result};
use belisarius_core::LanguageSummary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FleetConfig {
    #[serde(default)]
    pub apps: Vec<FleetApp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetApp {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(
        default,
        with = "time::serde::iso8601::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_synced: Option<OffsetDateTime>,
    /// Cached summary refreshed by `fleet sync`. Cheap to compute (just the
    /// scan stats), so we keep it inline rather than in a separate file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<FleetSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSummary {
    pub file_count: u32,
    pub loc: u32,
    pub languages: BTreeMap<String, LanguageSummary>,
    pub primary_language: Option<String>,
}

pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("BELISARIUS_FLEET") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".belisarius").join("fleet.toml")
}

pub fn load(path: &Path) -> Result<FleetConfig> {
    if !path.exists() {
        return Ok(FleetConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading fleet config at {}", path.display()))?;
    let cfg: FleetConfig = toml::from_str(&raw)
        .with_context(|| format!("parsing fleet config at {}", path.display()))?;
    Ok(cfg)
}

pub fn save(cfg: &FleetConfig, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = toml::to_string_pretty(cfg).context("encoding fleet config")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn find_app<'a>(cfg: &'a FleetConfig, name: &str) -> Option<&'a FleetApp> {
    cfg.apps.iter().find(|a| a.name == name)
}

pub fn add_app(cfg: &mut FleetConfig, app: FleetApp) -> Result<()> {
    if cfg.apps.iter().any(|a| a.name == app.name) {
        return Err(anyhow!(
            "fleet already contains an app named {:?}; remove it first to overwrite",
            app.name
        ));
    }
    cfg.apps.push(app);
    cfg.apps.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(())
}

pub fn remove_app(cfg: &mut FleetConfig, name: &str) -> Result<()> {
    let before = cfg.apps.len();
    cfg.apps.retain(|a| a.name != name);
    if cfg.apps.len() == before {
        return Err(anyhow!("no app named {name:?} in the fleet"));
    }
    Ok(())
}

/// Recompute the lightweight summary for one app (file count, LOC, languages).
/// Expensive analysis (cycles, complexity, hotspots) is left to the per-app
/// endpoints — they have their own caches.
pub fn refresh_summary(app: &mut FleetApp) -> Result<()> {
    let scan =
        belisarius_scan::scan(&app.path).with_context(|| format!("scanning {}", app.path))?;
    let file_count = scan.files.len() as u32;
    let loc: u32 = scan.files.iter().map(|f| f.loc).sum();
    let languages = scan.language_summary.clone();
    let primary_language = languages
        .iter()
        .max_by_key(|(_, s)| s.loc)
        .map(|(k, _)| k.clone());
    app.summary = Some(FleetSummary {
        file_count,
        loc,
        languages,
        primary_language,
    });
    app.last_synced = Some(OffsetDateTime::now_utc());
    Ok(())
}

/// Resolve a `name | path | "."` selector to a project path. Names take
/// precedence over paths; an unqualified string is treated as a path if it
/// doesn't match any registered name.
pub fn resolve_target(cfg: &FleetConfig, target: &str) -> String {
    if let Some(app) = find_app(cfg, target) {
        return app.path.clone();
    }
    target.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn add_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fleet.toml");
        let mut cfg = FleetConfig::default();
        add_app(
            &mut cfg,
            FleetApp {
                name: "alpha".into(),
                path: "/tmp/alpha".into(),
                description: Some("first".into()),
                tags: vec!["api".into()],
                last_synced: None,
                summary: None,
            },
        )
        .unwrap();
        save(&cfg, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.apps.len(), 1);
        assert_eq!(loaded.apps[0].name, "alpha");
        assert_eq!(loaded.apps[0].tags, vec!["api".to_string()]);
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut cfg = FleetConfig::default();
        add_app(
            &mut cfg,
            FleetApp {
                name: "a".into(),
                path: "/tmp/a".into(),
                description: None,
                tags: vec![],
                last_synced: None,
                summary: None,
            },
        )
        .unwrap();
        let res = add_app(
            &mut cfg,
            FleetApp {
                name: "a".into(),
                path: "/tmp/other".into(),
                description: None,
                tags: vec![],
                last_synced: None,
                summary: None,
            },
        );
        assert!(res.is_err());
    }

    #[test]
    fn resolve_target_prefers_name() {
        let mut cfg = FleetConfig::default();
        add_app(
            &mut cfg,
            FleetApp {
                name: "alpha".into(),
                path: "/repos/alpha".into(),
                description: None,
                tags: vec![],
                last_synced: None,
                summary: None,
            },
        )
        .unwrap();
        assert_eq!(resolve_target(&cfg, "alpha"), "/repos/alpha");
        assert_eq!(resolve_target(&cfg, "."), ".");
        assert_eq!(resolve_target(&cfg, "/elsewhere"), "/elsewhere");
    }
}
