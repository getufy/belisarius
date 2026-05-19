//! Git history signals — churn, last edit, authors — per file, scoped to a
//! time window. Combined with `FileMetrics::total_cyclomatic` this produces
//! the classic Tornhill hotspot ranking: code that changes often *and* is
//! complex is the most expensive code in the codebase.
//!
//! Soft-skip everywhere: if there's no `.git` directory we return an empty
//! report rather than failing the scan.

use anyhow::{Context, Result};
use belisarius_core::FileMetrics;
use git2::{DiffOptions, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStats {
    pub root: String,
    pub repo_present: bool,
    pub days_window: u32,
    pub head_oid: Option<String>,
    pub files: Vec<GitFileStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileStat {
    pub path: String,
    /// Commits in the configured window that touched this file.
    pub commits_in_window: u32,
    /// Lifetime commits touching this file (cheap to compute from the same walk).
    pub total_commits: u32,
    #[serde(with = "time::serde::iso8601::option")]
    pub last_edited: Option<OffsetDateTime>,
    #[serde(with = "time::serde::iso8601::option")]
    pub first_edited: Option<OffsetDateTime>,
    /// Author of the *most recent* commit that touched the file. The person
    /// to ask when something here broke. Not necessarily the same as the
    /// most-prolific contributor over the window.
    pub last_author: Option<String>,
    /// Top contributors **inside the configured window only.** Falls back to
    /// lifetime contributors when the window has no commits — surfacing a
    /// departed contributor would mislead callers asking "who owns this code
    /// right now?".
    pub top_authors: Vec<AuthorCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorCount {
    pub name: String,
    pub commits: u32,
}

/// One row in the hotspot ranking: code that's both churning and complex.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotspot {
    pub path: String,
    pub churn: u32, // commits_in_window
    pub total_commits: u32,
    pub complexity: u32, // total cyclomatic across all functions in the file
    pub function_count: u32,
    /// `log2(churn + 1) × max(complexity, 1)`. Log-damping the churn keeps a
    /// one-off rewrite from dominating; multiplying by raw complexity keeps
    /// dumb config files out of the ranking.
    pub score: f32,
    #[serde(with = "time::serde::iso8601::option")]
    pub last_edited: Option<OffsetDateTime>,
    /// Author of the latest commit that touched the file. Use this when the
    /// question is "who do I ask about this file right now?".
    pub last_author: Option<String>,
    /// Most-prolific contributor **in the window**. Use this when the question
    /// is "who's been working on this lately?". Window-scoped so departed
    /// contributors don't dominate the ranking.
    pub top_author: Option<String>,
    /// Owners attached by `.github/CODEOWNERS` (or `CODEOWNERS` / `docs/CODEOWNERS`).
    /// Empty when no rule matches or no CODEOWNERS file exists.
    #[serde(default)]
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotspotsReport {
    pub repo_present: bool,
    pub days_window: u32,
    pub hotspots: Vec<Hotspot>,
}

/// Walk the git history in the configured window and aggregate per-file
/// metadata. Files outside `keep_paths` (when non-empty) are filtered after
/// collection so we don't waste a second walk.
pub fn collect(
    project: &Path,
    days_window: u32,
    keep_paths: Option<&[String]>,
) -> Result<GitStats> {
    let mut report = GitStats {
        root: project.display().to_string(),
        repo_present: false,
        days_window,
        head_oid: None,
        files: Vec::new(),
    };

    let repo = match Repository::discover(project) {
        Ok(r) => r,
        Err(_) => return Ok(report),
    };
    report.repo_present = true;

    let head = repo.head().context("reading HEAD")?;
    let head_commit = head.peel_to_commit().context("HEAD is not a commit")?;
    report.head_oid = Some(head_commit.id().to_string());

    let cutoff_unix = OffsetDateTime::now_utc().unix_timestamp() - (days_window as i64) * 86_400;
    let workdir_root = repo
        .workdir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| project.to_path_buf());
    let scan_canon = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let repo_canon = std::fs::canonicalize(&workdir_root).unwrap_or(workdir_root);
    // If the scanned directory is a *subdirectory* of the repo root, we need
    // to prefix-filter paths so we only attribute commits that touched it.
    let path_prefix = scan_canon
        .strip_prefix(&repo_canon)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty());

    let keep_set: Option<std::collections::HashSet<&str>> =
        keep_paths.map(|paths| paths.iter().map(|s| s.as_str()).collect());

    let mut revwalk = repo.revwalk().context("creating revwalk")?;
    revwalk.set_sorting(Sort::TIME)?;
    revwalk.push_head().context("pushing HEAD onto revwalk")?;

    let mut per_file: HashMap<String, FileAgg> = HashMap::new();
    let mut diff_opts = DiffOptions::new();
    diff_opts.include_typechange(false).context_lines(0);

    for oid_res in revwalk {
        let oid = match oid_res {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let when_unix = commit.time().seconds();
        let in_window = when_unix >= cutoff_unix;

        let new_tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };
        let diff = match repo.diff_tree_to_tree(
            parent_tree.as_ref(),
            Some(&new_tree),
            Some(&mut diff_opts),
        ) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let author = commit.author().name().unwrap_or("unknown").to_string();
        let when = OffsetDateTime::from_unix_timestamp(when_unix).ok();

        for delta in diff.deltas() {
            let new = match delta.new_file().path() {
                Some(p) => p.to_string_lossy().to_string(),
                None => continue,
            };
            // Filter out paths that aren't inside the scanned directory.
            let Some(rel) = normalize_delta_path(&new, path_prefix.as_deref(), keep_set.as_ref())
            else {
                continue;
            };

            update_file_agg(per_file.entry(rel).or_default(), &author, when, in_window);
        }
    }

    let mut files: Vec<GitFileStat> = per_file
        .into_iter()
        .map(|(path, agg)| {
            // Prefer window contributors; fall back to lifetime only when the
            // window had zero touches (otherwise a long-departed dominant
            // author would still show up as "top").
            let source = if !agg.window_authors.is_empty() {
                agg.window_authors
            } else {
                agg.lifetime_authors
            };
            let mut authors: Vec<AuthorCount> = source
                .into_iter()
                .map(|(name, commits)| AuthorCount { name, commits })
                .collect();
            authors.sort_by(|a, b| b.commits.cmp(&a.commits).then(a.name.cmp(&b.name)));
            authors.truncate(3);
            GitFileStat {
                path,
                commits_in_window: agg.commits_in_window,
                total_commits: agg.total_commits,
                last_edited: agg.last_edited,
                first_edited: agg.first_edited,
                last_author: agg.last_author,
                top_authors: authors,
            }
        })
        .collect();
    files.sort_by(|a, b| {
        b.commits_in_window
            .cmp(&a.commits_in_window)
            .then(b.total_commits.cmp(&a.total_commits))
    });
    report.files = files;
    Ok(report)
}

/// Strip `path_prefix` off a delta's new path, normalize separators, and
/// apply the optional `keep_set` allowlist. Extracted from `collect` to
/// keep its cyclomatic complexity below the project cap.
fn normalize_delta_path(
    new: &str,
    path_prefix: Option<&str>,
    keep_set: Option<&std::collections::HashSet<&str>>,
) -> Option<String> {
    let normalized = if let Some(prefix) = path_prefix {
        new.strip_prefix(&format!("{prefix}/"))
            .map(str::to_string)?
    } else {
        new.to_string()
    };
    let rel = normalized.replace('\\', "/");
    if let Some(keep) = keep_set {
        if !keep.contains(rel.as_str()) {
            return None;
        }
    }
    Some(rel)
}

/// Fold one commit's contribution to a file into its `FileAgg`. Pulled out
/// of the per-delta loop in `collect` for the same complexity-budget reason.
fn update_file_agg(
    entry: &mut FileAgg,
    author: &str,
    when: Option<OffsetDateTime>,
    in_window: bool,
) {
    entry.total_commits += 1;
    if in_window {
        entry.commits_in_window += 1;
    }
    if let Some(w) = when {
        // Revwalk yields newest-first, so the first commit we see is the
        // most recent — capture both timestamp and author.
        if entry.last_edited.is_none_or(|cur| cur < w) {
            entry.last_edited = Some(w);
            entry.last_author = Some(author.to_string());
        }
        if entry.first_edited.is_none_or(|cur| cur > w) {
            entry.first_edited = Some(w);
        }
    }
    *entry
        .lifetime_authors
        .entry(author.to_string())
        .or_default() += 1;
    if in_window {
        *entry.window_authors.entry(author.to_string()).or_default() += 1;
    }
}

/// Combine GitStats with the existing FileMetrics into a Hotspot ranking. We
/// keep only files that have *both* signals (or only complexity for the case
/// of new files that haven't churned yet).
pub fn rank_hotspots(git: &GitStats, file_metrics: &[FileMetrics], limit: usize) -> HotspotsReport {
    let mut metric_by_path: HashMap<&str, &FileMetrics> = HashMap::new();
    for fm in file_metrics {
        metric_by_path.insert(fm.path.as_str(), fm);
    }
    let mut hotspots: Vec<Hotspot> = git
        .files
        .iter()
        .map(|gf| {
            let fm = metric_by_path.get(gf.path.as_str());
            let complexity = fm.map(|m| m.total_cyclomatic).unwrap_or(0);
            let fn_count = fm.map(|m| m.function_count).unwrap_or(0);
            let churn = gf.commits_in_window;
            // log2(churn+1) so a single rewrite doesn't dominate, multiplied by
            // raw complexity so trivial files (configs) sink to the bottom.
            let score = ((churn as f32) + 1.0).log2() * (complexity.max(1) as f32);
            Hotspot {
                path: gf.path.clone(),
                churn,
                total_commits: gf.total_commits,
                complexity,
                function_count: fn_count,
                score,
                last_edited: gf.last_edited,
                last_author: gf.last_author.clone(),
                top_author: gf.top_authors.first().map(|a| a.name.clone()),
                owners: Vec::new(),
            }
        })
        .collect();
    hotspots.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.churn.cmp(&a.churn))
    });
    hotspots.truncate(limit);
    HotspotsReport {
        repo_present: git.repo_present,
        days_window: git.days_window,
        hotspots,
    }
}

