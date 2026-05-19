//! Tree-sitter-based per-function extraction for the languages Belisarius
//! understands today: Rust, TypeScript, JavaScript, Python, Go.
//!
//! Skipped on purpose for v1: anonymous closures, lambdas, arrow functions, and
//! generated code. Those would inflate function counts without giving the
//! metric layer signal that's much better than file-level LOC.

use crate::complexity::{self, Complexity, LangSpec};
use anyhow::{anyhow, Result};
use belisarius_core::FunctionInfo;
use std::path::Path;
use tree_sitter::{Language, Node, Parser};

/// A single function extracted from a source file.
struct Extraction {
    name: String,
    start_line: u32,
    end_line: u32,
    params: u32,
    body: Option<(u32, u32, u32, u32)>, // start_byte, end_byte, start_row, end_row
    complexity: Complexity,
}

/// Extract functions from a file's source text. `language` is the Belisarius
/// language id (matches `languages::language_for_ext`). Returns an empty vec
/// for languages we don't have AST support for — callers can still rely on
/// file-level LOC.
pub fn extract_functions(
    language: &str,
    rel_path: &str,
    source: &str,
) -> Result<Vec<FunctionInfo>> {
    // `.tsx` files contain JSX; the plain TypeScript grammar bails on `<Foo />`
    // syntax. Route them through the TSX grammar — it's a superset.
    let effective = if language == "typescript" && rel_path.ends_with(".tsx") {
        "tsx"
    } else {
        language
    };
    let Some(spec) = lang_spec(effective) else {
        return Ok(Vec::new());
    };
    let mut parser = Parser::new();
    parser
        .set_language(&spec.language)
        .map_err(|e| anyhow!("set_language failed for {language}: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter failed to parse {rel_path}"))?;
    let bytes = source.as_bytes();
    let mut extractions: Vec<Extraction> = Vec::new();
    collect_functions(&spec, tree.root_node(), bytes, &mut extractions);
    let mut out = Vec::with_capacity(extractions.len());
    for ex in extractions {
        let (body_start_byte, body_end_byte, _, _) = ex.body.unwrap_or((0, 0, 0, 0));
        let body_text = if body_end_byte > body_start_byte {
            &source[body_start_byte as usize..body_end_byte as usize]
        } else {
            ""
        };
        let body_hash = hash16(body_text);
        let loc = ex.end_line.saturating_sub(ex.start_line) + 1;
        out.push(FunctionInfo {
            file: rel_path.to_string(),
            name: ex.name,
            start_line: ex.start_line + 1, // 1-indexed for display
            end_line: ex.end_line + 1,
            loc,
            params: ex.params,
            cyclomatic: ex.complexity.cyclomatic,
            cognitive: ex.complexity.cognitive,
            body_hash,
        });
    }
    Ok(out)
}

/// Convenience for `analyze()`: read the file from disk and extract.
pub fn extract_functions_from_path(
    language: &str,
    project_root: &Path,
    rel_path: &str,
) -> Result<Vec<FunctionInfo>> {
    let full = project_root.join(rel_path);
    let source = match std::fs::read_to_string(&full) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    extract_functions(language, rel_path, &source)
}

struct LangConfig {
    language: Language,
    fn_kinds: &'static [&'static str],
    name_field: &'static str,
    params_field: &'static str,
    body_field: &'static str,
    /// AST node kinds that *are* parameters (used when counting children of the param list).
    param_kinds: &'static [&'static str],
    /// Token strings inside the param list that mean "this counts as zero" (self/this).
    skip_param_texts: &'static [&'static str],
    complexity_spec: &'static LangSpec,
}

