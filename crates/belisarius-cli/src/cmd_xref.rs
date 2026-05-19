//! Transitive cross-reference commands: impact (backward), flow (forward),
//! and symbol (360° one-shot view). All operate on `.belisarius/scip/merged.scip`.

use anyhow::{Context, Result};
use belisarius_symbols::SymbolStore;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct ImpactArgs {
    pub symbol: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, default_value_t = 3)]
    pub depth: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct FlowArgs {
    pub symbol: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, default_value_t = 3)]
    pub depth: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct SymbolArgs {
    pub symbol: String,
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long)]
    pub json: bool,
}

fn load_store(project: &Path) -> Result<SymbolStore> {
    let p = project.join(".belisarius/scip/merged.scip");
    SymbolStore::from_path(&p).with_context(|| format!("load scip index at {}", p.display()))
}

pub async fn impact(args: ImpactArgs) -> Result<()> {
    let store = load_store(&args.path)?;
    let report = store.impact_of(&args.symbol, args.depth);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if report.nodes.is_empty() {
        println!(
            "no callers found for {} (depth ≤ {})",
            args.symbol, args.depth
        );
        return Ok(());
    }
    println!(
        "impact of {} — {} symbols across {} files{}",
        args.symbol,
        report.nodes.len(),
        report.files.len(),
        if report.truncated { " (TRUNCATED)" } else { "" }
    );
    for n in &report.nodes {
        let name = if n.display_name.is_empty() {
            &n.symbol
        } else {
            &n.display_name
        };
        println!("  d{}  {:<32}  via {}", n.depth, name, n.callers_of);
    }
    println!("\nfiles touched:");
    for f in &report.files {
        println!("  {f}");
    }
    Ok(())
}

pub async fn flow(args: FlowArgs) -> Result<()> {
    let store = load_store(&args.path)?;
    let report = store.flow_from(&args.symbol, args.depth);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if report.nodes.is_empty() {
        println!(
            "no callees found for {} (depth ≤ {})",
            args.symbol, args.depth
        );
        return Ok(());
    }
    println!(
        "flow from {} — {} symbols{}",
        args.symbol,
        report.nodes.len(),
        if report.truncated { " (TRUNCATED)" } else { "" }
    );
    for n in &report.nodes {
        let name = if n.display_name.is_empty() {
            &n.symbol
        } else {
            &n.display_name
        };
        println!("  d{}  {:<32}  ← {}", n.depth, name, n.called_from);
    }
    Ok(())
}

pub async fn symbol(args: SymbolArgs) -> Result<()> {
    let store = load_store(&args.path)?;
    let v = store.symbol_360(&args.symbol);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let display = if v.display_name.is_empty() {
        &v.symbol
    } else {
        &v.display_name
    };
    println!("symbol         {display}");
    println!("id             {}", v.symbol);
    println!("occurrences    {}", v.occurrence_count);
    println!("def sites      {}", v.def_sites.len());
    for d in &v.def_sites {
        println!("  {}:{}", d.file, d.range.start_line + 1);
    }
    println!("callers        {}", v.callers.len());
    for c in &v.callers {
        let n = if c.display_name.is_empty() {
            &c.symbol
        } else {
            &c.display_name
        };
        println!("  {:<5} call sites  {}", c.call_sites, n);
    }
    println!("callees        {}", v.callees.len());
    for c in &v.callees {
        let n = if c.display_name.is_empty() {
            &c.symbol
        } else {
            &c.display_name
        };
        println!("  {n}");
    }
    Ok(())
}
