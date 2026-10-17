//! Native literal boundary and content validation. This never invokes a compiler.
//! Decoding walks an already validated literal, not printed token transport.
use std::ops::Range;

use super::expansion_budget::{ByteLease, Counter, ExpansionBudget, Storage};
use super::token::{Edition, LiteralKind};
use super::token_session::TokenError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiteralInfo {
    pub body: Range<usize>,
    pub suffix: Range<usize>,
    pub decimal_point: Option<usize>,
    pub exponent: Option<usize>,
    pub raw_hashes: Option<u8>,
    pub has_continuation: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ScannedLiteral {
    pub kind: LiteralKind,
    pub end: usize,
    pub info: LiteralInfo,
}

#[derive(Debug)]
pub(super) enum LiteralError {
    Source {
        range: Range<usize>,
        message: &'static str,
    },
    Transport(TokenError),
}
impl From<TokenError> for LiteralError {
    fn from(error: TokenError) -> Self {
        Self::Transport(error)
    }
}

fn fault(text: &str, at: usize, message: &'static str) -> LiteralError {
    let end = text
        .get(at..)
        .and_then(|s| s.chars().next())
        .map_or(at, |c| at + c.len_utf8());
    LiteralError::Source {
        range: at..end,
        message,
    }
}
fn work(budget: &ExpansionBudget, amount: usize) -> Result<(), LiteralError> {
    budget
        .charge(Counter::Work, amount as u64)
        .map_err(TokenError::from)?;
    Ok(())
}
pub(super) fn ident_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}
pub(super) fn ident_continue(c: char) -> bool {
    unicode_ident::is_xid_continue(c)
}
fn suffix(text: &str, mut at: usize, budget: &ExpansionBudget) -> Result<usize, LiteralError> {
    if text[at..].chars().next().is_some_and(ident_start) {
        while let Some(c) = text[at..].chars().next() {
            if !ident_continue(c) {
                break;
            }
            work(budget, c.len_utf8())?;
            at += c.len_utf8();
        }
    }
    Ok(at)
}

