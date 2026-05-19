//! Regex-based import detection. Each match remembers the 1-indexed line in
//! `from` so consumers (DSM, refactor tools) can show the call site without
//! re-parsing the file.
//!
//! Known limitations: this is a line-oriented regex pass, not a parser. We do
//! not detect multiline TS imports (`import {\n  a,\n  b\n} from "x"`) or
//! computed dynamic imports (`require(varName)`, `import(varName)`). Literal
//! dynamic imports (`import('./foo')` inside a `lazy()` callback) ARE picked
//! up. Go block imports are handled via a small state machine.

use crate::languages::has_parser;
use anyhow::Result;
use belisarius_core::{EdgeKind, FileNode, ImportEdge};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

// Per-line regexes — anchored to the start of one line, no multiline flag.
static TS_IMPORT_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*import\s+(?:[^'"`]*?from\s+)?['"]([^'"]+)['"]"#).unwrap());
static TS_REQUIRE_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"require\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());
// Dynamic `import('...')` expressions — used by `lazy(() => import('...'))`
// in Preact/React and by route-based code splitting in Vite. The regex is
// intentionally not line-anchored: code-split call sites are usually
// nested inside callbacks. We still require a literal string argument so
// computed paths (`import(varName)`) don't poison the graph with garbage.
static TS_DYNAMIC_IMPORT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bimport\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());
// Barrel re-exports: `export { Foo } from "./Foo"`, `export * from "./mod"`,
// `export type { X } from "./X"`. The dead-file detector relied on every
// edge being captured, and `index.ts` files in `web/src/types/generated/`
// re-export ~50 types this way; without it every type module looks dead.
static TS_EXPORT_FROM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^\s*export\s+(?:type\s+)?(?:\*|\{[^}]*\})\s+from\s+['"]([^'"]+)['"]"#).unwrap()
});
static PY_FROM_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*from\s+([a-zA-Z0-9_.]+)\s+import\s+"#).unwrap());
static PY_IMPORT_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*import\s+([a-zA-Z0-9_.]+)"#).unwrap());
static RUST_USE_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*use\s+([a-zA-Z0-9_:{}*,\s]+);"#).unwrap());
static RUST_MOD_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^\s*(?:pub\s+(?:\([^)]+\)\s+)?)?mod\s+([a-zA-Z0-9_]+)\s*;"#).unwrap()
});
// Go: `import "fmt"`, `import alias "fmt"`, `import . "fmt"`, `import _ "fmt"`.
static GO_SINGLE_IMPORT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^\s*import\s+(?:(?:[a-zA-Z_][a-zA-Z0-9_]*|\.|_)\s+)?"([^"]+)""#).unwrap()
});
// Go: `import (` opens a block; we collect entries until a closing `)`.
static GO_BLOCK_OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^\s*import\s*\("#).unwrap());
// Inside a Go block: `"fmt"`, `alias "fmt"`, `. "fmt"`, `_ "fmt"`.
static GO_BLOCK_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*(?:(?:[a-zA-Z_][a-zA-Z0-9_]*|\.|_)\s+)?"([^"]+)""#).unwrap());

pub fn edges_for(root: &Path, files: &[FileNode]) -> Result<Vec<ImportEdge>> {
    let mut edges = Vec::new();
    for f in files {
        if !has_parser(&f.language) {
            continue;
        }
        let full = root.join(&f.path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        match f.language.as_str() {
            "typescript" | "javascript" => extract_ts(&f.path, &text, &mut edges),
            "python" => extract_py(&f.path, &text, &mut edges),
            "rust" => extract_rust(&f.path, &text, &mut edges),
            "go" => extract_go(&f.path, &text, &mut edges),
            _ => {}
        }
    }
    Ok(edges)
}

fn extract_ts(from: &str, text: &str, edges: &mut Vec<ImportEdge>) {
    for (i, line) in text.lines().enumerate() {
        let lineno = (i + 1) as u32;
        if let Some(cap) = TS_IMPORT_LINE.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
        }
        if let Some(cap) = TS_REQUIRE_LINE.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
        }
        for cap in TS_DYNAMIC_IMPORT.captures_iter(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
        }
        if let Some(cap) = TS_EXPORT_FROM.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
        }
    }
}

fn extract_py(from: &str, text: &str, edges: &mut Vec<ImportEdge>) {
    for (i, line) in text.lines().enumerate() {
        let lineno = (i + 1) as u32;
        if let Some(cap) = PY_FROM_LINE.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::From,
                line: lineno,
            });
        }
        if let Some(cap) = PY_IMPORT_LINE.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
        }
    }
}

fn extract_go(from: &str, text: &str, edges: &mut Vec<ImportEdge>) {
    let mut in_block = false;
    for (i, line) in text.lines().enumerate() {
        let lineno = (i + 1) as u32;
        if in_block {
            let trimmed = line.trim_start();
            if trimmed.starts_with(')') {
                in_block = false;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            if let Some(cap) = GO_BLOCK_LINE.captures(line) {
                edges.push(ImportEdge {
                    from: from.to_string(),
                    to: cap[1].to_string(),
                    kind: EdgeKind::Import,
                    line: lineno,
                });
            }
            continue;
        }
        if let Some(cap) = GO_SINGLE_IMPORT.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: cap[1].to_string(),
                kind: EdgeKind::Import,
                line: lineno,
            });
            continue;
        }
        if GO_BLOCK_OPEN.is_match(line) {
            in_block = true;
        }
    }
}

