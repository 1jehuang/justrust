//! Parser-only consumption of opaque literal captures. Matcher input stays opaque.
use super::*;
use crate::compiler::macro_fragments::FragmentKind;
use crate::compiler::semantic_scope::{ExpansionAdmission, ScopeError};
use crate::compiler::token_tree::Node;

#[derive(Debug)]
pub(crate) struct LiteralParts {
    pub(crate) minus: Option<Token>,
    pub(crate) value: Token,
    pub(crate) outer_occurrence: SpanId,
}

#[derive(Debug)]
pub(crate) enum LiteralError {
    Token(TokenError),
    Scope(ScopeError),
    Unsupported { span: SpanId },
}
impl From<TokenError> for LiteralError {
    fn from(value: TokenError) -> Self {
        Self::Token(value)
    }
}
impl From<ScopeError> for LiteralError {
    fn from(value: ScopeError) -> Self {
        Self::Scope(value)
    }
}
impl fmt::Display for LiteralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(error) => error.fmt(f),
            Self::Scope(error) => error.fmt(f),
            Self::Unsupported { .. } => f.write_str("unsupported opaque literal payload"),
        }
    }
}
impl std::error::Error for LiteralError {}

fn invalid() -> LiteralError {
    TokenError::InvalidInput("malformed opaque literal payload").into()
}

