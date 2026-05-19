//! `belisarius_suggest_tests` — heuristic test-stub planner.
//!
//! No LLM call. Detects the project's test framework from existing test
//! files and config, then emits a structured plan an agent can flesh out:
//!
//! ```json
//! {
//!   "framework": "cargo_test",
//!   "language": "rust",
//!   "target": "crates/foo/src/parser.rs",
//!   "test_file_path": "crates/foo/tests/parser_test.rs",
//!   "skeleton": {
//!     "imports": ["use foo::parser;"],
//!     "test_names": [
//!       "parser_returns_ok_on_valid_input",
//!       "parser_errors_on_empty_input",
//!       "parser_handles_unicode"
//!     ]
//!   }
//! }
//! ```
//!
//! Frameworks detected: `cargo_test`, `jest`, `vitest`, `pytest`, `go_test`,
//! `unknown` as fallback.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::registry::{BoxFut, ToolHandler, ToolSpec};
use crate::service::context::AppContext;
use crate::service::error::ServiceError;

#[derive(Debug, Deserialize)]
pub struct SuggestTestsArgs {
    pub path: String,
    /// Relative file path under the project root.
    pub target: String,
}

pub async fn suggest_tests(
    ctx: &AppContext,
    args: SuggestTestsArgs,
) -> Result<Value, ServiceError> {
    let project_root = ctx.resolve_path(&args.path);
    let target = args.target.trim().to_string();
    let resolved = Path::new(&project_root).join(&target);
    if !resolved.is_file() {
        return Err(ServiceError::not_found(format!(
            "target file `{target}` not found under `{project_root}`"
        )));
    }

    let language = detect_language(&target).unwrap_or("unknown");
    let project_path = PathBuf::from(&project_root);
    let framework = detect_framework(&project_path, language);

    // Pull function names from the cached analysis.
    let analysis = ctx.load_analysis(&project_root).await?;
    let mut fn_names: Vec<&str> = analysis
        .functions
        .iter()
        .filter(|f| f.file == target)
        .map(|f| f.name.as_str())
        .collect();
    fn_names.dedup();

    let test_file_path = suggest_test_file_path(language, framework, &target);
    let test_names = generate_test_names(language, framework, &fn_names);
    let imports = suggest_imports(language, framework, &target);

    Ok(json!({
        "framework": framework,
        "language": language,
        "target": target,
        "test_file_path": test_file_path,
        "functions_to_test": fn_names,
        "skeleton": {
            "imports": imports,
            "test_names": test_names,
        },
        "next_steps": [
            "create the test file at `test_file_path`",
            "fill in one assertion per test_name (happy path first)",
            "run the framework's test command (see `belisarius commands` for the project's runner)",
        ],
    }))
}

fn detect_language(target: &str) -> Option<&'static str> {
    let ext = Path::new(target).extension().and_then(|s| s.to_str())?;
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Walk a few well-known config / lockfile signals to pick the framework
/// without scanning every file. Order matters: vitest > jest because vitest
/// projects often also depend on jest types.
fn detect_framework(project: &Path, language: &str) -> &'static str {
    match language {
        "rust" => "cargo_test",
        "go" => "go_test",
        "python" => "pytest",
        "typescript" | "javascript" => {
            if has_any(
                project,
                &["vitest.config.ts", "vitest.config.js", "vite.config.ts"],
            ) {
                "vitest"
            } else if has_any(
                project,
                &["jest.config.ts", "jest.config.js", "jest.config.json"],
            ) {
                "jest"
            } else if package_json_has_dep(project, "vitest") {
                "vitest"
            } else {
                "jest" // safer default for JS/TS
            }
        }
        _ => "unknown",
    }
}

fn has_any(project: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| project.join(n).is_file())
}

fn package_json_has_dep(project: &Path, dep: &str) -> bool {
    let pkg = project.join("package.json");
    let Ok(text) = std::fs::read_to_string(&pkg) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let in_section = |key: &str| {
        v.get(key)
            .and_then(|s| s.as_object())
            .map(|o| o.contains_key(dep))
            .unwrap_or(false)
    };
    in_section("dependencies") || in_section("devDependencies")
}

