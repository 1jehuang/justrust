//! Checked, bounded lowering from native lexical events to the current parser.
//! This adapter does not admit macro expansion, hygienic names or new literals.
use super::{ParseError, ParserTokenKind, Token, TokenPart};
use crate::compiler::expansion_budget::Storage;
use crate::compiler::expansion_span::{SpanId, SyntheticReason};
use crate::compiler::lexer::{LexError, LexedEvent, LexedTokens, decode_normal_string};
use crate::compiler::source::PreparedLexInput;
use crate::compiler::source::{SourceError, SourceErrorKind};
use crate::compiler::token::{InputToken, Spacing, TokenKind};
use crate::compiler::token_session::{BudgetVec, TokenError, TokenSession};
use std::ops::{Deref, Range};

pub(super) struct NativeParserInput {
    tokens: BudgetVec<Token>,
    session: TokenSession,
    text_leases: BudgetVec<crate::compiler::expansion_budget::ByteLease>,
    names: BudgetVec<AdmittedRootName>,
}
/// Issued only after the native identifier's context, rawness and spelling have
/// passed the bounded root-only ingress gate. This is not a hygiene projection.
struct AdmittedRootName {
    symbol: crate::compiler::token_session::SymbolId,
    span: SpanId,
    edition: crate::compiler::token::Edition,
    logical: u32,
}
/// Independently owned lexical views. These strings never borrow the session,
/// so parsing may mint semantic identifiers while holding token views.
pub(super) struct NativeTokenBuffer {
    tokens: BudgetVec<Token>,
    names: BudgetVec<AdmittedRootName>,
    _text_leases: BudgetVec<crate::compiler::expansion_budget::ByteLease>,
}
impl Deref for NativeTokenBuffer {
    type Target = [Token];
    fn deref(&self) -> &Self::Target {
        self.tokens.as_slice()
    }
}
impl std::fmt::Debug for NativeTokenBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tokens.as_slice()).finish()
    }
}
impl NativeTokenBuffer {
    pub(super) fn mint_name(
        &self,
        token: &super::TokenView<'_>,
        names: &mut crate::compiler::semantic_name::NameStoreBuilder,
    ) -> Result<crate::compiler::semantic_name::AstIdent, ParseError> {
        let index = token
            .name
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let admitted = self
            .names
            .as_slice()
            .get(index.get() as usize - 1)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let logical = self
            .tokens
            .as_slice()
            .get(admitted.logical as usize)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if !std::ptr::eq(logical.text.as_str(), token.text)
            || logical.name != Some(index)
            || token.span != logical.whole.span
            || token.bytes != logical.whole.bytes()
        {
            return Err(transport_error(TokenError::ForeignHandle));
        }
        let token = crate::compiler::token::Token {
            kind: TokenKind::Ident(crate::compiler::token::Ident {
                symbol: admitted.symbol,
                context: names.session().root_context(),
                raw: false,
            }),
            span: admitted.span,
        };
        names
            .from_token(token, admitted.edition)
            .map_err(name_error)
    }
}

