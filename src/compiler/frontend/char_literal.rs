//! Allocation-free decoding of a single source Unicode scalar.
//! The lexer validates original input first. Expanded tokens take the same
//! checked decoding path, so no source or expansion can fabricate a scalar.
pub(super) fn parse_char_literal(text: &str) -> Result<char, &'static str> {
    let body = text
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .ok_or("character literal suffix or delimiter is invalid")?;
    let mut chars = body.chars();
    let first = chars.next().ok_or("character literal is empty")?;
    if first != '\\' {
        if matches!(first, '\'' | '\n' | '\r' | '\t') || chars.next().is_some() {
            return Err("character literal must contain one scalar or supported escape");
        }
        return Ok(first);
    }
    let escape = chars.next().ok_or("incomplete character escape")?;
    let scalar = match escape {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '0' => '\0',
        '\\' => '\\',
        '\'' => '\'',
        '"' => '"',
        'x' => {
            let high = chars
                .next()
                .and_then(|c| c.to_digit(16))
                .ok_or("character hex escape requires two digits")?;
            let low = chars
                .next()
                .and_then(|c| c.to_digit(16))
                .ok_or("character hex escape requires two digits")?;
            let value = high * 16 + low;
            if value > 0x7f {
                return Err("character hex escape must be ASCII");
            }
            char::from_u32(value).ok_or("invalid character scalar")?
        }
        'u' => {
            if chars.next() != Some('{') {
                return Err("expected Unicode escape brace");
            }
            let mut digits = 0;
            let mut value = 0;
            loop {
                match chars.next() {
                    Some('}') if digits > 0 => break,
                    Some('_') if digits > 0 => (),
                    Some(c) if c.is_ascii_hexdigit() && digits < 6 => {
                        digits += 1;
                        value = value * 16 + c.to_digit(16).unwrap();
                    }
                    _ => return Err("invalid Unicode character escape"),
                }
            }
            char::from_u32(value).ok_or("invalid Unicode scalar")?
        }
        _ => return Err("unsupported character escape"),
    };
    if chars.next().is_some() {
        return Err("character literal contains extra scalars");
    }
    Ok(scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_and_escape_decoding() {
        for (source, expected) in [
            ("'a'", 'a'),
            ("'水'", '水'),
            ("'🦀'", '🦀'),
            (r"'\n'", '\n'),
            (r"'\r'", '\r'),
            (r"'\t'", '\t'),
            (r"'\0'", '\0'),
            (r"'\\'", '\\'),
            (r"'\''", '\''),
            ("'\\\"'", '"'),
            (r"'\x7f'", '\u{7f}'),
            (r"'\u{0}'", '\0'),
            (r"'\u{10_FF_FF_}'", '\u{10ffff}'),
        ] {
            assert_eq!(parse_char_literal(source), Ok(expected), "{source}");
        }
    }
    #[test]
    fn rejects_non_scalar_and_non_character_spellings() {
        for source in [
            "''",
            "'ab'",
            "'a'u32",
            "b'a'",
            "'\n'",
            "'\t'",
            r"'\u{d800}'",
            r"'\u{dfff}'",
            r"'\u{110000}'",
            r"'\u{0000000}'",
            r"'\u{}'",
            r"'\u{_1}'",
            r"'\u{1'",
            r"'\x80'",
            r"'\x0'",
            r"'\q'",
            r"'\nX'",
            "'\\'",
        ] {
            assert!(parse_char_literal(source).is_err(), "{source}");
        }
    }
}