fn lang_spec(language: &str) -> Option<LangConfig> {
    match language {
        "rust" => Some(LangConfig {
            language: tree_sitter_rust::LANGUAGE.into(),
            fn_kinds: &["function_item"],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &["parameter"],
            skip_param_texts: &[],
            complexity_spec: &complexity::RUST,
        }),
        "typescript" => Some(LangConfig {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            fn_kinds: &["function_declaration", "method_definition"],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &["required_parameter", "optional_parameter", "rest_pattern"],
            skip_param_texts: &["this"],
            complexity_spec: &complexity::TS_JS,
        }),
        "tsx" => Some(LangConfig {
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            fn_kinds: &["function_declaration", "method_definition"],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &["required_parameter", "optional_parameter", "rest_pattern"],
            skip_param_texts: &["this"],
            complexity_spec: &complexity::TS_JS,
        }),
        "javascript" => Some(LangConfig {
            language: tree_sitter_javascript::LANGUAGE.into(),
            fn_kinds: &[
                "function_declaration",
                "method_definition",
                "generator_function_declaration",
            ],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &[
                "identifier",
                "assignment_pattern",
                "object_pattern",
                "array_pattern",
                "rest_pattern",
            ],
            skip_param_texts: &["this"],
            complexity_spec: &complexity::TS_JS,
        }),
        "python" => Some(LangConfig {
            language: tree_sitter_python::LANGUAGE.into(),
            fn_kinds: &["function_definition"],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &[
                "identifier",
                "typed_parameter",
                "default_parameter",
                "typed_default_parameter",
                "list_splat_pattern",
                "dictionary_splat_pattern",
            ],
            skip_param_texts: &["self", "cls"],
            complexity_spec: &complexity::PYTHON,
        }),
        "go" => Some(LangConfig {
            language: tree_sitter_go::LANGUAGE.into(),
            fn_kinds: &["function_declaration", "method_declaration"],
            name_field: "name",
            params_field: "parameters",
            body_field: "body",
            param_kinds: &["parameter_declaration", "variadic_parameter_declaration"],
            skip_param_texts: &[],
            complexity_spec: &complexity::GO,
        }),
        _ => None,
    }
}

fn collect_functions(spec: &LangConfig, node: Node, source: &[u8], out: &mut Vec<Extraction>) {
    if spec.fn_kinds.contains(&node.kind()) {
        if let Some(ex) = build_extraction(spec, node, source) {
            out.push(ex);
        }
        // Recurse into the body anyway so nested functions get picked up.
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(spec, child, source, out);
    }
}

fn build_extraction(spec: &LangConfig, node: Node, source: &[u8]) -> Option<Extraction> {
    let name_node = node.child_by_field_name(spec.name_field)?;
    let name = name_node.utf8_text(source).ok()?.to_string();
    let params_node = node.child_by_field_name(spec.params_field);
    let params = count_params(spec, params_node, source);
    let body_node = node.child_by_field_name(spec.body_field);
    let complexity = match body_node {
        Some(b) => complexity::compute(spec.complexity_spec, b, source),
        None => Complexity {
            cyclomatic: 1,
            cognitive: 0,
        },
    };
    let body = body_node.map(|b| {
        (
            b.start_byte() as u32,
            b.end_byte() as u32,
            b.start_position().row as u32,
            b.end_position().row as u32,
        )
    });
    Some(Extraction {
        name,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        params,
        body,
        complexity,
    })
}

fn count_params(spec: &LangConfig, params: Option<Node>, source: &[u8]) -> u32 {
    let Some(p) = params else {
        return 0;
    };
    let mut count: u32 = 0;
    let mut cursor = p.walk();
    for child in p.children(&mut cursor) {
        let kind = child.kind();
        if kind == "self_parameter" {
            continue;
        }
        let text = child.utf8_text(source).unwrap_or("");
        if spec.skip_param_texts.contains(&text) {
            continue;
        }
        if spec.param_kinds.contains(&kind) {
            count += 1;
        }
    }
    count
}