pub(super) fn scan(
    text: &str,
    start: usize,
    edition: Edition,
    budget: &ExpansionBudget,
) -> Result<Option<ScannedLiteral>, LiteralError> {
    let bytes = text.as_bytes();
    let Some(&c) = bytes.get(start) else {
        return Ok(None);
    };
    if c.is_ascii_digit() {
        return number(text, start, budget).map(Some);
    }
    let (prefix, raw, byte, c_string, quote) = if c == b'"' || c == b'\'' {
        (0, false, false, false, c)
    } else if bytes[start..].starts_with(b"b\"") || bytes[start..].starts_with(b"b'") {
        (1, false, true, false, bytes[start + 1])
    } else if bytes[start..].starts_with(b"c\"")
        && matches!(edition, Edition::E2021 | Edition::E2024)
    {
        (1, false, false, true, b'"')
    } else if c == b'r' && matches!(bytes.get(start + 1), Some(b'"' | b'#')) {
        (1, true, false, false, b'"')
    } else if bytes[start..].starts_with(b"br") && matches!(bytes.get(start + 2), Some(b'"' | b'#'))
    {
        (2, true, true, false, b'"')
    } else if bytes[start..].starts_with(b"cr")
        && matches!(edition, Edition::E2021 | Edition::E2024)
        && matches!(bytes.get(start + 2), Some(b'"' | b'#'))
    {
        (2, true, false, true, b'"')
    } else {
        return Ok(None);
    };
    if raw {
        let mut open = start + prefix;
        while bytes.get(open) == Some(&b'#') {
            work(budget, 1)?;
            open += 1;
        }
        // A single r# followed by an identifier is handled by the identifier scanner.
        if bytes.get(open) != Some(&b'"') {
            if prefix == 1 && open == start + 2 {
                return Ok(None);
            }
            return Err(fault(text, open, "expected quote after raw string prefix"));
        }
        let hashes = open - start - prefix;
        if hashes > 255 {
            return Err(fault(
                text,
                start,
                "raw string delimiter exceeds 255 hashes",
            ));
        }
        let body_start = open + 1;
        let mut at = body_start;
        loop {
            let Some(ch) = text[at..].chars().next() else {
                return Err(fault(text, start, "unterminated raw string"));
            };
            work(budget, ch.len_utf8())?;
            if ch == '"'
                && bytes
                    .get(at + 1..at + 1 + hashes)
                    .is_some_and(|tail| tail.iter().all(|b| *b == b'#'))
            {
                let end_body = at;
                let end_literal = at + 1 + hashes;
                let end = suffix(text, end_literal, budget)?;
                return Ok(Some(ScannedLiteral {
                    kind: if byte {
                        LiteralKind::RawByteString
                    } else if c_string {
                        LiteralKind::RawCString
                    } else {
                        LiteralKind::RawString
                    },
                    end,
                    info: LiteralInfo {
                        body: body_start..end_body,
                        suffix: end_literal..end,
                        decimal_point: None,
                        exponent: None,
                        raw_hashes: Some(hashes as u8),
                        has_continuation: false,
                    },
                }));
            }
            if byte && !ch.is_ascii() {
                return Err(fault(text, at, "non-ASCII byte string character"));
            }
            if c_string && ch == '\0' {
                return Err(fault(text, at, "nul in C string literal"));
            }
            if ch == '\r' && bytes.get(at + 1) != Some(&b'\n') {
                return Err(fault(text, at, "bare carriage return in string"));
            }
            at += ch.len_utf8();
        }
    }
    if quote == b'\'' && prefix == 0 {
        // 'name is a lifetime unless an immediately following quote closes a character.
        if text[start + 1..].chars().next().is_some_and(ident_start) {
            let mut at = start + 1;
            while let Some(ch) = text[at..].chars().next() {
                if !ident_continue(ch) {
                    break;
                }
                work(budget, ch.len_utf8())?;
                at += ch.len_utf8();
            }
            if bytes.get(at) != Some(&b'\'') {
                return Ok(None);
            }
        }
    }
    let body_start = start + prefix + 1;
    let mut at = body_start;
    let mut count = 0usize;
    let mut continuation = false;
    loop {
        let Some(ch) = text[at..].chars().next() else {
            return Err(fault(
                text,
                start,
                if quote == b'"' {
                    "unterminated string"
                } else {
                    "unterminated character literal"
                },
            ));
        };
        work(budget, ch.len_utf8())?;
        if ch as u32 == quote as u32 {
            if quote == b'\'' && count != 1 {
                return Err(fault(
                    text,
                    at,
                    "character literal must contain exactly one character",
                ));
            }
            let body_end = at;
            let suffix_start = at + 1;
            let end = suffix(text, suffix_start, budget)?;
            return Ok(Some(ScannedLiteral {
                kind: if quote == b'\'' {
                    if byte {
                        LiteralKind::Byte
                    } else {
                        LiteralKind::Char
                    }
                } else if byte {
                    LiteralKind::ByteString
                } else if c_string {
                    LiteralKind::CString
                } else {
                    LiteralKind::String
                },
                end,
                info: LiteralInfo {
                    body: body_start..body_end,
                    suffix: suffix_start..end,
                    decimal_point: None,
                    exponent: None,
                    raw_hashes: None,
                    has_continuation: continuation,
                },
            }));
        }
        if ch == '\\' {
            let (next, _, continued) = escape(text, at, quote == b'"', byte, c_string, budget)?;
            at = next;
            continuation |= continued;
            if !continued {
                count += 1;
            }
            continue;
        }
        if quote == b'\'' && matches!(ch, '\r' | '\n' | '\t') {
            return Err(fault(text, at, "unescaped whitespace in character literal"));
        }
        if byte && !ch.is_ascii() {
            return Err(fault(text, at, "non-ASCII byte literal character"));
        }
        if c_string && ch == '\0' {
            return Err(fault(text, at, "nul in C string literal"));
        }
        if ch == '\r' && bytes.get(at + 1) != Some(&b'\n') {
            return Err(fault(text, at, "bare carriage return in string"));
        }
        at += ch.len_utf8();
        count += 1;
    }
}

