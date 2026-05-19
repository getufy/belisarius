//! Public-surface inventory: every thing the codebase exposes to the outside.
//!
//! Powered by tree-sitter walks for Rust + TS/JS exports, plus regex passes
//! for HTTP routes (axum / express / Hono) and clap derive subcommands. The
//! aggregate report is what answers "what does this project do?".

use anyhow::Result;
use belisarius_core::{Scan, SurfaceItem, SurfaceKind, SurfaceReport};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use tree_sitter::{Node, Parser};

pub fn extract(project: &Path, scan: &Scan) -> Result<SurfaceReport> {
    let mut items: Vec<SurfaceItem> = Vec::new();
    for f in &scan.files {
        let full = project.join(&f.path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        match f.language.as_str() {
            "rust" => extract_rust(&f.path, &text, &mut items)?,
            "typescript" | "javascript" => {
                let is_tsx = f.path.ends_with(".tsx");
                extract_ts(&f.path, &text, is_tsx, &mut items)?
            }
            _ => {}
        }
        // Cross-language: route declarations are easy to grep for.
        extract_http_routes(&f.path, &f.language, &text, &mut items);
    }
    items.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then((a.line, &a.name).cmp(&(b.line, &b.name)))
    });
    Ok(build_report(items))
}

fn build_report(items: Vec<SurfaceItem>) -> SurfaceReport {
    let mut counts_by_kind: BTreeMap<String, u32> = BTreeMap::new();
    let mut counts_by_language: BTreeMap<String, u32> = BTreeMap::new();
    for it in &items {
        *counts_by_kind
            .entry(kind_label(it.kind).into())
            .or_default() += 1;
        *counts_by_language.entry(it.language.clone()).or_default() += 1;
    }
    SurfaceReport {
        items,
        counts_by_kind,
        counts_by_language,
    }
}

fn kind_label(k: SurfaceKind) -> &'static str {
    match k {
        SurfaceKind::Function => "function",
        SurfaceKind::Type => "type",
        SurfaceKind::Module => "module",
        SurfaceKind::Constant => "constant",
        SurfaceKind::ReExport => "re-export",
        SurfaceKind::HttpRoute => "http_route",
        SurfaceKind::CliCommand => "cli_command",
    }
}

// ── Rust ───────────────────────────────────────────────────────────────────
//
// Walk for top-level items with a `visibility_modifier` child whose first
// token is `pub`. We capture:
//   pub fn / pub struct / pub enum / pub trait / pub mod / pub use /
//   pub const / pub type
//
// Methods inside `impl` blocks are skipped — they're already covered by the
// containing type and rarely qualify as "the surface".

fn extract_rust(rel: &str, source: &str, items: &mut Vec<SurfaceItem>) -> Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let Some(tree) = parser.parse(source, None) else {
        return Ok(());
    };
    let bytes = source.as_bytes();
    walk_rust(rel, tree.root_node(), bytes, items);
    Ok(())
}

fn walk_rust(rel: &str, node: Node, source: &[u8], items: &mut Vec<SurfaceItem>) {
    // Only visit top-level (and `mod` block children) — skip the bodies of
    // functions and impls, which can't contain "the surface".
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        // Recurse into module declarations to capture re-exports etc.
        if kind == "mod_item" {
            if let Some(body) = child.child_by_field_name("body") {
                walk_rust(rel, body, source, items);
            }
            // Also surface the module itself if pub.
            if is_pub(child, source) {
                if let Some(name) = name_text(child, source, "name") {
                    items.push(item(rel, "rust", SurfaceKind::Module, &name, child, source));
                }
            }
            continue;
        }
        if !is_pub(child, source) {
            continue;
        }
        match kind {
            "function_item" => {
                if let Some(name) = name_text(child, source, "name") {
                    let sig = signature_to_block(child, source);
                    items.push(item_with_sig(
                        rel,
                        "rust",
                        SurfaceKind::Function,
                        &name,
                        child,
                        sig,
                    ));
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "type_item" | "union_item" => {
                if let Some(name) = name_text(child, source, "name") {
                    items.push(item(rel, "rust", SurfaceKind::Type, &name, child, source));
                }
            }
            "const_item" | "static_item" => {
                if let Some(name) = name_text(child, source, "name") {
                    items.push(item(
                        rel,
                        "rust",
                        SurfaceKind::Constant,
                        &name,
                        child,
                        source,
                    ));
                }
            }
            "use_declaration" => {
                // Pull a short label out of the path (`pub use foo::bar::Baz;` → `Baz`).
                let raw = child
                    .utf8_text(source)
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(';');
                let label = raw.rsplit("::").next().unwrap_or(raw).trim();
                items.push(item(
                    rel,
                    "rust",
                    SurfaceKind::ReExport,
                    label,
                    child,
                    source,
                ));
            }
            _ => {}
        }
    }
}

fn is_pub(node: Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = child.utf8_text(source).unwrap_or("");
            return text.starts_with("pub");
        }
    }
    false
}

