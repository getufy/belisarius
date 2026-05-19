//! Module resolution: turn raw `ImportEdge` specifiers into resolved file→file edges.
//!
//! Per-language heuristics. We intentionally only resolve specifiers that point
//! inside the scanned project — bare specifiers (`lodash`, `std::*`, `os.path`)
//! are treated as external and skipped, but counted.

use belisarius_core::{Graph, GraphEdge, GraphNode, Scan};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn build_graph(scan: &Scan) -> Graph {
    let file_set: HashSet<&str> = scan.files.iter().map(|f| f.path.as_str()).collect();
    let language_by_file: HashMap<&str, &str> = scan
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.language.as_str()))
        .collect();
    let aliases = load_ts_aliases(&scan.root);
    let rust_crates = collect_rust_crates(&scan.root, &file_set);
    let go_modules = collect_go_modules(&scan.root, &file_set);

    let mut resolved: Vec<GraphEdge> = Vec::new();
    let mut unresolved: u32 = 0;
    let mut seen = HashSet::new();

    for edge in &scan.edges {
        let lang = language_by_file
            .get(edge.from.as_str())
            .copied()
            .unwrap_or("");
        let target = match lang {
            "typescript" | "javascript" => resolve_ts(&edge.from, &edge.to, &file_set, &aliases),
            "rust" => resolve_rust(&edge.from, &edge.to, &file_set, &rust_crates),
            "python" => resolve_python(&edge.from, &edge.to, &file_set),
            "go" => resolve_go(&edge.to, &file_set, &go_modules),
            _ => None,
        };
        match target {
            Some(to) if to != edge.from => {
                if seen.insert((edge.from.clone(), to.clone())) {
                    resolved.push(GraphEdge {
                        from: edge.from.clone(),
                        to,
                        line: edge.line,
                    });
                }
            }
            Some(_) => {} // self-edge
            None => unresolved += 1,
        }
    }

    let mut in_deg: HashMap<&str, u32> = HashMap::new();
    let mut out_deg: HashMap<&str, u32> = HashMap::new();
    for e in &resolved {
        *in_deg.entry(e.to.as_str()).or_default() += 1;
        *out_deg.entry(e.from.as_str()).or_default() += 1;
    }

    let nodes: Vec<GraphNode> = scan
        .files
        .iter()
        .map(|f| GraphNode {
            id: f.path.clone(),
            language: f.language.clone(),
            loc: f.loc,
            in_degree: in_deg.get(f.path.as_str()).copied().unwrap_or(0),
            out_degree: out_deg.get(f.path.as_str()).copied().unwrap_or(0),
            is_entry_point: is_entry_point(&f.path),
            depth_from_entry: 0,
        })
        .collect();

    Graph {
        root: scan.root.clone(),
        nodes,
        edges: resolved,
        unresolved,
    }
}

fn is_entry_point(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        // Rust
        "main.rs" | "lib.rs" | "build.rs" | "mod.rs"
        // TS/JS entrypoints
        | "index.ts" | "index.tsx" | "index.js" | "index.jsx"
        | "index.mts" | "index.mjs" | "index.cts" | "index.cjs"
        | "main.ts" | "main.tsx" | "main.js" | "main.jsx"
        | "app.ts" | "app.tsx" | "app.js" | "app.jsx"
        | "_app.tsx" | "_app.jsx"
        | "server.ts" | "server.js"
        // Python
        | "__init__.py" | "__main__.py" | "manage.py" | "app.py" | "main.py"
        | "setup.py" | "conftest.py"
    ) {
        return true;
    }
    // Config files at any depth: foo.config.{ts,js,mjs,cjs} or {something}.config.*
    if lower.contains(".config.") {
        return true;
    }
    // Test-runner setup hooks: `vitest.setup.ts`, `jest.setup.js`, etc. The
    // runner loads these by string path from its config — no static import
    // exists for the file-graph resolver to follow, so without this they'd
    // surface as dead.
    if lower.contains(".setup.") {
        return true;
    }
    // Test files are roots in test runners' eyes.
    if lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".test.js")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with(".spec.js")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_test.py")
        || lower.starts_with("test_")
    {
        return true;
    }
    false
}

