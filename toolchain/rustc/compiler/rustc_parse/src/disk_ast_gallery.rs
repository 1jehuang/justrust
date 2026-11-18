//! Persist the lexical SymbolGallery effects of an admitted clean token stream.
//!
//! Capture immediately after successful, diagnostic-free source_file_to_stream,
//! before parsing/mutating tokens, with no span override. The stream must be the
//! complete stream for this SourceFile. Admission uses the resident lexer's
//! conservative side-effect proof. This does not admit arbitrary Rust syntax or
//! replay raw-identifier spans, gates, lints, or diagnostics.
//!
//! Never capture by filtering ParseSess.symbol_gallery: an earlier source may
//! already own a symbol's first span, hiding this source's lexical insertion.
//! Instead walk real tokens in lexical order, including delimiter interleaving.
//! Retain spellings and absolute root spans, never session-local Symbol indices.
//!
//! The caller must bind these bytes to its exact source/edition/strip/toolchain
//! cache key and corruption-checked private envelope, and validate the AST before
//! apply. This helper is not an authenticated format or a source identity check.
//! No lexer or parser is constructed by decode/apply. No module is wired here.

use std::collections::HashSet;

use rustc_ast::token::TokenKind;
use rustc_ast::tokenstream::{TokenStream, TokenTree};
use rustc_serialize::opaque::mem_encoder::MemEncoder;
use rustc_serialize::opaque::{MAGIC_END_BYTES, MemDecoder};
use rustc_serialize::{Decoder, Encoder};
use rustc_session::parse::ParseSess;
use rustc_span::{BytePos, SourceFile, Span, Symbol, SyntaxContext};

use crate::resident_eligibility::eligible_source;

const VERSION: u32 = 1;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_TREES: usize = 65536;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    spelling: String,
    lo: u32,
    hi: u32,
}

/// Owned, bounded, session-independent lexical effects. Fields are private so
/// only capture or validated decoding can produce an applicable gallery.
#[derive(Debug, PartialEq, Eq)]
pub struct Gallery {
    start: u32,
    end: u32,
    entries: Vec<Entry>,
}

fn bounds(source: &SourceFile) -> Option<(u32, u32)> {
    let text = source.src.as_ref()?;
    if text.len() > MAX_SOURCE {
        return None;
    }
    let start = source.start_pos.0;
    Some((start, start.checked_add(u32::try_from(text.len()).ok()?)?))
}

impl Gallery {
    /// Capture all lexical insertions, even if their symbols already occur in
    /// another file's gallery. Deduplicate only within this stream, first wins.
    /// Caller guarantees successful clean lexing of this complete source.
    pub fn capture(source: &SourceFile, stream: &TokenStream) -> Option<Self> {
        let text = source.src.as_ref()?;
        if !eligible_source(text, MAX_SOURCE) {
            return None;
        }
        let (start, end) = bounds(source)?;
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![stream.iter()];
        let mut trees = 0usize;
        let mut last_hi = start;
        while let Some(iter) = pending.last_mut() {
            let Some(tree) = iter.next() else {
                pending.pop();
                continue;
            };
            trees += 1;
            if trees > MAX_TREES {
                return None;
            }
            match tree {
                TokenTree::Delimited(_, _, _, inner) => pending.push(inner.iter()),
                TokenTree::Token(token, _) => {
                    if let TokenKind::Ident(symbol, _) = token.kind {
                        let span = token.span.data();
                        if !span.ctxt.is_root() || span.parent.is_some() {
                            return None;
                        }
                        let entry = Entry {
                            spelling: symbol.as_str().to_owned(),
                            lo: span.lo.0,
                            hi: span.hi.0,
                        };
                        if entry.lo < last_hi || !valid_entry(&entry, source, start, end) {
                            return None;
                        }
                        last_hi = entry.hi;
                        if seen.insert(symbol) {
                            entries.push(entry);
                        }
                    }
                }
            }
        }
        Some(Self {
            start,
            end,
            entries,
        })
    }

    /// Native primitive encoding with explicitly UTF-8-checked spelling bytes.
    /// This is a private compiler-versioned payload, not a stable wire format.
    pub fn encode(&self) -> Vec<u8> {
        let mut encoder = MemEncoder::new();
        encoder.emit_u32(VERSION);
        encoder.emit_u32(self.start);
        encoder.emit_u32(self.end);
        encoder.emit_usize(self.entries.len());
        for entry in &self.entries {
            encoder.emit_u32(entry.lo);
            encoder.emit_u32(entry.hi);
            encoder.emit_usize(entry.spelling.len());
            encoder.emit_raw_bytes(entry.spelling.as_bytes());
        }
        let mut bytes = encoder.finish();
        bytes.extend_from_slice(MAGIC_END_BYTES);
        bytes
    }

