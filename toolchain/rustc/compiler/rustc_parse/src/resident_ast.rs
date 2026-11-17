//! Owned pre-expansion AST retention inside one serial SessionGlobals lifetime.
//!
//! Lexing and Parser construction always run fresh, including their diagnostics
//! and SymbolGallery effects. Only a narrowly admitted, effect-free crate parse
//! may be replaced with a clone. Command-line attributes are injected afterwards.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use rustc_ast as ast;
use rustc_errors::PResult;
use rustc_session::parse::ParseSess;
use rustc_span::SourceFile;
use rustc_span::edition::Edition;
use rustc_span::source_map::SourceMap;

use crate::lexer::StripTokens;
use crate::resident_ast_eligibility::{Limits, inspect};

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_GALLERY: usize = 65536;

#[derive(Clone, Copy, Debug, Default)]
pub struct AstCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub rejected: usize,
    pub retained_entries: usize,
    pub retained_estimated_bytes: usize,
}

struct Entry {
    source: Arc<SourceFile>,
    source_map: Arc<SourceMap>,
    edition: Edition,
    strip: u8,
    krate: ast::Crate,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    stats: AstCacheStats,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
}

/// The caller owns an unchanged SessionGlobals universe throughout this serial
/// closure. The guard drops every AST before that universe, including on unwind.
pub fn with_resident_ast_cache<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    if !enabled {
        return f();
    }
    CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "resident AST scopes cannot nest");
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

pub fn enabled() -> bool {
    CACHE.with(|slot| slot.borrow().is_some())
}

pub fn stats() -> AstCacheStats {
    CACHE.with(|slot| slot.borrow().as_ref().map(|cache| cache.stats).unwrap_or_default())
}

fn strip_key(strip: &StripTokens) -> u8 {
    match strip {
        StripTokens::Nothing => 0,
        StripTokens::Shebang => 1,
        StripTokens::ShebangAndFrontmatter => 2,
    }
}

fn reject() {
    CACHE.with(|slot| {
        if let Some(cache) = slot.borrow_mut().as_mut() {
            cache.stats.rejected += 1;
        }
    });
}

fn miss() {
    CACHE.with(|slot| {
        if let Some(cache) = slot.borrow_mut().as_mut() {
            cache.stats.misses += 1;
        }
    });
}

fn clean(psess: &ParseSess) -> bool {
    psess.dcx().has_errors_or_delayed_bugs().is_none()
        && psess.raw_identifier_spans.get(0).is_none()
        && psess.bad_unicode_identifiers.lock().is_empty()
        && psess.buffered_lints.lock().is_empty()
        && psess.complete_stmt_exprs_before_bin_op_lookalike.lock().is_empty()
        && psess.gated_spans.spans.lock().is_empty()
        && psess.symbol_gallery.symbols.lock().len() <= MAX_GALLERY
}

fn source_eligible(source: &SourceFile) -> bool {
    source.src.as_ref().is_some_and(|src| {
        // No Pound spelling means parsing cannot allocate attribute IDs, even
        // for an attribute that recovery might later discard. Doc comments and
        // every other side-effectful lexical form are excluded by the scanner.
        !src.contains('#') && crate::resident_eligibility::eligible_source(src, MAX_SOURCE)
    })
}

/// File loading and lexical diagnostics use the same native helpers as ordinary
/// parsing. This entry is deliberately not an arbitrary-TokenStream cache API.
pub fn parse_crate_from_file<'a>(
    psess: &'a ParseSess,
    path: &Path,
    strip: StripTokens,
) -> PResult<'a, ast::Crate> {
    let source = crate::file_to_source_file(psess, path, None);
    parse_source_file(psess, source, strip)
}

