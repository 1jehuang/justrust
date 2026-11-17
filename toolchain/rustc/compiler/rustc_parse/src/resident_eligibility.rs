//! Conservative, state-free admission precheck for resident token caching.
//!
//! This is a lexer-side-effect proof, not a Rust syntax validator. In
//! `lexer::Lexer::next_token_from_cursor`, the admitted whitespace only updates
//! local spacing, punctuation maps directly to output tokens, and ordinary
//! ASCII identifiers take `ident`: NFC is the identity, and the only session
//! mutation is SymbolGallery insertion. Decimal, nonempty, suffix-free integers
//! take the diagnostic-free arm of `cook_lexer_literal`; their ASCII spelling
//! cannot trigger the text-flow lint. Interning identifiers and integer spellings
//! is part of producing those output tokens, not work performed by this scan.
//! None of these classes touches gates, raw-identifier spans, bad-Unicode maps,
//! buffered lints, or diagnostics. Lifetime-reset bookkeeping is local and inert
//! because no lifetime is admitted. Replaying a cached stream must separately
//! replay its SymbolGallery entries.
//!
//! Delimiter matching belongs to token-tree construction, not this proof:
//! unmatched delimiters are eligible here but MUST NOT enter the cache unless
//! actual tokenization succeeds without diagnostics. Likewise, this predicate
//! says nothing about subsequent parsing or integer overflow/type checking.

use rustc_lexer::{Base, FrontmatterAllowed, LiteralKind, TokenKind};

/// Scan at most `max_bytes` source bytes without interning or compiler state.
/// Oversized inputs are rejected before inspecting their contents. The lexer
/// uses u32 lengths, so also reject inputs outside that representable bound.
/// Two forward scans and disjoint token-spelling checks take O(src.len()) time
/// and O(1) auxiliary space; rejected inputs need not be scanned completely.
pub(super) fn eligible_source(src: &str, max_bytes: usize) -> bool {
    if src.len() > max_bytes || src.len() > u32::MAX as usize {
        return false;
    }
    if src.bytes().any(|b| !b.is_ascii() || b == b'#') {
        return false;
    }

    let mut offset = 0usize;
    // Use Yes even for fragments: rejecting possible document frontmatter is
    // conservative across all StripTokens modes of the real lexer.
    for token in rustc_lexer::tokenize(src, FrontmatterAllowed::Yes) {
        let len = token.len as usize;
        let Some(end) = offset.checked_add(len) else { return false };
        let Some(text) = src.get(offset..end) else { return false };
        if len == 0 {
            return false;
        }
        let accepted = match token.kind {
            TokenKind::Ident => true, // ASCII was checked before lexing.
            TokenKind::Whitespace => {
                text.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
            }
            TokenKind::Literal {
                kind: LiteralKind::Int { base: Base::Decimal, empty_int: false },
                suffix_start,
            } => {
                suffix_start == token.len
                    && text.as_bytes()[0].is_ascii_digit()
                    && text.bytes().all(|b| b.is_ascii_digit() || b == b'_')
            }
            TokenKind::Semi
            | TokenKind::Comma
            | TokenKind::Dot
            | TokenKind::OpenParen
            | TokenKind::CloseParen
            | TokenKind::OpenBrace
            | TokenKind::CloseBrace
            | TokenKind::OpenBracket
            | TokenKind::CloseBracket
            | TokenKind::At
            | TokenKind::Tilde
            | TokenKind::Question
            | TokenKind::Colon
            | TokenKind::Dollar
            | TokenKind::Eq
            | TokenKind::Bang
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::Minus
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Caret
            | TokenKind::Percent => true,
            // In particular: comments (including docs), frontmatter, every
            // other literal, lifetimes, raw/forced/invalid identifiers,
            // unknown prefixes, guarded strings, Pound, Unknown and Eof.
            // A future lexer class is denied by default.
            _ => false,
        };
        if !accepted {
            return false;
        }
        offset = end;
    }
    offset == src.len()
}

#[cfg(test)]
mod tests {
    use super::eligible_source;

    #[test]
    fn ordinary_ascii_tokens() {
        for src in [
            "",
            " \t\n\r\u{b}\u{c}",
            "fn sum(x: u32, y: u32) -> u32 { let n = 1_000 + 02; x + y + n }",
            "fn main() { let _ = [0, 1, 123456789012345678901234567890]; }",
            "a::b != 2 && x <= 9 || x >= 3; x += 1; x -= 1; x *= 2; x /= 2;",
            "x %= 2; x ^= 1; x &= 7; x |= 8; x << 1 >> 2; !x; ~x; @x; $x?;",
            "a.0; 0..2; 0..=2; 1__2; 1_; _; async await gen",
        ] {
            assert!(eligible_source(src, src.len()), "rejected {src:?}");
        }
    }

    #[test]
    fn omitted_syntax_and_lexical_errors() {
        for src in [
            "#",
            "a#b",
            "#[test]",
            "#!/usr/bin/env rustx\nfn main() {}",
            "// comment",
            "/// doc",
            "//! doc",
            "/* comment */",
            "/** doc */",
            "/*! doc */",
            "/* unclosed",
            "/* /* */",
            "\"str\"",
            "\"unclosed",
            "b\"str\"",
            "c\"str\"",
            "r\"str\"",
            "r#\"str\"#",
            "br\"str\"",
            "cr\"str\"",
            "'x'",
            "b'x'",
            "'ab'",
            "'",
            "'a",
            "'0",
            "'r#name",
            "'foo#",
            "r#name",
            "k#fn",
            "foo\"bar\"",
            "foo'x'",
            "foo#",
            "#\"guarded\"#",
            "##",
            "0x",
            "0x_",
            "0x12",
            "0xgg",
            "0b",
            "0b102",
            "0o89",
            "0o7",
            "1u8",
            "1_u8",
            "1f32",
            "1foo",
            "0X12",
            "1.0",
            "1.",
            "1e3",
            "1e+",
            "0x1.2",
            "\0",
            "`",
            "\\",
            "caf\u{e9}",
            "\u{1f980}",
            "\u{a0}",
            "\u{feff}fn f() {}",
            "\u{202e}",
            "---\nkey: value\n---\nfn f() {}",
            " ---\n---",
            "\n---\n---",
        ] {
            assert!(!eligible_source(src, usize::MAX), "accepted {src:?}");
        }
    }

    #[test]
    fn bounds_are_inclusive_and_checked_before_scanning() {
        assert!(eligible_source("", 0));
        assert!(!eligible_source("a", 0));
        assert!(!eligible_source("fn f() {}", 8));
        assert!(eligible_source("fn f() {}", 9));
        assert!(eligible_source("a", usize::MAX));
    }

    #[test]
    fn delimiter_validity_is_a_separate_cache_admission_requirement() {
        for src in ["(", ")", "{", "}", "[", "]", "([)]", "fn f() { ( ]"] {
            assert!(eligible_source(src, src.len()), "rejected {src:?}");
        }
        // These must still fail actual token-tree construction and must not be
        // cached. The integration tests must enforce successful tokenization.
    }
}