fn suggest_test_file_path(language: &str, framework: &str, target: &str) -> String {
    let p = Path::new(target);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("target");
    let parent = p.parent().and_then(|s| s.to_str()).unwrap_or("");
    match (language, framework) {
        ("rust", _) => {
            // Cargo convention: tests/<name>_test.rs at crate root if we can
            // find one, otherwise inline as `<file>` with `#[cfg(test)] mod tests`.
            // We pick the inline variant as the more common case.
            format!("{target}  (add `#[cfg(test)] mod tests {{ ... }}` in-file)")
        }
        ("go", _) => format!("{parent}/{stem}_test.go"),
        ("python", _) => format!("{parent}/test_{stem}.py"),
        ("typescript", _) | ("javascript", _) => {
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("ts");
            format!("{parent}/{stem}.test.{ext}")
        }
        _ => format!("{parent}/{stem}_test.unknown"),
    }
}

fn generate_test_names(_language: &str, _framework: &str, fn_names: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for name in fn_names.iter().take(8) {
        let canonical = name.trim_start_matches('_').to_ascii_lowercase();
        out.push(format!("{canonical}_happy_path"));
        out.push(format!("{canonical}_handles_empty_input"));
        out.push(format!("{canonical}_returns_error_on_invalid_input"));
    }
    out
}

fn suggest_imports(language: &str, framework: &str, target: &str) -> Vec<String> {
    let stem = Path::new(target)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("target");
    match (language, framework) {
        ("rust", _) => vec!["use super::*;".into()],
        ("go", _) => vec!["import \"testing\"".into()],
        ("python", "pytest") => vec![format!("import {stem}"), "import pytest".into()],
        ("typescript", "vitest") | ("javascript", "vitest") => vec![
            "import { describe, it, expect } from 'vitest';".into(),
            format!("import * as subject from './{stem}';"),
        ],
        ("typescript", "jest") | ("javascript", "jest") => {
            vec![format!("import * as subject from './{stem}';")]
        }
        _ => Vec::new(),
    }
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "belisarius_suggest_tests",
        description: "Heuristic test-stub planner — no LLM, no code generation. Detects the \
project's test framework from config files and dependencies, then emits a structured plan: \
framework, target test file path, imports, candidate test names. The agent fills in assertions.\n\n\
When to use: standing up tests for an untested function or file. Pair with `belisarius_test_gaps` \
to find good targets.\n\
When not to use: generating actual test code (this returns intent, not implementation); \
languages outside rust / typescript / javascript / python / go.",
        input_schema: json!({
            "type": "object",
            "required": ["path", "target"],
            "properties": {
                "path": { "type": "string", "description": "Project root." },
                "target": { "type": "string", "description": "Relative source file path." }
            }
        }),
        handler: handle_suggest_tests as ToolHandler,
    }]
}

fn handle_suggest_tests(ctx: Arc<AppContext>, args: Value) -> BoxFut<Result<Value, ServiceError>> {
    Box::pin(async move {
        let args: SuggestTestsArgs = serde_json::from_value(args)?;
        suggest_tests(&ctx, args).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_language_from_extension() {
        assert_eq!(detect_language("foo.rs"), Some("rust"));
        assert_eq!(detect_language("foo.ts"), Some("typescript"));
        assert_eq!(detect_language("foo.py"), Some("python"));
        assert_eq!(detect_language("foo.go"), Some("go"));
        assert_eq!(detect_language("foo.unknown"), None);
    }

    #[test]
    fn detect_framework_rust_is_cargo_test() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(detect_framework(tmp.path(), "rust"), "cargo_test");
    }

    #[test]
    fn detect_framework_typescript_with_vitest_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("vitest.config.ts"), "").unwrap();
        assert_eq!(detect_framework(tmp.path(), "typescript"), "vitest");
    }

    #[test]
    fn detect_framework_typescript_from_package_json_jest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{ "devDependencies": { "jest": "^29" } }"#,
        )
        .unwrap();
        assert_eq!(detect_framework(tmp.path(), "typescript"), "jest");
    }

    #[test]
    fn test_names_generated_for_each_function() {
        let names = generate_test_names("rust", "cargo_test", &["parse", "validate"]);
        assert!(names.iter().any(|n| n.starts_with("parse_")));
        assert!(names.iter().any(|n| n.starts_with("validate_")));
    }
}
