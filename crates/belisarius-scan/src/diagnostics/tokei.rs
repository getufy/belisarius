//! Tokei wrapper. Tokei is an in-process Rust crate, not a subprocess.
//!
//! Tokei doesn't produce *diagnostics* in the lint sense — it counts lines.
//! We surface its per-language `comments` and `blanks` numbers as informational
//! diagnostics keyed on the project root. The real value comes from the
//! tokei-backed LOC swap in `walker.rs`; this diagnostic is just there so the
//! UI shows Tokei's coverage in the same panel as the other tools.

use anyhow::Result;
use belisarius_core::{Diagnostic, Scan, Severity};
use std::path::Path;
use tokei::{Config, Languages};

use super::Tool;

pub struct TokeiTool;

impl Tool for TokeiTool {
    fn name(&self) -> &'static str {
        "tokei"
    }
    fn binary(&self) -> Option<&'static str> {
        None
    }
    fn applies_to(&self, _scan: &Scan) -> bool {
        true
    }
    fn run(&self, project: &Path, _scan: &Scan) -> Result<Vec<Diagnostic>> {
        let mut langs = Languages::new();
        let cfg = Config::default();
        langs.get_statistics(
            &[project],
            &[".git", "target", "node_modules", "dist"],
            &cfg,
        );

        let mut out = Vec::new();
        for (kind, lang) in langs.iter() {
            if lang.code == 0 && lang.comments == 0 && lang.blanks == 0 {
                continue;
            }
            let name = format!("{kind:?}");
            out.push(Diagnostic {
                tool: "tokei".into(),
                rule_id: format!("loc:{name}"),
                severity: Severity::Info,
                file: ".".into(),
                start_line: 1,
                end_line: 1,
                start_col: 0,
                end_col: 0,
                message: format!(
                    "{name}: {} code, {} comments, {} blanks across {} files",
                    lang.code,
                    lang.comments,
                    lang.blanks,
                    lang.reports.len()
                ),
                help: None,
                url: None,
            });
        }
        Ok(out)
    }
}

/// Counter helper used by `walker::walk` — Tokei-backed LOC for one file,
/// falling back to non-blank line count when Tokei doesn't recognize the
/// extension.
pub fn loc_for_file(path: &Path) -> u32 {
    use tokei::LanguageType;
    let lang = LanguageType::from_path(path, &Config::default());
    if let Some(lt) = lang {
        let cfg = Config::default();
        if let Ok(text) = std::fs::read_to_string(path) {
            let stats = lt.parse_from_str(text, &cfg);
            return stats.code as u32;
        }
    }
    // Fallback: original non-blank line count
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count() as u32)
        .unwrap_or(0)
}
