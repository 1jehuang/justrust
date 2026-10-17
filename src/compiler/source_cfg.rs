//! Closed, supplied configuration predicates and bounded attribute meta syntax.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CfgSet {
    pub names: BTreeSet<String>,
    pub values: BTreeMap<String, BTreeSet<String>>,
}

impl CfgSet {
    fn usage(&self) -> (usize, usize) {
        let entries =
            self.names.len() + self.values.values().map(|v| v.len().max(1)).sum::<usize>();
        let bytes = self.names.iter().map(String::len).sum::<usize>()
            + self
                .values
                .iter()
                .map(|(k, v)| k.len() + v.iter().map(String::len).sum::<usize>())
                .sum::<usize>();
        (entries, bytes)
    }

    /// Parse a bounded user predicate. Native target/profile facts cannot be overridden.
    pub fn insert_user(&mut self, spec: &str) -> Result<(), String> {
        if spec.len() > 64 * 1024 {
            return Err("user cfg byte limit exceeded".into());
        }
        let meta = parse_meta(spec, 0).map_err(|e| e.message)?;
        if meta.name.contains("::")
            || meta.name.starts_with("target_")
            || matches!(
                meta.name.as_str(),
                "unix" | "windows" | "proc_macro" | "panic" | "debug_assertions"
            )
        {
            return Err(format!("reserved or invalid user cfg name `{}`", meta.name));
        }
        let (entries, bytes) = self.usage();
        if entries >= 128 || bytes.saturating_add(spec.len()) > 64 * 1024 {
            return Err("user cfg count/byte limit exceeded".into());
        }
        match meta.value {
            MetaValue::Word => {
                self.names.insert(meta.name);
            }
            MetaValue::String(value) => {
                self.values.entry(meta.name).or_default().insert(value);
            }
            MetaValue::List(_) | MetaValue::OtherLiteral => {
                return Err("user cfg must be a name or key=\"string\"".into());
            }
        }
        Ok(())
    }

    /// Validate public option data before it can describe a native source snapshot.
    pub fn validate_native(&self) -> Result<(), String> {
        let native = Self::native_x86_64_linux();
        let reserved = |s: &str| {
            s.starts_with("target_")
                || matches!(
                    s,
                    "unix" | "windows" | "proc_macro" | "panic" | "debug_assertions"
                )
        };
        for name in self.names.iter().chain(native.names.iter()) {
            if reserved(name) && self.names.contains(name) != native.names.contains(name) {
                return Err(format!(
                    "native cfg name `{name}` differs from pinned target/profile"
                ));
            }
        }
        for key in self.values.keys().chain(native.values.keys()) {
            if reserved(key) && self.values.get(key) != native.values.get(key) {
                return Err(format!(
                    "native cfg value `{key}` differs from pinned target/profile"
                ));
            }
        }
        if self
            .names
            .iter()
            .any(|s| reserved(s) && !native.names.contains(s))
            || self
                .values
                .keys()
                .any(|s| reserved(s) && !native.values.contains_key(s))
        {
            return Err("reserved cfg name/value form differs from native target/profile".into());
        }
        let (entries, bytes) = self.usage();
        if entries > 128 || bytes > 64 * 1024 {
            return Err("cfg count/byte limit exceeded".into());
        }
        for name in self.names.iter().chain(self.values.keys()) {
            let parsed = parse_meta(name, 0).map_err(|e| e.message)?;
            if parsed.name != *name
                || !matches!(parsed.value, MetaValue::Word)
                || name.contains("::")
            {
                return Err("invalid cfg name in source options".into());
            }
        }
        Ok(())
    }

