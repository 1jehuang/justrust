//! `std::fmt` format-string grammar for the output macros.
//!
//! Parses `{[arg][:[[fill]align][sign][#][0][width][.precision][type]]}` into
//! literal pieces plus one [`Placeholder`] per argument use. Argument
//! references (positional index, named, implicit next) are resolved against
//! the macro's explicit argument list by the parser; inline captures
//! (`{name}` with no such named argument) become a local variable read.
//! `width$`/`precision$`/`.*` argument counts are rejected explicitly.
use super::super::ir;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatTrait {
    Display,
    Debug,
    LowerHex,
    UpperHex,
    Binary,
    Octal,
    LowerExp,
    UpperExp,
}

/// Resolved formatting options for one placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatSpec {
    pub fill: char,
    /// 0 none, 1 left, 2 right, 3 center (matches `std::fmt::Formatter`).
    pub align: u8,
    pub width: Option<usize>,
    pub precision: Option<usize>,
    /// bit0 `#`, bit1 `+`, bit2 `0`.
    pub flags: u32,
    pub format: FormatTrait,
}

impl FormatSpec {
    pub const PLAIN: FormatSpec = FormatSpec {
        fill: ' ',
        align: 0,
        width: None,
        precision: None,
        flags: 0,
        format: FormatTrait::Display,
    };
    /// True when the existing native scalar formatter prints this exactly.
    pub fn native_mode(&self) -> Option<ir::FormatMode> {
        if self.align != 0 || self.width.is_some() || self.precision.is_some() || self.flags != 0 {
            return None;
        }
        match self.format {
            FormatTrait::Display => Some(ir::FormatMode::Display),
            FormatTrait::Debug => Some(ir::FormatMode::Debug),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgRef {
    Next,
    Index(usize),
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placeholder {
    pub arg: ArgRef,
    pub spec: FormatSpec,
}

/// `pieces.len() == placeholders.len() + 1`.
pub fn parse_format(text: &str) -> Result<(Vec<String>, Vec<Placeholder>), String> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut holders = Vec::new();
    let mut piece = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                piece.push('{');
                i += 2;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '}' {
                if chars[end] == '{' {
                    return Err("invalid format string: nested `{` in placeholder".into());
                }
                end += 1;
            }
            if end == chars.len() {
                return Err("invalid format string: unmatched `{` in format string".into());
            }
            let inner: String = chars[start..end].iter().collect();
            holders.push(placeholder(&inner)?);
            pieces.push(std::mem::take(&mut piece));
            i = end + 1;
        } else if c == '}' {
            if chars.get(i + 1) == Some(&'}') {
                piece.push('}');
                i += 2;
                continue;
            }
            return Err("invalid format string: unmatched `}` found".into());
        } else {
            piece.push(c);
            i += 1;
        }
    }
    pieces.push(piece);
    Ok((pieces, holders))
}

fn placeholder(inner: &str) -> Result<Placeholder, String> {
    let (arg, spec) = match inner.find(':') {
        Some(at) => (&inner[..at], Some(&inner[at + 1..])),
        None => (inner, None),
    };
    let arg = arg.trim();
    let arg = if arg.is_empty() {
        ArgRef::Next
    } else if arg.bytes().all(|b| b.is_ascii_digit()) {
        ArgRef::Index(arg.parse().map_err(|_| "invalid format argument index")?)
    } else if arg.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        ArgRef::Name(arg.to_owned())
    } else {
        return Err(format!(
            "invalid format string: invalid argument name `{arg}`"
        ));
    };
    let spec = match spec {
        Some(spec) => parse_spec(spec)?,
        None => FormatSpec::PLAIN,
    };
    Ok(Placeholder { arg, spec })
}