fn escape(
    text: &str,
    slash: usize,
    string: bool,
    byte: bool,
    c_string: bool,
    budget: &ExpansionBudget,
) -> Result<(usize, Option<char>, bool), LiteralError> {
    let mut at = slash + 1;
    let Some(ch) = text[at..].chars().next() else {
        return Err(fault(text, at, "unsupported string escape"));
    };
    work(budget, ch.len_utf8())?;
    let decoded = match ch {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '0' => '\0',
        '\\' => '\\',
        '\'' => '\'',
        '"' => '"',
        '\n' | '\r' if string => {
            if ch == '\r' && text.as_bytes().get(at + 1) != Some(&b'\n') {
                return Err(fault(text, at, "bare carriage return in string"));
            }
            at += if ch == '\r' { 2 } else { 1 };
            while let Some(c) = text[at..].chars().next() {
                if !matches!(c, ' ' | '\t' | '\r' | '\n') {
                    break;
                }
                work(budget, 1)?;
                at += 1;
            }
            return Ok((at, None, true));
        }
        'x' => {
            let mut value = 0u32;
            for _ in 0..2 {
                at += 1;
                work(budget, 1)?;
                let digit = text
                    .as_bytes()
                    .get(at)
                    .copied()
                    .and_then(|b| (b as char).to_digit(16))
                    .ok_or_else(|| fault(text, at, "expected two hex digits"))?;
                value = value * 16 + digit;
            }
            if !byte && !c_string && value > 127 {
                return Err(fault(text, at, "hex string escape must be ASCII"));
            }
            char::from_u32(value).ok_or_else(|| fault(text, at, "invalid escape scalar"))?
        }
        'u' if !byte => {
            at += 1;
            if text.as_bytes().get(at) != Some(&b'{') {
                return Err(fault(text, at, "expected unicode escape brace"));
            }
            at += 1;
            let mut value = 0u32;
            let mut digits = 0usize;
            loop {
                let Some(&b) = text.as_bytes().get(at) else {
                    return Err(fault(text, at, "unterminated unicode escape"));
                };
                work(budget, 1)?;
                if b == b'}' {
                    break;
                }
                if b == b'_' && digits > 0 {
                    at += 1;
                    continue;
                }
                let digit = (b as char)
                    .to_digit(16)
                    .ok_or_else(|| fault(text, at, "invalid unicode escape"))?;
                digits += 1;
                if digits > 6 {
                    return Err(fault(text, at, "unicode escape exceeds six digits"));
                }
                value = value * 16 + digit;
                at += 1;
            }
            if digits == 0 {
                return Err(fault(text, at, "unterminated unicode escape"));
            }
            char::from_u32(value).ok_or_else(|| fault(text, at, "invalid unicode scalar"))?
        }
        _ => return Err(fault(text, at, "unsupported string escape")),
    };
    if c_string && decoded == '\0' {
        return Err(fault(text, at, "nul in C string literal"));
    }
    Ok((at + ch.len_utf8(), Some(decoded), false))
}