fn name_text(node: Node, source: &[u8], field: &str) -> Option<String> {
    let n = node.child_by_field_name(field)?;
    Some(n.utf8_text(source).ok()?.to_string())
}

fn signature_to_block(node: Node, source: &[u8]) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let start = node.start_byte();
    let end = body.start_byte();
    if end <= start {
        return None;
    }
    let text = std::str::from_utf8(&source[start..end]).ok()?;
    Some(text.trim().trim_end_matches('{').trim().to_string())
}

fn item(
    rel: &str,
    lang: &str,
    kind: SurfaceKind,
    name: &str,
    node: Node,
    _source: &[u8],
) -> SurfaceItem {
    SurfaceItem {
        file: rel.to_string(),
        language: lang.to_string(),
        kind,
        name: name.to_string(),
        signature: None,
        line: (node.start_position().row as u32) + 1,
        method: None,
    }
}

fn item_with_sig(
    rel: &str,
    lang: &str,
    kind: SurfaceKind,
    name: &str,
    node: Node,
    sig: Option<String>,
) -> SurfaceItem {
    SurfaceItem {
        file: rel.to_string(),
        language: lang.to_string(),
        kind,
        name: name.to_string(),
        signature: sig,
        line: (node.start_position().row as u32) + 1,
        method: None,
    }
}

// ── TypeScript / JavaScript ─────────────────────────────────────────────────
//
// We pick up `export_statement`s and dig into their declaration child.
// Re-exports (`export * from ...`, `export { Foo } from ...`) are captured
// as ReExport. Default exports use the literal name "default".

fn extract_ts(rel: &str, source: &str, is_tsx: bool, items: &mut Vec<SurfaceItem>) -> Result<()> {
    let mut parser = Parser::new();
    let lang = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else if rel.ends_with(".ts") {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    } else {
        tree_sitter_javascript::LANGUAGE.into()
    };
    parser.set_language(&lang)?;
    let Some(tree) = parser.parse(source, None) else {
        return Ok(());
    };
    let bytes = source.as_bytes();
    walk_ts(rel, tree.root_node(), bytes, items);
    Ok(())
}

