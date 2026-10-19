//! Native lexical transport for configured composite source views.
//! The entire lexical pass completes before original-source projection begins.
use std::fmt;
use std::ops::Range;

use super::expansion_budget::{Counter, ExpansionBudget, Storage};
use super::expansion_span::{SpanId, SyntheticReason};
use super::literal_scan::{self, LiteralError};
pub use super::literal_scan::{DecodedString, LiteralInfo};
use super::source::{PreparedLexInput, PreparedViewId, SourceError, SourceErrorKind};
use super::token::{Delimiter, Edition, Ident, InputToken, LiteralKind, Spacing, Token, TokenKind};
use super::token_session::{BudgetVec, SessionId, TokenError, TokenSession};

#[derive(Debug)]
pub enum LexError {
    Source {
        kind: SourceErrorKind,
        prepared: Range<usize>,
        line: usize,
        column: usize,
        message: &'static str,
    },
    Projection(SourceError),
    Transport(TokenError),
}
impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source {
                line,
                column,
                message,
                ..
            } => write!(f, "{line}:{column}: {message}"),
            Self::Projection(error) => write!(f, "{error}"),
            Self::Transport(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for LexError {}
impl From<TokenError> for LexError {
    fn from(error: TokenError) -> Self {
        Self::Transport(error)
    }
}
impl From<SourceError> for LexError {
    fn from(error: SourceError) -> Self {
        Self::Projection(error)
    }
}

#[derive(Clone, Debug)]
pub struct LexedEvent {
    pub token: InputToken,
    pub prepared: Range<usize>,
    pub line: usize,
    pub column: usize,
    pub literal: Option<LiteralInfo>,
}
#[derive(Clone, Debug)]
pub struct LexedEof {
    pub span: SpanId,
    pub prepared: Range<usize>,
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocStyle {
    Line,
    Block,
}
#[derive(Clone, Debug)]
pub struct DocComment {
    pub inner: bool,
    pub style: DocStyle,
    pub prepared: Range<usize>,
    pub body: Range<usize>,
    pub span: SpanId,
}
/// Immutable owned output. Handles cannot be adopted into a different session/view.
/// Docs are audit records, not configured attributes reintroduced into parser input.
pub struct LexedTokens {
    events: BudgetVec<LexedEvent>,
    docs: BudgetVec<DocComment>,
    eof: LexedEof,
    session: SessionId,
    view: PreparedViewId,
    edition: Edition,
}
impl LexedTokens {
    /// Edition used by the source lexer, retained independently of parser mode.
    pub fn edition(&self) -> Edition {
        self.edition
    }
    pub fn events(&self) -> &[LexedEvent] {
        self.events.as_slice()
    }
    pub fn docs(&self) -> &[DocComment] {
        self.docs.as_slice()
    }
    pub fn eof(&self) -> &LexedEof {
        &self.eof
    }
    pub fn validate_input(
        &self,
        session: &TokenSession,
        input: &PreparedLexInput<'_>,
    ) -> Result<(), LexError> {
        input.validate_session(session)?;
        session.budget().check().map_err(TokenError::from)?;
        if session.id() != self.session || input.view_id() != self.view {
            return Err(TokenError::ForeignHandle.into());
        }
        session.span(self.eof.span)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum RawKind {
    Ident {
        raw: bool,
        body: Range<usize>,
    },
    Literal {
        kind: LiteralKind,
        info: LiteralInfo,
    },
    Punct(char),
    Open(Delimiter),
    Close(Delimiter),
    Doc {
        inner: bool,
        style: DocStyle,
        body: Range<usize>,
    },
}
#[derive(Clone, Debug)]
struct RawEvent {
    kind: RawKind,
    range: Range<usize>,
    line: usize,
    column: usize,
}
struct RawScan {
    events: BudgetVec<RawEvent>,
    line: usize,
    column: usize,
}

fn location(text: &str, at: usize) -> (usize, usize) {
    let prefix = &text[..at];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}
fn source_error(text: &str, range: Range<usize>, message: &'static str) -> LexError {
    let (line, column) = location(text, range.start);
    LexError::Source {
        kind: SourceErrorKind::Syntax,
        prepared: range,
        line,
        column,
        message,
    }
}
fn at_error(text: &str, at: usize, message: &'static str) -> LexError {
    let end = text[at..]
        .chars()
        .next()
        .map_or(at, |ch| at + ch.len_utf8());
    source_error(text, at..end, message)
}
pub(super) fn literal_error(text: &str, error: LiteralError) -> LexError {
    match error {
        LiteralError::Source { range, message } => source_error(text, range, message),
        LiteralError::Transport(error) => LexError::Transport(error),
    }
}
fn work(budget: &ExpansionBudget, amount: usize) -> Result<(), LexError> {
    budget
        .charge(Counter::Work, amount as u64)
        .map_err(TokenError::from)?;
    Ok(())
}
fn advance(
    text: &str,
    from: usize,
    to: usize,
    line: &mut usize,
    column: &mut usize,
    budget: &ExpansionBudget,
) -> Result<(), LexError> {
    work(budget, to - from)?;
    for ch in text[from..to].chars() {
        if ch == '\n' {
            *line += 1;
            *column = 1;
        } else {
            *column += 1;
        }
    }
    Ok(())
}
fn white(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{0085}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
    )
}
fn inner_attribute_after_shebang(
    text: &str,
    mut at: usize,
    budget: &ExpansionBudget,
) -> Result<bool, LexError> {
    let bytes = text.as_bytes();
    while at < text.len() {
        work(budget, 1)?;
        let ch = text[at..].chars().next().ok_or(TokenError::InvalidRange)?;
        if white(ch) {
            at += ch.len_utf8();
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            while at < text.len() && bytes[at] != b'\n' {
                work(budget, 1)?;
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            let mut depth = 1usize;
            at += 2;
            while at < text.len() && depth != 0 {
                work(budget, 1)?;
                if bytes[at..].starts_with(b"/*") {
                    depth = depth.checked_add(1).ok_or(TokenError::IdExhausted)?;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            if depth != 0 {
                return Ok(false);
            }
            continue;
        }
        return Ok(ch == '[');
    }
    Ok(false)
}
fn scan(
    text: &str,
    edition: Edition,
    budget: &ExpansionBudget,
    retain: bool,
) -> Result<RawScan, LexError> {
    let bytes = text.as_bytes();
    let mut events = BudgetVec::new(budget, Storage::Temporary)?;
    let (mut at, mut line, mut column) = (0usize, 1usize, 1usize);
    if text.starts_with('\u{feff}') {
        at = '\u{feff}'.len_utf8();
    }
    if bytes.get(at..).is_some_and(|tail| tail.starts_with(b"#!"))
        && !inner_attribute_after_shebang(text, at + 2, budget)?
    {
        let end = text[at..]
            .find('\n')
            .map_or(text.len(), |offset| at + offset);
        advance(text, at, end, &mut line, &mut column, budget)?;
        at = end;
    }
    while at < bytes.len() {
        work(budget, 1)?;
        let start = at;
        let ch = text[at..].chars().next().ok_or(TokenError::InvalidRange)?;
        if white(ch) {
            at += ch.len_utf8();
            advance(text, start, at, &mut line, &mut column, budget)?;
            continue;
        }
        let start_line = line;
        let start_column = column;
        let kind;
        if bytes[at..].starts_with(b"//") {
            let doc = (bytes[at..].starts_with(b"///") && !bytes[at..].starts_with(b"////"))
                || bytes[at..].starts_with(b"//!");
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                work(budget, 1)?;
                at += 1;
            }
            if !doc {
                advance(text, start, at, &mut line, &mut column, budget)?;
                continue;
            }
            kind = RawKind::Doc {
                inner: bytes[start + 2] == b'!',
                style: DocStyle::Line,
                body: start + 3..at,
            };
        } else if bytes[at..].starts_with(b"/*") {
            let doc = (bytes[at..].starts_with(b"/**")
                && !bytes[at..].starts_with(b"/***")
                && !bytes[at..].starts_with(b"/**/"))
                || bytes[at..].starts_with(b"/*!");
            at += 2;
            let mut depth = 1usize;
            while at < bytes.len() && depth != 0 {
                work(budget, 1)?;
                if bytes[at..].starts_with(b"/*") {
                    depth = depth.checked_add(1).ok_or(TokenError::IdExhausted)?;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            if depth != 0 {
                return Err(at_error(text, start, "unterminated comment"));
            }
            if !doc {
                advance(text, start, at, &mut line, &mut column, budget)?;
                continue;
            }
            kind = RawKind::Doc {
                inner: bytes[start + 2] == b'!',
                style: DocStyle::Block,
                body: start + 3..at - 2,
            };
        } else if let Some(literal) =
            literal_scan::scan(text, at, edition, budget).map_err(|e| literal_error(text, e))?
        {
            at = literal.end;
            kind = RawKind::Literal {
                kind: literal.kind,
                info: literal.info,
            };
        } else if literal_scan::ident_start(ch) {
            let raw = bytes[at..].starts_with(b"r#");
            if raw {
                at += 2;
            }
            let body_start = at;
            if !text[at..]
                .chars()
                .next()
                .is_some_and(literal_scan::ident_start)
            {
                return Err(at_error(text, at, "expected raw identifier"));
            }
            while let Some(c) = text[at..].chars().next() {
                if !literal_scan::ident_continue(c) {
                    break;
                }
                work(budget, c.len_utf8())?;
                at += c.len_utf8();
            }
            if raw
                && matches!(
                    &text[body_start..at],
                    "_" | "crate" | "self" | "super" | "Self"
                )
            {
                return Err(at_error(text, start, "prohibited raw identifier"));
            }
            if !raw
                && matches!(edition, Edition::E2021 | Edition::E2024)
                && matches!(bytes.get(at), Some(b'"' | b'\'' | b'#'))
            {
                return Err(at_error(
                    text,
                    start,
                    "reserved literal or identifier prefix",
                ));
            }
            kind = RawKind::Ident {
                raw,
                body: body_start..at,
            };
        } else {
            at += ch.len_utf8();
            kind = match ch {
                '(' => RawKind::Open(Delimiter::Parenthesis),
                ')' => RawKind::Close(Delimiter::Parenthesis),
                '[' => RawKind::Open(Delimiter::Bracket),
                ']' => RawKind::Close(Delimiter::Bracket),
                '{' => RawKind::Open(Delimiter::Brace),
                '}' => RawKind::Close(Delimiter::Brace),
                '#' if edition == Edition::E2024 && matches!(bytes.get(at), Some(b'#' | b'"')) => {
                    return Err(at_error(
                        text,
                        start,
                        "reserved guarded string or hash sequence",
                    ));
                }
                '=' | '<' | '>' | '!' | '~' | '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|'
                | '@' | '.' | ',' | ';' | ':' | '#' | '$' | '?' | '\'' => RawKind::Punct(ch),
                _ => return Err(at_error(text, start, "invalid token")),
            };
        }
        if matches!(kind, RawKind::Doc { .. }) {
            work(budget, at - start)?;
            if let Some(offset) = bytes[start..at]
                .iter()
                .enumerate()
                .find_map(|(offset, byte)| {
                    (*byte == b'\r' && bytes.get(start + offset + 1) != Some(&b'\n'))
                        .then_some(offset)
                })
            {
                return Err(at_error(
                    text,
                    start + offset,
                    "bare carriage return in documentation comment",
                ));
            }
        }
        advance(text, start, at, &mut line, &mut column, budget)?;
        if retain {
            budget
                .charge(Counter::InputTokens, 1)
                .map_err(TokenError::from)?;
            events.push(RawEvent {
                kind,
                range: start..at,
                line: start_line,
                column: start_column,
            })?;
        }
    }
    Ok(RawScan {
        events,
        line,
        column,
    })
}

fn projected_span(
    input: &PreparedLexInput<'_>,
    range: Range<usize>,
    session: &mut TokenSession,
) -> Result<SpanId, LexError> {
    let projected = input.projected(range)?;
    // A foreign snapshot must fail before any span can be interned in this session.
    session.sources().render_span(projected.original)?;
    let anchor = session.original_span(projected.original)?;
    if projected.synthetic {
        Ok(session.synthetic_span(anchor, SyntheticReason::TransportDelimiter)?)
    } else {
        Ok(anchor)
    }
}

pub fn lex_prepared(
    input: &PreparedLexInput<'_>,
    edition: Edition,
    session: &mut TokenSession,
) -> Result<LexedTokens, LexError> {
    input.validate_session(session)?;
    let raw = scan(input.text(), edition, session.budget(), true)?;
    // No projection, source comparison, spelling interning or span construction
    // occurs before this point. Even later malformed literals beat earlier bad maps.
    let mut events = BudgetVec::new(session.budget(), Storage::Retained)?;
    let mut docs = BudgetVec::new(session.budget(), Storage::Retained)?;
    for (index, event) in raw.events.as_slice().iter().enumerate() {
        work(session.budget(), 1)?;
        let span = projected_span(input, event.range.clone(), session)?;
        let token = match &event.kind {
            RawKind::Ident { raw, body } => InputToken::Leaf(Token {
                kind: TokenKind::Ident(Ident {
                    symbol: session.intern_symbol(&input.text()[body.clone()])?,
                    raw: *raw,
                    context: session.root_context(),
                }),
                span,
            }),
            RawKind::Literal { kind, .. } => InputToken::Leaf(Token {
                kind: TokenKind::Literal {
                    kind: *kind,
                    spelling: session.intern_symbol(&input.text()[event.range.clone()])?,
                },
                span,
            }),
            RawKind::Punct(ch) => {
                let next = raw.events.as_slice().get(index + 1);
                let joint = next.is_some_and(|next| {
                    event.range.end == next.range.start
                        && (matches!(next.kind, RawKind::Punct(_))
                            || (*ch == '\'' && matches!(next.kind, RawKind::Ident { .. })))
                });
                InputToken::Leaf(Token {
                    kind: TokenKind::Punct {
                        ch: *ch,
                        spacing: if joint {
                            Spacing::Joint
                        } else {
                            Spacing::Alone
                        },
                    },
                    span,
                })
            }
            RawKind::Open(kind) => InputToken::Open { kind: *kind, span },
            RawKind::Close(kind) => InputToken::Close { kind: *kind, span },
            RawKind::Doc { inner, style, body } => {
                docs.push(DocComment {
                    inner: *inner,
                    style: *style,
                    prepared: event.range.clone(),
                    body: body.clone(),
                    span,
                })?;
                continue;
            }
        };
        events.push(LexedEvent {
            token,
            prepared: event.range.clone(),
            line: event.line,
            column: event.column,
            literal: match &event.kind {
                RawKind::Literal { info, .. } => Some(info.clone()),
                _ => None,
            },
        })?;
    }
    let prepared = input.text().len()..input.text().len();
    let span = projected_span(input, prepared.clone(), session)?;
    Ok(LexedTokens {
        events,
        docs,
        eof: LexedEof {
            span,
            prepared,
            line: raw.line,
            column: raw.column,
        },
        session: session.id(),
        view: input.view_id(),
        edition,
    })
}

pub fn decode_normal_string(
    spelling: &str,
    budget: &ExpansionBudget,
) -> Result<DecodedString, LexError> {
    literal_scan::decode_normal_string(spelling, budget).map_err(|e| literal_error(spelling, e))
}

pub(super) fn decode_text_string(
    spelling: &str,
    budget: &ExpansionBudget,
) -> Result<DecodedString, LexError> {
    literal_scan::decode_text_string(spelling, budget, true).map_err(|e| literal_error(spelling, e))
}

/// Original-file lexical validation before configuration. No projected tokens,
/// spelling interning or InputTokens charges are produced by this pass.
pub(super) fn validate_source(
    text: &str,
    edition: Edition,
    budget: &ExpansionBudget,
) -> Result<(), LexError> {
    scan(text, edition, budget, false).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::source::{OriginSegment, PreparedSource, SourceMap, SourceSpan};
    use std::path::Path;
    use std::sync::Arc;

    fn input<'a>(text: &'a str, budget: &ExpansionBudget) -> PreparedLexInput<'a> {
        PreparedLexInput::identity(Path::new("lex.rs"), text, budget).unwrap()
    }

    #[test]
    fn raw_validation_does_not_publish_or_double_count_logical_input() {
        let budget = ExpansionBudget::default();
        validate_source(
            "#[rustc_private] const fn f<T>(x:T)->T{x}",
            Edition::E2024,
            &budget,
        )
        .unwrap();
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.counters[Counter::InputTokens as usize], 0);
        assert_eq!(snapshot.counters[Counter::SpanRecords as usize], 0);
        assert_eq!(snapshot.retained_bytes, 0);
        assert_eq!(snapshot.temporary_bytes, 0);
        assert!(snapshot.counters[Counter::Work as usize] > 0);
    }

    #[test]
    fn source_envelope_bom_shebang_and_inner_attributes_are_distinct() {
        let budget = ExpansionBudget::default();
        for text in [
            "\u{feff}#!/bin/rust\nfn main(){}",
            "#![doc=\"x\"]fn main(){}",
            "#! /* gap */ [doc=\"x\"]fn main(){}",
            "#! // gap\n [doc=\"x\"]fn main(){}",
        ] {
            validate_source(text, Edition::E2024, &budget).unwrap();
        }
        assert!(validate_source("fn\u{feff} main(){}", Edition::E2024, &budget).is_err());
        let raw = scan(
            "#! /* gap */ [doc=\"x\"]fn main(){}",
            Edition::E2024,
            &budget,
            true,
        )
        .unwrap();
        assert!(matches!(raw.events.as_slice()[0].kind, RawKind::Punct('#')));
        assert!(matches!(raw.events.as_slice()[1].kind, RawKind::Punct('!')));
        assert!(matches!(
            raw.events.as_slice()[2].kind,
            RawKind::Open(Delimiter::Bracket)
        ));
    }

    #[test]
    fn kind_raw_edition_and_exact_literal_spellings_survive_transport() {
        let text = "r#fn café a\u{0301} 'a \"fn\" \"\" 2.5E+7f32 r#\"x\"# b'a' c\"水\" $ #";
        let budget = ExpansionBudget::default();
        let input = input(text, &budget);
        let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let InputToken::Leaf(Token {
            kind: TokenKind::Ident(name),
            ..
        }) = lexed.events()[0].token
        else {
            panic!("raw identifier")
        };
        assert!(name.raw);
        assert_eq!(session.symbol(name.symbol).unwrap(), "fn");
        for event in lexed.events() {
            if let InputToken::Leaf(Token {
                kind: TokenKind::Literal { spelling, .. },
                ..
            }) = event.token
            {
                assert_eq!(
                    session.symbol(spelling).unwrap(),
                    &text[event.prepared.clone()]
                );
            }
        }
        for text in [
            "r#_",
            "r#self",
            "r#super",
            "r#Self",
            "r#crate",
            "\u{0301}a",
            "🦀",
        ] {
            assert!(
                matches!(
                    validate_source(text, Edition::E2024, session.budget()),
                    Err(LexError::Source {
                        kind: SourceErrorKind::Syntax,
                        ..
                    })
                ),
                "{text}"
            );
        }
        assert!(validate_source("## #\"abc\"#", Edition::E2021, session.budget()).is_ok());
        assert!(validate_source("##", Edition::E2024, session.budget()).is_err());
        assert!(validate_source("#\"abc\"#", Edition::E2024, session.budget()).is_err());
    }

    #[test]
    fn punctuation_jointness_cannot_cross_trivia_and_lifetimes_keep_parts() {
        let text = ">>= > > = >/*gap*/>= && 'a";
        let budget = ExpansionBudget::default();
        let input = input(text, &budget);
        let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let punctuation: Vec<_> = lexed
            .events()
            .iter()
            .filter_map(|event| match event.token {
                InputToken::Leaf(Token {
                    kind: TokenKind::Punct { ch, spacing },
                    ..
                }) => Some((ch, spacing)),
                _ => None,
            })
            .collect();
        assert_eq!(
            &punctuation[..3],
            &[
                ('>', Spacing::Joint),
                ('>', Spacing::Joint),
                ('=', Spacing::Alone)
            ]
        );
        assert_eq!(
            &punctuation[3..7],
            &[
                ('>', Spacing::Alone),
                ('>', Spacing::Alone),
                ('=', Spacing::Alone),
                ('>', Spacing::Alone)
            ]
        );
        assert_eq!(punctuation.last(), Some(&('\'', Spacing::Joint)));
    }

    #[test]
    fn docs_are_retained_audit_records_not_reintroduced_attributes() {
        let text = "//// ordinary\n/// outer\n/**/ /*** ordinary */ /** block */ /*! inner */ //! line\nfn main(){}";
        let budget = ExpansionBudget::default();
        let input = input(text, &budget);
        let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        assert_eq!(lexed.docs().len(), 4);
        assert_eq!(lexed.docs().iter().filter(|doc| doc.inner).count(), 2);
        assert!(lexed.events().iter().all(|event| !matches!(
            event.token,
            InputToken::Leaf(Token {
                kind: TokenKind::Punct { ch: '#', .. },
                ..
            })
        )));
        for text in ["/// bad\rcarriage\n", "/** bad\rcarriage */"] {
            assert!(matches!(
                validate_source(text, Edition::E2024, session.budget()),
                Err(LexError::Source {
                    kind: SourceErrorKind::Syntax,
                    ..
                })
            ));
        }
        validate_source(
            "/// good\r\n/** good\r\n */",
            Edition::E2024,
            session.budget(),
        )
        .unwrap();
        validate_source(
            "// ordinary\rcarriage\n/* ordinary\rcarriage */",
            Edition::E2024,
            session.budget(),
        )
        .unwrap();
    }

    #[test]
    fn late_lexical_failure_precedes_earlier_unmapped_token() {
        let text = "name \"\\q\"";
        let mut sources = SourceMap::default();
        sources.add("lex.rs".into(), Arc::from(text));
        let mut prepared = PreparedSource::default();
        prepared.text = text.into();
        prepared.sources = sources;
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        let mut session = TokenSession::new(input.snapshot().clone(), budget.clone()).unwrap();
        assert!(matches!(
            lex_prepared(&input, Edition::E2024, &mut session),
            Err(LexError::Source {
                kind: SourceErrorKind::Syntax,
                message: "unsupported string escape",
                ..
            })
        ));
        assert_eq!(budget.snapshot().counters[Counter::SpanRecords as usize], 0);
    }

    #[test]
    fn synthetic_delimiters_and_eof_keep_indivisible_original_anchor() {
        let mut sources = SourceMap::default();
        let file = sources.add("root.rs".into(), Arc::from("mod child;"));
        let original = SourceSpan {
            file,
            lo: 0,
            hi: 10,
        };
        let mut prepared = PreparedSource::default();
        prepared.text = "{}".into();
        prepared.sources = sources;
        prepared.origins = vec![OriginSegment {
            prepared: 0..2,
            origin: original,
            synthetic: true,
        }];
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        for span in lexed
            .events()
            .iter()
            .map(|event| event.token.span())
            .chain(std::iter::once(lexed.eof().span))
        {
            assert!(matches!(
                session.span(span).unwrap().origin,
                crate::compiler::expansion_span::SpanOrigin::Synthetic {
                    reason: SyntheticReason::TransportDelimiter,
                    ..
                }
            ));
            assert_eq!(
                session
                    .sources()
                    .render_span(session.original(span, 8).unwrap())
                    .unwrap(),
                original
            );
        }
        assert_eq!(lexed.eof().prepared, 2..2);
    }

    #[test]
    fn retained_output_rejects_foreign_session_and_view() {
        let budget = ExpansionBudget::default();
        let input = input("", &budget);
        let mut session = TokenSession::new(input.snapshot().clone(), budget.clone()).unwrap();
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        lexed.validate_input(&session, &input).unwrap();
        let foreign = TokenSession::new(input.snapshot().clone(), budget.clone()).unwrap();
        assert!(matches!(
            lexed.validate_input(&foreign, &input),
            Err(LexError::Transport(TokenError::ForeignHandle))
        ));
        let different_view =
            PreparedLexInput::identity(Path::new("other.rs"), "", &budget).unwrap();
        assert!(matches!(
            lexed.validate_input(&session, &different_view),
            Err(LexError::Projection(SourceError {
                kind: SourceErrorKind::Contract,
                ..
            }))
        ));
        assert_eq!(lexed.events().len(), 0);
        assert_eq!(lexed.eof().prepared, 0..0);
    }

    #[test]
    fn split_source_and_session_ledgers_fail_before_lexical_work() {
        let source_budget = ExpansionBudget::default();
        let input = input("name", &source_budget);
        let other_budget = ExpansionBudget::default();
        let mut session =
            TokenSession::new(input.snapshot().clone(), other_budget.clone()).unwrap();
        let before_source = source_budget.snapshot();
        let before_other = other_budget.snapshot();
        assert!(matches!(
            lex_prepared(&input, Edition::E2024, &mut session),
            Err(LexError::Projection(SourceError {
                kind: SourceErrorKind::Contract,
                ..
            }))
        ));
        assert_eq!(source_budget.snapshot().counters, before_source.counters);
        assert_eq!(other_budget.snapshot().counters, before_other.counters);
    }
}