    /// Pinned native compiler contract, never inferred from the compiler host.
    pub fn native_x86_64_linux() -> Self {
        let mut cfg = Self::default();
        cfg.names.extend(["unix".into(), "debug_assertions".into()]);
        for (key, values) in [
            ("target_arch", &["x86_64"][..]),
            ("target_os", &["linux"][..]),
            ("target_family", &["unix"][..]),
            ("target_env", &["gnu"][..]),
            ("target_vendor", &["unknown"][..]),
            ("target_abi", &[""][..]),
            ("target_pointer_width", &["64"][..]),
            ("target_endian", &["little"][..]),
            ("target_feature", &["fxsr", "sse", "sse2"][..]),
            ("target_has_atomic", &["8", "16", "32", "64", "ptr"][..]),
            (
                "target_has_atomic_equal_alignment",
                &["8", "16", "32", "64", "ptr"][..],
            ),
            ("panic", &["abort"][..]),
        ] {
            cfg.values
                .insert(key.into(), values.iter().map(|v| (*v).into()).collect());
        }
        cfg
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Meta {
    pub name: String,
    pub value: MetaValue,
    pub range: Range<usize>,
}
#[derive(Clone, Debug)]
pub(crate) enum MetaValue {
    Word,
    String(String),
    OtherLiteral,
    List(Vec<Meta>),
}
#[derive(Clone, Debug)]
pub(crate) struct MetaError {
    pub range: Range<usize>,
    pub message: String,
    pub limit: bool,
}
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 4096;

pub(crate) fn parse_meta(text: &str, base: usize) -> Result<Meta, MetaError> {
    let mut p = Parser {
        text,
        base,
        pos: 0,
        nodes: 0,
    };
    let meta = p.meta(0)?;
    p.space()?;
    if p.pos != text.len() {
        return Err(p.error("unexpected trailing attribute tokens"));
    }
    Ok(meta)
}

pub(crate) fn validate_string_literal(text: &str, base: usize) -> Result<(), MetaError> {
    let mut parser = Parser {
        text,
        base,
        pos: 0,
        nodes: 0,
    };
    parser.string()?;
    if parser.pos != text.len() {
        return Err(parser.error("unexpected string literal tokens"));
    }
    Ok(())
}

pub(crate) fn evaluate(meta: &Meta, cfg: &CfgSet) -> Result<bool, MetaError> {
    let invalid = || MetaError {
        range: meta.range.clone(),
        message: "invalid cfg predicate or combinator arity".into(),
        limit: false,
    };
    match (&*meta.name, &meta.value) {
        ("all" | "any", MetaValue::List(args)) => {
            // Validate every argument, even when the logical result is already known.
            let values = args
                .iter()
                .map(|m| evaluate(m, cfg))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if meta.name == "all" {
                values.iter().all(|v| *v)
            } else {
                values.iter().any(|v| *v)
            })
        }
        ("not", MetaValue::List(args)) if args.len() == 1 => Ok(!evaluate(&args[0], cfg)?),
        ("all" | "any" | "not", _) => Err(invalid()),
        (_, MetaValue::Word) if !meta.name.contains("::") => Ok(cfg.names.contains(&meta.name)),
        (_, MetaValue::String(value)) if !meta.name.contains("::") => Ok(cfg
            .values
            .get(&meta.name)
            .is_some_and(|s| s.contains(value))),
        _ => Err(invalid()),
    }
}

struct Parser<'a> {
    text: &'a str,
    base: usize,
    pos: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn error(&self, message: &str) -> MetaError {
        MetaError {
            range: self.base + self.pos
                ..self.base
                    + self.pos
                    + self.text[self.pos..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8),
            message: message.into(),
            limit: false,
        }
    }
    fn space(&mut self) -> Result<(), MetaError> {
        loop {
            while self
                .text
                .as_bytes()
                .get(self.pos)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.pos += 1;
            }
            if self.text[self.pos..].starts_with("//") {
                while self.pos < self.text.len() && self.text.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if self.text[self.pos..].starts_with("/*") {
                self.pos += 2;
                let mut depth = 1;
                while self.pos < self.text.len() && depth > 0 {
                    if self.text[self.pos..].starts_with("/*") {
                        depth += 1;
                        self.pos += 2;
                    } else if self.text[self.pos..].starts_with("*/") {
                        depth -= 1;
                        self.pos += 2;
                    } else {
                        self.pos += self.text[self.pos..].chars().next().unwrap().len_utf8();
                    }
                }
                if depth != 0 {
                    return Err(self.error("unterminated comment"));
                }
            } else {
                return Ok(());
            }
        }
    }
    fn eat(&mut self, token: &str) -> Result<bool, MetaError> {
        self.space()?;
        if self.text[self.pos..].starts_with(token) {
            self.pos += token.len();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn ident(&mut self) -> Result<String, MetaError> {
        self.space()?;
        let start = self.pos;
        if self.text[self.pos..].starts_with("r#") {
            self.pos += 2;
        }
        let name_start = self.pos;
        let Some(c) = self.text[self.pos..].chars().next() else {
            return Err(self.error("expected attribute identifier"));
        };
        if !(c == '_' || c.is_alphabetic()) {
            return Err(self.error("expected attribute identifier"));
        }
        self.pos += c.len_utf8();
        while let Some(c) = self.text[self.pos..].chars().next() {
            if !(c == '_' || c.is_alphanumeric()) {
                break;
            }
            self.pos += c.len_utf8();
        }
        if self.pos == name_start {
            return Err(self.error("expected attribute identifier"));
        }
        let raw = &self.text[start..self.pos];
        Ok(raw.strip_prefix("r#").unwrap_or(raw).into())
    }
    fn meta(&mut self, depth: usize) -> Result<Meta, MetaError> {
        self.space()?;
        if depth > MAX_DEPTH || self.nodes >= MAX_NODES {
            let mut e = self.error("attribute meta resource limit exceeded");
            e.limit = true;
            return Err(e);
        }
        self.nodes += 1;
        let start = self.pos;
        let mut name = self.ident()?;
        while self.eat("::")? {
            name.push_str("::");
            name.push_str(&self.ident()?);
        }
        let value = if self.eat("=")? {
            self.space()?;
            let rest = &self.text[self.pos..];
            if rest.starts_with('"') || rest.starts_with("r\"") || rest.starts_with("r#") {
                MetaValue::String(self.string()?)
            } else if rest.chars().next().is_some_and(|c| c.is_ascii_digit())
                || rest.starts_with("true")
                || rest.starts_with("false")
            {
                while self.text[self.pos..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
                {
                    self.pos += self.text[self.pos..].chars().next().unwrap().len_utf8();
                }
                MetaValue::OtherLiteral
            } else {
                return Err(self.error("expected literal attribute value"));
            }
        } else if self.eat("(")? {
            let mut args = Vec::new();
            if !self.eat(")")? {
                loop {
                    args.push(self.meta(depth + 1)?);
                    if self.eat(")")? {
                        break;
                    }
                    if !self.eat(",")? {
                        return Err(self.error("expected comma or closing parenthesis"));
                    }
                    if self.eat(")")? {
                        break;
                    }
                }
            }
            MetaValue::List(args)
        } else {
            MetaValue::Word
        };
        Ok(Meta {
            name,
            value,
            range: self.base + start..self.base + self.pos,
        })
    }
    fn string(&mut self) -> Result<String, MetaError> {
        self.space()?;
        let start = self.pos;
        let bytes = self.text.as_bytes();
        if bytes.get(self.pos) == Some(&b'r') {
            self.pos += 1;
            let hashes_start = self.pos;
            while bytes.get(self.pos) == Some(&b'#') {
                self.pos += 1;
            }
            let hashes = self.pos - hashes_start;
            if hashes > 255 {
                return Err(self.error("too many raw string hashes"));
            }
            if bytes.get(self.pos) != Some(&b'"') {
                return Err(self.error("expected string literal"));
            }
            self.pos += 1;
            let content = self.pos;
            while self.pos < bytes.len() {
                if bytes[self.pos] == b'"'
                    && bytes
                        .get(self.pos + 1..self.pos + 1 + hashes)
                        .is_some_and(|s| s.iter().all(|b| *b == b'#'))
                {
                    let content_text = &self.text[content..self.pos];
                    if content_text.as_bytes().iter().enumerate().any(|(i, b)| {
                        *b == b'\r' && content_text.as_bytes().get(i + 1) != Some(&b'\n')
                    }) {
                        return Err(self.error("bare carriage return in raw string"));
                    }
                    let out = content_text.replace("\r\n", "\n");
                    self.pos += 1 + hashes;
                    return Ok(out);
                }
                self.pos += 1;
            }
            return Err(self.error("unterminated raw string"));
        }
        if bytes.get(self.pos) != Some(&b'"') {
            return Err(self.error("expected string literal attribute value"));
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.text.len() {
            let c = self.text[self.pos..].chars().next().unwrap();
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(c) = self.text[self.pos..].chars().next() else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.pos += c.len_utf8();
                    match c {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '0' => out.push('\0'),
                        '\\' => out.push('\\'),
                        '\'' => out.push('\''),
                        '"' => out.push('"'),
                        'x' => {
                            let end = self.pos + 2;
                            let Some(s) = self.text.get(self.pos..end) else {
                                return Err(self.error("invalid hex escape"));
                            };
                            let v = u8::from_str_radix(s, 16)
                                .ok()
                                .filter(|v| *v < 128)
                                .ok_or_else(|| self.error("invalid ASCII escape"))?;
                            out.push(char::from(v));
                            self.pos = end;
                        }
                        'u' => {
                            if self.text.as_bytes().get(self.pos) != Some(&b'{') {
                                return Err(self.error("invalid Unicode escape"));
                            }
                            self.pos += 1;
                            let begin = self.pos;
                            if !self
                                .text
                                .as_bytes()
                                .get(begin)
                                .is_some_and(u8::is_ascii_hexdigit)
                            {
                                return Err(self
                                    .error("Unicode escape must begin with a hexadecimal digit"));
                            }
                            while self
                                .text
                                .as_bytes()
                                .get(self.pos)
                                .is_some_and(|b| b.is_ascii_hexdigit() || *b == b'_')
                            {
                                self.pos += 1;
                            }
                            let digits = self.text[begin..self.pos].replace('_', "");
                            if digits.is_empty()
                                || digits.len() > 6
                                || self.text.as_bytes().get(self.pos) != Some(&b'}')
                            {
                                return Err(self.error("invalid Unicode escape"));
                            }
                            let c = u32::from_str_radix(&digits, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.error("invalid Unicode scalar"))?;
                            self.pos += 1;
                            out.push(c);
                        }
                        '\n' => {
                            while self
                                .text
                                .as_bytes()
                                .get(self.pos)
                                .is_some_and(u8::is_ascii_whitespace)
                            {
                                self.pos += 1;
                            }
                        }
                        '\r' if self.text.as_bytes().get(self.pos) == Some(&b'\n') => {
                            self.pos += 1;
                            while self
                                .text
                                .as_bytes()
                                .get(self.pos)
                                .is_some_and(u8::is_ascii_whitespace)
                            {
                                self.pos += 1;
                            }
                        }
                        _ => return Err(self.error("unsupported or invalid string escape")),
                    }
                }
                '\r' if self.text.as_bytes().get(self.pos) == Some(&b'\n') => {
                    self.pos += 1;
                    out.push('\n');
                }
                '\r' => return Err(self.error("bare carriage return in string")),
                _ => out.push(c),
            }
        }
        self.pos = start;
        Err(self.error("unterminated string literal"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(s: &str, cfg: &CfgSet) -> Result<bool, MetaError> {
        evaluate(&parse_meta(s, 0)?, cfg)
    }
    #[test]
    fn supplied_names_values_and_empty_combinators() {
        let mut cfg = CfgSet::default();
        cfg.names.insert("unix".into());
        cfg.values
            .entry("feature".into())
            .or_default()
            .insert("wide".into());
        for s in [
            "unix",
            "feature = \"wide\"",
            "all()",
            "not(any())",
            "all(unix,feature=r#\"wide\"#)",
        ] {
            assert!(eval(s, &cfg).unwrap(), "{s}");
        }
        for s in ["windows", "feature=\"missing\"", "any()", "not(unix)"] {
            assert!(!eval(s, &cfg).unwrap(), "{s}");
        }
    }
    #[test]
    fn malformed_predicates_never_short_circuit_to_false() {
        for s in [
            "not()",
            "not(a,b)",
            "any(all(),not())",
            "all(any(),wat())",
            "all",
            "x=1",
            "all(x y)",
            "x trailing",
            "foo::bar",
        ] {
            assert!(eval(s, &CfgSet::default()).is_err(), "{s}");
        }
    }
    #[test]
    fn bounded_meta_and_original_byte_offsets() {
        let e = parse_meta("cfg(not()) junk", 19).unwrap_err();
        assert_eq!(e.range.start, 30);
        let deep = format!("{}a{}", "not(".repeat(70), ")".repeat(70));
        assert!(parse_meta(&deep, 0).unwrap_err().limit);
    }
    #[test]
    fn raw_strings_normalize_crlf_and_reject_bare_cr() {
        let mut cfg = CfgSet::default();
        cfg.values
            .entry("mode".into())
            .or_default()
            .insert("a\nb".into());
        assert!(eval("mode=r#\"a\r\nb\"#", &cfg).unwrap());
        assert!(parse_meta("mode=r#\"a\rb\"#", 0).is_err());
    }
    #[test]
    fn unicode_escape_requires_initial_hex_digit() {
        assert!(parse_meta(r#"mode="\u{_41}""#, 0).is_err());
        assert!(parse_meta(r#"mode="\u{4_1}""#, 0).is_ok());
    }
    #[test]
    fn malformed_unicode_spans_cover_whole_scalar_or_empty_eof() {
        let text = "cfg(a β)";
        let error = parse_meta(text, 17).unwrap_err();
        assert_eq!(&text[error.range.start - 17..error.range.end - 17], "β");
        assert!(text.is_char_boundary(error.range.start - 17));
        assert!(text.is_char_boundary(error.range.end - 17));
        let eof = parse_meta("cfg(", 17).unwrap_err();
        assert_eq!(eof.range, 21..21);
    }
    #[test]
    fn native_configuration_cannot_be_spoofed() {
        let native = CfgSet::native_x86_64_linux();
        assert!(native.validate_native().is_ok());
        assert!(CfgSet::default().validate_native().is_err());
        let mut cfg = native.clone();
        cfg.insert_user("custom").unwrap();
        cfg.insert_user("feature=\"wide\"").unwrap();
        assert!(cfg.validate_native().is_ok());
        for spec in [
            "target_arch=\"arm\"",
            "target_unknown",
            "panic=\"unwind\"",
            "debug_assertions",
            "unix",
            "windows",
            "proc_macro",
            "custom()",
            "x=42",
        ] {
            assert!(cfg.insert_user(spec).is_err(), "{spec}");
        }
        let mut cfg = native.clone();
        cfg.values
            .get_mut("target_arch")
            .unwrap()
            .insert("arm".into());
        assert!(cfg.validate_native().is_err());
        let mut cfg = native.clone();
        cfg.names.insert("target_os".into());
        assert!(cfg.validate_native().is_err());
        let mut cfg = native.clone();
        cfg.names.remove("unix");
        assert!(cfg.validate_native().is_err());
        let mut cfg = native;
        cfg.values.insert("unix".into(), BTreeSet::new());
        assert!(cfg.validate_native().is_err());
    }
    #[test]
    fn user_and_public_configuration_resources_bounded() {
        let mut cfg = CfgSet::native_x86_64_linux();
        assert!(cfg.insert_user(&"a".repeat(65537)).is_err());
        for i in 0..129 {
            let _ = cfg.insert_user(&format!("c{i}"));
        }
        assert!(cfg.insert_user("last").is_err());
        let mut cfg = CfgSet::native_x86_64_linux();
        for i in 0..129 {
            cfg.values.insert(format!("empty{i}"), BTreeSet::new());
        }
        assert!(cfg.validate_native().is_err());
    }
    #[test]
    fn string_forms_decode_equally() {
        let mut cfg = CfgSet::default();
        cfg.values
            .entry("f".into())
            .or_default()
            .insert("aβ".into());
        assert!(eval("f=\"\\x61\\u{3b2}\"", &cfg).unwrap());
        assert!(eval("f=r##\"aβ\"##", &cfg).unwrap());
    }
}
