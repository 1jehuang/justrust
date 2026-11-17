//! Bounded token retention scoped strictly inside one serial SessionGlobals owner.
//!
//! This is not an AST cache. Parsing, attribute IDs, gates and diagnostics still
//! run for every revision. Only the conservative clean lexer subset is cached.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

use rustc_ast::token::TokenKind;
use rustc_ast::tokenstream::{TokenStream, TokenTree};
use rustc_session::parse::ParseSess;
use rustc_span::edition::Edition;
use rustc_span::{SourceFile, Span, Symbol};

use crate::lexer::StripTokens;
use crate::resident_eligibility::eligible_source;

const MAX_FILES: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_TREES: usize = 65536;

#[derive(Clone, Copy, Default, Debug)]
pub struct LexerCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub rejected: usize,
    pub retained_entries: usize,
    pub retained_estimated_bytes: usize,
}

struct Entry {
    source: Arc<SourceFile>,
    source_map: Arc<rustc_span::source_map::SourceMap>,
    edition: Edition,
    strip: u8,
    stream: TokenStream,
    gallery: Vec<(Symbol, Span)>,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    stats: LexerCacheStats,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
}

/// The caller must keep one unchanged SessionGlobals/Symbol universe alive for
/// the entire closure and must execute parsing on this same serial thread.
/// Dropping the guard, including unwinding, drops tokens before that universe.
pub fn with_resident_lexer_cache<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    if !enabled {
        return f();
    }
    CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "resident lexer scopes cannot nest");
        *slot = Some(Cache::default());
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CACHE.with(|slot| *slot.borrow_mut() = None);
        }
    }
    let _reset = Reset;
    f()
}

pub fn stats() -> LexerCacheStats {
    CACHE.with(|slot| slot.borrow().as_ref().map(|cache| cache.stats).unwrap_or_default())
}

fn strip_key(strip: &StripTokens) -> u8 {
    match strip {
        StripTokens::Nothing => 0,
        StripTokens::Shebang => 1,
        StripTokens::ShebangAndFrontmatter => 2,
    }
}

pub(super) fn lookup(
    psess: &ParseSess,
    source: &Arc<SourceFile>,
    strip: &StripTokens,
) -> Option<TokenStream> {
    let found = CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cache = slot.as_mut()?;
        if let Some(entry) = cache.entries.iter().find(|entry| {
            Arc::ptr_eq(&entry.source, source)
                && std::ptr::eq(entry.source_map.as_ref(), psess.source_map())
                && entry.edition == psess.edition
                && entry.strip == strip_key(strip)
        }) {
            cache.stats.hits += 1;
            Some((entry.stream.clone(), entry.gallery.clone()))
        } else {
            cache.stats.misses += 1;
            None
        }
    });
    let (stream, gallery) = found?;
    // Preserve the first occurrence in this ParseSess, including its ordering
    // relative to any identifiers already encountered in another source file.
    for (symbol, span) in gallery {
        psess.symbol_gallery.insert(symbol, span);
    }
    Some(stream)
}

pub(super) fn should_capture(src: &str) -> bool {
    CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cache) = slot.as_mut() else { return false };
        if eligible_source(src, MAX_SOURCE) {
            true
        } else {
            cache.stats.rejected += 1;
            false
        }
    })
}

