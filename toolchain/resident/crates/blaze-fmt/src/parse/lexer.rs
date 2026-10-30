//! Source text to token trees, as `compiler/rustc_parse/src/lexer/{mod,tokentrees}.rs` at
//! rust-lang/rust 12433c307 (MIT or Apache-2.0) do it: rustc's own lexer splits the text, each
//! token is cooked into a `rustc_ast` token, and adjacent single-character tokens are glued.
//! Lints and diagnostics are dropped; anything rustc would reject is an `Err` with rustc's
//! message, and identifiers are not NFC-normalised.

use rustc_lexer::{Base, Cursor, DocStyle, FrontmatterAllowed, LiteralKind};

use crate::ast::token::{self, CommentKind, Delimiter, IdentKind, Token, TokenKind};
use crate::ast::tokenstream::{DelimSpacing, DelimSpan, Spacing, TokenStream, TokenTree};
use crate::ast::{AttrStyle, ErrorGuaranteed};
use crate::span::edition::Edition;
use crate::span::{BytePos, Pos, Span, Symbol};

/// Whether a file's shebang and frontmatter are skipped, as `rustc_parse::lexer::StripTokens`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StripTokens {
    ShebangAndFrontmatter,
    Nothing,
}

#[derive(Debug, Clone)]
pub(crate) struct LexError {
    pub(crate) span: Span,
    pub(crate) message: String,
}

pub(crate) fn lex_token_trees(
    mut src: &str,
    mut start_pos: BytePos,
    edition: Edition,
    strip_tokens: StripTokens,
) -> Result<TokenStream, LexError> {
    if strip_tokens == StripTokens::ShebangAndFrontmatter
        && let Some(shebang_len) = rustc_lexer::strip_shebang(src)
    {
        src = &src[shebang_len..];
        start_pos += BytePos::from_usize(shebang_len);
    }
    let frontmatter_allowed = match strip_tokens {
        StripTokens::ShebangAndFrontmatter => FrontmatterAllowed::Yes,
        StripTokens::Nothing => FrontmatterAllowed::No,
    };
    let mut lexer = Lexer {
        start_pos,
        pos: start_pos,
        src,
        cursor: Cursor::new(src, frontmatter_allowed),
        edition,
        token: Token::dummy(),
        error: None,
        open_delimiters: Vec::new(),
    };
    let res = lexer.lex_token_trees(false);
    if let Some(error) = lexer.error {
        return Err(error);
    }
    res.map(|(_, stream)| stream)
}

struct Lexer<'src> {
    start_pos: BytePos,
    pos: BytePos,
    src: &'src str,
    cursor: Cursor<'src>,
    edition: Edition,
    token: Token,
    /// The first error rustc would report; the rest of the file is still lexed so the caller
    /// sees one clear reason.
    error: Option<LexError>,
    open_delimiters: Vec<(Delimiter, Span)>,
}

impl<'src> Lexer<'src> {
    fn mk_sp(&self, lo: BytePos, hi: BytePos) -> Span {
        Span::with_root_ctxt(lo, hi)
    }