/// Attach `owners` to each hotspot row using a project's CODEOWNERS file.
/// No-op when `owners` is `None` (no CODEOWNERS in the project). Idempotent.
pub fn attach_owners(
    report: &mut HotspotsReport,
    owners: Option<&crate::codeowners::CodeownersFile>,
) {
    let Some(co) = owners else { return };
    for h in &mut report.hotspots {
        h.owners = co.owners_for(&h.path).to_vec();
    }
}

#[derive(Default)]
struct FileAgg {
    commits_in_window: u32,
    total_commits: u32,
    last_edited: Option<OffsetDateTime>,
    first_edited: Option<OffsetDateTime>,
    last_author: Option<String>,
    /// Author → commits, scoped to the configured window. Empty when no
    /// in-window commits touched this file.
    window_authors: HashMap<String, u32>,
    /// Author → commits over the file's full history. Used only as fallback
    /// when `window_authors` is empty.
    lifetime_authors: HashMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::FileMetrics;

    fn fm(path: &str, total_cc: u32, fn_count: u32) -> FileMetrics {
        FileMetrics {
            path: path.into(),
            function_count: fn_count,
            max_cyclomatic: total_cc,
            total_cyclomatic: total_cc,
            max_cognitive: total_cc,
            longest_function_loc: 10,
            avg_cyclomatic: total_cc as f32 / fn_count.max(1) as f32,
        }
    }

