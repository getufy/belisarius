//! `belisarius mcp install` — wire Belisarius into a client's MCP config
//! without the user opening JSON.
//!
//! Friction model: today a fresh user has to (a) install the binary, (b) find
//! the right config file for their client, (c) hand-edit JSON without breaking
//! sibling entries, (d) restart the client. Most agents and most humans skip
//! steps b–c, the install seems broken, and the user files a "Belisarius
//! doesn't show up in Claude Code" bug.
//!
//! This command does b + c automatically:
//! - For each requested client, resolve its canonical config path.
//! - Read the existing JSON if present (empty `{}` otherwise).
//! - Merge `mcpServers.<name> = { command, args }`. Sibling server entries
//!   and unrelated keys are preserved byte-for-byte where possible.
//! - Write the file back atomically (write-to-temp + rename).
//!
//! `--dry-run` prints the resulting file without touching disk, which is the
//! mode `belisarius init` uses to preview the change before asking the user
//! to confirm.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::mcp_config::{server_entry, ClientKind};

#[derive(clap::Args)]
pub struct InstallArgs {
    /// Which client(s) to wire up. Repeat to install into several at once,
    /// e.g. `--client claude-code --client cursor`. Defaults to every
    /// installed client we detect.
    #[arg(long, value_enum)]
    pub client: Vec<ClientKind>,
    /// Override the resolved `belisarius` binary path written to the config.
    /// Default: the running executable's absolute path.
    #[arg(long)]
    pub bin: Option<String>,
    /// Entry key under `mcpServers`. Default: `belisarius`. Use this when
    /// running multiple Belisarius checkouts side-by-side.
    #[arg(long, default_value = "belisarius")]
    pub name: String,
    /// Print the resulting config without writing.
    #[arg(long)]
    pub dry_run: bool,
    /// Overwrite an existing entry with a different command/args. Without
    /// this flag, conflicting entries are left untouched and reported.
    #[arg(long)]
    pub force: bool,
    /// Emit a JSON summary of what changed (for agent consumption).
    #[arg(long)]
    pub json: bool,
}

#[derive(serde::Serialize)]
pub struct InstallReport {
    pub results: Vec<ClientResult>,
}

#[derive(serde::Serialize)]
pub struct ClientResult {
    pub client: String,
    pub path: String,
    pub status: String,
    pub note: Option<String>,
}

pub fn run(args: InstallArgs) -> Result<()> {
    let bin = args
        .bin
        .clone()
        .or_else(resolve_self)
        .unwrap_or_else(|| "belisarius".to_string());

    let clients = if args.client.is_empty() {
        detected_clients()
    } else {
        args.client.clone()
    };
    if clients.is_empty() {
        anyhow::bail!(
            "No MCP-capable client detected. Pass `--client claude-code|cursor|claude-desktop` \
            explicitly, or run `belisarius mcp config` to print the snippet for manual paste."
        );
    }

    let mut report = InstallReport { results: vec![] };
    for client in clients {
        let path = match client_config_path(client) {
            Some(p) => p,
            None => {
                report.results.push(ClientResult {
                    client: client_name(client).to_string(),
                    path: String::new(),
                    status: "unsupported".to_string(),
                    note: Some("no known config path on this platform".to_string()),
                });
                continue;
            }
        };
        match install_one(&path, &args.name, &bin, args.force, args.dry_run) {
            Ok((status, note)) => report.results.push(ClientResult {
                client: client_name(client).to_string(),
                path: path.to_string_lossy().into_owned(),
                status,
                note,
            }),
            Err(e) => report.results.push(ClientResult {
                client: client_name(client).to_string(),
                path: path.to_string_lossy().into_owned(),
                status: "error".to_string(),
                note: Some(format!("{e:#}")),
            }),
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report, args.dry_run);
    }
    Ok(())
}

fn install_one(
    path: &Path,
    name: &str,
    bin: &str,
    force: bool,
    dry_run: bool,
) -> Result<(String, Option<String>)> {
    let mut root: Value = if path.exists() {
        let txt =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if txt.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&txt)
                .with_context(|| format!("parsing existing JSON in {}", path.display()))?
        }
    } else {
        if let Some(parent) = path.parent() {
            if !dry_run {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
        }
        Value::Object(Map::new())
    };
    if !root.is_object() {
        anyhow::bail!("config root at {} is not a JSON object", path.display());
    }

    let entry = server_entry(bin);
    let obj = root.as_object_mut().unwrap();
    let servers = obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        anyhow::bail!(
            "`mcpServers` in {} is not an object; refusing to overwrite",
            path.display()
        );
    }
    let map = servers.as_object_mut().unwrap();
    let existing = map.get(name).cloned();
    let status: String;
    let note: Option<String>;
    match existing {
        Some(prev) if prev == entry => {
            status = "unchanged".to_string();
            note = Some("entry already matches".to_string());
        }
        Some(_prev) if !force => {
            status = "conflict".to_string();
            note =
                Some("entry exists with different command/args; re-run with --force".to_string());
        }
        _ => {
            map.insert(name.to_string(), entry);
            status = if dry_run {
                "would-write".to_string()
            } else {
                "wrote".to_string()
            };
            note = None;
            if !dry_run {
                atomic_write_json(path, &root)?;
            }
        }
    }
    Ok((status, note))
}