fn parse_spec(spec: &str) -> Result<FormatSpec, String> {
    let chars: Vec<char> = spec.chars().collect();
    let mut out = FormatSpec::PLAIN;
    let mut i = 0;
    let align_of = |c: char| match c {
        '<' => Some(1u8),
        '>' => Some(2u8),
        '^' => Some(3u8),
        _ => None,
    };
    if chars.len() >= 2
        && let Some(a) = align_of(chars[1])
    {
        out.fill = chars[0];
        out.align = a;
        i = 2;
    } else if let Some(a) = chars.first().and_then(|c| align_of(*c)) {
        out.align = a;
        i = 1;
    }
    if chars.get(i) == Some(&'+') {
        out.flags |= 2;
        i += 1;
    } else if chars.get(i) == Some(&'-') {
        i += 1;
    }
    if chars.get(i) == Some(&'#') {
        out.flags |= 1;
        i += 1;
    }
    if chars.get(i) == Some(&'0') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
        out.flags |= 4;
        i += 1;
    } else if chars.get(i) == Some(&'0') && !chars.get(i + 1).is_some_and(|c| *c == '$') {
        out.flags |= 4;
        i += 1;
    }
    let digits = |i: &mut usize| -> Option<usize> {
        let start = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        (start != *i).then(|| chars[start..*i].iter().collect::<String>().parse().ok())?
    };
    if let Some(w) = digits(&mut i) {
        if chars.get(i) == Some(&'$') {
            return Err("unsupported: width arguments (`N$`) in format specs".into());
        }
        out.width = Some(w);
    } else if chars
        .get(i)
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
        && chars[i..].contains(&'$')
    {
        return Err("unsupported: named width arguments (`name$`) in format specs".into());
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        if chars.get(i) == Some(&'*') {
            return Err("unsupported: `.*` precision arguments in format specs".into());
        }
        match digits(&mut i) {
            Some(p) => {
                if chars.get(i) == Some(&'$') {
                    return Err("unsupported: precision arguments (`N$`) in format specs".into());
                }
                out.precision = Some(p);
            }
            None => return Err("invalid format string: expected precision after `.`".into()),
        }
    }
    let rest: String = chars[i..].iter().collect();
    out.format = match rest.as_str() {
        "" => FormatTrait::Display,
        "?" => FormatTrait::Debug,
        "x" => FormatTrait::LowerHex,
        "X" => FormatTrait::UpperHex,
        "b" => FormatTrait::Binary,
        "o" => FormatTrait::Octal,
        "e" => FormatTrait::LowerExp,
        "E" => FormatTrait::UpperExp,
        "x?" | "X?" => FormatTrait::Debug,
        _ => {
            return Err(format!(
                "invalid format string: unknown format trait `{rest}`"
            ));
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_specs() {
        let (p, h) = parse_format("a {} b {:?} {x:>5} {0:08.3} {{}} {:#?} {:<3}").unwrap();
        assert_eq!(p, vec!["a ", " b ", " ", " ", " {} ", " ", ""]);
        assert_eq!(
            h[0],
            Placeholder {
                arg: ArgRef::Next,
                spec: FormatSpec::PLAIN
            }
        );
        assert_eq!(h[1].spec.format, FormatTrait::Debug);
        assert_eq!(h[2].arg, ArgRef::Name("x".into()));
        assert_eq!((h[2].spec.align, h[2].spec.width), (2, Some(5)));
        assert_eq!(h[3].arg, ArgRef::Index(0));
        assert_eq!(
            (h[3].spec.flags, h[3].spec.width, h[3].spec.precision),
            (4, Some(8), Some(3))
        );
        assert_eq!((h[4].spec.flags, h[4].spec.format), (1, FormatTrait::Debug));
        assert_eq!((h[5].spec.align, h[5].spec.width), (1, Some(3)));
        let (_, h) = parse_format("{:*^9}{:+}{:02}").unwrap();
        assert_eq!(
            (h[0].spec.fill, h[0].spec.align, h[0].spec.width),
            ('*', 3, Some(9))
        );
        assert_eq!(h[1].spec.flags, 2);
        assert_eq!((h[2].spec.flags, h[2].spec.width), (4, Some(2)));
        assert!(parse_format("{").is_err());
        assert!(parse_format("}").is_err());
        assert!(parse_format("{:1$}").is_err());
        assert!(parse_format("{:.*}").is_err());
        assert!(parse_format("{:q}").is_err());
    }
}