/// Resolve a TS/JS import. Tries (in order):
///   1. relative paths (`./foo`, `../foo`) against the importing file's directory.
///   2. tsconfig.json `compilerOptions.paths` aliases (e.g., `@/* -> ./src/*`).
///      For each pattern that matches, substitute and resolve against the project root.
fn resolve_ts(
    from: &str,
    raw: &str,
    file_set: &HashSet<&str>,
    aliases: &[Alias],
) -> Option<String> {
    if raw.starts_with('.') {
        let from_dir = Path::new(from).parent().unwrap_or(Path::new(""));
        return resolve_ts_path(from_dir, raw, file_set);
    }
    for alias in aliases {
        for substituted in alias.apply(raw) {
            if let Some(hit) = resolve_ts_path(Path::new(""), &substituted, file_set) {
                return Some(hit);
            }
        }
    }
    None
}

fn resolve_ts_path(base_dir: &Path, raw: &str, file_set: &HashSet<&str>) -> Option<String> {
    let joined = if base_dir.as_os_str().is_empty() {
        PathBuf::from(raw)
    } else {
        base_dir.join(raw)
    };
    let base = normalize(&joined);
    let stem = base.to_string_lossy().into_owned();

    let exts = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];
    if file_set.contains(stem.as_str()) {
        return Some(stem);
    }
    for ext in &exts {
        let candidate = format!("{stem}{ext}");
        if file_set.contains(candidate.as_str()) {
            return Some(candidate);
        }
    }
    for ext in &exts {
        let candidate = format!("{stem}/index{ext}");
        if file_set.contains(candidate.as_str()) {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug, Clone)]
struct Alias {
    /// Pattern with optional trailing `/*` wildcard (e.g., `@/*` or `~`).
    pattern: String,
    /// Replacement candidates (e.g., `["./src/*", "./vendor/*"]`).
    replacements: Vec<String>,
}

impl Alias {
    fn apply(&self, raw: &str) -> Vec<String> {
        if let Some(prefix) = self.pattern.strip_suffix("/*") {
            // wildcard match
            let with_slash = format!("{prefix}/");
            if let Some(rest) = raw.strip_prefix(&with_slash) {
                return self
                    .replacements
                    .iter()
                    .map(|r| {
                        let r = r.trim_start_matches("./");
                        if let Some(rp) = r.strip_suffix("/*") {
                            format!("{rp}/{rest}")
                        } else {
                            format!("{r}/{rest}")
                        }
                    })
                    .collect();
            }
            if raw == prefix {
                return self
                    .replacements
                    .iter()
                    .map(|r| {
                        r.trim_start_matches("./")
                            .trim_end_matches("/*")
                            .to_string()
                    })
                    .collect();
            }
        } else if raw == self.pattern {
            return self
                .replacements
                .iter()
                .map(|r| r.trim_start_matches("./").to_string())
                .collect();
        }
        Vec::new()
    }
}

