//! Lifetime tokens retain both original punctuation and identifier occurrences.
//! Identifier admission remains the same invocation-scoped capability used by
//! other expanded names. No root context or synthetic source range is minted.
use super::*;

pub(super) fn push(
    output: &mut NativeTokenBuffer,
    session: &TokenSession,
    quote: SpanId,
    ident: crate::compiler::token::Ident,
    span: SpanId,
    edition: Edition,
) -> Result<(), ParseError> {
    let token = Token {
        kind: TokenKind::Ident(ident),
        span,
    };
    token.validate(session).map_err(transport_error)?;
    let spelling = session.symbol(ident.symbol).map_err(transport_error)?;
    if ident.raw || !spelling.is_ascii() {
        return Err(failure(
            session,
            span,
            "raw and non-ASCII lifetimes are unsupported",
        ));
    }
    let capacity = spelling
        .len()
        .checked_add(1)
        .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
    let _lease = session
        .budget()
        .reserve(Storage::Temporary, capacity as u64)
        .map_err(|error| transport_error(error.into()))?;
    let mut text = String::new();
    text.try_reserve_exact(capacity)
        .map_err(|_| transport_error(TokenError::Budget(session.budget().poison_allocation())))?;
    text.push('\'');
    text.push_str(spelling);
    output.push_expanded(
        session,
        &text,
        ParserTokenKind::Lifetime,
        &[part(session, quote)?, part(session, span)?],
        Some((token, edition)),
    )
}
