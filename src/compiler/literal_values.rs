//! Semantic values for the lexer-validated Rust text and byte literal families.
//! The scanner is the single authority for boundaries, escapes and ASCII rules.
use super::expansion_budget::{ByteLease, Counter, ExpansionBudget, Storage};
use super::literal_scan::{self, LiteralError};
use super::token::{Edition, LiteralKind};
use super::token_session::TokenError;

pub(super) struct Value {
    bytes: Vec<u8>,
    _lease: ByteLease,
}
impl Value {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub(super) fn decode(text: &str, budget: &ExpansionBudget) -> Result<Value, LiteralError> {
    let literal = literal_scan::scan(text, 0, Edition::E2024, budget)?.ok_or(
        TokenError::InvalidInput("expected text or byte string literal"),
    )?;
    if !matches!(
        literal.kind,
        LiteralKind::String
            | LiteralKind::RawString
            | LiteralKind::ByteString
            | LiteralKind::RawByteString
    ) || literal.end != text.len()
        || !literal.info.suffix.is_empty()
    {
        return Err(LiteralError::Source {
            range: 0..text.len(),
            message: "expected unsuffixed text or byte string literal",
        });
    }
    let byte = matches!(
        literal.kind,
        LiteralKind::ByteString | LiteralKind::RawByteString
    );
    let raw = literal.info.raw_hashes.is_some();
    let mut lease = budget
        .reserve(Storage::Retained, text.len() as u64)
        .map_err(TokenError::from)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(text.len())
        .map_err(|_| TokenError::Budget(budget.poison_allocation()))?;
    lease
        .resize(bytes.capacity() as u64)
        .map_err(TokenError::from)?;
    let mut at = literal.info.body.start;
    while at < literal.info.body.end {
        let ch = text[at..].chars().next().ok_or(TokenError::InvalidRange)?;
        budget
            .charge(Counter::Work, ch.len_utf8() as u64)
            .map_err(TokenError::from)?;
        if ch == '\\' && !raw {
            let (next, decoded, _) = literal_scan::escape(text, at, true, byte, false, budget)?;
            if let Some(ch) = decoded {
                if byte {
                    bytes.push(ch as u8);
                } else {
                    bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
            }
            at = next;
        } else if ch == '\r' {
            bytes.push(b'\n');
            at += 2;
        } else {
            bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
            at += ch.len_utf8();
        }
    }
    Ok(Value {
        bytes,
        _lease: lease,
    })
}
