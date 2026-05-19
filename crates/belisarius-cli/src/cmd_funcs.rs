use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct FuncsArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Print at most this many rows.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Only show functions with cyclomatic ≥ this value.
    #[arg(long, default_value_t = 0)]
    pub min_cc: u32,
    /// Sort by `cc` (default), `cognitive`, `loc`, or `params`.
    #[arg(long, default_value = "cc")]
    pub sort_by: String,
}

pub async fn run(args: FuncsArgs) -> Result<()> {
    let report = belisarius_scan::analyze(&args.path)
        .with_context(|| format!("analyzing {}", args.path.display()))?;
    let mut fns: Vec<_> = report
        .functions
        .into_iter()
        .filter(|f| f.cyclomatic >= args.min_cc)
        .collect();
    fns.sort_by(|a, b| match args.sort_by.as_str() {
        "cognitive" => b.cognitive.cmp(&a.cognitive),
        "loc" => b.loc.cmp(&a.loc),
        "params" => b.params.cmp(&a.params),
        _ => b.cyclomatic.cmp(&a.cyclomatic),
    });
    fns.truncate(args.limit);
    println!(
        "{:>3}  {:>3}  {:>4}  {:>3}  {:<32}  file:line",
        "cc", "cog", "loc", "p", "name"
    );
    for f in fns {
        let name = if f.name.len() > 32 {
            format!("…{}", &f.name[f.name.len() - 31..])
        } else {
            f.name.clone()
        };
        println!(
            "{:>3}  {:>3}  {:>4}  {:>3}  {:<32}  {}:{}",
            f.cyclomatic, f.cognitive, f.loc, f.params, name, f.file, f.start_line
        );
    }
    Ok(())
}
