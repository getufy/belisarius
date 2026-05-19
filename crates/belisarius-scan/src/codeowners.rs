//! Minimal CODEOWNERS parser + matcher.
//!
//! Looks up `.github/CODEOWNERS`, `CODEOWNERS`, or `docs/CODEOWNERS` at the
//! project root (GitHub's documented locations, in order of precedence)
//! and converts each non-comment line into a (glob pattern, owners) pair.
//! `owners_for(path)` returns the owners attached to the **last matching
//! rule** — that's the GitHub semantics: later rules override earlier ones.
//!
//! Patterns are translated to gitignore-style globs and matched with the
//! `globset` crate. We support the cases that show up in 99% of real
//! CODEOWNERS files:
//!
//!   * `*.ext`            — anywhere
//!   * `path/to/file`     — exact, anchored to root
//!   * `/dir/`            — directory, anchored to root
//!   * `dir/`             — directory, anywhere
//!   * `dir/**`           — recursive
//!   * `dir/*`            — single segment
//!
//! Unsupported corner cases (e.g. negation with `!`) are silently skipped
//! rather than crashing on a malformed line.

use globset::{Glob, GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerRule {
    pub pattern: String,
    pub owners: Vec<String>,
}

pub struct CodeownersFile {
    rules: Vec<(OwnerRule, GlobMatcher)>,
}

impl CodeownersFile {
    /// Look for a CODEOWNERS file in the canonical GitHub locations. Returns
    /// `None` when no file exists; returns `Some(_)` with possibly-empty
    /// rules when the file is present but malformed.
    pub fn load(project_root: &Path) -> Option<Self> {
        for rel in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
            let p = project_root.join(rel);
            if let Ok(src) = std::fs::read_to_string(&p) {
                return Some(Self::parse(&src));
            }
        }
        None
    }

    pub fn parse(src: &str) -> Self {
        let mut rules = Vec::new();
        for raw in src.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            // Negation rules aren't supported by GitHub itself anymore;
            // skip to avoid silently producing wrong matches.
            if line.starts_with('!') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(pat) = parts.next() else { continue };
            let owners: Vec<String> = parts.map(|s| s.to_string()).collect();
            if owners.is_empty() {
                continue;
            }
            let Some(matcher) = build_matcher(pat) else {
                continue;
            };
            rules.push((
                OwnerRule {
                    pattern: pat.to_string(),
                    owners,
                },
                matcher,
            ));
        }
        Self { rules }
    }

    /// Returns the owners attached to the most-recently-declared rule whose
    /// pattern matches `relative_path`. Returns an empty slice when no rule
    /// matches.
    pub fn owners_for(&self, relative_path: &str) -> &[String] {
        let normalized = relative_path.trim_start_matches("./");
        let mut chosen: Option<&OwnerRule> = None;
        for (rule, m) in &self.rules {
            if m.is_match(normalized) {
                chosen = Some(rule);
            }
        }
        chosen.map(|r| r.owners.as_slice()).unwrap_or(&[])
    }

    pub fn rules(&self) -> impl Iterator<Item = &OwnerRule> {
        self.rules.iter().map(|(r, _)| r)
    }
}

fn build_matcher(pattern: &str) -> Option<GlobMatcher> {
    let glob = translate(pattern);
    GlobBuilder::new(&glob)
        .literal_separator(true)
        .build()
        .ok()
        .map(|g| g.compile_matcher())
        .or_else(|| Glob::new(&glob).ok().map(|g| g.compile_matcher()))
}

/// Convert GitHub's CODEOWNERS syntax to a gitignore-style glob `globset`
/// understands. The two interesting flips:
///   * leading `/` anchors → drop the slash; `globset` is already anchored.
///   * unanchored patterns → prepend `**/` so they match at any depth.
///   * trailing `/` (directory) → append `**` so descendants match too.
fn translate(pat: &str) -> String {
    let mut p = pat.to_string();
    let anchored = p.starts_with('/');
    if anchored {
        p.remove(0);
    }
    if p.ends_with('/') {
        p.push_str("**");
    }
    // Unanchored patterns without an explicit directory component should
    // match at any depth. A plain `*.rs` becomes `**/*.rs`; a `dir/file`
    // becomes `**/dir/file` (still anchored relatively to that segment).
    if !anchored {
        p = format!("**/{p}");
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_matches_common_patterns() {
        let src = r#"
            # Default owner
            *           @backend

            # Frontend bundle owned separately
            /web/       @frontend
            *.tsx       @frontend @design

            # Specific module overrides
            crates/belisarius-cli/  @cli-maintainers
        "#;
        let co = CodeownersFile::parse(src);

        assert_eq!(co.owners_for("README.md"), &["@backend".to_string()]);
        assert_eq!(
            co.owners_for("web/src/App.tsx"),
            &["@frontend".to_string(), "@design".to_string()]
        );
        assert_eq!(co.owners_for("web/index.html"), &["@frontend".to_string()]);
        assert_eq!(
            co.owners_for("crates/belisarius-cli/src/server.rs"),
            &["@cli-maintainers".to_string()]
        );
        // A path that no rule matches still finds the catch-all `*`.
        assert_eq!(
            co.owners_for("crates/belisarius-scan/src/lib.rs"),
            &["@backend".to_string()]
        );
    }

    #[test]
    fn double_star_works_mid_pattern_and_at_edges() {
        let src = r#"
            src/**/tests/**     @qa
            /web/src/           @frontend
            **/*.proto          @proto-team
        "#;
        let co = CodeownersFile::parse(src);
        assert_eq!(co.owners_for("src/foo/tests/bar.rs"), &["@qa".to_string()]);
        assert_eq!(co.owners_for("src/a/b/c/tests/x.rs"), &["@qa".to_string()]);
        assert_eq!(co.owners_for("src/foo/util.rs"), &[] as &[String]);
        assert_eq!(co.owners_for("web/src/App.tsx"), &["@frontend".to_string()]);
        assert_eq!(co.owners_for("web/index.html"), &[] as &[String]);
        assert_eq!(
            co.owners_for("proto/v1/users.proto"),
            &["@proto-team".to_string()]
        );
    }

    #[test]
    fn handles_comments_blanks_and_bad_lines() {
        let src = r#"
            # this is a comment

            !blocked      @nobody
            # no owners on this line:
            orphan-path
        "#;
        let co = CodeownersFile::parse(src);
        assert_eq!(co.owners_for("anything"), &[] as &[String]);
    }
}
