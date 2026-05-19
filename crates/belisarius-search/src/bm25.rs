//! Tantivy-backed BM25 leg with a code-aware tokenizer.
//!
//! The tokenizer lowercases input, splits on every non-alphanumeric character,
//! then *additionally* splits camelCase (`fooBar` → `foo bar`) and preserves
//! snake_case parts after the alnum split. Without this, queries like
//! `parse_scip` would only match `parse_scip` and miss `parseSCIP`.

use crate::chunker::Chunk;
use anyhow::{Context, Result};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, STRING};
use tantivy::tokenizer::{
    BoxTokenStream, Language, LowerCaser, Stemmer, TextAnalyzer, Token, TokenStream, Tokenizer,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

const TOKENIZER_NAME: &str = "code";

#[derive(Clone)]
pub struct Bm25Index {
    pub index: Index,
    pub reader: IndexReader,
    pub fields: Bm25Fields,
}

#[derive(Clone, Copy)]
pub struct Bm25Fields {
    pub chunk_id: Field,
    pub file: Field,
    pub name: Field,
    pub content: Field,
    pub lang: Field,
    pub kind: Field,
}

impl Bm25Index {
    pub fn open_or_create(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).ok();
        let mut sb = Schema::builder();
        let chunk_id = sb.add_i64_field("chunk_id", tantivy::schema::INDEXED | STORED);
        let file = sb.add_text_field("file", STRING | STORED);
        let name = sb.add_text_field(
            "name",
            tantivy::schema::TextOptions::default()
                .set_indexing_options(
                    tantivy::schema::TextFieldIndexing::default()
                        .set_tokenizer(TOKENIZER_NAME)
                        .set_index_option(
                            tantivy::schema::IndexRecordOption::WithFreqsAndPositions,
                        ),
                )
                .set_stored(),
        );
        let content = sb.add_text_field(
            "content",
            tantivy::schema::TextOptions::default()
                .set_indexing_options(
                    tantivy::schema::TextFieldIndexing::default()
                        .set_tokenizer(TOKENIZER_NAME)
                        .set_index_option(
                            tantivy::schema::IndexRecordOption::WithFreqsAndPositions,
                        ),
                )
                .set_stored(),
        );
        let lang = sb.add_text_field("lang", STRING | STORED);
        let kind = sb.add_text_field("kind", STRING | STORED);
        let schema = sb.build();

        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir).with_context(|| format!("open tantivy at {}", dir.display()))?
        } else {
            Index::create_in_dir(dir, schema.clone())
                .with_context(|| format!("create tantivy at {}", dir.display()))?
        };
        register_tokenizer(&index);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("build tantivy reader")?;

        Ok(Self {
            index,
            reader,
            fields: Bm25Fields {
                chunk_id,
                file,
                name,
                content,
                lang,
                kind,
            },
        })
    }

    pub fn writer(&self, mem_bytes: usize) -> Result<IndexWriter> {
        Ok(self.index.writer(mem_bytes)?)
    }

    pub fn add(&self, w: &mut IndexWriter, id: i64, c: &Chunk) -> Result<()> {
        w.add_document(doc!(
            self.fields.chunk_id => id,
            self.fields.file => c.file.as_str(),
            self.fields.name => c.name.as_str(),
            self.fields.content => c.content.as_str(),
            self.fields.lang => c.lang.as_str(),
            self.fields.kind => kind_str(c.kind),
        ))?;
        Ok(())
    }

    pub fn delete_for_file(&self, w: &mut IndexWriter, file: &str) -> Result<()> {
        let term = Term::from_field_text(self.fields.file, file);
        w.delete_term(term);
        Ok(())
    }

    pub fn commit(&self, w: &mut IndexWriter) -> Result<()> {
        w.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Run a query string against `name` + `content` (boosted name 2x).
    pub fn search(&self, q: &str, k: usize) -> Result<Vec<(i64, f32)>> {
        let searcher = self.reader.searcher();
        let mut qp =
            QueryParser::for_index(&self.index, vec![self.fields.name, self.fields.content]);
        qp.set_field_boost(self.fields.name, 2.0);
        // Be permissive: don't fail on syntax, fall back to terms.
        let query = match qp.parse_query(q) {
            Ok(q) => q,
            Err(_) => {
                let sanitized = sanitize_query(q);
                qp.parse_query(&sanitized).context("parse fallback query")?
            }
        };
        let top = searcher.search(&query, &TopDocs::with_limit(k))?;
        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr)?;
            if let Some(id_value) = doc.get_first(self.fields.chunk_id) {
                if let Some(id) = id_value.as_i64() {
                    out.push((id, score));
                }
            }
        }
        Ok(out)
    }
}

fn kind_str(k: crate::chunker::ChunkKind) -> &'static str {
    use crate::chunker::ChunkKind::*;
    match k {
        Function => "function",
        Window => "window",
        Artifact => "artifact",
    }
}