pub(super) fn name_error(error: crate::compiler::semantic_name::NameError) -> ParseError {
    use crate::compiler::semantic_name::NameError;
    match error {
        NameError::Token(error) => transport_error(error),
        error @ NameError::Unsupported(_) => {
            simple_error(SourceErrorKind::Unsupported, &error.to_string())
        }
        error @ (NameError::ExpectedIdentifier | NameError::InvalidIdentifier) => {
            simple_error(SourceErrorKind::Syntax, &error.to_string())
        }
    }
}
pub(super) fn name_diagnostic(error: crate::compiler::semantic_name::NameError) -> String {
    let error = name_error(error);
    let mut diagnostic = SourceError::syntax(error.primary, error.message);
    diagnostic.kind = error.kind;
    diagnostic.to_string()
}
impl std::fmt::Debug for NativeParserInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tokens.as_slice()).finish()
    }
}
impl Deref for NativeParserInput {
    type Target = [Token];
    fn deref(&self) -> &Self::Target {
        self.tokens.as_slice()
    }
}
impl NativeParserInput {
    pub(super) fn into_parts(self) -> (NativeTokenBuffer, TokenSession) {
        (
            NativeTokenBuffer {
                tokens: self.tokens,
                names: self.names,
                _text_leases: self.text_leases,
            },
            self.session,
        )
    }
    pub(super) fn new(
        lexed: LexedTokens,
        session: TokenSession,
        projection: &PreparedLexInput<'_>,
    ) -> Result<Self, ParseError> {
        lexed
            .validate_input(&session, projection)
            .map_err(lex_error)?;
        if projection.text().starts_with('\u{feff}') {
            let mut error = simple_error(
                SourceErrorKind::Unsupported,
                "byte-order-mark input is unsupported",
            );
            error.has_location = true;
            error.bytes = 0..3;
            return Err(error);
        }
        if projection.text().starts_with("#!") && !projection.text().starts_with("#![") {
            let mut error =
                simple_error(SourceErrorKind::Unsupported, "shebang input is unsupported");
            error.has_location = true;
            error.bytes = 0..2;
            return Err(error);
        }
        let budget = session.budget().clone();
        let edition = lexed.edition();
        let mut input = Self {
            tokens: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
            text_leases: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
            session,
            names: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
        };
        let events = lexed.events();
        let mut at = 0;
        while at < events.len() {
            let event = &events[at];
            input.check_event(event)?;
            match event.token {
                InputToken::Open { .. } | InputToken::Close { .. } => {
                    input.push_range(
                        projection,
                        event.prepared.clone(),
                        event.line,
                        event.column,
                        ParserTokenKind::Punct,
                        None,
                        &[native_part(event)?],
                    )?;
                    at += 1;
                }
                InputToken::Leaf(token) => match token.kind {
                    TokenKind::Ident(ident) => {
                        let text = input
                            .session
                            .symbol(ident.symbol)
                            .map_err(transport_error)?;
                        if ident.raw
                            || !text.is_ascii()
                            || ident.context != input.session.root_context()
                        {
                            return Err(event_error(
                                event,
                                SourceErrorKind::Unsupported,
                                "raw, non-ASCII and non-root identifiers are unsupported",
                            ));
                        }
                        input.push_range(
                            projection,
                            event.prepared.clone(),
                            event.line,
                            event.column,
                            ParserTokenKind::Ident,
                            None,
                            &[native_part(event)?],
                        )?;
                        input.attach_name(token, edition)?;
                        at += 1;
                    }
                    TokenKind::Literal { kind, spelling } => {
                        use crate::compiler::token::LiteralKind;
                        match kind {
                            LiteralKind::String => {
                                let info = event.literal.as_ref().ok_or_else(|| {
                                    event_error(
                                        event,
                                        SourceErrorKind::Contract,
                                        "literal metadata missing",
                                    )
                                })?;
                                if info.has_continuation {
                                    return Err(event_error(
                                        event,
                                        SourceErrorKind::Unsupported,
                                        "string continuation escapes are unsupported",
                                    ));
                                }
                                if !info.suffix.is_empty() {
                                    return Err(event_error(
                                        event,
                                        SourceErrorKind::Syntax,
                                        "string literal suffix is invalid",
                                    ));
                                }
                                let spelling =
                                    input.session.symbol(spelling).map_err(transport_error)?;
                                let decoded =
                                    decode_normal_string(spelling, &budget).map_err(|e| {
                                        let mut error = lex_error(e);
                                        if error.kind == SourceErrorKind::Syntax {
                                            error.kind = SourceErrorKind::Contract;
                                            error.message =
                                                "validated string decoding failed".into();
                                        }
                                        error
                                    })?;
                                input.push_range(
                                    projection,
                                    event.prepared.clone(),
                                    event.line,
                                    event.column,
                                    ParserTokenKind::String,
                                    Some(decoded.as_str()),
                                    &[native_part(event)?],
                                )?;
                            }
                            LiteralKind::Integer | LiteralKind::Float => {
                                input.push_number(projection, event)?;
                            }
                            _ => {
                                return Err(event_error(
                                    event,
                                    SourceErrorKind::Unsupported,
                                    "raw, byte, character and C literals are unsupported",
                                ));
                            }
                        }
                        at += 1;
                    }
                    TokenKind::Punct {
                        ch: '\'',
                        spacing: Spacing::Joint,
                    } if events.get(at + 1).is_some_and(|next| {
                        matches!(
                            next.token,
                            InputToken::Leaf(crate::compiler::token::Token {
                                kind: TokenKind::Ident(_),
                                ..
                            })
                        )
                    }) =>
                    {
                        let next = &events[at + 1];
                        input.check_event(next)?;
                        let InputToken::Leaf(crate::compiler::token::Token {
                            kind: TokenKind::Ident(ident),
                            ..
                        }) = next.token
                        else {
                            unreachable!()
                        };
                        let name = input
                            .session
                            .symbol(ident.symbol)
                            .map_err(transport_error)?;
                        if ident.raw
                            || !name.is_ascii()
                            || ident.context != input.session.root_context()
                        {
                            return Err(event_error(
                                next,
                                SourceErrorKind::Unsupported,
                                "raw, non-ASCII and non-root lifetimes are unsupported",
                            ));
                        }
                        if event.prepared.end != next.prepared.start {
                            return Err(event_error(
                                event,
                                SourceErrorKind::Contract,
                                "joint lifetime is not adjacent",
                            ));
                        }
                        input.push_range(
                            projection,
                            event.prepared.start..next.prepared.end,
                            event.line,
                            event.column,
                            ParserTokenKind::Lifetime,
                            None,
                            &[native_part(event)?, native_part(next)?],
                        )?;
                        let InputToken::Leaf(name_token) = next.token else {
                            unreachable!("validated lifetime identifier leaf")
                        };
                        input.attach_name(name_token, edition)?;
                        at += 2;
                    }
                    TokenKind::Punct { .. } => {
                        let mut count = 1;
                        let mut end = event.prepared.end;
                        for n in 1..3 {
                            let Some(next) = events.get(at + n) else {
                                break;
                            };
                            let previous = &events[at + n - 1];
                            if !matches!(
                                previous.token,
                                InputToken::Leaf(crate::compiler::token::Token {
                                    kind: TokenKind::Punct {
                                        spacing: Spacing::Joint,
                                        ..
                                    },
                                    ..
                                })
                            ) || !matches!(
                                next.token,
                                InputToken::Leaf(crate::compiler::token::Token {
                                    kind: TokenKind::Punct { .. },
                                    ..
                                })
                            ) || end != next.prepared.start
                            {
                                break;
                            }
                            input.check_event(next)?;
                            let candidate = projection
                                .text()
                                .get(event.prepared.start..next.prepared.end)
                                .ok_or_else(|| {
                                    event_error(
                                        event,
                                        SourceErrorKind::Contract,
                                        "invalid punctuation view range",
                                    )
                                })?;
                            if !compound(candidate) {
                                break;
                            }
                            count = n + 1;
                            end = next.prepared.end;
                        }
                        let range = event.prepared.start..end;
                        let text = projection.text().get(range.clone()).ok_or_else(|| {
                            event_error(
                                event,
                                SourceErrorKind::Contract,
                                "invalid punctuation range",
                            )
                        })?;
                        if !punctuation(text) {
                            return Err(event_error(
                                event,
                                if matches!(text, "#" | "@" | "?" | "$" | "~") {
                                    SourceErrorKind::Unsupported
                                } else {
                                    SourceErrorKind::Syntax
                                },
                                &format!(
                                    "{} token {text:?}",
                                    if matches!(text, "#" | "@" | "?" | "$" | "~") {
                                        "unsupported"
                                    } else {
                                        "invalid"
                                    }
                                ),
                            ));
                        }
                        let first = native_part(event)?;
                        let mut parts: [TokenPart; 3] = std::array::from_fn(|_| first.clone());
                        for n in 0..count {
                            parts[n] = native_part(&events[at + n])?;
                        }
                        input.push_range(
                            projection,
                            range,
                            event.line,
                            event.column,
                            ParserTokenKind::Punct,
                            None,
                            &parts[..count],
                        )?;
                        at += count;
                    }
                    TokenKind::DollarCrate(_) => {
                        return Err(event_error(
                            event,
                            SourceErrorKind::Unsupported,
                            "macro crate identities are unsupported",
                        ));
                    }
                    TokenKind::Opaque(_) => {
                        return Err(event_error(
                            event,
                            SourceErrorKind::Unsupported,
                            "opaque macro fragments are unsupported by this parser",
                        ));
                    }
                },
            }
        }
        let eof = lexed.eof();
        if eof.prepared != (projection.text().len()..projection.text().len()) {
            return Err(simple_error(
                SourceErrorKind::Contract,
                "invalid native EOF range",
            ));
        }
        input.push_range(
            projection,
            eof.prepared.clone(),
            eof.line,
            eof.column,
            ParserTokenKind::Eof,
            None,
            &[TokenPart::new(eof.span, eof.prepared.clone())?],
        )?;
        Ok(input)
    }