    fn err(&mut self, span: Span, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(LexError {
                span,
                message: message.into(),
            });
        }
    }

    fn src_index(&self, pos: BytePos) -> usize {
        (pos - self.start_pos).to_usize()
    }

    fn str_from(&self, start: BytePos) -> &'src str {
        self.str_from_to(start, self.pos)
    }

    fn str_from_to(&self, start: BytePos, end: BytePos) -> &'src str {
        &self.src[self.src_index(start)..self.src_index(end)]
    }

    fn symbol_from_to(&self, start: BytePos, end: BytePos) -> Symbol {
        Symbol::intern(self.str_from_to(start, end))
    }

    fn next_token_from_cursor(&mut self) -> (Token, bool) {
        let mut preceded_by_whitespace = false;
        loop {
            let str_before = self.cursor.as_str();
            let token = self.cursor.advance_token();
            let start = self.pos;
            self.pos += BytePos(token.len);

            let kind = match token.kind {
                rustc_lexer::TokenKind::LineComment { doc_style } => {
                    let Some(doc_style) = doc_style else {
                        preceded_by_whitespace = true;
                        continue;
                    };
                    let content_start = start + BytePos(3);
                    let content = self.str_from(content_start);
                    self.cook_doc_comment(content_start, content, CommentKind::Line, doc_style)
                }
                rustc_lexer::TokenKind::BlockComment {
                    doc_style,
                    terminated,
                } => {
                    if !terminated {
                        let msg = match doc_style {
                            Some(_) => "unterminated block doc-comment",
                            None => "unterminated block comment",
                        };
                        self.err(self.mk_sp(start, self.pos), msg);
                    }
                    let Some(doc_style) = doc_style else {
                        preceded_by_whitespace = true;
                        continue;
                    };
                    let content_start = start + BytePos(3);
                    let content_end = self.pos - BytePos(if terminated { 2 } else { 0 });
                    let content = self.str_from_to(content_start, content_end);
                    self.cook_doc_comment(content_start, content, CommentKind::Block, doc_style)
                }
                rustc_lexer::TokenKind::Frontmatter {
                    has_invalid_preceding_whitespace,
                    ..
                } => {
                    if has_invalid_preceding_whitespace {
                        self.err(
                            self.mk_sp(start, self.pos),
                            "invalid preceding whitespace for frontmatter opening",
                        );
                    }
                    preceded_by_whitespace = true;
                    continue;
                }
                rustc_lexer::TokenKind::Whitespace => {
                    preceded_by_whitespace = true;
                    continue;
                }
                rustc_lexer::TokenKind::Ident => self.ident(start),
                rustc_lexer::TokenKind::RawIdent => {
                    let sym = Symbol::intern(self.str_from(start + BytePos(2)));
                    if !sym.can_be_raw() {
                        self.err(
                            self.mk_sp(start, self.pos),
                            format!("`{sym}` cannot be a raw identifier"),
                        );
                    }
                    token::Ident(sym, IdentKind::Raw)
                }
                rustc_lexer::TokenKind::UnknownPrefix => {
                    let prefix = self.str_from(start);
                    if self.edition.at_least_rust_2021() {
                        self.err(
                            self.mk_sp(start, self.pos),
                            format!("prefix `{prefix}` is unknown"),
                        );
                    }
                    self.ident(start)
                }
                rustc_lexer::TokenKind::UnknownPrefixLifetime => {
                    let prefix = self.str_from(start);
                    if self.edition.at_least_rust_2021() {
                        self.err(
                            self.mk_sp(start, self.pos),
                            format!("prefix `{prefix}` is unknown"),
                        );
                    }
                    token::Lifetime(Symbol::intern(self.str_from(start)), IdentKind::Normal)
                }
                rustc_lexer::TokenKind::InvalidIdent => {
                    let sym = self.str_from(start);
                    self.err(
                        self.mk_sp(start, self.pos),
                        format!("identifiers cannot contain `{sym}`"),
                    );
                    token::Ident(Symbol::intern(sym), IdentKind::Normal)
                }
                rustc_lexer::TokenKind::Literal {
                    kind: kind @ (LiteralKind::CStr { .. } | LiteralKind::RawCStr { .. }),
                    suffix_start: _,
                } if !self.edition.at_least_rust_2021() => {
                    // Before 2021 `c"x"` is the identifier `c` followed by a string.
                    let prefix_len = match kind {
                        LiteralKind::CStr { .. } => 1,
                        _ => 2,
                    };
                    self.pos = start + BytePos(prefix_len);
                    self.cursor =
                        Cursor::new(&str_before[prefix_len as usize..], FrontmatterAllowed::No);
                    self.ident(start)
                }
                rustc_lexer::TokenKind::GuardedStrPrefix => {
                    if self.edition.at_least_rust_2024() {
                        self.err(
                            self.mk_sp(start, self.pos),
                            "reserved multi-hash token or guarded string is forbidden",
                        );
                        token::Pound
                    } else {
                        // Before 2024 only the first `#` is taken.
                        self.pos = start + BytePos(1);
                        self.cursor = Cursor::new(&str_before[1..], FrontmatterAllowed::No);
                        token::Pound
                    }
                }
                rustc_lexer::TokenKind::Literal { kind, suffix_start } => {
                    let suffix_start = start + BytePos(suffix_start);
                    let (kind, symbol) = self.cook_lexer_literal(start, suffix_start, kind);
                    let suffix = if suffix_start < self.pos {
                        let string = self.str_from(suffix_start);
                        if string == "_" {
                            self.err(
                                self.mk_sp(suffix_start, self.pos),
                                "underscore literal suffix is not allowed",
                            );
                            None
                        } else {
                            Some(Symbol::intern(string))
                        }
                    } else {
                        None
                    };
                    token::Literal(token::Lit {
                        kind,
                        symbol,
                        suffix,
                    })
                }
                rustc_lexer::TokenKind::Lifetime { starts_with_number } => {
                    if starts_with_number {
                        self.err(
                            self.mk_sp(start, self.pos),
                            "lifetimes cannot start with a number",
                        );
                    }
                    token::Lifetime(Symbol::intern(self.str_from(start)), IdentKind::Normal)
                }
                rustc_lexer::TokenKind::RawLifetime => {
                    let ident_start = start + BytePos(3);
                    if self.edition.at_least_rust_2021() {
                        if self.cursor.as_str().starts_with('\'') {
                            self.err(
                                self.mk_sp(start, self.pos + BytePos(1)),
                                "character literal may only contain one codepoint",
                            );
                        }
                        let without_tick = Symbol::intern(self.str_from(ident_start));
                        if !without_tick.can_be_raw() {
                            self.err(
                                self.mk_sp(start, self.pos),
                                format!("`{without_tick}` cannot be a raw lifetime"),
                            );
                        }
                        token::Lifetime(Symbol::intern(&format!("'{without_tick}")), IdentKind::Raw)
                    } else {
                        // Before 2021 `'r#a` is the lifetime `'r` followed by `#a`.
                        self.pos = start + BytePos(2);
                        self.cursor = Cursor::new(&str_before[2..], FrontmatterAllowed::No);
                        token::Lifetime(Symbol::intern(self.str_from(start)), IdentKind::Normal)
                    }
                }
                rustc_lexer::TokenKind::Semi => token::Semi,
                rustc_lexer::TokenKind::Comma => token::Comma,
                rustc_lexer::TokenKind::Dot => token::Dot,
                rustc_lexer::TokenKind::OpenParen => token::OpenParen,
                rustc_lexer::TokenKind::CloseParen => token::CloseParen,
                rustc_lexer::TokenKind::OpenBrace => token::OpenBrace,
                rustc_lexer::TokenKind::CloseBrace => token::CloseBrace,
                rustc_lexer::TokenKind::OpenBracket => token::OpenBracket,
                rustc_lexer::TokenKind::CloseBracket => token::CloseBracket,
                rustc_lexer::TokenKind::At => token::At,
                rustc_lexer::TokenKind::Pound => token::Pound,
                rustc_lexer::TokenKind::Tilde => token::Tilde,
                rustc_lexer::TokenKind::Question => token::Question,
                rustc_lexer::TokenKind::Colon => token::Colon,
                rustc_lexer::TokenKind::Dollar => token::Dollar,
                rustc_lexer::TokenKind::Eq => token::Eq,
                rustc_lexer::TokenKind::Bang => token::Bang,
                rustc_lexer::TokenKind::Lt => token::Lt,
                rustc_lexer::TokenKind::Gt => token::Gt,
                rustc_lexer::TokenKind::Minus => token::Minus,
                rustc_lexer::TokenKind::And => token::And,
                rustc_lexer::TokenKind::Or => token::Or,
                rustc_lexer::TokenKind::Plus => token::Plus,
                rustc_lexer::TokenKind::Star => token::Star,
                rustc_lexer::TokenKind::Slash => token::Slash,
                rustc_lexer::TokenKind::Caret => token::Caret,
                rustc_lexer::TokenKind::Percent => token::Percent,
                rustc_lexer::TokenKind::Unknown => {
                    let c = self.str_from(start).chars().next().unwrap_or('\0');
                    self.err(
                        self.mk_sp(start, self.pos),
                        format!("unknown start of token: {}", c.escape_default()),
                    );
                    preceded_by_whitespace = true;
                    continue;
                }
                rustc_lexer::TokenKind::Eof => token::Eof,
            };
            let span = self.mk_sp(start, self.pos);
            return (Token::new(kind, span), preceded_by_whitespace);
        }
    }

    fn ident(&self, start: BytePos) -> TokenKind {
        token::Ident(Symbol::intern(self.str_from(start)), IdentKind::Normal)
    }

    fn cook_doc_comment(
        &mut self,
        content_start: BytePos,
        content: &str,
        comment_kind: CommentKind,
        doc_style: DocStyle,
    ) -> TokenKind {
        if let Some(idx) = content.find('\r') {
            let lo = content_start + BytePos::from_usize(idx);
            let msg = match comment_kind {
                CommentKind::Block => "bare CR not allowed in block doc-comment",
                CommentKind::Line => "bare CR not allowed in doc-comment",
            };
            self.err(self.mk_sp(lo, lo + BytePos(1)), msg);
        }
        let attr_style = match doc_style {
            DocStyle::Outer => AttrStyle::Outer,
            DocStyle::Inner => AttrStyle::Inner,
        };
        token::DocComment(comment_kind, attr_style, Symbol::intern(content))
    }

    fn cook_lexer_literal(
        &mut self,
        start: BytePos,
        end: BytePos,
        kind: LiteralKind,
    ) -> (token::LitKind, Symbol) {
        let unterminated = |this: &mut Self, msg: &str| {
            this.err(this.mk_sp(start, end), msg);
        };
        match kind {
            LiteralKind::Char { terminated } => {
                if !terminated {
                    unterminated(self, "unterminated character literal");
                }
                self.cook_quoted(token::Char, start, end, 1, 1)
            }
            LiteralKind::Byte { terminated } => {
                if !terminated {
                    unterminated(self, "unterminated byte constant");
                }
                self.cook_quoted(token::Byte, start, end, 2, 1)
            }
            LiteralKind::Str { terminated } => {
                if !terminated {
                    unterminated(self, "unterminated double quote string");
                }
                self.cook_quoted(token::Str, start, end, 1, 1)
            }
            LiteralKind::ByteStr { terminated } => {
                if !terminated {
                    unterminated(self, "unterminated double quote byte string");
                }
                self.cook_quoted(token::ByteStr, start, end, 2, 1)
            }
            LiteralKind::CStr { terminated } => {
                if !terminated {
                    unterminated(self, "unterminated C string");
                }
                self.cook_quoted(token::CStr, start, end, 2, 1)
            }
            LiteralKind::RawStr { n_hashes } => match n_hashes {
                Some(n_hashes) => {
                    let n = u32::from(n_hashes);
                    self.cook_quoted(token::StrRaw(n_hashes), start, end, 2 + n, 1 + n)
                }
                None => self.raw_str_error(start, end),
            },
            LiteralKind::RawByteStr { n_hashes } => match n_hashes {
                Some(n_hashes) => {
                    let n = u32::from(n_hashes);
                    self.cook_quoted(token::ByteStrRaw(n_hashes), start, end, 3 + n, 1 + n)
                }
                None => self.raw_str_error(start, end),
            },
            LiteralKind::RawCStr { n_hashes } => match n_hashes {
                Some(n_hashes) => {
                    let n = u32::from(n_hashes);
                    self.cook_quoted(token::CStrRaw(n_hashes), start, end, 3 + n, 1 + n)
                }
                None => self.raw_str_error(start, end),
            },
            LiteralKind::Int { base, empty_int } => {
                let mut kind = token::Integer;
                if empty_int {
                    self.err(self.mk_sp(start, end), "no valid digits found for number");
                    kind = token::Err(ErrorGuaranteed::unchecked_error_guaranteed());
                } else if matches!(base, Base::Binary | Base::Octal) {
                    let radix = base as u32;
                    let s = self.str_from_to(start + BytePos(2), end);
                    if s.chars().any(|c| c != '_' && c.to_digit(radix).is_none()) {
                        self.err(
                            self.mk_sp(start, end),
                            format!("invalid digit for a base {radix} literal"),
                        );
                        kind = token::Err(ErrorGuaranteed::unchecked_error_guaranteed());
                    }
                }
                (kind, self.symbol_from_to(start, end))
            }
            LiteralKind::Float {
                base,
                empty_exponent,
            } => {
                let mut kind = token::Float;
                if empty_exponent {
                    self.err(
                        self.mk_sp(start, self.pos),
                        "expected at least one digit in exponent",
                    );
                    kind = token::Err(ErrorGuaranteed::unchecked_error_guaranteed());
                }
                let base = match base {
                    Base::Hexadecimal => Some("hexadecimal"),
                    Base::Octal => Some("octal"),
                    Base::Binary => Some("binary"),
                    Base::Decimal => None,
                };
                if let Some(base) = base {
                    self.err(
                        self.mk_sp(start, end),
                        format!("{base} float literal is not supported"),
                    );
                    kind = token::Err(ErrorGuaranteed::unchecked_error_guaranteed());
                }
                (kind, self.symbol_from_to(start, end))
            }
        }
    }

    fn raw_str_error(&mut self, start: BytePos, end: BytePos) -> (token::LitKind, Symbol) {
        self.err(self.mk_sp(start, end), "invalid raw string literal");
        (
            token::Err(ErrorGuaranteed::unchecked_error_guaranteed()),
            self.symbol_from_to(start, end),
        )
    }

    fn cook_quoted(
        &mut self,
        kind: token::LitKind,
        start: BytePos,
        end: BytePos,
        prefix_len: u32,
        postfix_len: u32,
    ) -> (token::LitKind, Symbol) {
        let content_start = start + BytePos(prefix_len);
        let content_end = end - BytePos(postfix_len);
        if content_end < content_start {
            return (
                token::Err(ErrorGuaranteed::unchecked_error_guaranteed()),
                self.symbol_from_to(start, end),
            );
        }
        let lit_content = self.str_from_to(content_start, content_end);
        if let Some(message) = escape_error(kind, lit_content) {
            self.err(self.mk_sp(start, end), message);
            return (
                token::Err(ErrorGuaranteed::unchecked_error_guaranteed()),
                self.symbol_from_to(start, end),
            );
        }
        (kind, Symbol::intern(lit_content))
    }

    fn lex_token_trees(&mut self, is_delimited: bool) -> Result<(Spacing, TokenStream), LexError> {
        let open_spacing = self.bump_minimal();
        let mut buf = Vec::new();
        loop {
            if let Some(delim) = self.token.kind.open_delim() {
                buf.push(self.lex_token_tree_open_delim(delim)?);
            } else if let Some(delim) = self.token.kind.close_delim() {
                return if is_delimited {
                    Ok((open_spacing, TokenStream::new(buf)))
                } else {
                    let _ = delim;
                    Err(LexError {
                        span: self.token.span,
                        message: format!(
                            "unexpected closing delimiter: `{}`",
                            crate::ast_pretty::token_to_string(&self.token)
                        ),
                    })
                };
            } else if self.token.kind == token::Eof {
                return if is_delimited {
                    Err(LexError {
                        span: self.token.span,
                        message: "this file contains an unclosed delimiter".to_owned(),
                    })
                } else {
                    Ok((open_spacing, TokenStream::new(buf)))
                };
            } else {
                let (this_tok, this_spacing) = self.bump();
                buf.push(TokenTree::Token(this_tok, this_spacing));
            }
        }
    }

    fn lex_token_tree_open_delim(&mut self, open_delim: Delimiter) -> Result<TokenTree, LexError> {
        let pre_span = self.token.span;
        self.open_delimiters.push((open_delim, pre_span));
        let (open_spacing, tts) = self.lex_token_trees(true)?;
        let delim_span = DelimSpan::from_pair(pre_span, self.token.span);
        let close_spacing = match self.token.kind.close_delim() {
            Some(close_delim) if close_delim == open_delim => {
                self.open_delimiters.pop();
                self.bump_minimal()
            }
            _ => {
                return Err(LexError {
                    span: self.token.span,
                    message: format!(
                        "mismatched closing delimiter: `{}`",
                        crate::ast_pretty::token_to_string(&self.token)
                    ),
                });
            }
        };
        let spacing = DelimSpacing::new(open_spacing, close_spacing);
        Ok(TokenTree::Delimited(delim_span, spacing, open_delim, tts))
    }

    fn bump(&mut self) -> (Token, Spacing) {
        let (this_spacing, next_tok) = loop {
            let (next_tok, is_next_tok_preceded_by_whitespace) = self.next_token_from_cursor();
            if is_next_tok_preceded_by_whitespace {
                break (Spacing::Alone, next_tok);
            } else if let Some(glued) = self.token.glue(&next_tok) {
                self.token = glued;
            } else {
                let this_spacing = self.calculate_spacing(&next_tok);
                break (this_spacing, next_tok);
            }
        };
        let this_tok = std::mem::replace(&mut self.token, next_tok);
        (this_tok, this_spacing)
    }

    fn bump_minimal(&mut self) -> Spacing {
        let (next_tok, is_next_tok_preceded_by_whitespace) = self.next_token_from_cursor();
        let this_spacing = if is_next_tok_preceded_by_whitespace {
            Spacing::Alone
        } else {
            self.calculate_spacing(&next_tok)
        };
        self.token = next_tok;
        this_spacing
    }

    fn calculate_spacing(&self, next_tok: &Token) -> Spacing {
        if next_tok.is_punct() {
            Spacing::Joint
        } else if *next_tok == token::Eof {
            Spacing::Alone
        } else {
            Spacing::JointHidden
        }
    }
}

/// The first escape error rustc reports for a quoted literal's contents, if any.
fn escape_error(kind: token::LitKind, content: &str) -> Option<String> {
    use syntax::unescape::{self, EscapeError};
    let mut first: Option<EscapeError> = None;
    let mut note = |_: std::ops::Range<usize>, err: EscapeError| {
        if !err.is_fatal() {
            return;
        }
        first.get_or_insert(err);
    };
    match kind {
        token::Char => {
            if let Err(err) = unescape::unescape_char(content) {
                note(0..content.len(), err);
            }
        }
        token::Byte => {
            if let Err(err) = unescape::unescape_byte(content) {
                note(0..content.len(), err);
            }
        }
        token::Str => unescape::unescape_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::ByteStr => unescape::unescape_byte_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::CStr => unescape::unescape_c_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::StrRaw(_) => unescape::check_raw_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::ByteStrRaw(_) => unescape::check_raw_byte_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::CStrRaw(_) => unescape::check_raw_c_str(content, |range, res| {
            if let Err(err) = res {
                note(range, err);
            }
        }),
        token::Bool | token::Integer | token::Float | token::Err(_) => {}
    }
    first.map(|err| format!("invalid literal: {err:?}"))
}