fn sanitize_query(q: &str) -> String {
    q.chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

fn register_tokenizer(index: &Index) {
    let tokenizer = TextAnalyzer::builder(CodeTokenizer)
        .filter(LowerCaser)
        .filter(Stemmer::new(Language::English))
        .build();
    index.tokenizers().register(TOKENIZER_NAME, tokenizer);
}

/// Splits on non-alphanumeric AND on camelCase boundaries.
#[derive(Clone)]
struct CodeTokenizer;

impl Tokenizer for CodeTokenizer {
    type TokenStream<'a> = BoxTokenStream<'a>;
    fn token_stream<'a>(&'a mut self, text: &'a str) -> BoxTokenStream<'a> {
        let toks = split_code(text);
        BoxTokenStream::new(VecTokenStream { toks, idx: 0 })
    }
}

struct VecTokenStream {
    toks: Vec<Token>,
    idx: usize,
}

impl TokenStream for VecTokenStream {
    fn advance(&mut self) -> bool {
        if self.idx >= self.toks.len() {
            return false;
        }
        self.idx += 1;
        true
    }
    fn token(&self) -> &Token {
        &self.toks[self.idx - 1]
    }
    fn token_mut(&mut self) -> &mut Token {
        &mut self.toks[self.idx - 1]
    }
}

fn split_code(text: &str) -> Vec<Token> {
    // Iterate over `char_indices` so byte offsets always land on a UTF-8
    // boundary. The previous byte-level loop walked each byte of a
    // multi-byte char as a separate "non-alnum" step and then sliced
    // `text[start..i]` — which panics when `i` falls mid-character (the
    // case that bit us on markdown bodies containing em-dashes, smart
    // quotes, etc.).
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut iter = text.char_indices().peekable();

    while let Some(&(_, c)) = iter.peek() {
        if !is_word_char(c) {
            iter.next();
            continue;
        }
        let start = iter.peek().map(|(i, _)| *i).unwrap_or(text.len());
        // Consume the run of word chars.
        let mut end = start;
        while let Some(&(i, c)) = iter.peek() {
            if !is_word_char(c) {
                break;
            }
            end = i + c.len_utf8();
            iter.next();
        }
        let raw = &text[start..end];
        push_token(&mut out, raw, start, end, &mut pos);
        for piece in split_camel(raw) {
            if piece.len() != raw.len() {
                push_token(&mut out, piece, start, end, &mut pos);
            }
        }
    }
    out
}

fn push_token(out: &mut Vec<Token>, text: &str, start: usize, end: usize, pos: &mut usize) {
    let tok = Token {
        offset_from: start,
        offset_to: end,
        position: *pos,
        text: text.to_string(),
        position_length: 1,
    };
    *pos += 1;
    out.push(tok);
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn split_camel(s: &str) -> Vec<&str> {
    // Mirrors `split_code`: walks chars, accumulates byte offsets, and only
    // splits on identifier transitions that fit ASCII identifier syntax
    // (lower→Upper, alpha↔digit). Unicode letters / digits pass through
    // unsplit — they're rare in identifiers and best left as one token.
    let mut out: Vec<&str> = Vec::new();
    for under in s.split('_') {
        if under.is_empty() {
            continue;
        }
        let mut last_byte = 0usize;
        let mut prev: Option<char> = None;
        for (i, c) in under.char_indices() {
            if let Some(a) = prev {
                let trans = (a.is_ascii_lowercase() && c.is_ascii_uppercase())
                    || (a.is_alphabetic() && c.is_ascii_digit())
                    || (a.is_ascii_digit() && c.is_alphabetic());
                if trans {
                    out.push(&under[last_byte..i]);
                    last_byte = i;
                }
            }
            prev = Some(c);
        }
        if last_byte < under.len() {
            out.push(&under[last_byte..]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_and_snake_split() {
        let toks: Vec<String> = split_code("parseScipIndex parse_scip_index loadSCIP42")
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert!(
            toks.contains(&"parsescipindex".to_string())
                || toks.contains(&"parseScipIndex".to_string())
        );
        // After camel-split we should see "parse", "Scip", "Index" pieces
        assert!(toks.iter().any(|t| t.eq_ignore_ascii_case("parse")));
        assert!(toks.iter().any(|t| t.eq_ignore_ascii_case("scip")));
        assert!(toks.iter().any(|t| t.eq_ignore_ascii_case("index")));
    }

    /// Regression: indexing a markdown body with non-ASCII separators
    /// (em-dash, smart quotes, ellipsis) used to panic in `split_code` with
    /// "byte index N is not a char boundary". Now char-aware. `split_code`
    /// preserves source case — lower-casing happens later in the analyzer
    /// chain — so the asserts use `eq_ignore_ascii_case`.
    #[test]
    fn tokenizer_handles_multibyte_chars() {
        let text = "Claude's interface is a literary salon — warm \u{201C}quiet\u{201D} prose…";
        let toks: Vec<String> = split_code(text).into_iter().map(|t| t.text).collect();
        for needle in [
            "claude",
            "interface",
            "literary",
            "salon",
            "warm",
            "quiet",
            "prose",
        ] {
            assert!(
                toks.iter().any(|t| t.eq_ignore_ascii_case(needle)),
                "missing {needle:?} in {toks:?}",
            );
        }
    }

    /// Long-form regression covering the exact panic-message shape from the
    /// field report (em-dash at byte 619 of a markdown intro).
    #[test]
    fn em_dash_in_long_text_no_panic() {
        let body = format!(
            "# Recipe App\n\n{}\n\nThis is a paragraph — with an em-dash that previously broke indexing.",
            "x".repeat(600),
        );
        let toks = split_code(&body);
        assert!(!toks.is_empty());
    }

    #[test]
    fn bm25_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Bm25Index::open_or_create(dir.path()).unwrap();
        let mut w = idx.writer(15_000_000).unwrap();
        let c = Chunk {
            file: "a.rs".into(),
            lang: "rust".into(),
            kind: crate::chunker::ChunkKind::Function,
            name: "parseScipIndex".into(),
            start_line: 1,
            end_line: 10,
            content: "fn parseScipIndex(data: &[u8]) -> Result<Index> { todo!() }".into(),
        };
        idx.add(&mut w, 1, &c).unwrap();
        idx.commit(&mut w).unwrap();
        let hits = idx.search("scip parse", 10).unwrap();
        assert!(!hits.is_empty(), "expected hits for 'scip parse'");
        assert_eq!(hits[0].0, 1);
    }
}