    fn check_event(&self, event: &LexedEvent) -> Result<(), ParseError> {
        let span = self
            .session
            .span(event.token.span())
            .map_err(transport_error)?;
        if span.context != self.session.root_context() || span.expansion.is_some() {
            return Err(event_error(
                event,
                SourceErrorKind::Unsupported,
                "expanded or hygienic tokens are unsupported by this parser",
            ));
        }
        if let InputToken::Leaf(token) = event.token {
            token.validate(&self.session).map_err(transport_error)?;
        }
        Ok(())
    }

    fn part(
        &mut self,
        projection: &PreparedLexInput<'_>,
        bytes: Range<usize>,
    ) -> Result<TokenPart, ParseError> {
        let projected = projection.projected(bytes.clone()).map_err(source_error)?;
        let anchor = self
            .session
            .original_span(projected.original)
            .map_err(transport_error)?;
        let span = if projected.synthetic {
            self.session
                .synthetic_span(anchor, SyntheticReason::TransportDelimiter)
                .map_err(transport_error)?
        } else {
            anchor
        };
        TokenPart::new(span, bytes)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_range(
        &mut self,
        projection: &PreparedLexInput<'_>,
        bytes: Range<usize>,
        line: usize,
        column: usize,
        kind: ParserTokenKind,
        decoded: Option<&str>,
        parts: &[TokenPart],
    ) -> Result<(), ParseError> {
        // Validate the whole legacy logical token, not merely its lexical atoms.
        let whole = if let [part] = parts
            && part.bytes() == bytes
        {
            projection.projected(bytes.clone()).map_err(source_error)?;
            part.clone()
        } else {
            self.part(projection, bytes.clone())?
        };
        let text = decoded.unwrap_or(projection.text().get(bytes.clone()).ok_or_else(|| {
            simple_error(SourceErrorKind::Contract, "invalid logical token range")
        })?);
        let budget = self.session.budget();
        let mut lease = budget
            .reserve(Storage::Retained, text.len() as u64)
            .map_err(|e| transport_error(TokenError::Budget(e)))?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(text.len())
            .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
        lease
            .resize(owned.capacity() as u64)
            .map_err(|e| transport_error(TokenError::Budget(e)))?;
        owned.push_str(text);
        let mut constituents = std::array::from_fn(|_| None);
        if parts.is_empty() {
            constituents[0] = Some(whole.span);
        } else {
            for (slot, part) in constituents.iter_mut().zip(parts) {
                *slot = Some(part.span);
            }
        }
        let mut suffixes = std::array::from_fn(|_| None);
        if kind == ParserTokenKind::Punct && matches!(text, ">>" | ">=" | ">>=" | "&&") {
            for prefix in 1..text.len() {
                suffixes[prefix - 1] =
                    Some(self.part(projection, bytes.start + prefix..bytes.end)?);
            }
        }
        self.text_leases.push(lease).map_err(transport_error)?;
        self.tokens
            .push(Token {
                text: owned,
                line,
                column,
                string: kind == ParserTokenKind::String,
                kind,
                whole,
                constituents,
                suffixes,
                name: None,
            })
            .map_err(transport_error)
    }

    fn attach_name(
        &mut self,
        token: crate::compiler::token::Token,
        edition: crate::compiler::token::Edition,
    ) -> Result<(), ParseError> {
        let TokenKind::Ident(ident) = token.kind else {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        };
        token.validate(&self.session).map_err(transport_error)?;
        if ident.raw
            || ident.context != self.session.root_context()
            || !self
                .session
                .symbol(ident.symbol)
                .map_err(transport_error)?
                .is_ascii()
        {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        }
        let index = u32::try_from(self.names.len())
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        let logical = self
            .tokens
            .len()
            .checked_sub(1)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        self.names
            .push(AdmittedRootName {
                symbol: ident.symbol,
                span: token.span,
                edition,
                logical,
            })
            .map_err(transport_error)?;
        let mut logical = self
            .tokens
            .pop()
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if logical.name.is_some()
            || !matches!(
                logical.kind,
                ParserTokenKind::Ident | ParserTokenKind::Lifetime
            )
        {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        }
        logical.name = Some(index);
        self.tokens.push(logical).map_err(transport_error)
    }

    fn push_number(
        &mut self,
        projection: &PreparedLexInput<'_>,
        event: &LexedEvent,
    ) -> Result<(), ParseError> {
        let after_dot = self
            .tokens
            .last()
            .is_some_and(|t| t.kind == ParserTokenKind::Punct && t.text == ".");
        let info = event.literal.as_ref().ok_or_else(|| {
            event_error(event, SourceErrorKind::Contract, "number metadata missing")
        })?;
        if !after_dot {
            return self.push_range(
                projection,
                event.prepared.clone(),
                event.line,
                event.column,
                ParserTokenKind::Number,
                None,
                &[native_part(event)?],
            );
        }
        // The old parser's tuple-number view divides the already lexed literal
        // at its known decimal point and signed exponent. This is not relexing.
        let mut start = event.prepared.start;
        if let Some(dot) = info.decimal_point {
            self.push_range(
                projection,
                start..dot,
                event.line,
                event.column,
                ParserTokenKind::Number,
                None,
                &[],
            )?;
            self.push_range(
                projection,
                dot..dot + 1,
                event.line,
                event.column + dot - event.prepared.start,
                ParserTokenKind::Punct,
                None,
                &[],
            )?;
            start = dot + 1;
        }
        if let Some(exponent) = info.exponent {
            let sign = exponent + 1;
            if matches!(projection.text().as_bytes().get(sign), Some(b'+' | b'-')) {
                self.push_range(
                    projection,
                    start..sign,
                    event.line,
                    event.column + start - event.prepared.start,
                    ParserTokenKind::Number,
                    None,
                    &[],
                )?;
                self.push_range(
                    projection,
                    sign..sign + 1,
                    event.line,
                    event.column + sign - event.prepared.start,
                    ParserTokenKind::Punct,
                    None,
                    &[],
                )?;
                start = sign + 1;
            }
        }
        if start < event.prepared.end {
            self.push_range(
                projection,
                start..event.prepared.end,
                event.line,
                event.column + start - event.prepared.start,
                ParserTokenKind::Number,
                None,
                &[],
            )?;
        }
        Ok(())
    }
}

pub(super) fn original(
    session: &TokenSession,
    span: SpanId,
) -> Result<crate::compiler::source::SourceSpan, ParseError> {
    use crate::compiler::expansion_span::SpanOrigin;
    let mut current = span;
    for _ in 0..512 {
        match session.span(current).map_err(transport_error)?.origin {
            SpanOrigin::Original(original) => {
                return session
                    .sources()
                    .render_span(original)
                    .map_err(transport_error);
            }
            SpanOrigin::Synthetic { anchor, .. } => current = anchor,
        }
    }
    Err(simple_error(
        SourceErrorKind::Contract,
        "parser token provenance is cyclic or too deep",
    ))
}
fn native_part(event: &LexedEvent) -> Result<TokenPart, ParseError> {
    TokenPart::new(event.token.span(), event.prepared.clone())
}
fn compound(s: &str) -> bool {
    matches!(
        s,
        "->" | "::"
            | "=="
            | "!="
            | "<="
            | ">="
            | "<<"
            | ">>"
            | "&&"
            | "||"
            | "=>"
            | "%="
            | "+="
            | "-="
            | "*="
            | "/="
            | "&="
            | "|="
            | "^="
            | ".."
            | "<<="
            | ">>="
    )
}
fn punctuation(s: &str) -> bool {
    compound(s)
        || matches!(
            s,
            "(" | ")"
                | "{"
                | "}"
                | "["
                | "]"
                | ":"
                | ";"
                | ","
                | "."
                | "+"
                | "-"
                | "*"
                | "/"
                | "%"
                | "="
                | "!"
                | "<"
                | ">"
                | "&"
                | "|"
                | "^"
        )
}
fn simple_error(kind: SourceErrorKind, message: &str) -> ParseError {
    ParseError {
        kind,
        has_location: false,
        bytes: 0..0,
        line: 1,
        column: 1,
        message: message.into(),
        primary: None,
    }
}
fn event_error(event: &LexedEvent, kind: SourceErrorKind, message: &str) -> ParseError {
    ParseError {
        kind,
        has_location: true,
        bytes: event.prepared.clone(),
        line: event.line,
        column: event.column,
        message: message.into(),
        primary: None,
    }
}
pub(super) fn source_error(error: SourceError) -> ParseError {
    ParseError {
        kind: error.kind,
        has_location: error.primary.is_some(),
        bytes: 0..0,
        line: 1,
        column: 1,
        message: error.message,
        primary: error.primary,
    }
}
pub(super) fn transport_error(error: TokenError) -> ParseError {
    source_error(SourceError::from_token(error))
}
pub(super) fn lex_error(error: LexError) -> ParseError {
    match error {
        LexError::Source {
            kind,
            prepared,
            line,
            column,
            message,
        } => ParseError {
            kind,
            has_location: true,
            bytes: prepared,
            line,
            column,
            message: message.into(),
            primary: None,
        },
        LexError::Projection(error) => source_error(error),
        LexError::Transport(error) => transport_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::source::{OriginSegment, SourceOptions, SourceSpan, prepare_text};
    use std::path::Path;

    fn split_source(text: &str, boundary: usize) -> crate::compiler::source::PreparedSource {
        let mut source = prepare_text("split.rs", text, &SourceOptions::default()).unwrap();
        let origin = source.origins[0].origin;
        source.origins = vec![
            OriginSegment {
                prepared: 0..boundary,
                origin: SourceSpan {
                    hi: boundary as u32,
                    ..origin
                },
                synthetic: false,
            },
            OriginSegment {
                prepared: boundary..text.len(),
                origin: SourceSpan {
                    lo: boundary as u32,
                    ..origin
                },
                synthetic: false,
            },
        ];
        source
    }

    #[test]
    fn logical_operators_and_lifetimes_reject_atom_only_mapping() {
        for (text, boundary) in [(">>=", 1), ("&&", 1), ("'region", 1)] {
            let source = split_source(text, boundary);
            let budget = ExpansionBudget::default();
            let projection = PreparedLexInput::prepared(&source, &budget).unwrap();
            let mut session = TokenSession::new(projection.snapshot().clone(), budget).unwrap();
            let lexed = crate::compiler::lexer::lex_prepared(
                &projection,
                crate::compiler::token::Edition::E2024,
                &mut session,
            )
            .unwrap();
            assert!(
                lexed
                    .events()
                    .iter()
                    .all(|event| projection.project(event.prepared.clone()).is_ok())
            );
            let error = NativeParserInput::new(lexed, session, &projection).unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Contract, "{text}: {error}");
            assert!(error.message.contains("segment boundary"));
        }
    }

    #[test]
    fn canonical_float_requires_whole_mapping_before_tuple_projection() {
        let source = split_source("t.0.1", 3);
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::prepared(&source, &budget).unwrap();
        let error = super::super::native_input(&projection, budget).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Contract);
        for text in ["t.0.1", "t. 0.1", "t./* comment */0.1"] {
            let input = super::super::lex(text).unwrap();
            assert_eq!(
                input
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["t", ".", "0", ".", "1", ""]
            );
        }
    }