    /// Fail closed on malformed/truncated native input, invalid UTF-8, oversized
    /// sequences, duplicate symbols, nonlexical order or foreign source spans.
    /// Does not intern symbols or mutate ParseSess, including on failure.
    /// Completeness relies on the caller's exact-key, integrity-checked envelope.
    pub fn decode(bytes: &[u8], source: &SourceFile) -> Option<Self> {
        if bytes.len() > MAX_BYTES || source.src.as_ref()?.len() > MAX_SOURCE {
            return None;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (start, end) = bounds(source)?;
            let mut decoder = MemDecoder::new(bytes, 0).ok()?;
            if decoder.read_u32() != VERSION
                || decoder.read_u32() != start
                || decoder.read_u32() != end
            {
                return None;
            }
            let count = decoder.read_usize();
            if count > MAX_TREES || count > (end - start) as usize {
                return None;
            }
            let mut entries = Vec::new();
            let mut seen = HashSet::new();
            let mut last_hi = start;
            let mut total = 0usize;
            for _ in 0..count {
                let lo = decoder.read_u32();
                let hi = decoder.read_u32();
                let len = decoder.read_usize();
                total = total.checked_add(len)?;
                if len == 0 || total > MAX_SOURCE {
                    return None;
                }
                // Do not use Decoder::read_str, which can trust unchecked UTF-8.
                let spelling = std::str::from_utf8(decoder.read_raw_bytes(len))
                    .ok()?
                    .to_owned();
                let entry = Entry { spelling, lo, hi };
                if lo < last_hi
                    || !valid_entry(&entry, source, start, end)
                    || !seen.insert(entry.spelling.clone())
                {
                    return None;
                }
                last_hi = hi;
                entries.push(entry);
            }
            if decoder.position() != bytes.len().checked_sub(MAGIC_END_BYTES.len())? {
                return None;
            }
            Some(Self {
                start,
                end,
                entries,
            })
        }))
        .ok()
        .flatten()
    }

    /// Replay only after all cache acceptance checks succeed. Existing entries
    /// in this session retain both their spans and their insertion order.
    /// SourceMap ownership and exact source identity are the caller's contract.
    pub fn apply(&self, psess: &ParseSess) {
        for entry in &self.entries {
            psess.symbol_gallery.insert(
                Symbol::intern(&entry.spelling),
                Span::new(
                    BytePos(entry.lo),
                    BytePos(entry.hi),
                    SyntaxContext::root(),
                    None,
                ),
            );
        }
    }
}