fn extract_rust(from: &str, text: &str, edges: &mut Vec<ImportEdge>) {
    for (i, line) in text.lines().enumerate() {
        let lineno = (i + 1) as u32;
        if let Some(cap) = RUST_USE_LINE.captures(line) {
            let raw = cap[1].split_whitespace().collect::<String>();
            edges.push(ImportEdge {
                from: from.to_string(),
                to: raw,
                kind: EdgeKind::Use,
                line: lineno,
            });
        }
        if let Some(cap) = RUST_MOD_LINE.captures(line) {
            edges.push(ImportEdge {
                from: from.to_string(),
                to: format!("self::{}", &cap[1]),
                kind: EdgeKind::Use,
                line: lineno,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_import_line_numbers() {
        let mut e = Vec::new();
        extract_ts(
            "a.ts",
            "// hello\nimport x from 'lodash';\nimport { y } from \"./y\";",
            &mut e,
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].line, 2);
        assert_eq!(e[1].line, 3);
        assert_eq!(e[0].to, "lodash");
    }

    #[test]
    fn ts_dynamic_import_in_lazy_callback() {
        // Mirrors the actual pattern in `web/src/routes/ScanView.tsx`: a
        // `lazy(() => import('./foo').then(...))` call. Before this fix the
        // import was invisible and `./foo` showed up as dead.
        let mut e = Vec::new();
        extract_ts(
            "ScanView.tsx",
            "const Foo = lazy(() => import('../components/Foo').then((m) => m.Foo));",
            &mut e,
        );
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].to, "../components/Foo");
        assert_eq!(e[0].kind, EdgeKind::Import);
    }

    #[test]
    fn ts_static_and_dynamic_imports_on_same_file() {
        let mut e = Vec::new();
        extract_ts(
            "a.tsx",
            "import { lazy } from 'preact/compat';\nconst V = lazy(() => import('./V'));\nconst W = lazy(() => import('./W'));",
            &mut e,
        );
        let tos: Vec<&str> = e.iter().map(|x| x.to.as_str()).collect();
        assert!(tos.contains(&"preact/compat"));
        assert!(tos.contains(&"./V"));
        assert!(tos.contains(&"./W"));
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn ts_dynamic_import_with_variable_is_ignored() {
        // `import(varName)` can't be resolved to a file — skip to avoid
        // poisoning the graph with a literal "varName" target.
        let mut e = Vec::new();
        extract_ts("a.ts", "const m = await import(name);", &mut e);
        assert!(e.is_empty());
    }

    #[test]
    fn ts_export_from_barrel() {
        // The shape used by `web/src/types/generated/index.ts`.
        let mut e = Vec::new();
        extract_ts(
            "index.ts",
            "export type { Foo } from './Foo';\nexport { Bar } from \"./Bar\";\nexport * from './star';",
            &mut e,
        );
        let tos: Vec<&str> = e.iter().map(|x| x.to.as_str()).collect();
        assert!(tos.contains(&"./Foo"));
        assert!(tos.contains(&"./Bar"));
        assert!(tos.contains(&"./star"));
    }

    #[test]
    fn py_import() {
        let mut e = Vec::new();
        extract_py("m.py", "from os.path import join\nimport sys\n", &mut e);
        let tos: Vec<&str> = e.iter().map(|x| x.to.as_str()).collect();
        assert!(tos.contains(&"os.path"));
        assert!(tos.contains(&"sys"));
    }

    #[test]
    fn rust_use() {
        let mut e = Vec::new();
        extract_rust(
            "lib.rs",
            "use std::collections::HashMap;\nuse crate::foo::bar;",
            &mut e,
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].line, 1);
        assert_eq!(e[1].line, 2);
    }

    #[test]
    fn go_single_import() {
        let mut e = Vec::new();
        extract_go(
            "main.go",
            "package main\n\nimport \"fmt\"\nimport log \"github.com/x/log\"\n",
            &mut e,
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].to, "fmt");
        assert_eq!(e[0].line, 3);
        assert_eq!(e[1].to, "github.com/x/log");
        assert_eq!(e[1].line, 4);
    }

    #[test]
    fn go_block_import_with_alias() {
        let mut e = Vec::new();
        extract_go(
            "main.go",
            "package main\n\nimport (\n    \"fmt\"\n    log \"github.com/x/log\"\n    _ \"github.com/blank/effect\"\n)\n",
            &mut e,
        );
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].to, "fmt");
        assert_eq!(e[1].to, "github.com/x/log");
        assert_eq!(e[2].to, "github.com/blank/effect");
        assert_eq!(e[0].line, 4);
    }

    #[test]
    fn go_block_skips_comments_and_blanks() {
        let mut e = Vec::new();
        extract_go(
            "main.go",
            "import (\n    // top comment\n\n    \"fmt\"\n    // mid\n    \"os\"\n)\n",
            &mut e,
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].to, "fmt");
        assert_eq!(e[1].to, "os");
    }

    #[test]
    fn rust_mod_routed_via_self() {
        let mut e = Vec::new();
        extract_rust("lib.rs", "// header\npub mod foo;\n  mod bar;\n", &mut e);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].to, "self::foo");
        assert_eq!(e[0].line, 2);
        assert_eq!(e[1].to, "self::bar");
        assert_eq!(e[1].line, 3);
    }
}