    fn stat(path: &str, churn: u32, total: u32) -> GitFileStat {
        GitFileStat {
            path: path.into(),
            commits_in_window: churn,
            total_commits: total,
            last_edited: None,
            first_edited: None,
            last_author: Some("alice".into()),
            top_authors: vec![AuthorCount {
                name: "alice".into(),
                commits: churn,
            }],
        }
    }

    #[test]
    fn rank_orders_by_churn_times_complexity() {
        let git = GitStats {
            root: ".".into(),
            repo_present: true,
            days_window: 30,
            head_oid: None,
            files: vec![
                stat("a.rs", 8, 80),  // moderate churn, baseline complexity
                stat("b.rs", 2, 30),  // low churn
                stat("c.rs", 16, 30), // hot but simpler
            ],
        };
        let metrics = vec![fm("a.rs", 200, 5), fm("b.rs", 50, 2), fm("c.rs", 40, 2)];
        let report = rank_hotspots(&git, &metrics, 10);
        // a.rs: log2(9) × 200 ≈ 633.985
        // c.rs: log2(17) × 40  ≈ 163.49
        // b.rs: log2(3) × 50   ≈ 79.2
        let order: Vec<&str> = report.hotspots.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(order, vec!["a.rs", "c.rs", "b.rs"]);
    }

    /// Regression: a former contributor who hasn't committed inside the
    /// configured window must NOT show up as `top_author`. Pre-fix the ranker
    /// counted lifetime commits and surfaced Tom-style departed authors as
    /// "top author" forever.
    #[test]
    fn top_author_is_window_scoped_not_lifetime() {
        use std::collections::HashMap;
        let mut window_authors = HashMap::new();
        window_authors.insert("Active Andy".to_string(), 3_u32);
        window_authors.insert("Recent Rita".to_string(), 1_u32);
        let mut lifetime_authors = window_authors.clone();
        lifetime_authors.insert("Departed Tom".to_string(), 50_u32);

        // Simulate the same aggregation `collect()` performs but without git.
        let agg = FileAgg {
            commits_in_window: 4,
            total_commits: 54,
            last_edited: None,
            first_edited: None,
            last_author: Some("Active Andy".to_string()),
            window_authors,
            lifetime_authors,
        };
        // Run the file-finalization path manually (mirrors the closure in collect).
        let source = if !agg.window_authors.is_empty() {
            agg.window_authors
        } else {
            agg.lifetime_authors
        };
        let mut authors: Vec<AuthorCount> = source
            .into_iter()
            .map(|(name, commits)| AuthorCount { name, commits })
            .collect();
        authors.sort_by(|a, b| b.commits.cmp(&a.commits).then(a.name.cmp(&b.name)));
        authors.truncate(3);

        assert_eq!(authors[0].name, "Active Andy");
        assert!(authors.iter().all(|a| a.name != "Departed Tom"));
    }

    #[test]
    fn missing_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let stats = collect(dir.path(), 30, None).unwrap();
        assert!(!stats.repo_present);
        assert!(stats.files.is_empty());
    }
}
