use anyhow::{Context, Result};
use belisarius_symbols::SymbolStore;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SymbolsCmd {
    /// Print a structural summary of a `.scip` index file.
    Inspect(InspectArgs),
    /// List references to a symbol, grouped by file.
    Refs(RefsArgs),
    /// List callers of a symbol (enclosing definitions that contain refs to it).
    Callers(CallersArgs),
    /// Substring search over symbol ids + display names.
    Search(SearchArgs),
    /// Summary of activity in one file (defs, incoming refs, outgoing refs).
    File(FileArgs),
}

#[derive(clap::Args)]
pub struct InspectArgs {
    pub path: PathBuf,
    #[arg(long, default_value_t = 20)]
    pub top: usize,
    #[arg(long, default_value_t = 10)]
    pub docs: usize,
}

#[derive(clap::Args)]
pub struct RefsArgs {
    pub symbol: String,
    /// Path to the SCIP index. Defaults to `.belisarius/scip/merged.scip` in CWD.
    #[arg(long, default_value = ".belisarius/scip/merged.scip")]
    pub index: PathBuf,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(clap::Args)]
pub struct CallersArgs {
    pub symbol: String,
    #[arg(long, default_value = ".belisarius/scip/merged.scip")]
    pub index: PathBuf,
}

#[derive(clap::Args)]
pub struct SearchArgs {
    pub query: String,
    #[arg(long, default_value = ".belisarius/scip/merged.scip")]
    pub index: PathBuf,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(clap::Args)]
pub struct FileArgs {
    pub path: String,
    #[arg(long, default_value = ".belisarius/scip/merged.scip")]
    pub index: PathBuf,
}

pub async fn run(cmd: SymbolsCmd) -> Result<()> {
    match cmd {
        SymbolsCmd::Inspect(args) => inspect(args),
        SymbolsCmd::Refs(args) => refs(args),
        SymbolsCmd::Callers(args) => callers(args),
        SymbolsCmd::Search(args) => search(args),
        SymbolsCmd::File(args) => file_cmd(args),
    }
}

fn load(path: &PathBuf) -> Result<SymbolStore> {
    SymbolStore::from_path(path).with_context(|| format!("loading {}", path.display()))
}

fn inspect(args: InspectArgs) -> Result<()> {
    let store = load(&args.path)?;
    println!("scip index   {}", args.path.display());
    println!("documents    {}", store.document_count());
    println!("symbols      {}", store.symbol_count());
    if let Some(m) = &store.index.metadata {
        if let Some(t) = &m.tool_info {
            println!("toolinfo     {} {}", t.name, t.version);
        }
        println!("project_root {}", m.project_root);
    }
    println!("\ntop {} symbols by occurrence count:", args.top);
    for (sym, count) in store.top_symbols(args.top) {
        let display = store
            .info_for(&sym)
            .map(|i| i.display_name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        println!("  {:>5}  {}  {}", count, display, truncate(&sym, 96));
    }
    println!("\nfirst {} documents:", args.docs);
    for (i, doc) in store.documents().take(args.docs).enumerate() {
        println!(
            "  {:>3}. {:80} {} symbols · {} occurrences",
            i + 1,
            truncate(&doc.relative_path, 80),
            doc.symbols.len(),
            doc.occurrences.len()
        );
    }
    Ok(())
}

fn refs(args: RefsArgs) -> Result<()> {
    let store = load(&args.index)?;
    let grouped = store.refs_by_file(&args.symbol);
    if grouped.is_empty() {
        println!("no references found for {}", args.symbol);
        return Ok(());
    }
    let total: usize = grouped.values().map(|v| v.len()).sum();
    println!("{} references across {} files:", total, grouped.len());
    let mut shown = 0;
    for (path, refs) in &grouped {
        println!("\n  {}", path);
        for r in refs {
            if shown >= args.limit {
                println!("    … {} more (use --limit to see)", total - shown);
                return Ok(());
            }
            let range = r.range();
            println!(
                "    {}:{}:{}",
                path,
                range.start_line + 1,
                range.start_char + 1
            );
            shown += 1;
        }
    }
    Ok(())
}

fn callers(args: CallersArgs) -> Result<()> {
    let store = load(&args.index)?;
    let cs = store.callers_of(&args.symbol);
    if cs.is_empty() {
        println!(
            "no callers found for {} (note: requires the indexer to emit enclosing_range)",
            args.symbol
        );
        return Ok(());
    }
    println!("{} caller(s) for {}:", cs.len(), args.symbol);
    for c in &cs {
        let display = c
            .info
            .map(|i| i.display_name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        println!(
            "\n  {}  ({} call site{})",
            display,
            c.call_sites.len(),
            if c.call_sites.len() == 1 { "" } else { "s" }
        );
        println!("    sym: {}", truncate(&c.symbol, 110));
        for site in &c.call_sites {
            let r = site.range();
            println!(
                "    {}:{}:{}",
                site.path(),
                r.start_line + 1,
                r.start_char + 1
            );
        }
    }
    Ok(())
}

fn search(args: SearchArgs) -> Result<()> {
    let store = load(&args.index)?;
    let hits = store.find_symbols(&args.query, args.limit);
    if hits.is_empty() {
        println!("no symbols match {:?}", args.query);
        return Ok(());
    }
    println!("{} matches for {:?}:", hits.len(), args.query);
    for h in hits {
        let display = h.info.map(|i| i.display_name.as_str()).unwrap_or("");
        println!(
            "  {:>5}  {:30}  {}",
            h.occurrences,
            truncate(display, 30),
            truncate(h.symbol, 90)
        );
    }
    Ok(())
}

fn file_cmd(args: FileArgs) -> Result<()> {
    let store = load(&args.index)?;
    let summary = match store.file_summary(&args.path) {
        Some(s) => s,
        None => {
            println!("no document in the index matches path {:?}", args.path);
            return Ok(());
        }
    };
    println!("file        {}", summary.path);
    println!("definitions {}", summary.definition_count);
    println!(
        "incoming    {} refs from other files",
        summary.incoming_refs
    );
    println!(
        "outgoing    {} refs to other symbols",
        summary.outgoing_refs
    );
    println!("total occs  {}", summary.total_occurrences);
    println!("\ndefined symbols:");
    for sym in summary.defines.iter().take(60) {
        let display = if sym.display_name.is_empty() {
            "(anon)"
        } else {
            sym.display_name.as_str()
        };
        println!(
            "  {:30}  {}",
            truncate(display, 30),
            truncate(&sym.symbol, 90)
        );
    }
    if summary.defines.len() > 60 {
        println!("  … {} more", summary.defines.len() - 60);
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (n - 1)..])
    }
}
