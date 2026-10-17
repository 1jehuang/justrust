//! Structural token transport only. This module is not a Rust lexer or parser.
//! Spelling interning does not validate Unicode identifiers or admit new syntax.
use super::expansion_span::{CrateId, SpanId};
use super::hygiene::SyntaxContextId;
use super::token_session::{SymbolId, TokenError, TokenSession};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edition {
    E2015,
    E2018,
    E2021,
    E2024,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spacing {
    Alone,
    Joint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    Integer,
    Float,
    Char,
    Byte,
    String,
    ByteString,
    RawString,
    RawByteString,
    CString,
    RawCString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ident {
    pub symbol: SymbolId,
    pub raw: bool,
    pub context: SyntaxContextId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Ident(Ident),
    Literal {
        kind: LiteralKind,
        spelling: SymbolId,
    },
    Punct {
        ch: char,
        spacing: Spacing,
    },
    /// A source-root reference, not a resolved cross-crate semantic identity.
    DollarCrate(CrateId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: TokenKind,
    pub span: SpanId,
}

impl Token {
    /// Checks ownership and structural invariants, not lexical Rust validity.
    pub fn validate(&self, session: &TokenSession) -> Result<(), TokenError> {
        let span = session.span(self.span)?;
        match self.kind {
            TokenKind::Ident(ident) => {
                session.symbol(ident.symbol)?;
                session.context(ident.context)?;
                if ident.context != span.context {
                    return Err(TokenError::InvalidInput("identifier/span contexts differ"));
                }
            }
            TokenKind::Literal { spelling, .. } => {
                session.symbol(spelling)?;
            }
            TokenKind::Punct { ch, .. } => {
                if !ch.is_ascii_punctuation() || matches!(ch, '(' | ')' | '[' | ']' | '{' | '}') {
                    return Err(TokenError::InvalidInput(
                        "punctuation must be a non-delimiter ASCII atom",
                    ));
                }
            }
            TokenKind::DollarCrate(krate) => {
                session.crate_root(krate)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delimiter {
    Parenthesis,
    Brace,
    Bracket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputToken {
    Leaf(Token),
    Open { kind: Delimiter, span: SpanId },
    Close { kind: Delimiter, span: SpanId },
}

impl InputToken {
    pub fn span(self) -> SpanId {
        match self {
            Self::Leaf(token) => token.span,
            Self::Open { span, .. } | Self::Close { span, .. } => span,
        }
    }
}