fn parse_source_file<'a>(
    psess: &'a ParseSess,
    source: Arc<SourceFile>,
    strip: StripTokens,
) -> PResult<'a, ast::Crate> {
    let strip_mode = strip_key(&strip);
    let mut parser = match crate::new_parser_from_source_file(psess, source.clone(), strip) {
        Ok(parser) => parser,
        Err(errors) => {
            miss();
            reject();
            crate::unwrap_or_emit_fatal(Err(errors))
        }
    };
    if !enabled() {
        return parser.parse_crate_mod();
    }
    let admissible = source_eligible(&source) && clean(psess);
    if admissible {
        let found = CACHE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let cache = slot.as_mut().expect("active AST scope");
            let krate = cache
                .entries
                .iter()
                .find(|entry| {
                    Arc::ptr_eq(&entry.source, &source)
                        && std::ptr::eq(entry.source_map.as_ref(), psess.source_map())
                        && entry.edition == psess.edition
                        && entry.strip == strip_mode
                })
                .map(|entry| entry.krate.clone());
            if krate.is_some() {
                cache.stats.hits += 1;
            }
            krate
        });
        if let Some(krate) = found {
            return Ok(krate);
        }
    }
    miss();
    if !admissible {
        reject();
        return parser.parse_crate_mod();
    }
    // Parsing this subset must add no effects beyond the lexing already done.
    // A clone is temporary and bounded by the source/token/gallery admission.
    let gallery = psess.symbol_gallery.symbols.lock().clone();
    let result = parser.parse_crate_mod();
    let Ok(krate) = &result else {
        reject();
        return result;
    };
    if !clean(psess) || *psess.symbol_gallery.symbols.lock() != gallery {
        reject();
        return result;
    }
    let Ok(admission) = inspect(krate, Limits::default()) else {
        reject();
        return result;
    };
    let Some(bytes) = admission
        .estimated_bytes
        .checked_add(source.src.as_ref().map_or(0, |src| src.len()))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Entry>()))
    else {
        reject();
        return result;
    };
    if bytes > MAX_BYTES {
        reject();
        return result;
    }
    CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cache = slot.as_mut().expect("active AST scope");
        while cache.entries.len() >= MAX_ENTRIES
            || cache.stats.retained_estimated_bytes + bytes > MAX_BYTES
        {
            let old = cache.entries.pop_front().expect("nonempty bounded AST cache");
            cache.stats.retained_estimated_bytes -= old.bytes;
        }
        cache.entries.push_back(Entry {
            source,
            source_map: psess.clone_source_map(),
            edition: psess.edition,
            strip: strip_mode,
            krate: krate.clone(),
            bytes,
        });
        cache.stats.retained_entries = cache.entries.len();
        cache.stats.retained_estimated_bytes += bytes;
    });
    result
}

#[cfg(test)]
mod tests {
    use rustc_errors::DiagCtxt;
    use rustc_span::{FileName, create_default_session_globals_then};

    use super::*;

    fn session(map: Arc<SourceMap>) -> ParseSess {
        ParseSess::with_dcx(DiagCtxt::new(Box::new(rustc_errors::emitter::SilentEmitter)), map)
    }

