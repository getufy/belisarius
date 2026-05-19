use crate::languages::language_for_ext;
use anyhow::Result;
use belisarius_core::FileNode;
use ignore::WalkBuilder;
use std::path::Path;

/// Walk the project respecting `.gitignore`. Returns one `FileNode` per file Belisarius
/// considers a "source" file (i.e., we know the language).
pub fn walk(root: &Path) -> Result<Vec<FileNode>> {
    let mut out = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true)
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        let Some(ext) = ext else { continue };
        let Some(lang) = language_for_ext(&ext) else {
            continue;
        };

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        let bytes = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        let loc = crate::diagnostics::tokei::loc_for_file(path);
        out.push(FileNode {
            path: rel,
            language: lang.to_string(),
            loc,
            bytes,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