fn load_ts_aliases(root: &str) -> Vec<Alias> {
    for name in ["tsconfig.json", "jsconfig.json"] {
        let path = format!("{root}/{name}");
        if let Ok(raw) = std::fs::read_to_string(&path) {
            let stripped = strip_jsonc_comments(&raw);
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) {
                if let Some(paths) = value
                    .get("compilerOptions")
                    .and_then(|v| v.get("paths"))
                    .and_then(|v| v.as_object())
                {
                    let mut out = Vec::new();
                    for (k, v) in paths {
                        if let Some(arr) = v.as_array() {
                            let replacements: Vec<String> = arr
                                .iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .filter(|s| !s.contains("node_modules"))
                                .collect();
                            if !replacements.is_empty() {
                                out.push(Alias {
                                    pattern: k.clone(),
                                    replacements,
                                });
                            }
                        }
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
        }
    }
    // Fallback: assume `@/* -> src/*`, the convention used by Vite/Next.js scaffolds.
    vec![Alias {
        pattern: "@/*".to_string(),
        replacements: vec!["./src/*".to_string()],
    }]
}

/// Strip `//` line comments and `/* ... */` block comments from JSONC.
/// Honors string literals so comments inside strings are preserved.
fn strip_jsonc_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_str = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < bytes.len() {
            let n = bytes[i + 1] as char;
            if n == '/' {
                while i < bytes.len() && bytes[i] as char != '\n' {
                    i += 1;
                }
                continue;
            }
            if n == '*' {
                i += 2;
                while i + 1 < bytes.len()
                    && !(bytes[i] as char == '*' && bytes[i + 1] as char == '/')
                {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn resolve_rust(
    from: &str,
    raw: &str,
    file_set: &HashSet<&str>,
    crates: &HashMap<String, String>,
) -> Option<String> {
    let trimmed = raw.trim_start_matches("pub").trim();
    let head: Vec<&str> = trimmed
        .split("::")
        .map(|s| s.trim_matches(|c: char| c == '{' || c == '}' || c == ',' || c.is_whitespace()))
        .filter(|s| !s.is_empty() && !s.contains([',', '{', '}']))
        .collect();
    if head.is_empty() {
        return None;
    }

    let first = head[0];
    let src_root = find_rust_src_root(from)?;

    let rest: Vec<&str> = match first {
        "crate" => head[1..].to_vec(),
        "super" => {
            let mut up = head.iter().take_while(|s| **s == "super").count();
            let mut dir = Path::new(from)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            while up > 0 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                }
                up -= 1;
            }
            let remainder: Vec<&str> = head
                .iter()
                .skip_while(|s| **s == "super")
                .copied()
                .collect();
            return try_rust_paths(&dir, &remainder, file_set);
        }
        "self" => {
            let dir = Path::new(from)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            let remainder: Vec<&str> = head[1..].to_vec();
            return try_rust_paths(&dir, &remainder, file_set);
        }
        other => {
            // Cross-crate import: `use other_crate::foo::Bar`. We can't cheaply
            // resolve `foo::Bar` to a specific submodule file without parsing
            // the target crate's module tree, so we settle for the crate's
            // entry file (lib.rs or main.rs). That still produces the
            // architecturally interesting edge: "this file depends on that
            // crate". Skip standard / commonly-external roots so we don't
            // generate noise for std / core / alloc / proc_macro.
            if matches!(other, "std" | "core" | "alloc" | "proc_macro") {
                return None;
            }
            let normalized = other.replace('-', "_");
            if let Some(src) = crates.get(&normalized) {
                let lib = format!("{src}/lib.rs");
                if file_set.contains(lib.as_str()) {
                    return Some(lib);
                }
                let main_rs = format!("{src}/main.rs");
                if file_set.contains(main_rs.as_str()) {
                    return Some(main_rs);
                }
            }
            return None;
        }
    };

    try_rust_paths(Path::new(&src_root), &rest, file_set)
}

/// Walk the project for `Cargo.toml` files and build a `crate_name → src_root`
/// map. `src_root` is relative to the scan root (e.g.
/// `crates/belisarius-core/src`), matching the format of paths in
/// `Scan.files`. The crate name is normalized — Rust converts `-` to `_`
/// in identifiers, so `belisarius-core` is stored as `belisarius_core`.
pub fn collect_rust_crates(scan_root: &str, file_set: &HashSet<&str>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let root_path = Path::new(scan_root);
    for rel in file_set {
        if !rel.ends_with("/Cargo.toml") && *rel != "Cargo.toml" {
            continue;
        }
        let full = root_path.join(rel);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        let Ok(toml_value) = text.parse::<toml::Value>() else {
            continue;
        };
        let Some(name) = toml_value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        let dir = Path::new(rel)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let src = if dir.as_os_str().is_empty() {
            "src".to_string()
        } else {
            format!("{}/src", dir.to_string_lossy())
        };
        out.insert(name.replace('-', "_"), src);
    }
    out
}

fn try_rust_paths(base: &Path, parts: &[&str], file_set: &HashSet<&str>) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    for end in (1..=parts.len()).rev() {
        let joined = parts[..end].join("/");
        let a = base.join(format!("{joined}.rs"));
        let b = base.join(format!("{joined}/mod.rs"));
        for c in [a, b] {
            let s = c.to_string_lossy().into_owned();
            if file_set.contains(s.as_str()) {
                return Some(s);
            }
        }
    }
    None
}

fn find_rust_src_root(from: &str) -> Option<String> {
    let mut p = PathBuf::from(from);
    while let Some(parent) = p.parent() {
        if parent.file_name().and_then(|s| s.to_str()) == Some("src") {
            return Some(parent.to_string_lossy().into_owned());
        }
        p = parent.to_path_buf();
    }
    None
}

/// Walk the project for `go.mod` files and build a `module_path → dir` map,
/// where `dir` is the directory containing the go.mod (relative to scan root,
/// matching the format of paths in `Scan.files`). The empty string means the
/// module lives at the scan root.
pub fn collect_go_modules(scan_root: &str, file_set: &HashSet<&str>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let root_path = Path::new(scan_root);
    for rel in file_set {
        if !rel.ends_with("/go.mod") && *rel != "go.mod" {
            continue;
        }
        let full = root_path.join(rel);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        let Some(module_path) = text.lines().find_map(parse_go_module_line) else {
            continue;
        };
        let dir = Path::new(rel)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let dir_str = if dir.as_os_str().is_empty() {
            String::new()
        } else {
            dir.to_string_lossy().into_owned()
        };
        out.insert(module_path, dir_str);
    }
    out
}

fn parse_go_module_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("module")?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let p = rest.trim().trim_matches('"');
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}

/// Resolve a Go import specifier (a full module path like
/// `github.com/foo/bar/sub`) to a file inside the scanned project.
///
/// For each in-repo module, we longest-prefix-match the specifier against the
/// module path, strip the prefix to get a sub-path, and try:
///   1. `<module_dir>/<sub>.go`
///   2. `<module_dir>/<sub>/<last_segment>.go`
///   3. any direct-child `.go` file (excluding `*_test.go`) under the dir.
fn resolve_go(
    raw: &str,
    file_set: &HashSet<&str>,
    modules: &HashMap<String, String>,
) -> Option<String> {
    let mut sorted: Vec<(&String, &String)> = modules.iter().collect();
    sorted.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));

    for (module_path, dir) in sorted {
        let sub = if raw == module_path {
            String::new()
        } else if let Some(s) = raw.strip_prefix(&format!("{module_path}/")) {
            s.to_string()
        } else {
            continue;
        };
        let base = match (dir.is_empty(), sub.is_empty()) {
            (true, true) => return None, // module root with no sub — nothing to point at
            (true, false) => sub.clone(),
            (false, true) => dir.clone(),
            (false, false) => format!("{dir}/{sub}"),
        };

        let as_file = format!("{base}.go");
        if file_set.contains(as_file.as_str()) {
            return Some(as_file);
        }

        let last_segment = Path::new(&base)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let preferred = format!("{base}/{last_segment}.go");
        if file_set.contains(preferred.as_str()) {
            return Some(preferred);
        }

        let prefix = format!("{base}/");
        let mut candidates: Vec<&str> = file_set
            .iter()
            .copied()
            .filter(|f| {
                f.starts_with(&prefix)
                    && f.ends_with(".go")
                    && !f.ends_with("_test.go")
                    && !f[prefix.len()..].contains('/')
            })
            .collect();
        candidates.sort();
        if let Some(c) = candidates.first() {
            return Some((*c).to_string());
        }
    }
    None
}

