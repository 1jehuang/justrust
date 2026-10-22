//! Expanded token ingress. No printing, relexing, source-range fabrication or
//! semantic identity issuance occurs here. The parser owns expression admission.
use super::native_adapter::{self, NativeTokenBuffer, scope_error, transport_error};
use super::{ParseError, ParserTokenKind, TokenPart};
use crate::compiler::expansion_budget::{Counter, Storage};
use crate::compiler::expansion_span::SpanId;
use crate::compiler::macro_cursor::{LogicalToken, MacroCursor, TokenSlice};
use crate::compiler::semantic_name::NameStoreBuilder;
use crate::compiler::semantic_scope::{
    ExpansionAdmission, ExpansionTokenAdmission, ItemExpansionAdmission, ModuleAccess,
};
use crate::compiler::source::SourceErrorKind;
use crate::compiler::token::{Delimiter, Edition, LiteralKind, Token, TokenKind};
use crate::compiler::token_session::{BudgetVec, TokenError, TokenSession};
use crate::compiler::token_tree::Node;

#[path = "expanded_adapter/lifetime.rs"]
mod lifetime;

struct Frame {
    cursor: MacroCursor,
    close: Option<(Delimiter, SpanId)>,
}

/// The only semantic unwrapping boundary. This private buffer is consumed by
/// the existing expression parser as one atom, never returned to macro matching.
pub(super) fn literal_buffer(
    names: &mut NameStoreBuilder,
    opaque: Token,
    admission: &ExpansionTokenAdmission,
) -> Result<NativeTokenBuffer, ParseError> {
    use crate::compiler::semantic_name::LiteralError;
    let parts: crate::compiler::semantic_name::LiteralParts = match admission {
        ExpansionTokenAdmission::Expression(proof) => names.literal_parts(opaque, proof),
        ExpansionTokenAdmission::Item(_) => names.literal_parts_in(opaque, admission),
    }
    .map_err(|error| match error {
        LiteralError::Token(error) => transport_error(error),
        LiteralError::Scope(error) => scope_error(error),
        LiteralError::Unsupported { span } => {
            failure(names.session(), span, "unsupported opaque literal class")
        }
    })?;
    if parts.outer_occurrence != opaque.span {
        return Err(transport_error(TokenError::InvalidHandle));
    }
    let eof = match admission {
        ExpansionTokenAdmission::Expression(proof) => names.expansion_eof(proof),
        ExpansionTokenAdmission::Item(proof) => names.item_expansion_eof(proof),
    }
    .map_err(scope_error)?;
    let session = names.session();
    let mut output = NativeTokenBuffer::expanded_in(session, admission)?;
    if let Some(minus) = parts.minus {
        if !matches!(minus.kind, TokenKind::Punct { ch: '-', .. }) {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        output.push_expanded(
            session,
            "-",
            ParserTokenKind::Punct,
            &[part(session, minus.span)?],
            None,
        )?;
    }
    // No arbitrary payload token can become a name or a recursive opaque atom.
    match parts.value.kind {
        TokenKind::Literal {
            kind:
                LiteralKind::Integer
                | LiteralKind::Float
                | LiteralKind::String
                | LiteralKind::RawString
                | LiteralKind::ByteString
                | LiteralKind::RawByteString
                | LiteralKind::Char
                | LiteralKind::Byte,
            ..
        } => {}
        TokenKind::Ident(ident)
            if !ident.raw
                && matches!(
                    session.symbol(ident.symbol).map_err(transport_error)?,
                    "true" | "false"
                ) => {}
        _ => {
            return Err(failure(
                session,
                parts.value.span,
                "unsupported opaque literal class",
            ));
        }
    }
    push_logical(
        &mut output,
        session,
        LogicalToken::Leaf(parts.value),
        names.edition(),
    )?;
    output.push_expanded(
        session,
        "",
        ParserTokenKind::Eof,
        &[part(session, eof)?],
        None,
    )?;
    Ok(output)
}

/// A private anchor proof is required even for empty or punctuation-only output.
/// The buffer is independent of the mutable name owner used by the parser.
pub(super) fn lower(
    slice: TokenSlice,
    names: &mut NameStoreBuilder,
    edition: Edition,
    eof: SpanId,
    admission: &ExpansionAdmission,
) -> Result<NativeTokenBuffer, ParseError> {
    lower_in(
        slice,
        names,
        edition,
        eof,
        &ExpansionTokenAdmission::Expression(*admission),
    )
}

pub(super) fn lower_item(
    slice: TokenSlice,
    names: &mut NameStoreBuilder,
    modules: &impl ModuleAccess,
    edition: Edition,
    eof: SpanId,
    admission: &ItemExpansionAdmission,
) -> Result<NativeTokenBuffer, ParseError> {
    admission
        .check_modules(names, modules)
        .map_err(scope_error)?;
    lower_in(
        slice,
        names,
        edition,
        eof,
        &ExpansionTokenAdmission::Item(*admission),
    )
}

fn lower_in(
    slice: TokenSlice,
    names: &mut NameStoreBuilder,
    edition: Edition,
    eof: SpanId,
    admission: &ExpansionTokenAdmission,
) -> Result<NativeTokenBuffer, ParseError> {
    let session = names.session();
    session
        .budget()
        .check()
        .map_err(|error| transport_error(error.into()))?;
    slice.len(session).map_err(transport_error)?;
    session.span(eof).map_err(transport_error)?;
    admission.validate_session(session).map_err(scope_error)?;
    admission.validate_span(session, eof).map_err(scope_error)?;
    let mut output = match admission {
        ExpansionTokenAdmission::Expression(proof) => NativeTokenBuffer::expanded(session, proof),
        ExpansionTokenAdmission::Item(_) => NativeTokenBuffer::expanded_in(session, admission),
    }?;
    output.retain_macro_slice(session, slice.clone())?;
    let mut frames =
        BudgetVec::new(session.budget(), Storage::Temporary).map_err(transport_error)?;
    frames
        .push(Frame {
            cursor: MacroCursor::new(session, slice).map_err(transport_error)?,
            close: None,
        })
        .map_err(transport_error)?;
    while let Some(mut frame) = frames.pop() {
        let next = frame.cursor.bump(session).map_err(transport_error)?;
        match next {
            None => {
                if let Some((delimiter, span)) = frame.close {
                    let (_, close) = delimiters(delimiter);
                    output.push_expanded(
                        session,
                        close,
                        ParserTokenKind::Punct,
                        &[part(session, span)?],
                        None,
                    )?;
                }
            }
            Some(LogicalToken::Group {
                kind,
                open,
                children,
                close,
            }) => {
                session.span(open).map_err(transport_error)?;
                session.span(close).map_err(transport_error)?;
                let (start, _) = delimiters(kind);
                output.push_expanded(
                    session,
                    start,
                    ParserTokenKind::Punct,
                    &[part(session, open)?],
                    None,
                )?;
                frames.push(frame).map_err(transport_error)?;
                frames
                    .push(Frame {
                        cursor: MacroCursor::new(session, children).map_err(transport_error)?,
                        close: Some((kind, close)),
                    })
                    .map_err(transport_error)?;
            }
            Some(logical) => {
                push_logical(&mut output, session, logical, edition)?;
                frames.push(frame).map_err(transport_error)?;
            }
        }
    }
    output.push_expanded(
        session,
        "",
        ParserTokenKind::Eof,
        &[part(session, eof)?],
        None,
    )?;
    Ok(output)
}

pub(super) fn validate_slice(
    session: &TokenSession,
    slice: &TokenSlice,
    admission: &ExpansionTokenAdmission,
) -> Result<(), ParseError> {
    let mut pending =
        BudgetVec::new(session.budget(), Storage::Temporary).map_err(transport_error)?;
    pending.push(slice.clone()).map_err(transport_error)?;
    while let Some(slice) = pending.pop() {
        let count = slice.len(session).map_err(transport_error)?;
        session
            .budget()
            .charge(Counter::Work, count as u64)
            .map_err(|error| transport_error(error.into()))?;
        for index in 0..count {
            match slice.node(session, index).map_err(transport_error)? {
                Node::Leaf(token) => admission
                    .validate_token(session, *token)
                    .map_err(scope_error)?,
                Node::Delimited { open, close, .. } => {
                    admission
                        .validate_span(session, *open)
                        .map_err(scope_error)?;
                    admission
                        .validate_span(session, *close)
                        .map_err(scope_error)?;
                    let children = slice
                        .children(session, index)
                        .map_err(transport_error)?
                        .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
                    pending.push(children).map_err(transport_error)?;
                }
            }
        }
    }
    Ok(())
}

fn delimiters(delimiter: Delimiter) -> (&'static str, &'static str) {
    match delimiter {
        Delimiter::Parenthesis => ("(", ")"),
        Delimiter::Brace => ("{", "}"),
        Delimiter::Bracket => ("[", "]"),
    }
}

fn part(session: &TokenSession, span: SpanId) -> Result<TokenPart, ParseError> {
    let original = session.original(span, 512).map_err(transport_error)?;
    let (_, line, column) = session
        .sources()
        .location(original)
        .map_err(transport_error)?;
    TokenPart::expanded(span, line, column)
}

fn failure(session: &TokenSession, span: SpanId, message: &str) -> ParseError {
    native_adapter::expanded_error(session, span, SourceErrorKind::Unsupported, message)
}

fn push_logical(
    output: &mut NativeTokenBuffer,
    session: &TokenSession,
    logical: LogicalToken,
    edition: Edition,
) -> Result<(), ParseError> {
    match logical {
        LogicalToken::Leaf(token) => {
            token.validate(session).map_err(transport_error)?;
            match token.kind {
                TokenKind::Ident(ident) => {
                    let text = session.symbol(ident.symbol).map_err(transport_error)?;
                    if ident.raw || !text.is_ascii() {
                        return Err(failure(
                            session,
                            token.span,
                            "raw and non-ASCII identifiers are unsupported",
                        ));
                    }
                    output.push_expanded(
                        session,
                        text,
                        ParserTokenKind::Ident,
                        &[part(session, token.span)?],
                        Some((token, edition)),
                    )
                }
                TokenKind::Literal {
                    kind: LiteralKind::Integer | LiteralKind::Float,
                    spelling,
                } => {
                    if matches!(
                        token.kind,
                        TokenKind::Literal {
                            kind: LiteralKind::Float,
                            ..
                        }
                    ) && output.last().is_some_and(|previous| {
                        previous.kind == ParserTokenKind::Punct && previous.text == "."
                    }) {
                        return Err(failure(
                            session,
                            token.span,
                            "expanded tuple float projections require constituent source metadata",
                        ));
                    }
                    let text = session.symbol(spelling).map_err(transport_error)?;
                    output.push_expanded(
                        session,
                        text,
                        ParserTokenKind::Number,
                        &[part(session, token.span)?],
                        None,
                    )
                }
                TokenKind::Literal {
                    kind: kind @ (LiteralKind::Char | LiteralKind::Byte),
                    spelling,
                } => {
                    let text = session.symbol(spelling).map_err(transport_error)?;
                    output.push_expanded(
                        session,
                        text,
                        if kind == LiteralKind::Byte {
                            ParserTokenKind::Byte
                        } else {
                            ParserTokenKind::Char
                        },
                        &[part(session, token.span)?],
                        None,
                    )
                }
                TokenKind::Literal {
                    kind: LiteralKind::String | LiteralKind::RawString,
                    spelling,
                } => {
                    let text = session.symbol(spelling).map_err(transport_error)?;
                    session
                        .budget()
                        .charge(Counter::Work, text.len() as u64)
                        .map_err(|error| transport_error(error.into()))?;
                    let decoded =
                        crate::compiler::lexer::decode_text_string(text, session.budget())
                            .map_err(native_adapter::lex_error)?;
                    output.push_expanded(
                        session,
                        decoded.as_str(),
                        ParserTokenKind::String,
                        &[part(session, token.span)?],
                        None,
                    )
                }
                TokenKind::Literal {
                    kind: LiteralKind::ByteString | LiteralKind::RawByteString,
                    spelling,
                } => {
                    let text = session.symbol(spelling).map_err(transport_error)?;
                    output.push_expanded(
                        session,
                        text,
                        ParserTokenKind::ByteString,
                        &[part(session, token.span)?],
                        None,
                    )
                }
                TokenKind::Literal { .. } => Err(failure(
                    session,
                    token.span,
                    "C string literals are unsupported",
                )),
                TokenKind::Opaque(_) => {
                    output.push_opaque_literal(session, token, part(session, token.span)?)
                }
                TokenKind::DollarCrate(_) => Err(failure(
                    session,
                    token.span,
                    "$crate is unsupported by the expression parser",
                )),
                TokenKind::Punct { ch, .. } => {
                    let mut bytes = [0; 4];
                    let text = ch.encode_utf8(&mut bytes);
                    push_punctuation(output, session, text, &[part(session, token.span)?])
                }
            }
        }
        LogicalToken::Operator { chars, spans, len } => {
            let len = usize::from(len);
            let mut bytes = [0; 3];
            let mut parts: [Option<TokenPart>; 3] = std::array::from_fn(|_| None);
            if len == 0 || len > bytes.len() {
                return Err(transport_error(TokenError::InvalidRange));
            }
            for index in 0..len {
                bytes[index] = u8::try_from(chars[index])
                    .map_err(|_| transport_error(TokenError::InvalidRange))?;
                let span =
                    spans[index].ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
                parts[index] = Some(part(session, span)?);
            }
            let text = std::str::from_utf8(&bytes[..len])
                .map_err(|_| transport_error(TokenError::InvalidRange))?;
            let first = parts[0]
                .take()
                .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
            let second = parts[1].take().unwrap_or_else(|| first.clone());
            let third = parts[2].take().unwrap_or_else(|| second.clone());
            let parts = [first, second, third];
            push_punctuation(output, session, text, &parts[..len])
        }
        LogicalToken::Lifetime { quote, ident, span } => {
            lifetime::push(output, session, quote, ident, span, edition)
        }
        LogicalToken::Group { .. } => Err(transport_error(TokenError::InvalidInput(
            "group must be lowered by the iterative adapter",
        ))),
    }
}

fn push_punctuation(
    output: &mut NativeTokenBuffer,
    session: &TokenSession,
    text: &str,
    parts: &[TokenPart],
) -> Result<(), ParseError> {
    if text == "<-" {
        return Err(native_adapter::expanded_error(
            session,
            parts[0].span,
            SourceErrorKind::Syntax,
            "unexpected token: <-",
        ));
    }
    if !native_adapter::punctuation(text) {
        if output.current_macro_slice().is_some() {
            return output.push_expanded(
                session,
                text,
                ParserTokenKind::DeferredUnsupported,
                parts,
                None,
            );
        }
        return Err(failure(
            session,
            parts[0].span,
            "unsupported expanded punctuation",
        ));
    }
    output.push_expanded(session, text, ParserTokenKind::Punct, parts, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::macro_rules::{CallSite, DefinitionSite};
    use crate::compiler::semantic_scope::ScopeArenaBuilder;
    use crate::compiler::source::PreparedLexInput;
    use crate::compiler::token_tree::{GroupPolicy, group};
    use std::path::Path;

    struct Fixture {
        names: NameStoreBuilder,
        tokens: TokenSlice,
        eof: SpanId,
        original_eof: SpanId,
        admission: ExpansionAdmission,
        _scopes: ScopeArenaBuilder,
    }
    struct ItemFixture {
        names: NameStoreBuilder,
        modules: crate::compiler::module_identity::ModuleArenaBuilder,
        tokens: TokenSlice,
        eof: SpanId,
        original_eof: SpanId,
        admission: ItemExpansionAdmission,
        scopes: ScopeArenaBuilder,
    }
    fn item_fixture() -> ItemFixture {
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::identity(
            Path::new("item.rs"),
            "($name:ident,$x:literal)=>{fn $name()->i32{$x}}; (generated,7)",
            &budget,
        )
        .unwrap();
        let (mut session, session_cap) =
            TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
        let lexed =
            crate::compiler::lexer::lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let original_eof = lexed.eof().span;
        let definition = lexed.events()[0].token.span();
        let mut events = BudgetVec::new(&budget, Storage::Temporary).unwrap();
        for event in lexed.events() {
            events.push(event.token).unwrap();
        }
        let stream = group(&mut session, events.as_slice(), GroupPolicy::OriginalSource).unwrap();
        let root = TokenSlice::roots(&session, stream).unwrap();
        let last = root.len(&session).unwrap() - 1;
        let Node::Delimited { open: call, .. } = root.node(&session, last).unwrap() else {
            panic!("expected invocation group")
        };
        let call = *call;
        let args = root.children(&session, last).unwrap().unwrap();
        let body = root.subslice(&session, 0..last).unwrap();
        let file = session.sources().file(0).unwrap();
        let mut names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let krate = names.register_crate_root(file).unwrap();
        let rules = names
            .compile_local_rules(
                body,
                DefinitionSite {
                    span: definition,
                    edition: Edition::E2024,
                    defining_crate: krate,
                },
            )
            .unwrap();
        let expanded = names
            .expand_local_rules(
                &rules,
                args,
                CallSite {
                    span: call,
                    parent: None,
                },
            )
            .unwrap();
        let expansion = expanded.expansion(names.session()).unwrap();
        let tokens = expanded.tokens(names.session()).unwrap();
        let modules = crate::compiler::module_identity::ModuleArenaBuilder::for_source(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        let module = modules.source_root().unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let frontier = scopes.module_root(&names, &modules, module, None).unwrap();
        let admission = scopes
            .anchor_item_expansion(&names, &modules, expansion, frontier, frontier)
            .unwrap();
        let eof = names.item_expansion_eof(&admission).unwrap();
        ItemFixture {
            names,
            modules,
            tokens,
            eof,
            original_eof,
            admission,
            scopes,
        }
    }

    #[test]
    fn item_transport_and_literal_consumption_use_real_module_proof_without_function_authority() {
        let mut source = item_fixture();
        let buffer = lower_item(
            source.tokens.clone(),
            &mut source.names,
            &source.modules,
            Edition::E2024,
            source.eof,
            &source.admission,
        )
        .unwrap();
        assert!(buffer.current_macro_slice().is_some());
        let value = buffer
            .iter()
            .position(|token| token.kind == ParserTokenKind::OpaqueLiteral)
            .unwrap();
        let view = super::super::TokenCursor::new(&buffer).peek(value).unwrap();
        let isolated = buffer
            .opaque_literal_buffer(&view, &mut source.names)
            .unwrap();
        assert_eq!(isolated[0].text, "7");
        assert!(isolated.current_macro_slice().is_none());
        source
            .admission
            .validate_span(source.names.session(), isolated[0].whole.span)
            .unwrap();
        let index = buffer
            .iter()
            .position(|token| token.text == "generated")
            .unwrap();
        let name_view = super::super::TokenCursor::new(&buffer).peek(index).unwrap();
        let declaration = buffer.mint_name(&name_view, &mut source.names).unwrap();
        assert_eq!(source.names.data(declaration).unwrap().span, name_view.span);
        // Only this explicit product association issues real function authority.
        let root = source
            .scopes
            .generated_function_root(
                &source.names,
                &source.modules,
                &source.admission,
                declaration,
            )
            .unwrap();
        let function = source.scopes.function(root).unwrap();
        source
            .scopes
            .seal_item_product(
                &source.names,
                &source.modules,
                &source.admission,
                function,
                declaration,
            )
            .unwrap();
        source
            .scopes
            .check_generated_function(
                &source.names,
                &source.modules,
                function,
                declaration,
                source.modules.source_root().unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn empty_item_transport_checks_exact_module_owner_and_actual_expansion_eof() {
        let mut source = item_fixture();
        let empty = source
            .tokens
            .subslice(source.names.session(), 0..0)
            .unwrap();
        let foreign_modules = crate::compiler::module_identity::ModuleArenaBuilder::for_source(
            &source.names,
            crate::compiler::owner_path::standalone_issued(
                &source.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            lower_item(
                empty.clone(),
                &mut source.names,
                &foreign_modules,
                Edition::E2024,
                source.eof,
                &source.admission
            )
            .unwrap_err()
            .kind,
            SourceErrorKind::Contract
        );
        assert_eq!(
            lower_item(
                empty.clone(),
                &mut source.names,
                &source.modules,
                Edition::E2024,
                source.original_eof,
                &source.admission
            )
            .unwrap_err()
            .kind,
            SourceErrorKind::Contract
        );
        let buffer = lower_item(
            empty,
            &mut source.names,
            &source.modules,
            Edition::E2024,
            source.eof,
            &source.admission,
        )
        .unwrap();
        assert_eq!(buffer.len(), 1);
        assert!(buffer.current_macro_slice().is_some());
    }

    #[test]
    fn expansion_proof_singleton_capacity_is_retained_until_buffer_drop() {
        let source = fixture("1");
        let budget = source.names.budget().clone();
        let before = budget.snapshot();
        let buffer =
            NativeTokenBuffer::expanded(source.names.session(), &source.admission).unwrap();
        let live = budget.snapshot();
        assert!(
            live.retained_bytes
                >= before.retained_bytes + std::mem::size_of::<ExpansionTokenAdmission>() as u64
        );
        assert_eq!(live.temporary_bytes, before.temporary_bytes);
        drop(buffer);
        assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }
    fn fixture_rules(rules: &str, args: &str) -> Fixture {
        let budget = ExpansionBudget::default();
        let text = format!("{rules} {args}");
        let input = PreparedLexInput::identity(Path::new("expanded.rs"), &text, &budget).unwrap();
        let (mut session, session_cap) =
            TokenSession::standalone(input.snapshot().clone(), budget.clone()).unwrap();
        let lexed =
            crate::compiler::lexer::lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let original_eof = lexed.eof().span;
        let definition = lexed.events()[0].token.span();
        let mut events = BudgetVec::new(&budget, Storage::Temporary).unwrap();
        for event in lexed.events() {
            events.push(event.token).unwrap();
        }
        let stream = group(&mut session, events.as_slice(), GroupPolicy::OriginalSource).unwrap();
        let root = TokenSlice::roots(&session, stream).unwrap();
        let last = root.len(&session).unwrap() - 1;
        let Node::Delimited { open: call, .. } = root.node(&session, last).unwrap() else {
            panic!("invocation argument group expected")
        };
        let call = *call;
        let args = root.children(&session, last).unwrap().unwrap();
        let body = root.subslice(&session, 0..last).unwrap();
        let file = session.sources().file(0).unwrap();
        let mut names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let defining_crate = names.register_crate_root(file).unwrap();
        let rules = names
            .compile_local_rules(
                body,
                DefinitionSite {
                    span: definition,
                    edition: Edition::E2024,
                    defining_crate,
                },
            )
            .unwrap();
        let expanded = names
            .expand_local_rules(
                &rules,
                args,
                CallSite {
                    span: call,
                    parent: None,
                },
            )
            .unwrap();
        let expansion = expanded.expansion(names.session()).unwrap();
        let tokens = expanded.tokens(names.session()).unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let frontier = scopes.function_root(&names).unwrap();
        let admission = scopes
            .anchor_expansion(&names, expansion, frontier, frontier)
            .unwrap();
        let eof = names.expansion_eof(&admission).unwrap();
        Fixture {
            names,
            tokens,
            eof,
            original_eof,
            admission,
            _scopes: scopes,
        }
    }
    fn fixture(output: &str) -> Fixture {
        fixture_rules(&format!("()=>{{{output}}};"), "()")
    }
    fn run(fixture: &mut Fixture) -> Result<NativeTokenBuffer, ParseError> {
        lower(
            fixture.tokens.clone(),
            &mut fixture.names,
            Edition::E2024,
            fixture.eof,
            &fixture.admission,
        )
    }

    #[test]
    fn joint_less_minus_is_syntax_while_spaced_tokens_remain_distinct() {
        let mut joint = fixture("x<-1");
        let error = run(&mut joint).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Syntax);
        assert!(error.message.contains("unexpected token: <-"));
        assert!(error.primary.is_some());
        assert!(!error.has_location);
        let mut spaced = fixture("x < -1");
        let buffer = run(&mut spaced).unwrap();
        assert_eq!(
            buffer
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<_>>(),
            ["x", "<", "-", "1", ""]
        );

        let joint = "fn main(){let x=0;macro_rules! compare{()=>{x<-1};}let _=compare!();}";
        let error = super::super::parse_syntax_unprepared(joint).err().unwrap();
        assert_eq!(error.kind, SourceErrorKind::Syntax);
        super::super::parse_syntax_unprepared(
            "fn main(){let x=0;macro_rules! compare{()=>{x < -1};}let _=compare!();}",
        )
        .unwrap();
    }

    #[test]
    fn opaque_literals_use_checked_isolated_buffers_without_changing_matcher_tokens() {
        for (argument, expected) in [
            ("(1)", vec!["1", ""]),
            ("(-1)", vec!["-", "1", ""]),
            ("(1.25)", vec!["1.25", ""]),
            ("(true)", vec!["true", ""]),
            ("(-false)", vec!["-", "false", ""]),
            ("(\"hello\\nworld\")", vec!["hello\nworld", ""]),
        ] {
            let mut source = fixture_rules("($x:literal)=>{$x};", argument);
            let buffer = run(&mut source).unwrap();
            assert_eq!(buffer.len(), 2);
            let mut cursor = super::super::TokenCursor::new(&buffer);
            let view = cursor.current();
            assert!(view.is_opaque_literal());
            assert!(!view.is_ident());
            assert!(!view.matches(view.text));
            assert!(!cursor.consume_type_gt());
            assert!(!cursor.consume_type_amp_prefix());
            assert_eq!(
                buffer.mint_name(&view, &mut source.names).unwrap_err().kind,
                SourceErrorKind::Contract
            );
            let isolated = buffer
                .opaque_literal_buffer(&view, &mut source.names)
                .unwrap();
            assert!(buffer.current_macro_slice().is_some());
            assert!(isolated.current_macro_slice().is_none());
            assert_eq!(
                isolated
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            let Node::Leaf(Token {
                kind: TokenKind::Opaque(_),
                span,
            }) = source.tokens.node(source.names.session(), 0).unwrap()
            else {
                panic!("semantic consumption must not replace the matcher fragment")
            };
            assert_eq!(*span, view.span);
            cursor.bump();
            assert!(cursor.current().matches(""));
        }
    }

    #[test]
    fn current_macro_slice_is_exact_engine_output_not_underlying_siblings() {
        let mut source = fixture("left!(1) + right!(2)");
        let session = source.names.session();
        assert_eq!(source.tokens.len(session).unwrap(), 7);
        let Node::Delimited { open: right, .. } = source.tokens.node(session, 6).unwrap() else {
            panic!("expected right invocation group")
        };
        let right = *right;
        let selected = source.tokens.subslice(session, 0..3).unwrap();
        let buffer = lower(
            selected.clone(),
            &mut source.names,
            Edition::E2024,
            source.eof,
            &source.admission,
        )
        .unwrap();
        let actual = buffer.current_macro_slice().unwrap();
        let session = source.names.session();
        assert_eq!(actual.len(session).unwrap(), 3);
        for index in 0..3 {
            assert!(std::ptr::eq(
                actual.node(session, index).unwrap(),
                selected.node(session, index).unwrap()
            ));
            if let Node::Delimited { open, .. } = actual.node(session, index).unwrap() {
                assert_ne!(*open, right);
            }
        }
        let Node::Delimited { open, .. } = actual.node(session, 2).unwrap() else {
            panic!("expected retained invocation group")
        };
        let view = super::super::TokenCursor::new(&buffer).peek(2).unwrap();
        assert_eq!(view.span, *open);
        let args = actual.children(session, 2).unwrap().unwrap();
        assert_eq!(args.len(session).unwrap(), 1);
        let Node::Leaf(token) = args.node(session, 0).unwrap() else {
            panic!("expected actual argument leaf")
        };
        let TokenKind::Literal { spelling, .. } = token.kind else {
            panic!("expected literal argument")
        };
        assert_eq!(session.symbol(spelling).unwrap(), "1");
    }

    #[test]
    fn deferred_punctuation_keeps_actual_matcher_tokens_but_not_literal_inventory() {
        let mut source = fixture("@ # ? ~");
        let buffer = run(&mut source).unwrap();
        let actual = buffer.current_macro_slice().unwrap();
        let session = source.names.session();
        for (index, ch) in ['@', '#', '?', '~'].into_iter().enumerate() {
            let view = super::super::TokenCursor::new(&buffer).peek(index).unwrap();
            assert_eq!(view.is_deferred_unsupported(), matches!(ch, '#' | '~'));
            assert_eq!(view.matches(view.text), matches!(ch, '@' | '?'));
            assert!(!view.is_ident());
            assert!(!view.string);
            assert!(matches!(actual.node(session, index).unwrap(),
                Node::Leaf(Token { kind: TokenKind::Punct { ch: actual, .. }, span })
                    if *actual == ch && *span == view.span));
        }
        let mut isolated = NativeTokenBuffer::expanded(session, &source.admission).unwrap();
        assert!(isolated.current_macro_slice().is_none());
        let span = buffer[0].whole.span;
        push_punctuation(&mut isolated, session, "@", &[part(session, span).unwrap()]).unwrap();
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated[0].kind, ParserTokenKind::Punct);
        let error = push_punctuation(&mut isolated, session, "#", &[part(session, span).unwrap()])
            .unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported);
        assert_eq!(isolated.len(), 1);
    }

    #[test]
    fn macro_slice_installation_rejects_foreign_or_unadmitted_payload_and_replacement() {
        let source = fixture_rules("($x:literal)=>{$x};", "(1)");
        let session = source.names.session();
        let Node::Leaf(Token {
            kind: TokenKind::Opaque(id),
            ..
        }) = source.tokens.node(session, 0).unwrap()
        else {
            panic!("expected actual opaque output")
        };
        let original_payload = session.fragment(*id).unwrap().tokens().clone();
        let mut buffer = NativeTokenBuffer::expanded(session, &source.admission).unwrap();
        assert_eq!(
            buffer
                .retain_macro_slice(session, original_payload)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        assert!(buffer.current_macro_slice().is_none());
        let foreign = fixture("1");
        assert_eq!(
            buffer
                .retain_macro_slice(session, foreign.tokens.clone())
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        assert!(buffer.current_macro_slice().is_none());
        buffer
            .retain_macro_slice(session, source.tokens.clone())
            .unwrap();
        assert_eq!(
            buffer
                .retain_macro_slice(session, source.tokens.clone())
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let actual = buffer.current_macro_slice().unwrap();
        assert!(std::ptr::eq(
            actual.node(session, 0).unwrap(),
            source.tokens.node(session, 0).unwrap()
        ));
        assert!(matches!(actual.node(session, 0).unwrap(),
            Node::Leaf(Token { kind: TokenKind::Opaque(actual), .. }) if actual == id));
    }

    #[test]
    fn opaque_payload_parts_keep_original_occurrences_and_current_expansion() {
        let mut source = fixture_rules("($x:literal)=>{$x};", "(-17)");
        let Node::Leaf(outer) = source.tokens.node(source.names.session(), 0).unwrap() else {
            panic!("expected opaque leaf")
        };
        let TokenKind::Opaque(id) = outer.kind else {
            panic!("expected opaque fragment")
        };
        let payload = source.names.session().fragment(id).unwrap().tokens();
        let original = (0..2)
            .map(|index| {
                let Node::Leaf(token) = payload.node(source.names.session(), index).unwrap() else {
                    panic!("expected literal constituent")
                };
                (
                    token.span,
                    source.names.session().original(token.span, 512).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let expansion = source.names.session().span(outer.span).unwrap().expansion;
        let buffer = run(&mut source).unwrap();
        let view = super::super::TokenCursor::new(&buffer).current();
        let isolated = buffer
            .opaque_literal_buffer(&view, &mut source.names)
            .unwrap();
        for (token, (previous, original)) in isolated.iter().zip(original) {
            assert_ne!(token.whole.span, previous);
            assert_eq!(
                source
                    .names
                    .session()
                    .original(token.whole.span, 512)
                    .unwrap(),
                original
            );
            assert_eq!(
                source
                    .names
                    .session()
                    .span(token.whole.span)
                    .unwrap()
                    .expansion,
                expansion
            );
            assert_eq!(
                token.whole.provenance(),
                super::super::TokenProvenance::Expanded
            );
            assert_eq!(token.whole.bytes(), 0..0);
        }
    }

    #[test]
    fn opaque_accessor_rejects_foreign_buffers_and_name_owners() {
        let mut source = fixture_rules("($x:literal)=>{$x};", "(1)");
        let buffer = run(&mut source).unwrap();
        let second = run(&mut source).unwrap();
        let foreign = super::super::TokenCursor::new(&second).current();
        assert_eq!(
            buffer
                .opaque_literal_buffer(&foreign, &mut source.names)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let view = super::super::TokenCursor::new(&buffer).current();
        let mut other = fixture("1");
        assert_eq!(
            buffer
                .opaque_literal_buffer(&view, &mut other.names)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let mut forged = view.clone();
        forged.name = None;
        assert_eq!(
            buffer
                .opaque_literal_buffer(&forged, &mut source.names)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let mut forged = view.clone();
        forged.span = source.eof;
        assert_eq!(
            buffer
                .opaque_literal_buffer(&forged, &mut source.names)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        buffer
            .opaque_literal_buffer(&view, &mut source.names)
            .unwrap();
    }

    #[test]
    fn isolated_literal_capacity_is_owned_until_buffer_drop_and_richer_classes_stay_unsupported() {
        let mut source = fixture_rules("($x:literal)=>{$x};", "(\"retained payload\")");
        let budget = source.names.budget().clone();
        let buffer = run(&mut source).unwrap();
        let view = super::super::TokenCursor::new(&buffer).current();
        let isolated = buffer
            .opaque_literal_buffer(&view, &mut source.names)
            .unwrap();
        let live = budget.snapshot();
        drop(isolated);
        let released = budget.snapshot();
        assert!(released.retained_bytes < live.retained_bytes);
        assert_eq!(released.temporary_bytes, live.temporary_bytes);
        {
            let argument = "(c\"C\")";
            let mut source = fixture_rules("($x:literal)=>{$x};", argument);
            let buffer = run(&mut source).unwrap();
            let view = super::super::TokenCursor::new(&buffer).current();
            let error = buffer
                .opaque_literal_buffer(&view, &mut source.names)
                .unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Unsupported, "{argument}");
            assert!(error.primary.is_some());
            assert!(!error.has_location);
        }
    }

    #[test]
    fn normal_float_string_and_nested_groups_keep_native_literal_kinds() {
        let mut source = fixture("((1.25 + 2))");
        let buffer = run(&mut source).unwrap();
        assert_eq!(
            buffer
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<_>>(),
            ["(", "(", "1.25", "+", "2", ")", ")", ""]
        );
        assert_eq!(buffer[2].kind, ParserTokenKind::Number);
        assert!(
            buffer
                .iter()
                .all(|token| token.whole.provenance() == super::super::TokenProvenance::Expanded)
        );
        let mut source = fixture(r#""ok\n""#);
        let buffer = run(&mut source).unwrap();
        assert_eq!(buffer[0].text, "ok\n");
        assert_eq!(buffer[0].kind, ParserTokenKind::String);
        let mut source = fixture("'水'");
        let buffer = run(&mut source).unwrap();
        assert_eq!(buffer[0].text, "'水'");
        assert_eq!(buffer[0].kind, ParserTokenKind::Char);
        assert_eq!(
            buffer[0].whole.provenance(),
            super::super::TokenProvenance::Expanded
        );
        assert!(buffer[0].name.is_none());
    }

    #[test]
    fn byte_literals_keep_spelling_kind_and_opaque_literal_forwarding() {
        let mut source = fixture("b'x'");
        let buffer = run(&mut source).unwrap();
        assert_eq!(buffer[0].kind, ParserTokenKind::Byte);
        assert_eq!(buffer[0].text, "b'x'");
        assert!(buffer[0].name.is_none());
        let mut source = fixture_rules("($x:literal)=>{$x};", "(b'\\xff')");
        let buffer = run(&mut source).unwrap();
        let view = super::super::TokenCursor::new(&buffer).current();
        let isolated = buffer
            .opaque_literal_buffer(&view, &mut source.names)
            .unwrap();
        assert_eq!(isolated[0].kind, ParserTokenKind::Byte);
        assert_eq!(isolated[0].text, "b'\\xff'");
    }

    #[test]
    fn unsupported_expanded_namespaces_literals_and_tuple_metadata_are_explicit() {
        let output = "(1,2).0.1";
        {
            let mut source = fixture(output);
            let error = run(&mut source).unwrap_err();
            assert_eq!(
                error.kind,
                SourceErrorKind::Unsupported,
                "{output}: {error}"
            );
            assert!(error.primary.is_some());
            assert!(!error.has_location);
        }
        let mut source = fixture("'a");
        let buffer = run(&mut source).unwrap();
        let token = super::super::TokenCursor::new(&buffer).current();
        assert!(token.is_lifetime());
        assert_eq!(token.text, "'a");
        assert!(token.name.is_some());
        assert_eq!(token.provenance, super::super::TokenProvenance::Expanded);
        assert_eq!(token.bytes, 0..0);
        assert_eq!(buffer[0].constituents.iter().flatten().count(), 2);
        for span in buffer[0].constituents.iter().flatten() {
            source.names.session().original(*span, 512).unwrap();
        }
    }

    #[test]
    fn expanded_suffixes_select_actual_constituent_spans_and_restore_exactly() {
        let mut source = fixture(">>= &&");
        let buffer = run(&mut source).unwrap();
        let spans = buffer[0].constituents;
        let mut cursor = super::super::TokenCursor::new(&buffer);
        let start = cursor.checkpoint();
        assert!(cursor.consume_type_gt());
        let suffix = cursor.current();
        assert_eq!(suffix.text, ">=");
        assert_eq!(suffix.span, spans[1].unwrap());
        assert_eq!(suffix.bytes, 0..0);
        assert_eq!(suffix.provenance, super::super::TokenProvenance::Expanded);
        let original = source.names.session().original(suffix.span, 512).unwrap();
        let (_, line, column) = source.names.session().sources().location(original).unwrap();
        assert_eq!((suffix.line, suffix.column), (line, column));
        assert!(cursor.consume_type_gt());
        assert_eq!(cursor.current().span, spans[2].unwrap());
        assert!(cursor.restore(start));
        assert_eq!(cursor.current().text, ">>=");
        cursor.bump();
        assert!(cursor.consume_type_amp_prefix());
        assert_eq!(cursor.current().span, buffer[1].constituents[1].unwrap());
    }

    #[test]
    fn same_session_unrelated_eof_and_foreign_proofs_reject_even_empty_output() {
        let mut source = fixture("");
        let error = lower(
            source.tokens.clone(),
            &mut source.names,
            Edition::E2024,
            source.original_eof,
            &source.admission,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Contract);
        let other = fixture("");
        let error = lower(
            source.tokens.clone(),
            &mut source.names,
            Edition::E2024,
            source.eof,
            &other.admission,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Contract);
        let buffer = run(&mut source).unwrap();
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer[0].kind, ParserTokenKind::Eof);
    }

    #[test]
    fn captured_joint_operator_suffix_can_precede_its_whole_in_original_source() {
        let mut source = fixture_rules("($a:tt $discard:tt)=>{$a=};", "(>>+)");
        let buffer = run(&mut source).unwrap();
        assert_eq!(buffer[0].text, ">>=");
        let mut cursor = super::super::TokenCursor::new(&buffer);
        let whole = cursor.current();
        assert!(cursor.consume_type_gt());
        assert!(cursor.consume_type_gt());
        let suffix = cursor.current();
        assert_eq!(suffix.text, "=");
        let whole_source = source.names.session().original(whole.span, 512).unwrap();
        let suffix_source = source.names.session().original(suffix.span, 512).unwrap();
        let whole_source = source
            .names
            .session()
            .sources()
            .render_span(whole_source)
            .unwrap();
        let suffix_source = source
            .names
            .session()
            .sources()
            .render_span(suffix_source)
            .unwrap();
        assert!(suffix_source.lo < whole_source.lo);
        assert_eq!(suffix.span, buffer[0].constituents[2].unwrap());
        assert_ne!(suffix.column, whole.column + 2);
        assert_eq!(whole.bytes, 0..0);
        assert_eq!(suffix.bytes, 0..0);
    }

    #[test]
    fn minted_expanded_identifier_keeps_original_context_and_checked_buffer_owner() {
        let mut source = fixture("local");
        let buffer = run(&mut source).unwrap();
        let view = super::super::TokenCursor::new(&buffer).current();
        let ident = buffer.mint_name(&view, &mut source.names).unwrap();
        let data = source.names.data(ident).unwrap();
        assert_eq!(data.span, view.span);
        assert_ne!(data.context, source.names.session().root_context());
        let mut other = fixture("local");
        assert_eq!(
            buffer.mint_name(&view, &mut other.names).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        let other_buffer = run(&mut other).unwrap();
        let other_view = super::super::TokenCursor::new(&other_buffer).current();
        assert_eq!(
            buffer
                .mint_name(&other_view, &mut source.names)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
    }

    #[test]
    fn expanded_diagnostic_owns_actual_capacity_through_final_error_transfer() {
        let source = fixture("1");
        let budget = source.names.budget().clone();
        let before = budget.snapshot();
        let error = native_adapter::expanded_error(
            source.names.session(),
            source.eof,
            SourceErrorKind::Syntax,
            "expected expression",
        );
        assert!(error.message.contains("expanded at file"));
        let capacity = error.message.capacity() as u64;
        assert_eq!(
            budget.snapshot().retained_bytes,
            before.retained_bytes + capacity
        );
        assert_eq!(
            budget.snapshot().counters[Counter::DiagnosticBytes as usize],
            before.counters[Counter::DiagnosticBytes as usize] + capacity
        );
        let mut final_error =
            crate::compiler::source::SourceError::syntax(error.primary, error.message);
        final_error._diagnostic_lease = error._diagnostic_lease;
        assert_eq!(
            budget.snapshot().retained_bytes,
            before.retained_bytes + capacity
        );
        assert!(final_error.to_string().contains("expanded at file"));
        drop(final_error);
        assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }

    #[test]
    fn diagnostic_exhaustion_stays_typed_and_drops_partial_message_storage() {
        let source = fixture("1");
        let budget = source.names.budget().clone();
        budget
            .charge(
                Counter::DiagnosticBytes,
                budget.limits().counters[Counter::DiagnosticBytes as usize],
            )
            .unwrap();
        let before = budget.snapshot();
        let error = native_adapter::expanded_error(
            source.names.session(),
            source.eof,
            SourceErrorKind::Syntax,
            "expected expression",
        );
        assert_eq!(error.kind, SourceErrorKind::Limit);
        assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    }
}