pub(super) fn insert(
    psess: &ParseSess,
    source: Arc<SourceFile>,
    strip: &StripTokens,
    stream: &TokenStream,
) {
    let mut gallery = Vec::new();
    let mut pending = vec![stream];
    let mut trees = 0usize;
    while let Some(stream) = pending.pop() {
        for tree in stream.iter() {
            trees += 1;
            if trees > MAX_TREES {
                return;
            }
            if let TokenTree::Delimited(_, _, _, inner) = tree {
                pending.push(inner);
            }
        }
    }
    // Explicit stack over individual token trees preserves interleaving of
    // identifiers outside and inside delimiters, unlike a stream-only stack.
    let mut pending: Vec<_> = (0..stream.len()).rev().map(|i| stream.get(i).unwrap()).collect();
    while let Some(tree) = pending.pop() {
        match tree {
            TokenTree::Token(token, _) => {
                if let TokenKind::Ident(symbol, _) = token.kind {
                    gallery.push((symbol, token.span));
                }
            }
            TokenTree::Delimited(_, _, _, inner) => {
                pending.extend((0..inner.len()).rev().map(|i| inner.get(i).unwrap()));
            }
        }
    }
    let source_bytes = source.src.as_ref().map_or(0, |src| src.len());
    let bytes = source_bytes
        + trees * std::mem::size_of::<TokenTree>()
        + gallery.len() * std::mem::size_of::<(Symbol, Span)>()
        + std::mem::size_of::<Entry>();
    if bytes > MAX_BYTES {
        return;
    }
    CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cache) = slot.as_mut() else { return };
        while cache.entries.len() >= MAX_FILES
            || cache.stats.retained_estimated_bytes + bytes > MAX_BYTES
        {
            let old = cache.entries.pop_front().expect("nonempty bounded lexer cache");
            cache.stats.retained_estimated_bytes -= old.bytes;
        }
        cache.entries.push_back(Entry {
            source,
            source_map: psess.clone_source_map(),
            edition: psess.edition,
            strip: strip_key(strip),
            stream: stream.clone(),
            gallery,
            bytes,
        });
        cache.stats.retained_entries = cache.entries.len();
        cache.stats.retained_estimated_bytes += bytes;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_file_to_stream;
    use rustc_errors::DiagCtxt;
    use rustc_span::{FileName, create_default_session_globals_then};

    fn session(sm: Arc<rustc_span::source_map::SourceMap>) -> ParseSess {
        ParseSess::with_dcx(DiagCtxt::new(Box::new(rustc_errors::emitter::SilentEmitter)), sm)
    }

    #[test]
    fn gallery_order_exact_tokens_and_shared_source_hit() {
        create_default_session_globals_then(|| {
            with_resident_lexer_cache(true, || {
                let first = ParseSess::new();
                let source = first.source_map().new_source_file(
                    FileName::anon_source_code("clean"),
                    "fn value(x: u32) -> u32 { let y = x + 7; y }".to_owned(),
                );
                let a = source_file_to_stream(&first, source.clone(), None, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("valid tokens"));
                let gallery: Vec<_> = first
                    .symbol_gallery
                    .symbols
                    .lock()
                    .iter()
                    .map(|(symbol, span)| (*symbol, *span))
                    .collect();
                let next = session(first.clone_source_map());
                let b = source_file_to_stream(&next, source, None, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("cached tokens"));
                assert_eq!(a, b);
                assert_eq!(
                    gallery,
                    next.symbol_gallery
                        .symbols
                        .lock()
                        .iter()
                        .map(|(symbol, span)| (*symbol, *span))
                        .collect::<Vec<_>>()
                );
                assert_eq!(stats().hits, 1);
                assert_eq!(stats().retained_entries, 1);
                assert!(next.raw_identifier_spans.get(0).is_none());
                assert!(next.buffered_lints.lock().is_empty());
                assert!(next.bad_unicode_identifiers.lock().is_empty());
                let preseeded = session(first.clone_source_map());
                preseeded.symbol_gallery.insert(gallery[0].0, rustc_span::DUMMY_SP);
                let source = first.source_map().files()[0].clone();
                source_file_to_stream(&preseeded, source, None, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("preseeded cached tokens"));
                assert_eq!(
                    preseeded.symbol_gallery.symbols.lock()[&gallery[0].0],
                    rustc_span::DUMMY_SP
                );
            });
            assert_eq!(stats().retained_entries, 0);
        });
    }

    #[test]
    fn malformed_and_ineligible_sources_never_enter_cache() {
        create_default_session_globals_then(|| {
            with_resident_lexer_cache(true, || {
                let psess = ParseSess::new();
                for text in ["(", "([)]", "// comment\nfn f() {}", "r#value", "\"text\""] {
                    let source = psess
                        .source_map()
                        .new_source_file(FileName::anon_source_code(text), text.to_owned());
                    for _ in 0..2 {
                        if let Err(errors) = source_file_to_stream(
                            &psess,
                            source.clone(),
                            None,
                            StripTokens::Nothing,
                        ) {
                            for error in errors {
                                error.cancel();
                            }
                        }
                    }
                }
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().retained_entries, 0);
                assert_eq!(stats().rejected, 6);
            });
        });
    }

    #[test]
    fn strip_edition_override_and_distinct_files_do_not_alias() {
        create_default_session_globals_then(|| {
            with_resident_lexer_cache(true, || {
                let mut psess = ParseSess::new();
                let source = psess
                    .source_map()
                    .new_source_file(FileName::anon_source_code("a"), "fn f() {}".to_owned());
                source_file_to_stream(&psess, source.clone(), None, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("valid"));
                let foreign_map = ParseSess::new();
                assert!(lookup(&foreign_map, &source, &StripTokens::Nothing).is_none());
                assert!(lookup(&psess, &source, &StripTokens::Shebang).is_none());
                psess.edition = Edition::Edition2024;
                assert!(lookup(&psess, &source, &StripTokens::Nothing).is_none());
                let different = psess
                    .source_map()
                    .new_source_file(FileName::anon_source_code("b"), "fn f() {}".to_owned());
                assert!(lookup(&psess, &different, &StripTokens::Nothing).is_none());
                source_file_to_stream(
                    &psess,
                    source,
                    Some(rustc_span::DUMMY_SP),
                    StripTokens::Nothing,
                )
                .unwrap_or_else(|_| panic!("override"));
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().retained_entries, 1);
            });
        });
    }

    #[test]
    fn unwind_discards_scope_and_disabled_mode_remains_zero() {
        let result = std::panic::catch_unwind(|| {
            with_resident_lexer_cache(true, || panic!("test unwind"));
        });
        assert!(result.is_err());
        with_resident_lexer_cache(false, || assert_eq!(stats().hits, 0));
        with_resident_lexer_cache(true, || assert_eq!(stats().retained_entries, 0));
    }

    #[test]
    fn file_eviction_and_oversized_source_are_bounded() {
        create_default_session_globals_then(|| {
            with_resident_lexer_cache(true, || {
                let psess = ParseSess::new();
                let mut oldest = None;
                for index in 0..=MAX_FILES {
                    let text = format!("fn value_{index}() {{}}");
                    let source =
                        psess.source_map().new_source_file(FileName::anon_source_code(&text), text);
                    oldest.get_or_insert_with(|| source.clone());
                    source_file_to_stream(&psess, source, None, StripTokens::Nothing)
                        .unwrap_or_else(|_| panic!("bounded valid tokens"));
                    assert!(stats().retained_entries <= MAX_FILES);
                    assert!(stats().retained_estimated_bytes <= MAX_BYTES);
                }
                assert_eq!(stats().retained_entries, MAX_FILES);
                assert!(lookup(&psess, &oldest.unwrap(), &StripTokens::Nothing).is_none());
                assert!(!should_capture(&" ".repeat(MAX_SOURCE + 1)));
                assert_eq!(stats().rejected, 1);
            });
        });
    }
}
