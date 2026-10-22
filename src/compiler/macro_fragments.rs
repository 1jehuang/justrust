//! Flat, session-owned literal fragment identities. No AST or semantic authority.
use super::expansion_budget::Counter;
use super::macro_cursor::TokenSlice;
use super::token::{Token, TokenKind};
use super::token_session::{SessionId, TokenError, TokenSession};
use super::token_tree::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FragmentKind {
    Tt,
    Ident,
    Literal,
    Expr,
    Ty,
    Pat,
    PatParam,
    Path,
    Stmt,
    Block,
    Item,
    Vis,
    Lifetime,
    Meta,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralCapture {
    NoMatch,
    Matched(FragmentId),
    CommittedSyntax {
        primary: super::expansion_span::SpanId,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FragmentId {
    session: SessionId,
    index: u32,
}
pub struct FragmentPayload {
    kind: FragmentKind,
    tokens: TokenSlice,
    weight: u64,
    // Recognizer fact only, not a semantic value. Cached through opaque
    // forwarding to avoid recursive or repeated payload-chain traversal.
    leading_minus: bool,
}
impl FragmentPayload {
    pub fn kind(&self) -> FragmentKind {
        self.kind
    }
    pub fn tokens(&self) -> &TokenSlice {
        &self.tokens
    }
    pub fn physical_weight(&self) -> u64 {
        self.weight
    }
}

impl TokenSession {
    pub fn fragment(&self, id: FragmentId) -> Result<&FragmentPayload, TokenError> {
        self.budget().check()?;
        if id.session != self.id() {
            return Err(TokenError::ForeignHandle);
        }
        self.fragments
            .as_slice()
            .get(id.index as usize)
            .ok_or(TokenError::InvalidHandle)
    }
    /// Engine-only issuance. Every referenced opaque ID is already present in
    /// this flat arena, so retained streams cannot create ownership cycles.
    pub(super) fn capture_literal(
        &mut self,
        tokens: TokenSlice,
    ) -> Result<LiteralCapture, TokenError> {
        self.budget().charge(Counter::Work, 1)?;
        let len = tokens.len(self)?;
        if len == 0 {
            return Ok(LiteralCapture::NoMatch);
        }
        let minus = matches!(
            tokens.node(self, 0)?,
            Node::Leaf(Token {
                kind: TokenKind::Punct { ch: '-', .. },
                ..
            })
        );
        let position = usize::from(minus);
        let mut inherited_minus = false;
        let literal = if position < len {
            match tokens.node(self, position)? {
                Node::Leaf(Token {
                    kind: TokenKind::Literal { .. },
                    ..
                }) => true,
                Node::Leaf(Token {
                    kind: TokenKind::Ident(ident),
                    ..
                }) if !ident.raw => matches!(self.symbol(ident.symbol)?, "true" | "false"),
                Node::Leaf(Token {
                    kind: TokenKind::Opaque(id),
                    ..
                }) => {
                    let payload = self.fragment(*id)?;
                    inherited_minus = payload.leading_minus;
                    payload.kind() == FragmentKind::Literal && !(minus && inherited_minus)
                }
                _ => false,
            }
        } else {
            false
        };
        if minus && !literal {
            let Node::Leaf(token) = tokens.node(self, 0)? else {
                return Err(TokenError::InvalidHandle);
            };
            return Ok(LiteralCapture::CommittedSyntax {
                primary: token.span,
            });
        }
        if !literal || len != position + 1 {
            return Ok(LiteralCapture::NoMatch);
        }
        let weight = tokens.physical_weight(self)?;
        let index = self.budget().checked_u32(self.fragments.len() as u64)?;
        self.fragments.push(FragmentPayload {
            kind: FragmentKind::Literal,
            tokens,
            weight,
            leading_minus: minus || inherited_minus,
        })?;
        Ok(LiteralCapture::Matched(FragmentId {
            session: self.id(),
            index,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::ExpansionBudget;
    use super::super::macro_cursor::{LogicalToken, MacroCursor};
    use super::super::token::{InputToken, LiteralKind, Spacing};
    use super::super::token_session::SourceSnapshot;
    use super::super::token_tree::{GroupPolicy, group};
    use super::*;
    use std::path::Path;

    fn session() -> (TokenSession, super::super::expansion_span::SpanId) {
        let budget = ExpansionBudget::default();
        let sources = SourceSnapshot::from_text(Path::new("fragment.rs"), "-1", &budget).unwrap();
        let source = sources.span(sources.file(0).unwrap(), 0, 1).unwrap();
        let mut session = TokenSession::new(sources, budget).unwrap();
        let span = session.original_span(source).unwrap();
        (session, span)
    }
    fn slice(session: &mut TokenSession, tokens: &[Token]) -> TokenSlice {
        let mut input = super::super::token_session::BudgetVec::new(
            session.budget(),
            super::super::expansion_budget::Storage::Temporary,
        )
        .unwrap();
        for token in tokens {
            input.push(InputToken::Leaf(*token)).unwrap();
        }
        let stream = group(session, input.as_slice(), GroupPolicy::Generated).unwrap();
        TokenSlice::roots(session, stream).unwrap()
    }
    #[test]
    fn literal_opacity_weight_and_foreign_restore() {
        let (mut session, span) = session();
        let spelling = session.intern_symbol("1").unwrap();
        let input = slice(
            &mut session,
            &[Token {
                kind: TokenKind::Literal {
                    kind: LiteralKind::Integer,
                    spelling,
                },
                span,
            }],
        );
        let LiteralCapture::Matched(id) = session.capture_literal(input).unwrap() else {
            panic!("literal expected")
        };
        let opaque = slice(
            &mut session,
            &[Token {
                kind: TokenKind::Opaque(id),
                span,
            }],
        );
        assert_eq!(opaque.physical_weight(&session).unwrap(), 1);
        let mut cursor = MacroCursor::new(&session, opaque.clone()).unwrap();
        let mark = cursor.mark(&session).unwrap();
        assert!(
            matches!(cursor.bump(&session).unwrap(), Some(LogicalToken::Leaf(Token { kind: TokenKind::Opaque(actual), .. })) if actual == id)
        );
        let recaptured = cursor.capture_since(&session, mark).unwrap();
        assert!(
            matches!(recaptured.node(&session, 0).unwrap(), Node::Leaf(Token { kind: TokenKind::Opaque(actual), .. }) if *actual == id)
        );
        let other = MacroCursor::new(&session, opaque.subslice(&session, 0..0).unwrap()).unwrap();
        let before = cursor.mark(&session).unwrap();
        assert!(matches!(
            cursor.restore(&session, other.mark(&session).unwrap()),
            Err(TokenError::ForeignHandle)
        ));
        assert_eq!(cursor.mark(&session).unwrap(), before);
    }
    #[test]
    fn minus_commits_and_boolean_literal_is_syntactic() {
        let (mut session, span) = session();
        let minus = Token {
            kind: TokenKind::Punct {
                ch: '-',
                spacing: Spacing::Alone,
            },
            span,
        };
        let invalid = slice(&mut session, &[minus]);
        assert!(matches!(
            session.capture_literal(invalid),
            Ok(LiteralCapture::CommittedSyntax { .. })
        ));
        let symbol = session.intern_symbol("false").unwrap();
        let boolean = Token {
            kind: TokenKind::Ident(super::super::token::Ident {
                symbol,
                raw: false,
                context: session.root_context(),
            }),
            span,
        };
        let input = slice(&mut session, &[minus, boolean]);
        let LiteralCapture::Matched(id) = session.capture_literal(input).unwrap() else {
            panic!("literal expected")
        };
        assert_eq!(session.fragment(id).unwrap().physical_weight(), 2);
        let (other, _) = self::session();
        assert!(matches!(other.fragment(id), Err(TokenError::ForeignHandle)));
    }

    #[test]
    fn joint_operator_keeps_constituent_occurrences_and_reissue_keeps_input() {
        use super::super::expansion_span::{ExpansionRecord, SpanOrigin};
        use super::super::token::Edition;
        let (mut session, span) = session();
        let second = session
            .synthetic_span(
                span,
                super::super::expansion_span::SyntheticReason::ExpansionToken,
            )
            .unwrap();
        let input = slice(
            &mut session,
            &[
                Token {
                    kind: TokenKind::Punct {
                        ch: '>',
                        spacing: Spacing::Joint,
                    },
                    span,
                },
                Token {
                    kind: TokenKind::Punct {
                        ch: '=',
                        spacing: Spacing::Alone,
                    },
                    span: second,
                },
            ],
        );
        let mut cursor = MacroCursor::new(&session, input).unwrap();
        assert!(
            matches!(cursor.bump(&session).unwrap(), Some(LogicalToken::Operator { spans: [Some(a), Some(b), None], len: 2, .. }) if a == span && b == second)
        );
        let defining_crate = session
            .register_crate_root(session.sources().file(0).unwrap())
            .unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: span,
                call: second,
                parent: None,
                defining_crate,
                edition: Edition::E2021,
            })
            .unwrap();
        let issued = session
            .reissue_span(second, session.root_context(), expansion)
            .unwrap();
        assert!(
            matches!(session.span(issued).unwrap().origin, SpanOrigin::Synthetic { anchor, .. } if anchor == second)
        );
        assert_eq!(session.span(issued).unwrap().expansion, Some(expansion));
    }

    #[test]
    fn minus_before_opaque_uses_checked_cached_literal_sign() {
        let (mut session, span) = session();
        let spelling = session.intern_symbol("1").unwrap();
        let positive = slice(
            &mut session,
            &[Token {
                kind: TokenKind::Literal {
                    kind: LiteralKind::Integer,
                    spelling,
                },
                span,
            }],
        );
        let LiteralCapture::Matched(positive) = session.capture_literal(positive).unwrap() else {
            panic!("positive literal")
        };
        let minus = Token {
            kind: TokenKind::Punct {
                ch: '-',
                spacing: Spacing::Alone,
            },
            span,
        };
        let negative = slice(
            &mut session,
            &[
                minus,
                Token {
                    kind: TokenKind::Opaque(positive),
                    span,
                },
            ],
        );
        let LiteralCapture::Matched(negative) = session.capture_literal(negative).unwrap() else {
            panic!("forwarded negative literal")
        };
        assert_eq!(session.fragment(negative).unwrap().physical_weight(), 2);
        // A no-minus forwarding hop must retain the negative grammar fact.
        let forwarded = slice(
            &mut session,
            &[Token {
                kind: TokenKind::Opaque(negative),
                span,
            }],
        );
        let LiteralCapture::Matched(forwarded) = session.capture_literal(forwarded).unwrap() else {
            panic!("forwarded literal")
        };
        let invalid = slice(
            &mut session,
            &[
                minus,
                Token {
                    kind: TokenKind::Opaque(forwarded),
                    span,
                },
            ],
        );
        assert!(matches!(
            session.capture_literal(invalid).unwrap(),
            LiteralCapture::CommittedSyntax { .. }
        ));
    }
}
