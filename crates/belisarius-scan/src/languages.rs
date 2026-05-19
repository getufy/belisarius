/// Map a file extension (lowercase, no leading dot) to a language id.
pub fn language_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "ts" | "tsx" | "mts" | "cts" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "rs" => Some("rust"),
        "py" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "rb" => Some("ruby"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "md" => Some("markdown"),
        "html" => Some("html"),
        "css" => Some("css"),
        _ => None,
    }
}

/// Languages for which we know how to extract import edges in v1.
pub fn has_parser(lang: &str) -> bool {
    matches!(lang, "typescript" | "javascript" | "rust" | "python" | "go")
}