fn hash16(text: &str) -> String {
    let hash = blake3::hash(text.as_bytes());
    hash.to_hex().as_str()[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_function_count_and_cc() {
        let src = r#"
fn one() -> i32 { 1 }

fn two(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

fn three(xs: &[i32]) -> i32 {
    let mut total = 0;
    for x in xs {
        if *x > 0 {
            total += x;
        }
    }
    total
}
"#;
        let fns = extract_functions("rust", "x.rs", src).unwrap();
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["one", "two", "three"]);
        let two = fns.iter().find(|f| f.name == "two").unwrap();
        assert_eq!(two.params, 2);
        assert!(two.cyclomatic >= 2, "two cc = {}", two.cyclomatic);
        let three = fns.iter().find(|f| f.name == "three").unwrap();
        assert!(three.cyclomatic >= 3, "three cc = {}", three.cyclomatic);
    }

    #[test]
    fn typescript_method_extraction() {
        let src = r#"
export class Greeter {
  greet(name: string): string {
    if (name) {
      return `hi ${name}`;
    }
    return "hi";
  }
}

export function add(a: number, b: number): number {
  return a + b;
}
"#;
        let fns = extract_functions("typescript", "x.ts", src).unwrap();
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));
        let greet = fns.iter().find(|f| f.name == "greet").unwrap();
        assert_eq!(greet.params, 1);
        assert!(greet.cyclomatic >= 2);
    }

    #[test]
    fn python_self_excluded() {
        let src = r#"
class C:
    def m(self, x, y):
        if x and y:
            return 1
        elif x:
            return 2
        return 0
"#;
        let fns = extract_functions("python", "x.py", src).unwrap();
        let m = fns.iter().find(|f| f.name == "m").unwrap();
        assert_eq!(m.params, 2, "should exclude self");
        // 1 (base) + if + elif + `and` = 4
        assert!(m.cyclomatic >= 3, "cc = {}", m.cyclomatic);
    }

    #[test]
    fn go_function_count() {
        let src = r#"
package main

func add(a int, b int) int {
    return a + b
}

func choose(x int) int {
    if x > 0 {
        return 1
    }
    return 0
}
"#;
        let fns = extract_functions("go", "x.go", src).unwrap();
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"choose"));
    }

    #[test]
    fn javascript_arrow_skipped() {
        let src = r#"
function decl(a, b) { return a + b; }
const arrow = (x) => x + 1;
class K { method(z) { return z; } }
"#;
        let fns = extract_functions("javascript", "x.js", src).unwrap();
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"decl"));
        assert!(names.contains(&"method"));
        // arrows are intentionally not extracted in v1
        assert!(!names.contains(&"arrow"));
    }

    #[test]
    fn nested_function_doesnt_inflate_outer() {
        // Outer has 1 if = cc 2. Inner has 5 ifs but should NOT bleed into outer.
        let src = r#"
fn outer(x: i32) -> i32 {
    if x > 0 {
        fn inner(y: i32) -> i32 {
            if y > 1 { y } else if y < -1 { -y } else if y == 0 { 1 } else if y == 2 { 2 } else { 3 }
        }
        inner(x)
    } else {
        0
    }
}
"#;
        let fns = extract_functions("rust", "x.rs", src).unwrap();
        let outer = fns.iter().find(|f| f.name == "outer").unwrap();
        let inner = fns.iter().find(|f| f.name == "inner").unwrap();
        // outer: base 1 + one if = 2.
        assert_eq!(
            outer.cyclomatic, 2,
            "outer cc was inflated by inner: {}",
            outer.cyclomatic
        );
        // inner: base 1 + 4 ifs/else-ifs (each `else if` parses as another `if_expression`) = 5.
        assert!(inner.cyclomatic >= 5, "inner cc = {}", inner.cyclomatic);
    }

    #[test]
    fn rust_match_doesnt_inflate_cognitive() {
        // 5 arms — cc should grow but cog should stay small (match counts once
        // for cognitive; arms add 0).
        let src = r#"
fn classify(n: i32) -> &'static str {
    match n {
        0 => "zero",
        1 => "one",
        2 => "two",
        3 => "three",
        _ => "other",
    }
}
"#;
        let fns = extract_functions("rust", "x.rs", src).unwrap();
        let f = &fns[0];
        // cc: 1 + 5 arms = 6.
        assert!(f.cyclomatic >= 5, "cc = {}", f.cyclomatic);
        // cog: just the match itself = 1.
        assert!(
            f.cognitive <= 2,
            "cognitive should be ~1 for flat match, got {}",
            f.cognitive
        );
    }

    #[test]
    fn ts_switch_doesnt_inflate_cognitive() {
        let src = r#"
function classify(n: number): string {
    switch (n) {
        case 0: return "zero";
        case 1: return "one";
        case 2: return "two";
        case 3: return "three";
        default: return "other";
    }
}
"#;
        let fns = extract_functions("typescript", "x.ts", src).unwrap();
        let f = &fns[0];
        // cc includes each case
        assert!(f.cyclomatic >= 5, "cc = {}", f.cyclomatic);
        // cog: switch itself = 1, cases add 0.
        assert!(
            f.cognitive <= 2,
            "cognitive should be ~1 for flat switch, got {}",
            f.cognitive
        );
    }

    #[test]
    fn tsx_file_parses_jsx() {
        let src = r#"
export function Greet({ name }: { name: string }) {
  if (!name) return <div>anon</div>;
  return <div>hi {name}</div>;
}
"#;
        // Calling with "typescript" + .tsx extension routes through the TSX grammar.
        let fns = extract_functions("typescript", "comp.tsx", src).unwrap();
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"Greet"), "Greet not found in {names:?}");
    }
}
