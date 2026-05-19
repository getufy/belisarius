//! Branch/PR awareness — surface which files changed between two git refs
//! and overlay the change set on the existing analysis signals.
//!
//! The pure-git half lives in `compute()`: it opens the repository, resolves
//! `base` and `head` to commits (defaulting `base` to the first parent of
//! HEAD and `head` to HEAD itself), and emits a list of `DiffFile` rows.
//!
//! The cross-reference half lives in `overlay()`: given an `AnalysisReport`
//! plus the diff, it picks out the hotspot files, the untested files, and
//! the files exposing public surface — the three signals an agent (or
//! reviewer) wants to know about a PR before reading any code.

use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Repository};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    /// For renames, the previous path. None otherwise.
    pub old_path: Option<String>,
    pub status: DiffStatus,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub repo_present: bool,
    pub base: String,
    pub head: String,
    pub base_resolved: Option<String>,
    pub head_resolved: Option<String>,
    pub files: Vec<DiffFile>,
}

impl DiffReport {
    pub fn empty(base: &str, head: &str) -> Self {
        Self {
            repo_present: false,
            base: base.to_string(),
            head: head.to_string(),
            base_resolved: None,
            head_resolved: None,
            files: Vec::new(),
        }
    }
}

/// Compute a file-level diff between two git refs. `base` defaults to
/// HEAD's first parent if it's empty; `head` defaults to HEAD. Both
/// accept any rev that `git2::Repository::revparse_single` understands —
/// branch name, tag, sha, `HEAD~1`, …
pub fn compute(project: &Path, base: &str, head: &str) -> Result<DiffReport> {
    let repo = match Repository::discover(project) {
        Ok(r) => r,
        Err(_) => return Ok(DiffReport::empty(base, head)),
    };

    let head_ref = if head.is_empty() { "HEAD" } else { head };
    let head_obj = repo
        .revparse_single(head_ref)
        .with_context(|| format!("revparse {head_ref}"))?;
    let head_commit = head_obj
        .peel_to_commit()
        .with_context(|| "head not a commit")?;
    let head_tree = head_commit.tree()?;

    let base_obj = if base.is_empty() {
        // Default base = HEAD's first parent (i.e. "what changed in the
        // latest commit"). When HEAD is the root commit, base stays None
        // and we diff against an empty tree.
        head_commit.parent(0).ok().map(|c| c.into_object())
    } else {
        Some(
            repo.revparse_single(base)
                .with_context(|| format!("revparse {base}"))?,
        )
    };

    let base_tree = match &base_obj {
        Some(obj) => Some(obj.peel_to_commit()?.tree()?),
        None => None,
    };

    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    let diff = match &base_tree {
        Some(bt) => repo.diff_tree_to_tree(Some(bt), Some(&head_tree), Some(&mut opts))?,
        None => repo.diff_tree_to_tree(None, Some(&head_tree), Some(&mut opts))?,
    };

    // Build the file map first (file-level metadata) …
    let mut by_path: std::collections::BTreeMap<String, DiffFile> =
        std::collections::BTreeMap::new();
    diff.foreach(
        &mut |delta, _progress| {
            let status = match delta.status() {
                Delta::Added => DiffStatus::Added,
                Delta::Deleted => DiffStatus::Deleted,
                Delta::Modified | Delta::Typechange => DiffStatus::Modified,
                Delta::Renamed => DiffStatus::Renamed,
                _ => DiffStatus::Other,
            };
            let new_path = delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().to_string());
            let old_path = delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().to_string());
            // For deletes, use the old path as the row identity so we still
            // surface the change. For everything else, the new path wins.
            let key = match (status, new_path.clone(), old_path.clone()) {
                (DiffStatus::Deleted, _, Some(o)) => o,
                (_, Some(n), _) => n,
                (_, None, Some(o)) => o,
                _ => return true,
            };
            by_path.insert(
                key.clone(),
                DiffFile {
                    path: key,
                    old_path: if matches!(status, DiffStatus::Renamed) {
                        old_path
                    } else {
                        None
                    },
                    status,
                    additions: 0,
                    deletions: 0,
                },
            );
            true
        },
        None,
        None,
        None,
    )?;

    // … then a second pass for line counts. git2's borrow checker doesn't
    // allow both closures to share `by_path`, so we keep them separate.
    let mut line_counts: std::collections::HashMap<String, (u32, u32)> =
        std::collections::HashMap::new();
    diff.foreach(
        &mut |_delta, _progress| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            let key = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string());
            let Some(key) = key else { return true };
            let entry = line_counts.entry(key).or_insert((0, 0));
            match line.origin() {
                '+' => entry.0 += 1,
                '-' => entry.1 += 1,
                _ => {}
            }
            true
        }),
    )?;
    for (k, (adds, dels)) in line_counts {
        if let Some(f) = by_path.get_mut(&k) {
            f.additions = adds;
            f.deletions = dels;
        }
    }

    Ok(DiffReport {
        repo_present: true,
        base: base.to_string(),
        head: head_ref.to_string(),
        base_resolved: base_obj.map(|o| o.id().to_string()),
        head_resolved: Some(head_commit.id().to_string()),
        files: by_path.into_values().collect(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOverlay {
    /// Files in the diff that show up in the current hotspots ranking.
    pub hotspot_overlap: Vec<String>,
    /// Files in the diff that have no covering test (after inline-test detection).
    pub untested_changes: Vec<String>,
    /// Files in the diff that expose public surface items (HTTP routes,
    /// exposed functions/types, etc.). API-shape changes hide here.
    pub surface_changes: Vec<String>,
    /// Per-app code-owners that touch any file in the diff. Empty when no
    /// CODEOWNERS file is present.
    pub owners: Vec<String>,
}

/// Cross-reference the diff against a pre-computed AnalysisReport. The
/// caller assembles the report once (typically already cached by the MCP
/// server) and passes references in; this function is pure.
pub fn overlay(
    report: &belisarius_core::AnalysisReport,
    diff: &DiffReport,
    surface: Option<&belisarius_core::SurfaceReport>,
    hotspots: Option<&crate::git_stats::HotspotsReport>,
    inline_tested: &std::collections::HashSet<String>,
    owners_file: Option<&crate::codeowners::CodeownersFile>,
) -> DiffOverlay {
    let changed: HashSet<&str> = diff.files.iter().map(|f| f.path.as_str()).collect();

    let hotspot_overlap: Vec<String> = hotspots
        .map(|h| {
            h.hotspots
                .iter()
                .filter(|x| changed.contains(x.path.as_str()))
                .map(|x| x.path.clone())
                .collect()
        })
        .unwrap_or_default();

    let test_map = crate::test_map::compute(report, inline_tested);
    let gap_set: HashSet<&str> = test_map.gaps.iter().map(|g| g.source.as_str()).collect();
    let mut untested_changes: Vec<String> = changed
        .iter()
        .filter(|p| gap_set.contains(*p))
        .map(|p| (*p).to_string())
        .collect();
    untested_changes.sort();

    let surface_changes: Vec<String> = surface
        .map(|s| {
            let files: HashSet<&str> = s.items.iter().map(|i| i.file.as_str()).collect();
            changed
                .iter()
                .filter(|p| files.contains(*p))
                .map(|p| (*p).to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut owners_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(co) = owners_file {
        for f in &diff.files {
            for o in co.owners_for(&f.path) {
                owners_set.insert(o.clone());
            }
        }
    }

    DiffOverlay {
        hotspot_overlap,
        untested_changes,
        surface_changes,
        owners: owners_set.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_when_no_repo() {
        let r = compute(Path::new("/tmp/does-not-exist-belisarius"), "", "HEAD").unwrap();
        assert!(!r.repo_present);
        assert!(r.files.is_empty());
    }
}
