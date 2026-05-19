use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::fleet::{
    add_app, default_config_path, find_app, load, refresh_summary, remove_app, save, FleetApp,
};
use crate::fleet_db;

#[derive(Subcommand)]
pub enum FleetCmd {
    /// Register an app in the fleet.
    Add {
        /// Short name used to refer to the app (e.g. `belisarius_fleet_summary --app foo`).
        name: String,
        /// Project root on disk.
        path: PathBuf,
        /// One-line description.
        #[arg(long)]
        description: Option<String>,
        /// Comma-separated tags.
        #[arg(long)]
        tags: Option<String>,
    },
    /// Remove an app from the fleet.
    Remove { name: String },
    /// List every registered app with a lightweight summary.
    List {
        /// Emit JSON instead of the human table.
        #[arg(long)]
        json: bool,
    },
    /// Recompute the summary (file count, LOC, languages) for one or all apps.
    Sync {
        /// App to sync. Omit to sync every registered app.
        name: Option<String>,
    },
    /// Detailed info for one app.
    Info { name: String },
    /// Search public-surface items across the fleet. e.g. `fleet find --kind http_route users`.
    Find {
        /// Substring pattern; `%` makes it a SQL LIKE.
        pattern: String,
        /// Filter to a specific kind (function, type, http_route, …).
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Top hotspots across every app in the fleet.
    Hotspots {
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Show what one app's surface has that another's doesn't, and vice versa.
    Diff {
        a: String,
        b: String,
        #[arg(long)]
        json: bool,
    },
    /// Top untested files across the fleet, ranked by total cyclomatic complexity.
    TestGaps {
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
pub struct FleetArgs {
    /// Override the fleet config path (defaults to ~/.belisarius/fleet.toml).
    #[arg(long, global = true)]
    pub fleet_config: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: FleetCmd,
}

pub async fn run(args: FleetArgs) -> Result<()> {
    let config_path = args.fleet_config.unwrap_or_else(default_config_path);
    let mut cfg = load(&config_path)?;

    match args.cmd {
        FleetCmd::Add {
            name,
            path,
            description,
            tags,
        } => {
            let canonical = std::fs::canonicalize(&path)
                .with_context(|| format!("resolving {}", path.display()))?
                .to_string_lossy()
                .to_string();
            let tags: Vec<String> = tags
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            add_app(
                &mut cfg,
                FleetApp {
                    name: name.clone(),
                    path: canonical.clone(),
                    description,
                    tags,
                    last_synced: None,
                    summary: None,
                },
            )?;
            save(&cfg, &config_path)?;
            println!("registered {name} → {canonical}");
        }
        FleetCmd::Remove { name } => {
            remove_app(&mut cfg, &name)?;
            save(&cfg, &config_path)?;
            println!("removed {name}");
        }
        FleetCmd::List { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&cfg)?);
                return Ok(());
            }
            if cfg.apps.is_empty() {
                println!(
                    "fleet is empty — register an app with `belisarius fleet add <name> <path>`"
                );
                return Ok(());
            }
            println!(
                "{:<20}  {:>7}  {:>9}  {:<14}  {:<24}  path",
                "name", "files", "loc", "primary", "last sync"
            );
            for app in &cfg.apps {
                let (files, loc, primary) = app
                    .summary
                    .as_ref()
                    .map(|s| {
                        (
                            s.file_count.to_string(),
                            s.loc.to_string(),
                            s.primary_language.clone().unwrap_or_default(),
                        )
                    })
                    .unwrap_or_else(|| ("—".into(), "—".into(), "—".into()));
                let sync = app
                    .last_synced
                    .map(|t| t.format(&time::format_description::well_known::Iso8601::DEFAULT))
                    .transpose()
                    .ok()
                    .flatten()
                    .map(|s| s.split('T').next().unwrap_or(&s).to_string())
                    .unwrap_or_else(|| "—".to_string());
                println!(
                    "{:<20}  {:>7}  {:>9}  {:<14}  {:<24}  {}",
                    truncate(&app.name, 20),
                    files,
                    loc,
                    truncate(&primary, 14),
                    truncate(&sync, 24),
                    app.path
                );
            }
        }
        FleetCmd::Sync { name } => {
            let targets: Vec<String> = match name {
                Some(n) => {
                    if find_app(&cfg, &n).is_none() {
                        return Err(anyhow::anyhow!("no app named {n:?} in the fleet"));
                    }
                    vec![n]
                }
                None => cfg.apps.iter().map(|a| a.name.clone()).collect(),
            };
            sync_in_parallel(&mut cfg, &targets).await?;
            save(&cfg, &config_path)?;
        }
        FleetCmd::Info { name } => {
            let app =
                find_app(&cfg, &name).ok_or_else(|| anyhow::anyhow!("no app named {name:?}"))?;
            println!("{}", serde_json::to_string_pretty(app)?);
        }
        FleetCmd::Find {
            pattern,
            kind,
            limit,
            json,
        } => {
            let conn = fleet_db::open(&fleet_db::default_db_path())?;
            let rows = fleet_db::find_surface(&conn, kind.as_deref(), Some(&pattern), limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("no matches — run `belisarius fleet sync` first if you haven't");
                return Ok(());
            }
            println!(
                "{:<14}  {:<7}  {:<14}  {:<40}  file:line",
                "app", "method", "kind", "name"
            );
            for r in &rows {
                println!(
                    "{:<14}  {:<7}  {:<14}  {:<40}  {}:{}",
                    truncate(&r.app, 14),
                    r.method.clone().unwrap_or_default(),
                    r.kind,
                    truncate(&r.name, 40),
                    r.file,
                    r.line,
                );
            }
        }
        FleetCmd::Hotspots { limit, json } => {
            let conn = fleet_db::open(&fleet_db::default_db_path())?;
            let rows = fleet_db::top_hotspots(&conn, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("no hotspots — run `belisarius fleet sync` first");
                return Ok(());
            }
            let has_owners = rows.iter().any(|r| !r.owners.is_empty());
            if has_owners {
                println!(
                    "{:>6}  {:<14}  {:>5}  {:>5}  {:<18}  {:<22}  file",
                    "score", "app", "churn", "cc", "last commit by", "owners"
                );
                for r in &rows {
                    let owners = if r.owners.is_empty() {
                        "—".into()
                    } else {
                        r.owners.join(" ")
                    };
                    println!(
                        "{:>6.0}  {:<14}  {:>5}  {:>5}  {:<18}  {:<22}  {}",
                        r.score,
                        truncate(&r.app, 14),
                        r.churn,
                        r.complexity,
                        truncate(r.last_author.as_deref().unwrap_or("—"), 18),
                        truncate(&owners, 22),
                        r.file
                    );
                }
            } else {
                println!(
                    "{:>6}  {:<14}  {:>5}  {:>5}  {:<22}  file",
                    "score", "app", "churn", "cc", "last commit by"
                );
                for r in &rows {
                    println!(
                        "{:>6.0}  {:<14}  {:>5}  {:>5}  {:<22}  {}",
                        r.score,
                        truncate(&r.app, 14),
                        r.churn,
                        r.complexity,
                        r.last_author.clone().unwrap_or_else(|| "—".into()),
                        r.file
                    );
                }
            }
        }
        FleetCmd::TestGaps { limit, json } => {
            let conn = fleet_db::open(&fleet_db::default_db_path())?;
            let rows = fleet_db::top_test_gaps(&conn, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("no untested files — run `belisarius fleet sync` first");
                return Ok(());
            }
            println!(
                "{:<14}  {:>5}  {:>5}  {:>5}  {:<12}  file",
                "app", "loc", "fns", "cc", "lang"
            );
            for r in &rows {
                println!(
                    "{:<14}  {:>5}  {:>5}  {:>5}  {:<12}  {}",
                    truncate(&r.app, 14),
                    r.loc,
                    r.function_count,
                    r.total_cyclomatic,
                    truncate(&r.language, 12),
                    r.file,
                );
            }
        }
        FleetCmd::Diff { a, b, json } => {
            let conn = fleet_db::open(&fleet_db::default_db_path())?;
            let diff = fleet_db::surface_diff(&conn, &a, &b)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&diff)?);
                return Ok(());
            }
            println!("common items: {}", diff.common_count);
            println!("\nonly in {a} ({}):", diff.only_in_a.len());
            for e in &diff.only_in_a {
                println!(
                    "  {:<14}  {:<6}  {}",
                    e.kind,
                    e.method.clone().unwrap_or_default(),
                    e.name
                );
            }
            println!("\nonly in {b} ({}):", diff.only_in_b.len());
            for e in &diff.only_in_b {
                println!(
                    "  {:<14}  {:<6}  {}",
                    e.kind,
                    e.method.clone().unwrap_or_default(),
                    e.name
                );
            }
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

/// Run `fleet sync` for many apps in parallel. Each task owns its own
/// SQLite connection (WAL mode handles concurrent writers) and does the
/// full analyze + populate-tables pipeline on a blocking thread; the join
/// merges the refreshed `FleetApp` summaries back into the config.
///
/// Concurrency is bounded — `BELISARIUS_FLEET_SYNC_PAR` overrides the
/// default of `min(num_cpus, 8)`. Going wider rarely helps because the
/// per-task work is CPU-bound (tree-sitter + tokei) and disk-heavy.
async fn sync_in_parallel(cfg: &mut crate::fleet::FleetConfig, targets: &[String]) -> Result<()> {
    let cap = std::env::var("BELISARIUS_FLEET_SYNC_PAR")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8)
        });
    let sem = Arc::new(Semaphore::new(cap.max(1)));

    let mut handles = Vec::with_capacity(targets.len());
    for name in targets {
        let Some(app) = find_app(cfg, name) else {
            continue;
        };
        let app_clone = app.clone();
        let sem_clone = sem.clone();
        let name_owned = name.clone();
        let handle =
            tokio::task::spawn_blocking(move || -> (String, Result<(FleetApp, String)>) {
                let _permit = sem_clone.try_acquire_owned().ok();
                let result = sync_one(app_clone);
                (name_owned, result)
            });
        handles.push(handle);
    }

    let pb = crate::progress::bar_for(handles.len() as u64, false);
    if let Some(b) = &pb {
        b.set_prefix("sync");
        b.set_message("apps");
    }
    for h in handles {
        match h.await {
            Ok((name, Ok((updated, line)))) => {
                if let Some(slot) = cfg.apps.iter_mut().find(|a| a.name == name) {
                    *slot = updated;
                }
                if let Some(b) = &pb {
                    b.println(line);
                } else {
                    println!("{line}");
                }
            }
            Ok((name, Err(e))) => {
                let msg = format!("synced {name:<20}  ERROR: {e:#}");
                if let Some(b) = &pb {
                    b.println(msg);
                } else {
                    println!("{msg}");
                }
            }
            Err(e) => {
                let msg = format!("sync task panicked: {e}");
                if let Some(b) = &pb {
                    b.println(msg);
                } else {
                    println!("{msg}");
                }
            }
        }
        if let Some(b) = &pb {
            b.inc(1);
        }
    }
    if let Some(b) = pb {
        b.finish_and_clear();
    }
    Ok(())
}

