//! Private parser views over immutable output from the existing lexer.
//!
//! This is not a second lexer or a token-tree/hygiene implementation. Splitting
//! punctuation changes only cursor state, never the spelling or origin tokens.
use super::{ParserTokenKind, Token};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TokenView<'input> {
    pub(super) text: &'input str,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) string: bool,
    pub(super) bytes: Range<usize>,
    pub(super) span: crate::compiler::expansion_span::SpanId,
    pub(super) name: Option<std::num::NonZeroU32>,
    pub(super) provenance: super::TokenProvenance,
    kind: ParserTokenKind,
}

impl TokenView<'_> {
    pub(super) fn matches(&self, text: &str) -> bool {
        if text.is_empty() {
            self.kind == ParserTokenKind::Eof
        } else {
            self.kind != ParserTokenKind::Eof
                && self.kind != ParserTokenKind::String
                && self.kind != ParserTokenKind::Char
                && self.kind != ParserTokenKind::ByteString
                && self.kind != ParserTokenKind::Byte
                && self.kind != ParserTokenKind::OpaqueLiteral
                && self.kind != ParserTokenKind::DeferredLiteral
                && self.kind != ParserTokenKind::DeferredUnsupported
                && self.text == text
        }
    }
    pub(super) fn dollar_crate(
        &self,
    ) -> Option<crate::compiler::semantic_path::RetainedDollarCrateAnchor> {
        match &self.kind {
            ParserTokenKind::DollarCrate(anchor) => Some(anchor.clone()),
            _ => None,
        }
    }
    pub(super) fn is_ident(&self) -> bool {
        self.kind == ParserTokenKind::Ident
    }
    pub(super) fn is_char(&self) -> bool {
        self.kind == ParserTokenKind::Char
    }
    pub(super) fn is_byte(&self) -> bool {
        self.kind == ParserTokenKind::Byte
    }
    pub(super) fn is_byte_string(&self) -> bool {
        self.kind == ParserTokenKind::ByteString
    }
    pub(super) fn is_lifetime(&self) -> bool {
        self.kind == ParserTokenKind::Lifetime
    }
    pub(super) fn is_opaque_literal(&self) -> bool {
        self.kind == ParserTokenKind::OpaqueLiteral
    }
    pub(super) fn is_deferred_unsupported(&self) -> bool {
        self.kind == ParserTokenKind::DeferredUnsupported
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CursorMark<'input> {
    input: &'input [Token],
    at: usize,
    prefix: usize,
}

pub(super) struct TokenCursor<'input> {
    input: &'input [Token],
    at: usize,
    prefix: usize,
}

impl<'input> TokenCursor<'input> {
    pub(super) fn new(input: &'input [Token]) -> Self {
        assert!(
            input.last().is_some_and(|t| t.kind == ParserTokenKind::Eof),
            "parser input must end with the lexer's EOF token"
        );
        Self {
            input,
            at: 0,
            prefix: 0,
        }
    }