    #[test]
    fn native_input_rejects_foreign_session_and_view_without_adoption() {
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::identity(Path::new("a.rs"), "name", &budget).unwrap();
        let mut session = TokenSession::new(projection.snapshot().clone(), budget.clone()).unwrap();
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let wrong = TokenSession::new(projection.snapshot().clone(), budget.clone()).unwrap();
        assert_eq!(
            NativeParserInput::new(lexed, wrong, &projection)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let other = PreparedLexInput::identity(Path::new("a.rs"), "name", &budget).unwrap();
        assert_eq!(
            NativeParserInput::new(lexed, session, &other)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
    }

    #[test]
    fn constituent_and_suffix_spans_retain_original_unicode_crlf_coordinates() {
        let input = {
            let text = String::from("/* 水 */\r\n  >>= &&");
            super::super::lex(&text).unwrap()
        };
        let token = &input[0];
        for (part, expected) in token
            .constituents
            .iter()
            .flatten()
            .zip([13..14, 14..15, 15..16])
        {
            let original = original(&input.session, *part).unwrap();
            assert_eq!((original.lo as usize..original.hi as usize), expected);
        }
        let mut cursor = super::super::TokenCursor::new(&input);
        assert!(cursor.consume_type_gt());
        let suffix = cursor.current();
        assert_eq!(suffix.bytes, 14..16);
        let projected = original(&input.session, suffix.span).unwrap();
        assert_eq!((projected.lo, projected.hi), (14, 16));
        assert!(cursor.consume_type_gt());
        assert_eq!(
            original(&input.session, cursor.current().span).unwrap().lo,
            15
        );
        cursor.bump();
        assert!(cursor.consume_type_amp_prefix());
        assert_eq!(
            original(&input.session, cursor.current().span).unwrap().lo,
            18
        );
    }

    #[test]
    fn native_leaf_identity_is_retained_and_compact_ranges_remain_checked() {
        let budget = ExpansionBudget::default();
        let projection =
            PreparedLexInput::identity(Path::new("single.rs"), "name", &budget).unwrap();
        let mut session = TokenSession::new(projection.snapshot().clone(), budget).unwrap();
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let span = lexed.events()[0].token.span();
        let eof = lexed.eof().span;
        let input = NativeParserInput::new(lexed, session, &projection).unwrap();
        assert_eq!(input[0].whole.span, span);
        assert_eq!(input[0].constituents[0].unwrap(), span);
        assert_eq!(input.last().unwrap().whole.span, eof);
        assert_eq!(
            TokenPart::new(span, Range { start: 1, end: 0 })
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        assert_eq!(
            TokenPart::new(span, 0..usize::MAX).unwrap_err().kind,
            SourceErrorKind::Contract
        );
    }

    #[test]
    fn nonroot_identifiers_literals_and_punctuation_are_not_semantic_input() {
        use crate::compiler::expansion_span::ExpansionRecord;
        use crate::compiler::hygiene::Transparency;
        use crate::compiler::token::{Edition, Ident, LiteralKind};
        let mut input = super::super::lex("name").unwrap();
        let original = input[0].whole.span;
        let file = input.session.sources().file(0).unwrap();
        let krate = input.session.register_crate_root(file).unwrap();
        let expansion = input
            .session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: None,
                defining_crate: krate,
                edition: Edition::E2024,
            })
            .unwrap();
        let context = input
            .session
            .apply_mark(
                input.session.root_context(),
                expansion,
                Transparency::Opaque,
            )
            .unwrap();
        let span = input
            .session
            .with_context(original, context, Some(expansion))
            .unwrap();
        let symbol = input.session.intern_symbol("name").unwrap();
        for kind in [
            TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context,
            }),
            TokenKind::Literal {
                kind: LiteralKind::String,
                spelling: symbol,
            },
            TokenKind::Punct {
                ch: '+',
                spacing: Spacing::Alone,
            },
        ] {
            let event = LexedEvent {
                token: InputToken::Leaf(crate::compiler::token::Token { kind, span }),
                prepared: 0..4,
                line: 1,
                column: 1,
                literal: None,
            };
            assert_eq!(
                input.check_event(&event).unwrap_err().kind,
                SourceErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn richer_lexical_classes_are_rejected_before_string_named_ast_lowering() {
        for text in [
            "\u{feff}fn f(){}",
            "#!/bin/native\nfn f(){}",
            "r#name",
            "水",
            "'x'",
            "b'x'",
            "b\"x\"",
            "r\"x\"",
            "c\"x\"",
            "\"a\\\nb\"",
        ] {
            assert_eq!(
                super::super::lex(text).unwrap_err().kind,
                SourceErrorKind::Unsupported,
                "{text:?}"
            );
        }
        let input = super::super::lex("\"\" \"fn\" \"&&\"").unwrap();
        let mut cursor = super::super::TokenCursor::new(&input);
        assert!(!cursor.current().matches(""));
        cursor.bump();
        assert!(!cursor.current().matches("fn"));
        cursor.bump();
        assert!(!cursor.consume_type_amp_prefix());
        cursor.bump();
        assert!(cursor.current().matches(""));
    }
}
