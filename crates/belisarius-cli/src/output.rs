//! Shared output layer — `Renderable` makes CLI `--json` and MCP tool output
//! identical by construction, and `Paginated<T>` advertises truncation /
//! pagination so agents never silently miss results.
//!
//! Pattern for new commands:
//! ```ignore
//! #[derive(serde::Serialize)]
//! struct MyReport { ... }
//! impl crate::output::Renderable for MyReport {
//!     fn render_human(&self, w: &mut dyn std::io::Write) -> std::io::Result<()> {
//!         writeln!(w, "tidy human summary")
//!     }
//! }
//! // in run():
//! crate::output::emit(&report, args.json)?;
//! ```

use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};

/// Output format selector. `Format::from_json_flag(args.json)` is the bridge
/// between the existing `--json` clap flag and the shared layer.
#[derive(Debug, Clone, Copy)]
pub enum Format {
    Json,
    Human,
}

impl Format {
    pub fn from_json_flag(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Human
        }
    }
}

/// Types that produce both a JSON shape and a human-readable summary. The
/// JSON shape must be the canonical schema — CLI `--json` and the matching
/// MCP tool MUST emit byte-identical bytes (after key sorting). The MCP↔CLI
/// parity test in `tests/parity.rs` enforces this.
///
/// Default implementations rely on `Serialize`. Override `render_human` for a
/// nicer terminal view; leave `render_json` alone unless the JSON output
/// must differ from the struct's serde shape (rare; usually a design smell).
pub trait Renderable: Serialize {
    fn render_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn render_human(&self, w: &mut dyn Write) -> io::Result<()> {
        let v = self.render_json();
        let s = serde_json::to_string_pretty(&v).unwrap_or_default();
        writeln!(w, "{s}")
    }
}

/// Render to an arbitrary writer. Useful for tests and the `belisarius help`
/// subcommands.
pub fn render<R: Renderable>(value: &R, fmt: Format, w: &mut dyn Write) -> io::Result<()> {
    match fmt {
        Format::Json => {
            let v = value.render_json();
            let s = serde_json::to_string_pretty(&v).unwrap_or_default();
            writeln!(w, "{s}")
        }
        Format::Human => value.render_human(w),
    }
}

/// Emit to stdout based on the `--json` flag convention every command already
/// uses. The single source of truth for "JSON or pretty?".
pub fn emit<R: Renderable>(value: &R, json: bool) -> io::Result<()> {
    let fmt = Format::from_json_flag(json);
    render(value, fmt, &mut std::io::stdout().lock())
}

/// Pagination envelope for list-returning operations. Every service that
/// applies a `limit` should wrap its result in `Paginated` so clients can
/// detect truncation (`truncated: true`) and continue with `next_offset`.
///
/// `next_offset` is `None` when the page is the last one.
///
/// **Note on dead-code allowance:** today every service that needs
/// pagination signaling does so via flat fields on the existing response
/// (e.g. `belisarius_hotspots` adds `total_count`/`returned`/`truncated`
/// alongside its `hotspots` array). `Paginated<T>` is kept here as the
/// canonical envelope for *new* list-only services so they don't have to
/// reinvent the shape. The unit tests below exercise both constructors;
/// the suppression is solely about silencing the "infrastructure waiting
/// to be wired up" warning at the public API boundary.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub total_count: usize,
    pub returned: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[allow(dead_code)]
impl<T> Paginated<T> {
    /// Build a Paginated from a fully-materialized `Vec<T>` by slicing
    /// `[offset..offset+limit]`. The caller does NOT have to truncate first.
    pub fn slice(mut all: Vec<T>, offset: usize, limit: usize) -> Self {
        let total_count = all.len();
        if offset >= total_count || limit == 0 {
            return Self {
                items: Vec::new(),
                total_count,
                returned: 0,
                truncated: offset < total_count && limit == 0,
                next_offset: None,
            };
        }
        all.drain(..offset);
        let truncated = all.len() > limit;
        all.truncate(limit);
        let returned = all.len();
        let next_offset = if truncated {
            Some(offset + returned)
        } else {
            None
        };
        Self {
            items: all,
            total_count,
            returned,
            truncated,
            next_offset,
        }
    }

    /// Build a Paginated from an already-sliced page when you know the total
    /// count separately (e.g. SQL `COUNT(*)` + `LIMIT` query). Cheaper than
    /// materializing the whole list in memory. The page size is implied by
    /// `items.len()` — callers don't need to pass it again.
    pub fn from_page(items: Vec<T>, offset: usize, total_count: usize) -> Self {
        let returned = items.len();
        let truncated = offset + returned < total_count;
        let next_offset = if truncated {
            Some(offset + returned)
        } else {
            None
        };
        Self {
            items,
            total_count,
            returned,
            truncated,
            next_offset,
        }
    }
}

impl<T: Serialize> Renderable for Paginated<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Item {
        n: u32,
    }

    fn items(n: u32) -> Vec<Item> {
        (0..n).map(|n| Item { n }).collect()
    }

    #[test]
    fn slice_first_page_marks_truncated() {
        let p = Paginated::slice(items(10), 0, 3);
        assert_eq!(p.total_count, 10);
        assert_eq!(p.returned, 3);
        assert!(p.truncated);
        assert_eq!(p.next_offset, Some(3));
    }

    #[test]
    fn slice_last_page_clears_truncated() {
        let p = Paginated::slice(items(10), 8, 3);
        assert_eq!(p.total_count, 10);
        assert_eq!(p.returned, 2);
        assert!(!p.truncated);
        assert!(p.next_offset.is_none());
    }

    #[test]
    fn slice_offset_past_end_is_empty() {
        let p = Paginated::slice(items(3), 10, 3);
        assert_eq!(p.total_count, 3);
        assert_eq!(p.returned, 0);
        assert!(!p.truncated);
    }

    #[test]
    fn from_page_with_known_total() {
        let p = Paginated::from_page(items(3), 0, 12);
        assert_eq!(p.total_count, 12);
        assert!(p.truncated);
        assert_eq!(p.next_offset, Some(3));
    }

    #[test]
    fn from_page_last_page_clears_truncated() {
        let p = Paginated::from_page(items(2), 10, 12);
        assert_eq!(p.total_count, 12);
        assert!(!p.truncated);
        assert!(p.next_offset.is_none());
    }

    #[test]
    fn paginated_json_skips_none_next_offset() {
        let p = Paginated::slice(items(2), 0, 5);
        let v = p.render_json();
        assert!(
            v.get("next_offset").is_none(),
            "next_offset should be omitted when None"
        );
    }

    #[test]
    fn renderable_default_human_prints_pretty_json() {
        #[derive(Serialize)]
        struct Foo {
            a: i32,
            b: String,
        }
        impl Renderable for Foo {}
        let foo = Foo {
            a: 1,
            b: "x".into(),
        };
        let mut buf = Vec::new();
        render(&foo, Format::Human, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"a\": 1"));
        assert!(s.contains("\"b\": \"x\""));
    }
}