fn atomic_write_json(path: &Path, value: &Value) -> Result<()> {
    let pretty = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("belisarius.tmp");
    std::fs::write(&tmp, pretty.as_bytes())
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

fn print_human(report: &InstallReport, dry_run: bool) {
    if dry_run {
        println!("dry run — nothing written\n");
    }
    for r in &report.results {
        let mark = match r.status.as_str() {
            "wrote" | "would-write" => "✓",
            "unchanged" => "·",
            "conflict" => "!",
            "unsupported" | "error" => "✗",
            _ => "?",
        };
        let path_label = if r.path.is_empty() {
            "(no path)".to_string()
        } else {
            r.path.clone()
        };
        println!("  {mark} {:<14} {} — {}", r.client, r.status, path_label);
        if let Some(note) = &r.note {
            println!("      {note}");
        }
    }
    println!();
    let conflicts = report
        .results
        .iter()
        .any(|r| r.status == "conflict" || r.status == "error");
    if conflicts {
        println!("Some entries weren't written. Re-run with --force to overwrite conflicts.");
    } else if !dry_run {
        println!("Restart the client(s) above to load the new MCP server.");
    }
}

/// Detect which clients have a config file under their canonical path. We
/// don't check whether they're running — only whether their settings dir
/// exists, which is a good proxy for "this client is installed."
fn detected_clients() -> Vec<ClientKind> {
    [
        ClientKind::ClaudeCode,
        ClientKind::ClaudeDesktop,
        ClientKind::Cursor,
    ]
    .into_iter()
    .filter(|c| {
        client_config_path(*c)
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .map(|p| p.exists())
            .unwrap_or(false)
    })
    .collect()
}

fn client_name(c: ClientKind) -> &'static str {
    match c {
        ClientKind::ClaudeCode => "claude-code",
        ClientKind::ClaudeDesktop => "claude-desktop",
        ClientKind::Cursor => "cursor",
        ClientKind::Generic => "generic",
    }
}

/// Canonical per-user MCP config path on the current OS. Returns `None`
/// when we don't know where this client stores its settings on this
/// platform — caller falls back to `mcp config` for a copy-paste flow.
fn client_config_path(c: ClientKind) -> Option<PathBuf> {
    let home = dirs_home()?;
    Some(match c {
        // Claude Code CLI keeps user-scoped MCP servers in `~/.claude.json`.
        // Project-scoped servers live in `.mcp.json` at the project root — we
        // don't touch those without an explicit `--project` flag (TODO).
        ClientKind::ClaudeCode => home.join(".claude.json"),
        ClientKind::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support/Claude/claude_desktop_config.json")
            }
            #[cfg(target_os = "windows")]
            {
                home.join("AppData/Roaming/Claude/claude_desktop_config.json")
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                home.join(".config/Claude/claude_desktop_config.json")
            }
        }
        // Cursor's `~/.cursor/mcp.json` is the global slot. Project-scope
        // would be `.cursor/mcp.json` next to the repo.
        ClientKind::Cursor => home.join(".cursor/mcp.json"),
        ClientKind::Generic => return None,
    })
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn resolve_self() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// Walk the merge logic by hand against a real temp file so we exercise
    /// both the JSON shape and the atomic-write path. install_one is the
    /// kernel of the command; if it's right, the rest is glue.
    #[test]
    fn install_one_writes_into_fresh_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let (status, _note) =
            install_one(&p, "belisarius", "/usr/local/bin/belisarius", false, false).unwrap();
        assert_eq!(status, "wrote");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["belisarius"],
            json!({ "command": "/usr/local/bin/belisarius", "args": ["mcp"] })
        );
    }

    #[test]
    fn install_one_preserves_sibling_entries() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let existing = json!({
            "mcpServers": {
                "other-server": { "command": "/usr/bin/other", "args": [] }
            },
            "unrelated_key": "untouched"
        });
        std::fs::write(&p, serde_json::to_string_pretty(&existing).unwrap()).unwrap();
        let (status, _) = install_one(&p, "belisarius", "/x/belisarius", false, false).unwrap();
        assert_eq!(status, "wrote");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        // Sibling untouched.
        assert_eq!(v["mcpServers"]["other-server"]["command"], "/usr/bin/other");
        assert_eq!(v["unrelated_key"], "untouched");
        // Our entry landed.
        assert_eq!(v["mcpServers"]["belisarius"]["command"], "/x/belisarius");
    }

    #[test]
    fn install_one_detects_no_op_when_entry_matches() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        install_one(&p, "belisarius", "/x/belisarius", false, false).unwrap();
        let (status, _) = install_one(&p, "belisarius", "/x/belisarius", false, false).unwrap();
        assert_eq!(status, "unchanged");
    }

    #[test]
    fn install_one_refuses_to_overwrite_without_force() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        install_one(&p, "belisarius", "/x/belisarius", false, false).unwrap();
        let (status, note) = install_one(&p, "belisarius", "/y/belisarius", false, false).unwrap();
        assert_eq!(status, "conflict");
        assert!(note.unwrap().contains("--force"));
    }

    #[test]
    fn install_one_force_overwrites_conflict() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        install_one(&p, "belisarius", "/x/belisarius", false, false).unwrap();
        let (status, _) = install_one(&p, "belisarius", "/y/belisarius", true, false).unwrap();
        assert_eq!(status, "wrote");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["belisarius"]["command"], "/y/belisarius");
    }

    #[test]
    fn install_one_dry_run_doesnt_touch_disk() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let (status, _) = install_one(&p, "belisarius", "/x/belisarius", false, true).unwrap();
        assert_eq!(status, "would-write");
        assert!(!p.exists(), "dry-run must not create the file");
    }
}