fn resolve_python(from: &str, raw: &str, file_set: &HashSet<&str>) -> Option<String> {
    let path_form = raw.replace('.', "/");
    let candidates = [
        format!("{path_form}.py"),
        format!("{path_form}/__init__.py"),
    ];
    for c in &candidates {
        if file_set.contains(c.as_str()) {
            return Some(c.clone());
        }
    }
    let from_dir = Path::new(from).parent().unwrap_or(Path::new(""));
    let prefix = from_dir.to_string_lossy();
    if prefix.is_empty() {
        return None;
    }
    let rel = [
        format!("{prefix}/{path_form}.py"),
        format!("{prefix}/{path_form}/__init__.py"),
    ];
    for c in &rel {
        if file_set.contains(c.as_str()) {
            return Some(c.clone());
        }
    }
    None
}

fn normalize(p: &Path) -> PathBuf {
    let mut parts: Vec<std::path::Component> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            other => parts.push(other),
        }
    }
    let mut out = PathBuf::new();
    for c in parts {
        out.push(c.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use belisarius_core::{EdgeKind, FileNode, ImportEdge};
    use std::collections::BTreeMap;
    use time::OffsetDateTime;

    fn mk_scan(files: &[(&str, &str)], edges: &[(&str, &str)]) -> Scan {
        Scan {
            root: ".".into(),
            scanned_at: OffsetDateTime::now_utc(),
            files: files
                .iter()
                .map(|(p, l)| FileNode {
                    path: (*p).into(),
                    language: (*l).into(),
                    loc: 10,
                    bytes: 100,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(f, t)| ImportEdge {
                    from: (*f).into(),
                    to: (*t).into(),
                    kind: EdgeKind::Import,
                    line: 1,
                })
                .collect(),
            language_summary: BTreeMap::new(),
        }
    }

    #[test]
    fn resolves_ts_relative_with_ext() {
        let scan = mk_scan(
            &[("src/a.ts", "typescript"), ("src/b.ts", "typescript")],
            &[("src/a.ts", "./b")],
        );
        let g = build_graph(&scan);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].to, "src/b.ts");
    }

    #[test]
    fn resolves_ts_index() {
        let scan = mk_scan(
            &[
                ("src/a.ts", "typescript"),
                ("src/lib/index.ts", "typescript"),
            ],
            &[("src/a.ts", "./lib")],
        );
        let g = build_graph(&scan);
        assert_eq!(g.edges[0].to, "src/lib/index.ts");
    }

    #[test]
    fn resolves_rust_crate_path() {
        let scan = mk_scan(
            &[
                ("crates/foo/src/lib.rs", "rust"),
                ("crates/foo/src/bar.rs", "rust"),
            ],
            &[("crates/foo/src/lib.rs", "crate::bar")],
        );
        let g = build_graph(&scan);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].to, "crates/foo/src/bar.rs");
    }

    #[test]
    fn counts_unresolved_external() {
        let scan = mk_scan(&[("src/a.ts", "typescript")], &[("src/a.ts", "lodash")]);
        let g = build_graph(&scan);
        assert_eq!(g.edges.len(), 0);
        assert_eq!(g.unresolved, 1);
    }

    #[test]
    fn entry_point_flag() {
        let scan = mk_scan(&[("src/main.rs", "rust"), ("src/util.rs", "rust")], &[]);
        let g = build_graph(&scan);
        let main = g.nodes.iter().find(|n| n.id == "src/main.rs").unwrap();
        let util = g.nodes.iter().find(|n| n.id == "src/util.rs").unwrap();
        assert!(main.is_entry_point);
        assert!(!util.is_entry_point);
    }

    #[test]
    fn alias_wildcard_substitution() {
        let alias = Alias {
            pattern: "@/*".into(),
            replacements: vec!["./src/*".into()],
        };
        assert_eq!(
            alias.apply("@/components/foo"),
            vec!["src/components/foo".to_string()]
        );
        assert_eq!(alias.apply("@/foo"), vec!["src/foo".to_string()]);
        assert!(alias.apply("react").is_empty());
    }

    #[test]
    fn resolves_via_default_at_alias_fallback() {
        // No tsconfig present, so the resolver should fall back to `@/* -> src/*`.
        let scan = mk_scan(
            &[
                ("src/App.tsx", "typescript"),
                ("src/components/widget.tsx", "typescript"),
            ],
            &[("src/App.tsx", "@/components/widget")],
        );
        let g = build_graph(&scan);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].to, "src/components/widget.tsx");
    }

    #[test]
    fn go_resolves_sibling_package_via_file() {
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), String::new());
        let files: HashSet<&str> = ["foo/foo.go", "main.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/foo", &files, &modules);
        assert_eq!(r, Some("foo/foo.go".to_string()));
    }

    #[test]
    fn go_resolves_with_module_subdir() {
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), "backend".to_string());
        let files: HashSet<&str> = ["backend/api/api.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/api", &files, &modules);
        assert_eq!(r, Some("backend/api/api.go".to_string()));
    }

    #[test]
    fn go_external_import_unresolved() {
        let modules: HashMap<String, String> = HashMap::new();
        let files: HashSet<&str> = HashSet::new();
        assert_eq!(resolve_go("fmt", &files, &modules), None);
        assert_eq!(resolve_go("github.com/other/thing", &files, &modules), None);
    }

    #[test]
    fn go_picks_preferred_segment_match() {
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), String::new());
        let files: HashSet<&str> = ["api/api.go", "api/helpers.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/api", &files, &modules);
        assert_eq!(r, Some("api/api.go".to_string()));
    }

    #[test]
    fn go_falls_back_to_any_go_file() {
        // Directory exists with .go files but none match the last segment.
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), String::new());
        let files: HashSet<&str> = ["util/helpers.go", "util/types.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/util", &files, &modules);
        // Sorted order picks helpers.go (alphabetically first).
        assert_eq!(r, Some("util/helpers.go".to_string()));
    }

    #[test]
    fn go_skips_test_files_in_fallback() {
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), String::new());
        let files: HashSet<&str> = ["pkg/pkg_test.go", "pkg/impl.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/pkg", &files, &modules);
        assert_eq!(r, Some("pkg/impl.go".to_string()));
    }

    #[test]
    fn go_longest_prefix_wins_for_nested_modules() {
        let mut modules = HashMap::new();
        modules.insert("example.com/proj".to_string(), String::new());
        modules.insert("example.com/proj/sub".to_string(), "sub".to_string());
        let files: HashSet<&str> = ["sub/x/x.go"].into_iter().collect();
        let r = resolve_go("example.com/proj/sub/x", &files, &modules);
        assert_eq!(r, Some("sub/x/x.go".to_string()));
    }

    #[test]
    fn parse_go_module_line_handles_quotes_and_garbage() {
        assert_eq!(
            parse_go_module_line("module example.com/foo").as_deref(),
            Some("example.com/foo")
        );
        assert_eq!(
            parse_go_module_line("module \"example.com/foo\"").as_deref(),
            Some("example.com/foo")
        );
        assert_eq!(
            parse_go_module_line("  module   example.com/foo  ").as_deref(),
            Some("example.com/foo")
        );
        assert_eq!(parse_go_module_line("modules example.com/foo"), None);
        assert_eq!(parse_go_module_line("// module example.com/foo"), None);
        assert_eq!(parse_go_module_line("go 1.21"), None);
    }

    #[test]
    fn strips_jsonc_comments() {
        let raw = r#"{
  // line comment
  "a": 1, /* block */ "b": "// not a comment"
}"#;
        let cleaned = strip_jsonc_comments(raw);
        let parsed: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(parsed["a"], 1);
        assert_eq!(parsed["b"], "// not a comment");
    }

    #[test]
    fn entry_point_classifier_recognizes_test_runner_files() {
        // Test runners load these via string paths in their config — no
        // static import exists, so the resolver must treat them as roots.
        assert!(super::is_entry_point("web/vitest.setup.ts"));
        assert!(super::is_entry_point("web/vitest.config.ts"));
        assert!(super::is_entry_point("web/jest.setup.js"));
        assert!(super::is_entry_point("vite.config.ts"));
        // Sanity: regular source files still aren't entry points.
        assert!(!super::is_entry_point("src/components/widget.tsx"));
        assert!(!super::is_entry_point("src/lib/helper.ts"));
    }
}