/// Per-app sync: refresh the lightweight summary in the TOML config AND
/// populate the cross-app SQLite tables. Returns the updated FleetApp plus
/// a human-readable status line for the CLI to print.
fn sync_one(mut app: FleetApp) -> Result<(FleetApp, String)> {
    refresh_summary(&mut app)?;
    let project_path = std::path::Path::new(&app.path);
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_default();

    let mut db = fleet_db::open(&fleet_db::default_db_path()).context("opening fleet DB")?;
    fleet_db::upsert_app(&db, &app.name, &app.path, &now)?;

    if let Ok(report) = belisarius_scan::analyze(&app.path) {
        if let Ok(surface) = belisarius_scan::surface::extract(project_path, &report.scan) {
            fleet_db::replace_surface(&mut db, &app.name, &surface.items)?;
        }
        fleet_db::replace_file_complexity(
            &mut db,
            &app.name,
            &report.scan.files,
            &report.file_metrics,
        )?;
        let inline = belisarius_scan::test_map::detect_inline_tests(project_path, &report.scan);
        let tmap = belisarius_scan::test_map::compute(&report, &inline);
        fleet_db::replace_test_mappings(&mut db, &app.name, &tmap.mappings)?;

        let keep: Vec<String> = report.scan.files.iter().map(|f| f.path.clone()).collect();
        if let Ok(git) = belisarius_scan::git_stats::collect(project_path, 90, Some(&keep)) {
            if git.repo_present {
                let mut hot =
                    belisarius_scan::git_stats::rank_hotspots(&git, &report.file_metrics, 100);
                let co = belisarius_scan::codeowners::CodeownersFile::load(project_path);
                belisarius_scan::git_stats::attach_owners(&mut hot, co.as_ref());
                fleet_db::replace_hotspots(&mut db, &app.name, 90, &hot.hotspots)?;
            }
        }
    }

    let s = app.summary.as_ref().expect("refresh_summary populated it");
    let line = format!(
        "synced {:<20}  {} files · {} loc · {}",
        app.name,
        s.file_count,
        s.loc,
        s.primary_language.as_deref().unwrap_or("?")
    );
    Ok((app, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::load;
    use std::path::Path;

    /// Build a fleet-friendly project on disk and a temp fleet.toml path.
    /// Both are returned as a `tempfile::TempDir` so they're cleaned up at
    /// the end of the test. Using one tempdir for both keeps every test
    /// hermetic and avoids leaking state into `~/.belisarius/fleet.toml`.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg = tmp.path().join("fleet.toml");
        let proj = tmp.path().join("app1");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(
            proj.join("Cargo.toml"),
            "[package]\nname = \"app1\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        std::fs::write(proj.join("src").join("lib.rs"), "pub fn hello() {}\n").unwrap();
        (tmp, cfg, proj)
    }

    fn args(cfg: &Path, cmd: FleetCmd) -> FleetArgs {
        FleetArgs {
            fleet_config: Some(cfg.to_path_buf()),
            cmd,
        }
    }

    // ── add ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_persists_registered_app() {
        let (_tmp, cfg, proj) = fixture();
        run(args(
            &cfg,
            FleetCmd::Add {
                name: "app1".into(),
                path: proj.clone(),
                description: Some("test app".into()),
                tags: Some("rust,test".into()),
            },
        ))
        .await
        .expect("add should succeed");

        let loaded = load(&cfg).expect("config must exist after add");
        assert_eq!(loaded.apps.len(), 1);
        let app = &loaded.apps[0];
        assert_eq!(app.name, "app1");
        assert_eq!(app.description.as_deref(), Some("test app"));
        assert_eq!(app.tags, vec!["rust".to_string(), "test".to_string()]);
        // path must have been canonicalized (no '..', no symlinks).
        assert!(
            !app.path.contains(".."),
            "registered path should be canonical, got {:?}",
            app.path
        );
    }

    #[tokio::test]
    async fn add_rejects_nonexistent_path() {
        let (_tmp, cfg, _proj) = fixture();
        let err = run(args(
            &cfg,
            FleetCmd::Add {
                name: "ghost".into(),
                path: PathBuf::from("/this/path/does/not/exist/zz"),
                description: None,
                tags: None,
            },
        ))
        .await
        .expect_err("missing path must fail canonicalize");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("resolving") || msg.contains("/this/path"),
            "error should mention path resolution, got: {msg}"
        );
    }

    #[tokio::test]
    async fn add_duplicate_name_is_rejected() {
        let (_tmp, cfg, proj) = fixture();
        run(args(
            &cfg,
            FleetCmd::Add {
                name: "dup".into(),
                path: proj.clone(),
                description: None,
                tags: None,
            },
        ))
        .await
        .unwrap();
        let err = run(args(
            &cfg,
            FleetCmd::Add {
                name: "dup".into(),
                path: proj.clone(),
                description: None,
                tags: None,
            },
        ))
        .await
        .expect_err("duplicate name must be rejected");
        assert!(format!("{err:#}").to_lowercase().contains("dup"));
    }

    // ── tag splitting ────────────────────────────────────────────────────

    /// `--tags "a, b ,c"` must yield `["a","b","c"]` — whitespace trimmed,
    /// empties dropped. Exercised via the public `run` path so the
    /// splitter inside `cmd_fleet` stays correct.
    #[tokio::test]
    async fn tags_are_trimmed_and_split() {
        let (_tmp, cfg, proj) = fixture();
        run(args(
            &cfg,
            FleetCmd::Add {
                name: "tagged".into(),
                path: proj,
                description: None,
                tags: Some(" a, b ,c ,,d".into()),
            },
        ))
        .await
        .unwrap();
        let loaded = load(&cfg).unwrap();
        assert_eq!(
            loaded.apps[0].tags,
            vec!["a".to_string(), "b".into(), "c".into(), "d".into()]
        );
    }

    // ── remove ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn remove_unknown_name_errors() {
        let (_tmp, cfg, _proj) = fixture();
        let err = run(args(
            &cfg,
            FleetCmd::Remove {
                name: "never-added".into(),
            },
        ))
        .await
        .expect_err("removing a nonexistent app must error");
        let msg = format!("{err:#}").to_lowercase();
        assert!(
            msg.contains("never-added") || msg.contains("not"),
            "got: {msg}"
        );
    }

    #[tokio::test]
    async fn add_then_remove_round_trip() {
        let (_tmp, cfg, proj) = fixture();
        run(args(
            &cfg,
            FleetCmd::Add {
                name: "ephemeral".into(),
                path: proj,
                description: None,
                tags: None,
            },
        ))
        .await
        .unwrap();
        assert_eq!(load(&cfg).unwrap().apps.len(), 1);
        run(args(
            &cfg,
            FleetCmd::Remove {
                name: "ephemeral".into(),
            },
        ))
        .await
        .unwrap();
        assert_eq!(load(&cfg).unwrap().apps.len(), 0);
    }

    // ── info ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn info_unknown_name_errors() {
        let (_tmp, cfg, _proj) = fixture();
        let err = run(args(
            &cfg,
            FleetCmd::Info {
                name: "missing".into(),
            },
        ))
        .await
        .expect_err("info on unknown app must error");
        assert!(format!("{err:#}").to_lowercase().contains("missing"));
    }

    // ── list ─────────────────────────────────────────────────────────────

    /// `List` is mostly stdout work, but on an empty config it must still
    /// succeed (helpful first-run behavior — a hard error here would be
    /// hostile to anyone running `belisarius fleet list` on a fresh install).
    #[tokio::test]
    async fn list_on_empty_config_succeeds() {
        let (_tmp, cfg, _proj) = fixture();
        run(args(&cfg, FleetCmd::List { json: false }))
            .await
            .expect("empty fleet list must not error");
    }

    #[tokio::test]
    async fn list_json_on_populated_config_succeeds() {
        let (_tmp, cfg, proj) = fixture();
        run(args(
            &cfg,
            FleetCmd::Add {
                name: "a".into(),
                path: proj,
                description: None,
                tags: None,
            },
        ))
        .await
        .unwrap();
        run(args(&cfg, FleetCmd::List { json: true }))
            .await
            .expect("populated json list must succeed");
    }
}