fn valid_entry(entry: &Entry, source: &SourceFile, start: u32, end: u32) -> bool {
    if entry.lo < start || entry.hi > end || entry.lo >= entry.hi {
        return false;
    }
    let bytes = entry.spelling.as_bytes();
    let ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    ident_start(bytes.first().copied().unwrap_or(0))
        && bytes.iter().all(|&b| ident_start(b) || b.is_ascii_digit())
        && source
            .src
            .as_ref()
            .and_then(|text| text.get((entry.lo - start) as usize..(entry.hi - start) as usize))
            == Some(entry.spelling.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::StripTokens;
    use crate::source_file_to_stream;
    use rustc_errors::DiagCtxt;
    use rustc_span::{DUMMY_SP, FileName, create_default_session_globals_then};

    fn fresh(psess: &ParseSess) -> ParseSess {
        ParseSess::with_dcx(
            DiagCtxt::new(Box::new(rustc_errors::emitter::SilentEmitter)),
            psess.clone_source_map(),
        )
    }

    #[test]
    fn native_roundtrip_nested_order_comments_and_preseeded_capture() {
        create_default_session_globals_then(|| {
            let psess = ParseSess::new();
            let text = "outer (inner [deep] inner) after /* absent */ outer // ignored\n";
            let source = psess
                .source_map()
                .new_source_file(FileName::anon_source_code(text), text.to_owned());
            let preseeded = fresh(&psess);
            preseeded
                .symbol_gallery
                .insert(Symbol::intern("inner"), DUMMY_SP);
            let stream =
                source_file_to_stream(&preseeded, source.clone(), None, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("clean stream"));
            let gallery = Gallery::capture(&source, &stream).unwrap();
            assert_eq!(
                gallery
                    .entries
                    .iter()
                    .map(|e| e.spelling.as_str())
                    .collect::<Vec<_>>(),
                ["outer", "inner", "deep", "after"]
            );
            let bytes = gallery.encode();
            let decoded = Gallery::decode(&bytes, &source).unwrap();
            assert_eq!(gallery, decoded);
            let baseline = fresh(&psess);
            source_file_to_stream(&baseline, source.clone(), None, StripTokens::Nothing)
                .unwrap_or_else(|_| panic!("baseline"));
            let replay = fresh(&psess);
            decoded.apply(&replay);
            assert_eq!(
                *baseline.symbol_gallery.symbols.lock(),
                *replay.symbol_gallery.symbols.lock()
            );
            let existing = fresh(&psess);
            existing
                .symbol_gallery
                .insert(Symbol::intern("inner"), DUMMY_SP);
            decoded.apply(&existing);
            assert_eq!(
                *preseeded.symbol_gallery.symbols.lock(),
                *existing.symbol_gallery.symbols.lock()
            );
            assert_eq!(
                existing.symbol_gallery.symbols.lock()[&Symbol::intern("inner")],
                DUMMY_SP
            );
            for len in 0..bytes.len() {
                assert!(Gallery::decode(&bytes[..len], &source).is_none());
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert!(Gallery::decode(&trailing, &source).is_none());
        });
    }

    #[test]
    fn empty_and_rejected_sources_and_foreign_spans() {
        create_default_session_globals_then(|| {
            let psess = ParseSess::new();
            for text in ["", "/// doc\n", "r#name", "\"string\"", "café", "'life"] {
                let source = psess
                    .source_map()
                    .new_source_file(FileName::anon_source_code(text), text.to_owned());
                let result = Gallery::capture(&source, &TokenStream::default());
                assert_eq!(result.is_some(), text.is_empty());
                if let Some(gallery) = result {
                    assert_eq!(Gallery::decode(&gallery.encode(), &source), Some(gallery));
                }
            }
            let a = psess
                .source_map()
                .new_source_file(FileName::anon_source_code("a"), "a".to_owned());
            let b = psess
                .source_map()
                .new_source_file(FileName::anon_source_code("b"), "a".to_owned());
            let stream = source_file_to_stream(&psess, a.clone(), None, StripTokens::Nothing)
                .unwrap_or_else(|_| panic!("valid"));
            let gallery = Gallery::capture(&a, &stream).unwrap();
            assert!(Gallery::capture(&b, &stream).is_none());
            assert!(Gallery::decode(&gallery.encode(), &b).is_none());
            let huge = psess.source_map().new_source_file(
                FileName::anon_source_code("huge"),
                " ".repeat(MAX_SOURCE + 1),
            );
            assert!(Gallery::capture(&huge, &TokenStream::default()).is_none());
            assert!(Gallery::decode(&vec![0; MAX_BYTES + 1], &a).is_none());
        });
    }

    #[test]
    fn malformed_native_entries_are_rejected_without_interning() {
        create_default_session_globals_then(|| {
            let psess = ParseSess::new();
            let source = psess
                .source_map()
                .new_source_file(FileName::anon_source_code("a a"), "a a".to_owned());
            let (start, end) = bounds(&source).unwrap();
            for (count, lo, hi, spelling) in [
                (1, start, start + 1, vec![0xff]),
                (1, start, start + 1, b"b".to_vec()),
                (1, end, start, b"a".to_vec()),
                (MAX_TREES + 1, start, start + 1, b"a".to_vec()),
                (2, start, start + 1, b"a".to_vec()),
            ] {
                let mut encoder = MemEncoder::new();
                encoder.emit_u32(VERSION);
                encoder.emit_u32(start);
                encoder.emit_u32(end);
                encoder.emit_usize(count);
                for _ in 0..count.min(2) {
                    encoder.emit_u32(lo);
                    encoder.emit_u32(hi);
                    encoder.emit_usize(spelling.len());
                    encoder.emit_raw_bytes(&spelling);
                }
                let mut bytes = encoder.finish();
                bytes.extend_from_slice(MAGIC_END_BYTES);
                assert!(Gallery::decode(&bytes, &source).is_none());
                assert!(psess.symbol_gallery.symbols.lock().is_empty());
            }
        });
    }
}