fn number(
    text: &str,
    start: usize,
    budget: &ExpansionBudget,
) -> Result<ScannedLiteral, LiteralError> {
    let bytes = text.as_bytes();
    let mut at = start;
    let radix = if bytes.get(start) == Some(&b'0') {
        match bytes.get(start + 1) {
            Some(b'x') => 16,
            Some(b'o') => 8,
            Some(b'b') => 2,
            _ => 10,
        }
    } else {
        10
    };
    let mut point = None;
    let mut exponent = None;
    if radix != 10 {
        at += 2;
        let mut digits = 0usize;
        while let Some(&b) = bytes.get(at) {
            if b == b'_' {
                at += 1;
                work(budget, 1)?;
                continue;
            }
            if (b as char).is_digit(radix) {
                digits += 1;
                at += 1;
                work(budget, 1)?;
                continue;
            }
            if b.is_ascii_digit() {
                return Err(fault(text, at, "invalid digit for integer radix"));
            }
            break;
        }
        if digits == 0 {
            return Err(fault(text, at, "integer literal requires a digit"));
        }
        if bytes.get(at) == Some(&b'.')
            && bytes.get(at + 1) != Some(&b'.')
            && !text
                .get(at + 1..)
                .and_then(|tail| tail.chars().next())
                .is_some_and(ident_start)
        {
            return Err(fault(text, at, "nondecimal float literal is not supported"));
        }
        if radix != 16 && matches!(bytes.get(at), Some(b'e' | b'E')) {
            return Err(fault(
                text,
                at,
                "nondecimal float exponent is not supported",
            ));
        }
    } else {
        while bytes
            .get(at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
        {
            work(budget, 1)?;
            at += 1;
        }
        if bytes.get(at) == Some(&b'.')
            && bytes.get(at + 1) != Some(&b'.')
            && !text
                .get(at + 1..)
                .and_then(|s| s.chars().next())
                .is_some_and(ident_start)
        {
            point = Some(at);
            at += 1;
            while bytes
                .get(at)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
            {
                work(budget, 1)?;
                at += 1;
            }
        }
        if matches!(bytes.get(at), Some(b'e' | b'E')) {
            exponent = Some(at);
            at += 1;
            if matches!(bytes.get(at), Some(b'+' | b'-')) {
                at += 1;
            }
            let mut digits = 0usize;
            while let Some(&b) = bytes.get(at) {
                if !b.is_ascii_digit() && b != b'_' {
                    break;
                }
                work(budget, 1)?;
                if b.is_ascii_digit() {
                    digits += 1;
                }
                at += 1;
            }
            if digits == 0 {
                return Err(fault(text, at, "float exponent requires a digit"));
            }
        }
    }
    let suffix_start = at;
    let end = suffix(text, at, budget)?;
    let float_suffix = matches!(&text[suffix_start..end], "f32" | "f64");
    Ok(ScannedLiteral {
        kind: if point.is_some() || exponent.is_some() || (radix == 10 && float_suffix) {
            LiteralKind::Float
        } else {
            LiteralKind::Integer
        },
        end,
        info: LiteralInfo {
            body: start..suffix_start,
            suffix: suffix_start..end,
            decimal_point: point,
            exponent,
            raw_hashes: None,
            has_continuation: false,
        },
    })
}

pub struct DecodedString {
    value: String,
    _lease: ByteLease,
}
impl DecodedString {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

pub(super) fn decode_normal_string(
    text: &str,
    budget: &ExpansionBudget,
) -> Result<DecodedString, LiteralError> {
    let literal = scan(text, 0, Edition::E2024, budget)?
        .ok_or_else(|| fault(text, 0, "expected normal string literal"))?;
    if literal.kind != LiteralKind::String
        || literal.end != text.len()
        || !literal.info.suffix.is_empty()
    {
        return Err(fault(text, 0, "expected unsuffixed normal string literal"));
    }
    let mut lease = budget
        .reserve(Storage::Retained, text.len() as u64)
        .map_err(TokenError::from)?;
    let mut value = String::new();
    if value.try_reserve_exact(text.len()).is_err() {
        return Err(TokenError::Budget(budget.poison_allocation()).into());
    }
    lease
        .resize(value.capacity() as u64)
        .map_err(TokenError::from)?;
    let mut at = literal.info.body.start;
    while at < literal.info.body.end {
        let ch = text[at..]
            .chars()
            .next()
            .ok_or_else(|| fault(text, at, "invalid decoded string boundary"))?;
        work(budget, ch.len_utf8())?;
        if ch == '\\' {
            let (next, decoded, _) = escape(text, at, true, false, false, budget)?;
            if let Some(c) = decoded {
                value.push(c);
            }
            at = next;
        } else if ch == '\r' {
            value.push('\n');
            at += 2;
        } else {
            value.push(ch);
            at += ch.len_utf8();
        }
    }
    Ok(DecodedString {
        value,
        _lease: lease,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(text: &str) -> ScannedLiteral {
        scan(text, 0, Edition::E2024, &ExpansionBudget::default())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn pinned_xid_tables_and_identifier_properties_are_exact() {
        assert_eq!(unicode_ident::UNICODE_VERSION, (17, 0, 0));
        assert!(ident_start('_'));
        assert!(ident_start('水'));
        assert!(!ident_start('\u{0301}'));
        assert!(ident_continue('\u{0301}'));
        assert!(!ident_start('🦀'));
        assert!(!ident_continue('🦀'));
    }

    #[test]
    fn all_literal_classes_keep_exact_boundaries_and_suffixes() {
        for (text, kind) in [
            ("0xff_u8", LiteralKind::Integer),
            ("1u8x", LiteralKind::Integer),
            ("1.", LiteralKind::Float),
            ("1e-3", LiteralKind::Float),
            ("2.5E+7f32", LiteralKind::Float),
            ("1.5wat", LiteralKind::Float),
            ("'水'", LiteralKind::Char),
            ("b'\\xff'", LiteralKind::Byte),
            ("\"水\\u{41}\"tag", LiteralKind::String),
            ("b\"abc\\xff\"", LiteralKind::ByteString),
            ("r##\"a\"#b\"##tag", LiteralKind::RawString),
            ("br#\"abc\"#", LiteralKind::RawByteString),
            ("c\"水\\xff\"", LiteralKind::CString),
            ("cr#\"水\"#", LiteralKind::RawCString),
        ] {
            let parsed = literal(text);
            assert_eq!(parsed.kind, kind, "{text}");
            assert_eq!(parsed.end, text.len(), "{text}");
            assert!(text.get(parsed.info.body).is_some(), "{text}");
            assert!(text.get(parsed.info.suffix).is_some(), "{text}");
        }
        let parsed = literal("2.5E+7f32");
        assert_eq!(parsed.info.decimal_point, Some(1));
        assert_eq!(parsed.info.exponent, Some(3));
        assert_eq!(parsed.info.suffix, 6..9);
        assert_eq!(literal("r##\"a\"##tag").info.raw_hashes, Some(2));
        assert_eq!(literal("1..2").end, 1);
        assert_eq!(literal("1.foo").end, 1);
        assert_eq!(literal("0x1..2").end, 3);
        assert_eq!(literal("0x1.foo").end, 3);
        assert_eq!(literal("0x1e2").kind, LiteralKind::Integer);
        assert_eq!(literal("0b1u8x").info.suffix, 3..6);
        assert_eq!(literal("0b1f32").info.suffix, 3..6);
    }

    #[test]
    fn malformed_unsupported_classes_are_still_lexically_invalid() {
        for text in [
            "0x",
            "0x1.0",
            "0x1.",
            "0b1.0",
            "0o7.5",
            "0b1efoo",
            "0b1e",
            "0b1e+",
            "1example",
            "0b1e2",
            "0o1E2",
            "0b1e-2",
            "0o1E+2",
            "0b2",
            "0o8",
            "1e",
            "1e+",
            "''",
            "'ab'",
            "'\\u{d800}'",
            "b'水'",
            "b'\\u{41}'",
            "b\"水\"",
            "br\"水\"",
            "r#\"unterminated",
            "\"\\q\"",
            "\"\\x80\"",
            "\"\\u{}\"",
            "\"\\u{110000}\"",
            "\"a\rb\"",
            "c\"a\\0\"",
            "cr\"a\0\"",
            "\"\\",
        ] {
            assert!(
                matches!(
                    scan(text, 0, Edition::E2024, &ExpansionBudget::default()),
                    Err(LiteralError::Source { .. })
                ),
                "{text:?}"
            );
        }
        let hashes = "#".repeat(256);
        let text = format!("r{hashes}\"a\"{hashes}");
        assert!(scan(&text, 0, Edition::E2024, &ExpansionBudget::default()).is_err());
    }

    #[test]
    fn normal_string_value_is_separate_accounted_and_validated() {
        let budget = ExpansionBudget::default();
        let before = budget.snapshot();
        let decoded = decode_normal_string("\"é\r\n水\\n\\x41\\u{1f600}\"", &budget).unwrap();
        assert_eq!(decoded.as_str(), "é\n水\nA😀");
        assert!(budget.snapshot().retained_bytes > before.retained_bytes);
        let work = budget.snapshot().counters[Counter::Work as usize];
        drop(decoded);
        assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], work);
        assert!(literal("\"a\\\n   b\"").info.has_continuation);
        assert_eq!(
            decode_normal_string("\"a\\\n   b\"", &budget)
                .unwrap()
                .as_str(),
            "ab"
        );
        // Stock accepts CR among whitespace skipped after an escaped LF. This
        // differs from an unescaped bare CR in literal content.
        assert_eq!(
            decode_normal_string("\"a\\\n\r b\"", &budget)
                .unwrap()
                .as_str(),
            "ab"
        );
        assert!(decode_normal_string("", &budget).is_err());
    }
}
