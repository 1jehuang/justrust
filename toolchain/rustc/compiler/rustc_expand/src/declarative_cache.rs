//! Native, deliberately bounded declarative match/transcribe cache boundary.
//!
//! No proc macro is ever executed here. Callers admit only literal matchers and
//! literal transcribers, with no parser fragments or interpolation. Parsing the
//! result and recording rule usage remain outside the cached operation.

use rustc_ast::token::{Delimiter, TokenKind};
use rustc_ast::tokenstream::{TokenStream, TokenTree};
use rustc_data_structures::AtomicRef;
use rustc_span::{LocalExpnId, Span};

pub type DeclarativeExpansion = Option<(u32, TokenStream)>;
pub type DeclarativeCompute<'a> = dyn Fn() -> DeclarativeExpansion + 'a;

/// Fail open when the query integration is not installed.
pub static EXPAND_DECLARATIVE_MACRO_CACHED: AtomicRef<
    fn(
        LocalExpnId,
        TokenStream,
        TokenStream,
        (Span, u16),
        &DeclarativeCompute<'_>,
    ) -> DeclarativeExpansion,
> = AtomicRef::new(&((|_, _, _, _, compute: &DeclarativeCompute<'_>| compute()) as _));

/// Only ordinary lexical tokens have been audited for this private codec path.
/// Reject all invisible groups (including parser-produced AST fragments), and
/// the interpolated ident/lifetime variants even though those encode today.
pub fn cacheable_tokens(tokens: &TokenStream) -> bool {
    tokens.iter().all(|tree| match tree {
        TokenTree::Token(token, _) => !matches!(
            token.kind,
            TokenKind::NtIdent(..)
                | TokenKind::NtLifetime(..)
                | TokenKind::OpenInvisible(..)
                | TokenKind::CloseInvisible(..)
                | TokenKind::DocComment(..)
                | TokenKind::Eof
        ),
        TokenTree::Delimited(_, _, Delimiter::Invisible(_), _) => false,
        TokenTree::Delimited(_, _, _, inner) => cacheable_tokens(inner),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_ast::token::{IdentKind, InvisibleOrigin};
    use rustc_ast::tokenstream::{DelimSpacing, DelimSpan, Spacing};
    use rustc_span::{DUMMY_SP, Ident, create_default_session_globals_then};

    #[test]
    fn lexical_scope_rejects_interpolation_recursively() {
        create_default_session_globals_then(|| {
            assert!(cacheable_tokens(&TokenStream::default()));
            assert!(cacheable_tokens(&TokenStream::token_alone(TokenKind::Plus, DUMMY_SP)));
            let interpolated = TokenStream::token_alone(
                TokenKind::NtIdent(Ident::empty(), IdentKind::Normal),
                DUMMY_SP,
            );
            assert!(!cacheable_tokens(&interpolated));
            let group = |delimiter, inner| {
                TokenStream::new(vec![TokenTree::Delimited(
                    DelimSpan::dummy(),
                    DelimSpacing::new(Spacing::Alone, Spacing::Alone),
                    delimiter,
                    inner,
                )])
            };
            assert!(!cacheable_tokens(&group(Delimiter::Brace, interpolated)));
            assert!(!cacheable_tokens(&group(
                Delimiter::Invisible(InvisibleOrigin::ProcMacro),
                TokenStream::default(),
            )));
            assert!(!cacheable_tokens(&TokenStream::token_alone(TokenKind::Eof, DUMMY_SP)));
        });
    }
}