    #[test]
    fn fresh_parser_effects_and_independent_ast_clones() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let first = ParseSess::new();
                let source = first.source_map().new_source_file(
                    FileName::anon_source_code("ast"),
                    "pub fn value(x: u32) -> u32 { (x ^ 7) & 255 }".to_owned(),
                );
                let mut a = parse_source_file(&first, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("valid AST"));
                assert_eq!(stats().retained_entries, 1);
                let gallery = first.symbol_gallery.symbols.lock().clone();
                a.items.clear();
                let next = session(first.clone_source_map());
                let b = parse_source_file(&next, source, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("cached AST"));
                assert_eq!(b.items.len(), 1);
                assert_eq!(stats().hits, 1);
                assert_eq!(*next.symbol_gallery.symbols.lock(), gallery);
            });
            assert_eq!(stats().retained_entries, 0);
        });
    }

    #[test]
    fn unsupported_syntax_is_parsed_but_not_retained() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let psess = ParseSess::new();
                let source = psess.source_map().new_source_file(
                    FileName::anon_source_code("attribute"),
                    "#[inline] pub fn value() -> u32 { 7 }".to_owned(),
                );
                let parsed = parse_source_file(&psess, source, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("ordinary parser accepts attributes"));
                assert_eq!(parsed.items.len(), 1);
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().retained_entries, 0);
                assert_eq!(stats().rejected, 1);
            });
        });
    }

    #[test]
    fn exact_source_and_source_map_identity_are_required() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let first = ParseSess::new();
                let text = "pub fn value() -> u32 { 7 }";
                let source = first
                    .source_map()
                    .new_source_file(FileName::anon_source_code("identity-a"), text.to_owned());
                parse_source_file(&first, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("first AST"));
                let distinct = first
                    .source_map()
                    .new_source_file(FileName::anon_source_code("identity-b"), text.to_owned());
                assert!(!Arc::ptr_eq(&source, &distinct));
                assert_ne!(source.start_pos, distinct.start_pos);
                parse_source_file(
                    &session(first.clone_source_map()),
                    distinct,
                    StripTokens::Nothing,
                )
                .unwrap_or_else(|_| panic!("equal text with distinct spans"));
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().misses, 2);
                assert_eq!(stats().retained_entries, 2);

                // Hold the exact SourceFile constant so only the SourceMap key differs.
                let foreign = ParseSess::new();
                assert!(!std::ptr::eq(first.source_map(), foreign.source_map()));
                parse_source_file(&foreign, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("source under another map"));
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().misses, 3);
                assert_eq!(stats().retained_entries, 3);
                parse_source_file(&session(first.clone_source_map()), source, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("original map still hits"));
                assert_eq!(stats().hits, 1);
                assert_eq!(stats().misses, 3);
            });
        });
    }

    #[test]
    fn strip_and_edition_are_part_of_the_key() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let first = ParseSess::new();
                let source = first.source_map().new_source_file(
                    FileName::anon_source_code("key"),
                    "fn value() -> u32 { 7 }".to_owned(),
                );
                parse_source_file(&first, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("initial AST"));
                parse_source_file(
                    &session(first.clone_source_map()),
                    source.clone(),
                    StripTokens::Shebang,
                )
                .unwrap_or_else(|_| panic!("different strip mode"));
                let mut next = session(first.clone_source_map());
                next.edition = if first.edition == Edition::Edition2024 {
                    Edition::Edition2021
                } else {
                    Edition::Edition2024
                };
                parse_source_file(&next, source, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("different edition"));
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().misses, 3);
                assert_eq!(stats().retained_entries, 3);
            });
        });
    }

    #[test]
    fn entry_eviction_discards_oldest_and_preserves_newest() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let psess = ParseSess::new();
                let mut sources = Vec::new();
                for index in 0..=MAX_ENTRIES {
                    let text = format!("fn value_{index}() -> u32 {{ 7 }}");
                    let source =
                        psess.source_map().new_source_file(FileName::anon_source_code(&text), text);
                    parse_source_file(&psess, source.clone(), StripTokens::Nothing)
                        .unwrap_or_else(|_| panic!("bounded AST"));
                    sources.push(source);
                    assert!(stats().retained_entries <= MAX_ENTRIES);
                    assert!(stats().retained_estimated_bytes <= MAX_BYTES);
                }
                assert_eq!(stats().retained_entries, MAX_ENTRIES);
                assert_eq!(stats().rejected, 0);
                parse_source_file(&psess, sources[MAX_ENTRIES].clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("newest retained AST"));
                assert_eq!(stats().hits, 1);
                let misses = stats().misses;
                parse_source_file(&psess, sources[0].clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("evicted AST reparsed"));
                assert_eq!(stats().hits, 1);
                assert_eq!(stats().misses, misses + 1);
                CACHE.with(|slot| {
                    let slot = slot.borrow();
                    let cache = slot.as_ref().unwrap();
                    assert_eq!(
                        cache.stats.retained_estimated_bytes,
                        cache.entries.iter().map(|entry| entry.bytes).sum::<usize>()
                    );
                });
            });
        });
    }

    #[test]
    fn byte_eviction_and_oversized_source_are_bounded() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(true, || {
                let psess = ParseSess::new();
                let mut oldest = None;
                for index in 0..MAX_ENTRIES {
                    let mut text = format!("fn padded_{index}() -> u32 {{ 7 }}");
                    text.extend(std::iter::repeat_n(' ', MAX_SOURCE - text.len()));
                    let source =
                        psess.source_map().new_source_file(FileName::anon_source_code(&text), text);
                    oldest.get_or_insert_with(|| source.clone());
                    parse_source_file(&psess, source, StripTokens::Nothing)
                        .unwrap_or_else(|_| panic!("source at admission limit"));
                    assert!(stats().retained_estimated_bytes <= MAX_BYTES);
                }
                assert_eq!(stats().rejected, 0);
                // Source bytes alone fill the budget, so AST overhead must evict
                // an entry before the independent entry-count cap is reached.
                assert!(stats().retained_entries < MAX_ENTRIES);
                CACHE.with(|slot| {
                    let slot = slot.borrow();
                    let cache = slot.as_ref().unwrap();
                    assert!(
                        !cache
                            .entries
                            .iter()
                            .any(|entry| { Arc::ptr_eq(&entry.source, oldest.as_ref().unwrap()) })
                    );
                    assert_eq!(
                        cache.stats.retained_estimated_bytes,
                        cache.entries.iter().map(|entry| entry.bytes).sum::<usize>()
                    );
                });
                let retained = stats().retained_entries;
                let bytes = stats().retained_estimated_bytes;
                let source = psess.source_map().new_source_file(
                    FileName::anon_source_code("oversized"),
                    format!("fn oversized() {{}}{}", " ".repeat(MAX_SOURCE)),
                );
                let parsed = parse_source_file(&psess, source, StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("oversized source still parses"));
                assert_eq!(parsed.items.len(), 1);
                assert_eq!(stats().rejected, 1);
                assert_eq!(stats().retained_entries, retained);
                assert_eq!(stats().retained_estimated_bytes, bytes);
            });
        });
    }

    #[test]
    fn disabled_parsing_retains_nothing_and_preserves_fresh_effects() {
        create_default_session_globals_then(|| {
            with_resident_ast_cache(false, || {
                let first = ParseSess::new();
                let source = first.source_map().new_source_file(
                    FileName::anon_source_code("disabled"),
                    "pub fn value(x: u32) -> u32 { x + 7 }".to_owned(),
                );
                let mut a = parse_source_file(&first, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("disabled ordinary parse"));
                let gallery = first.symbol_gallery.symbols.lock().clone();
                a.items.clear();
                let next = session(first.clone_source_map());
                let b = parse_source_file(&next, source.clone(), StripTokens::Nothing)
                    .unwrap_or_else(|_| panic!("disabled repeat parse"));
                assert_eq!(b.items.len(), 1);
                assert_eq!(*next.symbol_gallery.symbols.lock(), gallery);
                assert!(!enabled());
                assert_eq!(stats().hits, 0);
                assert_eq!(stats().misses, 0);
                assert_eq!(stats().rejected, 0);
                assert_eq!(stats().retained_entries, 0);
                assert_eq!(stats().retained_estimated_bytes, 0);
                with_resident_ast_cache(true, || {
                    parse_source_file(
                        &session(first.clone_source_map()),
                        source,
                        StripTokens::Nothing,
                    )
                    .unwrap_or_else(|_| panic!("enabled scope starts cold"));
                    assert_eq!(stats().hits, 0);
                    assert_eq!(stats().misses, 1);
                });
                assert!(!enabled());
            });
        });
    }

    #[test]
    fn disabled_and_unwind_drop_scoped_asts() {
        create_default_session_globals_then(|| {
            assert!(!enabled());
            with_resident_ast_cache(false, || assert_eq!(stats().retained_entries, 0));
            let unwind = std::panic::catch_unwind(|| {
                with_resident_ast_cache(true, || {
                    assert!(enabled());
                    panic!("test unwind");
                });
            });
            assert!(unwind.is_err());
            assert!(!enabled());
            assert_eq!(stats().hits, 0);
        });
    }
}
