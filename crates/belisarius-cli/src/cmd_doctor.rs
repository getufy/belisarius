//! `belisarius doctor` — environment health check.
//!
//! Probes everything Belisarius needs to actually work end-to-end:
//!   - per-language SCIP indexers (`rust-analyzer`, `scip-typescript`, …)
//!   - `.belisarius/` directory presence
//!   - SQLite state DB (pins, snapshots) — can it open?
//!   - hybrid search index status
//!   - `rules.toml` parsability
//!   - fleet config presence
//!
//! Returns exit 0 when every probe is OK and prints a concise table.
//! `--json` emits the same shape an MCP `belisarius_doctor` tool would
//! return (registered in a later phase).

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use crate::service::context::AppContext;

#[derive(clap::Args)]
pub struct DoctorArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Emit JSON instead of the human table.
    #[arg(long)]
    pub json: bool,
}

#[derive(serde::Serialize)]
pub struct DoctorReport {
    pub project: String,
    pub indexers: Vec<IndexerProbe>,
    pub belisarius_dir: DirProbe,
    pub state_db: StateDbProbe,
    pub search_index: SearchProbe,
    pub rules: RulesProbe,
    pub fleet_config: FleetProbe,
    /// Count of probes that are not OK. Doctor exits non-zero when > 0.
    pub problems: u32,
}

#[derive(serde::Serialize)]
pub struct IndexerProbe {
    pub name: String,
    pub language: String,
    pub binary: String,
    pub installed: bool,
    pub applies: bool,
    pub status: String,
}

#[derive(serde::Serialize)]
pub struct DirProbe {
    pub path: String,
    pub exists: bool,
}

#[derive(serde::Serialize)]
pub struct StateDbProbe {
    pub path: String,
    pub exists: bool,
    pub openable: bool,
    pub schema_version: Option<i32>,
}

#[derive(serde::Serialize)]
pub struct SearchProbe {
    pub indexed: bool,
    pub status: String,
}