    pub(super) fn current(&self) -> TokenView<'input> {
        self.view(self.at, self.prefix)
    }

    pub(super) fn peek(&self, ahead: usize) -> Option<TokenView<'input>> {
        let at = self.at.checked_add(ahead)?;
        self.input.get(at)?;
        Some(self.view(at, if ahead == 0 { self.prefix } else { 0 }))
    }

    pub(super) fn bump(&mut self) -> TokenView<'input> {
        let token = self.current();
        if self.at < self.input.len() - 1 {
            self.at += 1;
            self.prefix = 0;
        }
        token
    }

    /// Whole-token index of the current position, if not mid-token.
    pub(super) fn index(&self) -> Option<usize> {
        (self.prefix == 0).then_some(self.at)
    }

    pub(super) fn checkpoint(&self) -> CursorMark<'input> {
        CursorMark {
            input: self.input,
            at: self.at,
            prefix: self.prefix,
        }
    }

    /// Compare exact logical positions without importing a foreign input mark.
    pub(super) fn is_at(&self, mark: CursorMark<'input>) -> bool {
        std::ptr::eq(self.input, mark.input) && self.at == mark.at && self.prefix == mark.prefix
    }

    /// Foreign or invalid marks leave the current position unchanged. Marks can
    /// only be created by this private cursor, not by source-language programs.
    pub(super) fn restore(&mut self, mark: CursorMark<'input>) -> bool {
        if !std::ptr::eq(self.input, mark.input)
            || !self
                .input
                .get(mark.at)
                .is_some_and(|token| mark.prefix == 0 || Self::valid_prefix(token, mark.prefix))
        {
            return false;
        }
        self.at = mark.at;
        self.prefix = mark.prefix;
        true
    }

    pub(super) fn consume_type_gt(&mut self) -> bool {
        let token = self.current();
        if token.kind != ParserTokenKind::Punct {
            return false;
        }
        match token.text {
            ">" => {
                self.bump();
            }
            ">>" | ">=" | ">>=" => {
                self.prefix += 1;
            }
            _ => return false,
        }
        true
    }

    pub(super) fn consume_type_lt(&mut self) -> bool {
        let token = self.current();
        if token.kind != ParserTokenKind::Punct {
            return false;
        }
        match token.text {
            "<" => {
                self.bump();
            }
            "<<" | "<<=" => {
                self.prefix += 1;
            }
            _ => return false,
        }
        true
    }

    pub(super) fn consume_type_amp_prefix(&mut self) -> bool {
        let token = self.current();
        if token.kind != ParserTokenKind::Punct || token.text != "&&" {
            return false;
        }
        self.prefix += 1;
        true
    }

    fn valid_prefix(token: &Token, prefix: usize) -> bool {
        token.kind == ParserTokenKind::Punct
            && prefix > 0
            && token.suffixes.get(prefix - 1).is_some_and(Option::is_some)
            && matches!(
                (token.text.as_str(), prefix),
                ("&&" | ">>" | ">=" | "<<", 1) | (">>=" | "<<=", 1 | 2)
            )
    }

    fn view(&self, at: usize, prefix: usize) -> TokenView<'input> {
        let token = &self.input[at];
        debug_assert!(prefix == 0 || Self::valid_prefix(token, prefix));
        let part = if prefix == 0 {
            &token.whole
        } else {
            token.suffixes[prefix - 1]
                .as_ref()
                .expect("validated cursor suffix")
        };
        debug_assert!(token.constituents.get(0).is_some());
        let (line, column) = part.position(token.line as usize, token.column as usize, prefix);
        TokenView {
            text: &token.text[prefix..],
            line,
            column,
            string: token.string,
            bytes: part.bytes(),
            span: part.span,
            name: token.name,
            provenance: part.provenance(),
            kind: token.kind.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lex;
    use super::*;

    #[test]
    fn authenticated_crate_proofs_keep_all_lexer_token_kinds_compact() {
        assert!(std::mem::size_of::<ParserTokenKind>() <= 2 * std::mem::size_of::<usize>());
        assert!(std::mem::size_of::<Token>() <= 19 * std::mem::size_of::<usize>());
    }

    #[test]
    fn leading_angle_split_keeps_actual_suffix_and_normal_shift_consumption() {
        for text in ["<<", "<<="] {
            let tokens = lex(text).unwrap();
            let mut cursor = TokenCursor::new(&tokens);
            let start = cursor.checkpoint();
            assert!(cursor.consume_type_lt());
            assert_eq!(cursor.current().text, &text[1..]);
            assert_eq!(cursor.current().bytes, 1..text.len());
            assert_eq!(
                cursor.current().span,
                tokens[0].suffixes[0].as_ref().unwrap().span
            );
            assert!(cursor.restore(start));
            assert_eq!(cursor.bump().text, text);
        }
        let tokens = lex("\"<<\"").unwrap();
        assert!(!TokenCursor::new(&tokens).consume_type_lt());
    }

    #[test]
    fn angle_suffix_checkpoints_restore_exact_views_without_mutating_input() {
        for (text, suffix) in [(">>", ">"), (">=", "="), (">>=", ">=")] {
            let tokens = lex(text).unwrap();
            let original = format!("{tokens:?}");
            let mut cursor = TokenCursor::new(&tokens);
            let start = cursor.checkpoint();
            assert!(cursor.is_at(start));
            assert!(cursor.consume_type_gt());
            assert!(!cursor.is_at(start));
            assert_eq!(cursor.current().text, suffix);
            assert_eq!(cursor.current().bytes, 1..text.len());
            assert_eq!(cursor.current().column, 2);
            let middle = cursor.checkpoint();
            assert!(cursor.is_at(middle));
            let remaining = cursor.current();
            if text == ">>=" {
                assert!(cursor.consume_type_gt());
                assert_eq!(cursor.current().text, "=");
                assert_eq!(cursor.current().bytes, 2..3);
                assert_eq!(cursor.current().column, 3);
                assert!(!cursor.consume_type_gt());
            } else {
                cursor.bump();
            }
            assert!(cursor.restore(middle));
            assert!(cursor.is_at(middle));
            assert_eq!(cursor.current(), remaining);
            assert!(cursor.restore(start));
            assert!(cursor.is_at(start));
            assert_eq!(cursor.current().text, text);
            assert_eq!(cursor.current().bytes, 0..text.len());
            assert_eq!(format!("{tokens:?}"), original);
        }
    }

    #[test]
    fn amp_suffix_lookahead_and_repeated_eof_are_stable() {
        let tokens = lex("&& next last").unwrap();
        let original = format!("{tokens:?}");
        let mut cursor = TokenCursor::new(&tokens);
        let start = cursor.checkpoint();
        assert!(cursor.consume_type_amp_prefix());
        let split = cursor.checkpoint();
        assert_eq!(cursor.peek(0).unwrap().text, "&");
        assert_eq!(cursor.peek(0).unwrap().bytes, 1..2);
        assert_eq!(cursor.peek(1).unwrap().text, "next");
        assert_eq!(cursor.peek(2).unwrap().text, "last");
        assert!(cursor.peek(usize::MAX).is_none());
        assert!(!cursor.consume_type_amp_prefix());
        assert_eq!(cursor.bump().text, "&");
        assert_eq!(cursor.bump().text, "next");
        assert_eq!(cursor.bump().text, "last");
        let eof = cursor.current();
        for _ in 0..4 {
            assert_eq!(cursor.bump(), eof);
            assert_eq!(cursor.current(), eof);
            assert!(cursor.peek(1).is_none());
        }
        assert!(cursor.restore(split));
        assert_eq!(cursor.current().bytes, 1..2);
        assert!(cursor.restore(start));
        assert_eq!(cursor.current().text, "&&");
        assert_eq!(format!("{tokens:?}"), original);

        let tokens = lex("&&").unwrap();
        let mut cursor = TokenCursor::new(&tokens);
        assert!(cursor.consume_type_amp_prefix());
        cursor.bump();
        assert_eq!(cursor.current().bytes, 2..2);
    }

    #[test]
    fn foreign_and_invalid_marks_are_rejected_without_changing_position() {
        let left = lex(">>= name").unwrap();
        let right = lex(">>= name").unwrap();
        let mut cursor = TokenCursor::new(&left);
        assert!(cursor.consume_type_gt());
        let position = cursor.current();
        assert!(!cursor.is_at(TokenCursor::new(&right).checkpoint()));
        assert!(!cursor.is_at(TokenCursor::new(&left[1..]).checkpoint()));
        assert!(!cursor.restore(TokenCursor::new(&right).checkpoint()));
        assert!(!cursor.restore(TokenCursor::new(&left[1..]).checkpoint()));
        for mark in [
            CursorMark {
                input: &left,
                at: left.len(),
                prefix: 0,
            },
            CursorMark {
                input: &left,
                at: 0,
                prefix: usize::MAX,
            },
            CursorMark {
                input: &left,
                at: 1,
                prefix: 1,
            },
        ] {
            assert!(!cursor.restore(mark));
            assert_eq!(cursor.current(), position);
        }
        assert!(cursor.peek(usize::MAX).is_none());
    }

    #[test]
    fn normal_consumption_keeps_lexer_operators_literal_kinds_and_boundaries() {
        for text in [
            "x>>=1 x>>y x>=y x&&y > > = >/*gap*/>=",
            "t.0.1 0.1 1. 1e-3 2.5E+7f32 1..2",
            "'region: loop { break 'region; }",
            "/* 水 */\r\n\"é\r\n水 >> &&\" >>=",
            "\"\" \"fn\" \"&&\" \">>=\"",
            "",
        ] {
            let tokens = lex(text).unwrap();
            let mut cursor = TokenCursor::new(&tokens);
            for token in tokens.iter() {
                let view = cursor.bump();
                assert_eq!(view.text, token.text);
                assert_eq!(view.string, token.string);
                assert_eq!(
                    (view.line, view.column),
                    (token.line as usize, token.column as usize)
                );
                assert_eq!(view.bytes, token.whole.bytes());
            }
            assert_eq!(cursor.current().bytes, text.len()..text.len());
        }
    }

    #[test]
    fn unicode_crlf_split_views_survive_split_bump_and_restore() {
        let text = "/* 水 */\r\n  >>= &&";
        let tokens = lex(text).unwrap();
        let mut cursor = TokenCursor::new(&tokens);
        let start = cursor.checkpoint();
        let whole = cursor.current();
        assert_eq!((whole.line, whole.column), (2, 3));
        assert_eq!(whole.bytes, 13..16);
        assert!(cursor.consume_type_gt());
        let suffix = cursor.current();
        let middle = cursor.checkpoint();
        assert_eq!(suffix.text, ">=");
        assert_eq!((suffix.line, suffix.column), (2, 4));
        assert_eq!(suffix.bytes, 14..16);
        assert!(cursor.consume_type_gt());
        assert_eq!(cursor.bump().bytes, 15..16);
        assert!(cursor.consume_type_amp_prefix());
        let amp = cursor.current();
        assert_eq!(amp.text, "&");
        assert_eq!((amp.line, amp.column), (2, 8));
        assert_eq!(amp.bytes, 18..19);
        cursor.bump();
        assert!(cursor.restore(middle));
        assert_eq!(cursor.current(), suffix);
        assert!(cursor.restore(start));
        assert_eq!(cursor.current(), whole);
        assert_eq!(whole.text, ">>=");
        assert_eq!(whole.bytes, 13..16);
        assert_eq!(suffix.text, ">=");
        assert_eq!(suffix.bytes, 14..16);
        assert_eq!(amp.text, "&");
        assert_eq!(amp.bytes, 18..19);
    }

    #[test]
    fn quoted_punctuation_cannot_split_and_empty_strings_are_not_eof() {
        let tokens = lex("\"\" \"&&\" \">>=\" >").unwrap();
        let mut cursor = TokenCursor::new(&tokens);
        assert!(cursor.current().string);
        assert_eq!(cursor.bump().text, "");
        assert!(!cursor.consume_type_amp_prefix());
        assert!(!cursor.consume_type_gt());
        cursor.bump();
        assert!(!cursor.consume_type_gt());
        cursor.bump();
        assert!(cursor.consume_type_gt());
        assert!(!cursor.current().string);
        assert_eq!(cursor.current().bytes, 15..15);
    }
}