impl NameStoreBuilder {
    /// Consume a grammar fragment only at the compiler's isolated expression
    /// boundary. These leaves must never replace the opaque matcher occurrence.
    pub(crate) fn literal_parts(
        &mut self,
        opaque: Token,
        proof: &ExpansionAdmission,
    ) -> Result<LiteralParts, LiteralError> {
        proof.validate_token(self.session(), opaque)?;
        let TokenKind::Opaque(mut id) = opaque.kind else {
            return Err(invalid());
        };
        let context = self.session().span(opaque.span)?.context;
        let expansion = proof.expansion(self)?;
        // Issuance retains only already existing fragment IDs. The fixed arena
        // count is also a fail-closed bound if a future producer breaks that DAG.
        let mut remaining = self.session().fragments.len();
        let mut minus = None;
        let mut value = loop {
            self.budget()
                .charge(Counter::Work, 1)
                .map_err(TokenError::from)?;
            remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
            let payload = self.session().fragment(id)?;
            if payload.kind() != FragmentKind::Literal {
                return Err(LiteralError::Unsupported { span: opaque.span });
            }
            let tokens = payload.tokens();
            let len = tokens.len(self.session())?;
            if !(1..=2).contains(&len) {
                return Err(invalid());
            }
            self.budget()
                .charge(Counter::Work, len as u64)
                .map_err(TokenError::from)?;
            let mut position = 0;
            let Node::Leaf(first) = tokens.node(self.session(), 0)? else {
                return Err(invalid());
            };
            first.validate(self.session())?;
            if matches!(first.kind, TokenKind::Punct { ch: '-', .. }) {
                if minus.is_some() || len != 2 {
                    return Err(invalid());
                }
                minus = Some(*first);
                position = 1;
            } else if len != 1 {
                return Err(invalid());
            }
            let Node::Leaf(terminal) = tokens.node(self.session(), position)? else {
                return Err(invalid());
            };
            terminal.validate(self.session())?;
            match terminal.kind {
                TokenKind::Opaque(next) => id = next,
                TokenKind::Literal { .. } => break *terminal,
                TokenKind::Ident(ident)
                    if !ident.raw
                        && matches!(self.session().symbol(ident.symbol)?, "true" | "false") =>
                {
                    break *terminal;
                }
                _ => return Err(invalid()),
            }
        };
        // No payload borrows survive arena mutation. Each real constituent gets
        // its own original anchor plus the admitted outer expansion call trace.
        value.span = self
            .store
            .session
            .reissue_span(value.span, context, expansion)?;
        if let TokenKind::Ident(ref mut ident) = value.kind {
            ident.context = context;
        }
        if let Some(ref mut token) = minus {
            token.span = self
                .store
                .session
                .reissue_span(token.span, context, expansion)?;
            proof.validate_token(self.session(), *token)?;
        }
        proof.validate_token(self.session(), value)?;
        Ok(LiteralParts {
            minus,
            value,
            outer_occurrence: opaque.span,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_span::{ExpansionRecord, SpanOrigin, SyntheticReason};
    use crate::compiler::hygiene::Transparency;
    use crate::compiler::macro_cursor::TokenSlice;
    use crate::compiler::macro_fragments::LiteralCapture;
    use crate::compiler::semantic_scope::ScopeArenaBuilder;
    use crate::compiler::token::{Ident, InputToken, LiteralKind, Spacing};
    use crate::compiler::token_session::SourceSnapshot;
    use crate::compiler::token_tree::{GroupPolicy, group};
    use std::path::Path;

    fn capture(session: &mut TokenSession, tokens: &[Token]) -> Token {
        let input: Vec<_> = tokens.iter().copied().map(InputToken::Leaf).collect();
        let tree = group(session, &input, GroupPolicy::OriginalSource).unwrap();
        let slice = TokenSlice::roots(session, tree).unwrap();
        let LiteralCapture::Matched(id) = session.capture_literal(slice).unwrap() else {
            panic!("expected literal capture")
        };
        Token {
            kind: TokenKind::Opaque(id),
            span: tokens.last().unwrap().span,
        }
    }

    fn fixture(
        boolean: bool,
        negative: bool,
        forwards: usize,
    ) -> (NameStoreBuilder, ExpansionAdmission, Token, SpanId, SpanId) {
        let budget = ExpansionBudget::default();
        let text = if boolean { "-true" } else { "-1" };
        let sources = SourceSnapshot::from_text(Path::new("literal.rs"), text, &budget).unwrap();
        let file = sources.file(0).unwrap();
        let minus_origin = sources.span(file, 0, 1).unwrap();
        let value_origin = sources
            .span(file, 1, text.len().try_into().unwrap())
            .unwrap();
        let mut session = TokenSession::new(sources, budget).unwrap();
        let minus_span = session.original_span(minus_origin).unwrap();
        let value_span = session.original_span(value_origin).unwrap();
        let symbol = session
            .intern_symbol(if boolean { "true" } else { "1" })
            .unwrap();
        let value = Token {
            kind: if boolean {
                TokenKind::Ident(Ident {
                    symbol,
                    raw: false,
                    context: session.root_context(),
                })
            } else {
                TokenKind::Literal {
                    kind: LiteralKind::Integer,
                    spelling: symbol,
                }
            },
            span: value_span,
        };
        let minus = Token {
            kind: TokenKind::Punct {
                ch: '-',
                spacing: Spacing::Alone,
            },
            span: minus_span,
        };
        let mut opaque = if negative {
            capture(&mut session, &[minus, value])
        } else {
            capture(&mut session, &[value])
        };
        for _ in 0..forwards {
            opaque = capture(&mut session, &[opaque]);
        }
        let defining_crate = session.register_crate_root(file).unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: value_span,
                call: minus_span,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let context = session
            .apply_mark(
                session.root_context(),
                expansion,
                Transparency::SemiTransparent,
            )
            .unwrap();
        opaque.span = session
            .reissue_span(opaque.span, context, expansion)
            .unwrap();
        let names = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
        let frontier = scopes.function_root(&names).unwrap();
        let proof = scopes
            .anchor_expansion(&names, expansion, frontier, frontier)
            .unwrap();
        (names, proof, opaque, minus_span, value_span)
    }

    #[test]
    fn forwarded_negative_retains_each_physical_origin_and_outer_trace() {
        let (mut names, proof, opaque, minus, value) = fixture(false, true, 128);
        let original_id = match opaque.kind {
            TokenKind::Opaque(id) => id,
            _ => unreachable!(),
        };
        let original_len = names
            .session()
            .fragment(original_id)
            .unwrap()
            .tokens()
            .len(names.session())
            .unwrap();
        let parts = names.literal_parts(opaque, &proof).unwrap();
        assert_eq!(parts.outer_occurrence, opaque.span);
        for (token, original) in [(parts.minus.unwrap(), minus), (parts.value, value)] {
            let span = names.session().span(token.span).unwrap();
            assert!(matches!(span.origin, SpanOrigin::Synthetic {
                anchor, reason: SyntheticReason::ExpansionToken,
            } if anchor == original));
            assert_eq!(span.expansion, Some(proof.expansion(&names).unwrap()));
            proof.validate_token(names.session(), token).unwrap();
        }
        // Consumption never replaces/rewrites the fragment used by matching.
        assert_eq!(
            names
                .session()
                .fragment(original_id)
                .unwrap()
                .tokens()
                .len(names.session())
                .unwrap(),
            original_len
        );
    }

    #[test]
    fn boolean_is_not_reparsed_and_receives_outer_validated_context() {
        let (mut names, proof, opaque, _, _) = fixture(true, false, 2);
        let parts = names.literal_parts(opaque, &proof).unwrap();
        assert!(parts.minus.is_none());
        let TokenKind::Ident(ident) = parts.value.kind else {
            panic!("expected bool token")
        };
        assert_eq!(names.session().symbol(ident.symbol).unwrap(), "true");
        assert_eq!(
            ident.context,
            names.session().span(opaque.span).unwrap().context
        );
    }

    #[test]
    fn foreign_fragment_foreign_proof_and_nonopaque_input_reject() {
        let (mut names, proof, opaque, _, _) = fixture(false, false, 0);
        let (_, other_proof, other, _, _) = fixture(false, false, 0);
        let foreign = Token {
            kind: other.kind,
            span: opaque.span,
        };
        assert!(matches!(
            names.literal_parts(foreign, &proof),
            Err(LiteralError::Scope(ScopeError::Token(
                TokenError::ForeignHandle
            )))
        ));
        assert!(names.literal_parts(opaque, &other_proof).is_err());
        let invalid = Token {
            kind: TokenKind::Punct {
                ch: '+',
                spacing: Spacing::Alone,
            },
            span: opaque.span,
        };
        assert!(matches!(
            names.literal_parts(invalid, &proof),
            Err(LiteralError::Token(TokenError::InvalidInput(_)))
        ));
    }

    #[test]
    fn poisoned_work_is_preserved_and_all_retained_storage_drops() {
        let (mut names, proof, opaque, _, _) = fixture(false, true, 64);
        let budget = names.budget().clone();
        let failure = budget.charge(Counter::Work, u64::MAX).unwrap_err();
        let error = names.literal_parts(opaque, &proof).unwrap_err();
        assert!(
            matches!(error, LiteralError::Scope(ScopeError::Token(TokenError::Budget(ref e)))
            if *e == failure)
        );
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn traversal_exhausts_work_before_output_allocation() {
        let (mut names, proof, opaque, _, _) = fixture(false, true, 64);
        let budget = names.budget().clone();
        let used = budget.snapshot().counters[Counter::Work as usize];
        let limit = budget.limits().counters[Counter::Work as usize];
        budget.charge(Counter::Work, limit - used - 4).unwrap();
        let spans = budget.snapshot().counters[Counter::SpanRecords as usize];
        assert!(matches!(
            names.literal_parts(opaque, &proof),
            Err(LiteralError::Token(TokenError::Budget(_)))
                | Err(LiteralError::Scope(ScopeError::Token(TokenError::Budget(
                    _
                ))))
        ));
        assert_eq!(
            budget.snapshot().counters[Counter::SpanRecords as usize],
            spans
        );
        assert!(budget.check().is_err());
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}