#[derive(serde::Serialize)]
pub struct RulesProbe {
    pub present: bool,
    pub path: Option<String>,
    pub parseable: bool,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct FleetProbe {
    pub path: String,
    pub exists: bool,
    pub apps_count: Option<u32>,
}

impl crate::output::Renderable for DoctorReport {
    fn render_human(&self, w: &mut dyn std::io::Write) -> std::io::Result<()> {
        writeln!(w, "project    {}", self.project)?;
        writeln!(w)?;
        writeln!(w, "indexers")?;
        for ix in &self.indexers {
            let marker = if ix.status == "ready" { "ok" } else { "--" };
            writeln!(
                w,
                "  {marker:>3}  {:14}  {:18}  {}",
                ix.language, ix.binary, ix.status
            )?;
        }
        writeln!(w)?;
        writeln!(
            w,
            ".belisarius/  {}  ({})",
            yes_no(self.belisarius_dir.exists),
            self.belisarius_dir.path
        )?;
        let ver = self
            .state_db
            .schema_version
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".into());
        writeln!(
            w,
            "state.db      {}  schema=v{ver}  ({})",
            yes_no(self.state_db.openable),
            self.state_db.path
        )?;
        writeln!(
            w,
            "search index  {}  ({})",
            yes_no(self.search_index.indexed),
            self.search_index.status
        )?;
        let rules_label = if !self.rules.present {
            "absent".to_string()
        } else if self.rules.parseable {
            "ok".to_string()
        } else {
            format!(
                "parse error: {}",
                self.rules.error.as_deref().unwrap_or("?")
            )
        };
        writeln!(w, "rules.toml    {rules_label}")?;
        let fleet_label = match self.fleet_config.apps_count {
            Some(n) => format!("{n} app(s)"),
            None if self.fleet_config.exists => "present but unreadable".to_string(),
            None => "absent".to_string(),
        };
        writeln!(w, "fleet.toml    {fleet_label}")?;
        writeln!(w)?;
        if self.problems == 0 {
            writeln!(w, "ok: all probes pass")?;
        } else {
            writeln!(w, "{} probe(s) need attention", self.problems)?;
        }
        Ok(())
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        " no"
    }
}

pub async fn run(args: DoctorArgs) -> Result<()> {
    let ctx = Arc::new(AppContext::new());
    let project = std::fs::canonicalize(&args.path).unwrap_or_else(|_| args.path.clone());
    let project_str = project.to_string_lossy().into_owned();
    let bel_dir = project.join(".belisarius");
    let state_db_path = bel_dir.join("state.db");
    let fleet_cfg_path = bel_dir.join("fleet.toml");

    let mut problems = 0u32;

    // ── SCIP indexer probes ─────────────────────────────────────────────
    let indexers: Vec<IndexerProbe> = belisarius_symbols::indexer::registry()
        .iter()
        .map(|ix| {
            let installed = ix.is_installed();
            let applies = ix.applies_to(&project);
            let status = match ix.status(&project) {
                belisarius_symbols::indexer::IndexerStatus::Ready => "ready",
                belisarius_symbols::indexer::IndexerStatus::NotInstalled => "not_installed",
                belisarius_symbols::indexer::IndexerStatus::DoesNotApply => "does_not_apply",
            };
            // Only count "not_installed" as a problem if the language applies.
            if applies && !installed {
                problems += 1;
            }
            IndexerProbe {
                name: ix.name().to_string(),
                language: ix.language().to_string(),
                binary: ix.binary().to_string(),
                installed,
                applies,
                status: status.to_string(),
            }
        })
        .collect();

    // ── .belisarius/ directory ───────────────────────────────────────────
    let bel_dir_exists = bel_dir.is_dir();
    if !bel_dir_exists {
        problems += 1;
    }
    let belisarius_dir = DirProbe {
        path: bel_dir.to_string_lossy().into_owned(),
        exists: bel_dir_exists,
    };

    // ── state.db probe ───────────────────────────────────────────────────
    let state_db_exists = state_db_path.is_file();
    let (state_db_openable, schema_version) = if state_db_exists {
        match crate::state_db::open(&project) {
            Ok(conn) => {
                let ver: Option<i32> = conn
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .ok();
                (true, ver)
            }
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };
    if state_db_exists && !state_db_openable {
        problems += 1;
    }
    let state_db = StateDbProbe {
        path: state_db_path.to_string_lossy().into_owned(),
        exists: state_db_exists,
        openable: state_db_openable,
        schema_version,
    };

    // ── search index probe ───────────────────────────────────────────────
    let search_args = crate::service::search::PathArgs {
        path: project_str.clone(),
    };
    let (search_indexed, search_status) =
        match crate::service::search::status(&ctx, search_args).await {
            Ok(v) => {
                let state = v
                    .get("state")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let indexed = state == "indexed" || state == "fresh" || state == "ready";
                (indexed, state)
            }
            Err(e) => (false, format!("error: {e}")),
        };
    let search_index = SearchProbe {
        indexed: search_indexed,
        status: search_status,
    };

    // ── rules.toml probe ─────────────────────────────────────────────────
    let rules = match belisarius_scan::rules::load(&project) {
        Ok(Some((_cfg, path))) => RulesProbe {
            present: true,
            path: Some(path.to_string_lossy().into_owned()),
            parseable: true,
            error: None,
        },
        Ok(None) => RulesProbe {
            present: false,
            path: None,
            parseable: false,
            error: None,
        },
        Err(e) => {
            problems += 1;
            RulesProbe {
                present: true,
                path: Some(bel_dir.join("rules.toml").to_string_lossy().into_owned()),
                parseable: false,
                error: Some(format!("{e:#}")),
            }
        }
    };

    // ── fleet.toml probe ─────────────────────────────────────────────────
    let fleet_config = if fleet_cfg_path.is_file() {
        let apps = std::fs::read_to_string(&fleet_cfg_path)
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .and_then(|v| {
                v.get("apps")
                    .and_then(|x| x.as_array())
                    .map(|a| a.len() as u32)
            });
        FleetProbe {
            path: fleet_cfg_path.to_string_lossy().into_owned(),
            exists: true,
            apps_count: apps,
        }
    } else {
        FleetProbe {
            path: fleet_cfg_path.to_string_lossy().into_owned(),
            exists: false,
            apps_count: None,
        }
    };

    let report = DoctorReport {
        project: project_str,
        indexers,
        belisarius_dir,
        state_db,
        search_index,
        rules,
        fleet_config,
        problems,
    };
    crate::output::emit(&report, args.json)?;
    if !args.json {
        println!();
        println!("Next: belisarius next   # state-aware recommendation");
    }
    if problems > 0 {
        std::process::exit(1);
    }
    Ok(())
}