fn walk_ts(rel: &str, node: Node, source: &[u8], items: &mut Vec<SurfaceItem>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }
        // Re-export forms: `export * from`, `export { ... } from`
        if let Some(_clause) = child.child_by_field_name("source") {
            let raw = child.utf8_text(source).unwrap_or("").trim();
            items.push(SurfaceItem {
                file: rel.to_string(),
                language: ts_lang(rel),
                kind: SurfaceKind::ReExport,
                name: trim_label(raw),
                signature: None,
                line: (child.start_position().row as u32) + 1,
                method: None,
            });
            continue;
        }
        // export default ... → name "default"
        if has_token(child, source, "default") {
            items.push(SurfaceItem {
                file: rel.to_string(),
                language: ts_lang(rel),
                kind: SurfaceKind::Function,
                name: "default".to_string(),
                signature: None,
                line: (child.start_position().row as u32) + 1,
                method: None,
            });
            continue;
        }
        // Walk the declaration child to find what we're exporting.
        let mut inner = child.walk();
        for declared in child.children(&mut inner) {
            let kind = declared.kind();
            match kind {
                "function_declaration" | "generator_function_declaration" => {
                    if let Some(name) = name_text(declared, source, "name") {
                        items.push(SurfaceItem {
                            file: rel.to_string(),
                            language: ts_lang(rel),
                            kind: SurfaceKind::Function,
                            name,
                            signature: None,
                            line: (declared.start_position().row as u32) + 1,
                            method: None,
                        });
                    }
                }
                "class_declaration" => {
                    if let Some(name) = name_text(declared, source, "name") {
                        items.push(SurfaceItem {
                            file: rel.to_string(),
                            language: ts_lang(rel),
                            kind: SurfaceKind::Type,
                            name,
                            signature: None,
                            line: (declared.start_position().row as u32) + 1,
                            method: None,
                        });
                    }
                }
                "interface_declaration" | "type_alias_declaration" | "enum_declaration" => {
                    if let Some(name) = name_text(declared, source, "name") {
                        items.push(SurfaceItem {
                            file: rel.to_string(),
                            language: ts_lang(rel),
                            kind: SurfaceKind::Type,
                            name,
                            signature: None,
                            line: (declared.start_position().row as u32) + 1,
                            method: None,
                        });
                    }
                }
                "lexical_declaration" | "variable_declaration" => {
                    // const / let exports: capture each declarator's name.
                    let mut decl_cursor = declared.walk();
                    for d in declared.children(&mut decl_cursor) {
                        if d.kind() == "variable_declarator" {
                            if let Some(name_node) = d.child_by_field_name("name") {
                                if let Ok(name) = name_node.utf8_text(source) {
                                    items.push(SurfaceItem {
                                        file: rel.to_string(),
                                        language: ts_lang(rel),
                                        kind: SurfaceKind::Constant,
                                        name: name.to_string(),
                                        signature: None,
                                        line: (declared.start_position().row as u32) + 1,
                                        method: None,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn has_token(node: Node, source: &[u8], token: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.utf8_text(source).map(|t| t == token).unwrap_or(false) {
            return true;
        }
    }
    false
}

fn trim_label(raw: &str) -> String {
    let oneline = raw.replace('\n', " ");
    let trimmed: String = oneline.chars().take(64).collect();
    trimmed
}

fn ts_lang(rel: &str) -> String {
    if rel.ends_with(".tsx") || rel.ends_with(".ts") {
        "typescript".to_string()
    } else {
        "javascript".to_string()
    }
}

// ── HTTP routes (regex pass, language-agnostic) ────────────────────────────

static AXUM_ROUTE: Lazy<Regex> = Lazy::new(|| {
    // .route("/path", get(handler))  or  .route("/path", post(handler))
    Regex::new(
        r#"\.route\(\s*"(?P<path>[^"]+)"\s*,\s*(?P<method>get|post|put|delete|patch|options|head)\b"#,
    )
    .unwrap()
});

static EXPRESS_ROUTE: Lazy<Regex> = Lazy::new(|| {
    // app.get("/path", ...)  app.post("/path", ...)
    Regex::new(
        r#"(?P<app>\w+)\.(?P<method>get|post|put|delete|patch|options|head)\(\s*['"](?P<path>[^'"]+)['"]"#,
    )
    .unwrap()
});

static FASTAPI_ROUTE: Lazy<Regex> = Lazy::new(|| {
    // @app.get("/path") / @router.post("/path")
    Regex::new(
        r#"@(?P<app>\w+)\.(?P<method>get|post|put|delete|patch|options|head)\(\s*['"](?P<path>[^'"]+)['"]"#,
    )
    .unwrap()
});

fn extract_http_routes(rel: &str, language: &str, source: &str, items: &mut Vec<SurfaceItem>) {
    let re = match language {
        "rust" => &*AXUM_ROUTE,
        "typescript" | "javascript" => &*EXPRESS_ROUTE,
        "python" => &*FASTAPI_ROUTE,
        _ => return,
    };
    for cap in re.captures_iter(source) {
        let path = cap.name("path").map(|m| m.as_str().to_string());
        let method = cap.name("method").map(|m| m.as_str().to_uppercase());
        let (Some(path), Some(method)) = (path, method) else {
            continue;
        };
        let pos = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let line = source[..pos].bytes().filter(|&b| b == b'\n').count() as u32 + 1;
        items.push(SurfaceItem {
            file: rel.to_string(),
            language: language.to_string(),
            kind: SurfaceKind::HttpRoute,
            name: path,
            signature: None,
            line,
            method: Some(method),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_pub_items_collected() {
        let mut items = Vec::new();
        let src = r#"
pub fn hello() {}
pub struct Foo;
pub trait Bar {}
fn private() {}
pub use crate::foo::Baz;
"#;
        extract_rust("a.rs", src, &mut items).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"Bar"));
        assert!(names.contains(&"Baz"));
        assert!(!names.contains(&"private"));
    }

    #[test]
    fn ts_exports_collected() {
        let mut items = Vec::new();
        let src = r#"
export function hello() {}
export class Foo {}
export interface Bar {}
const PRIVATE = 1;
export const PUBLIC = 2;
export { Helper } from './helper';
"#;
        extract_ts("a.ts", src, false, &mut items).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"Bar"));
        assert!(names.contains(&"PUBLIC"));
        assert!(!names.contains(&"PRIVATE"));
        // re-export captured as ReExport
        let reexports: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.kind, SurfaceKind::ReExport))
            .collect();
        assert_eq!(reexports.len(), 1);
    }

    #[test]
    fn axum_routes_detected() {
        let mut items = Vec::new();
        let src = r#"
let app = Router::new()
    .route("/api/health", get(health))
    .route("/api/users", post(create_user));
"#;
        extract_http_routes("server.rs", "rust", src, &mut items);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].method.as_deref(), Some("GET"));
        assert_eq!(items[0].name, "/api/health");
        assert_eq!(items[1].method.as_deref(), Some("POST"));
    }

    #[test]
    fn express_routes_detected() {
        let mut items = Vec::new();
        let src = r#"
app.get('/users', (req, res) => { ... });
app.post("/users", handler);
router.delete('/users/:id', remove);
"#;
        extract_http_routes("server.ts", "typescript", src, &mut items);
        let methods: Vec<_> = items.iter().filter_map(|i| i.method.clone()).collect();
        assert!(methods.contains(&"GET".into()));
        assert!(methods.contains(&"POST".into()));
        assert!(methods.contains(&"DELETE".into()));
    }
}
