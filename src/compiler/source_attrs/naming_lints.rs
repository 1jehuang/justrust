//! `non_camel_case_types` and `non_upper_case_globals`, checked on the
//! configured source tokens of one item when the lint is at deny/forbid.
//! Mirrors stock rustc's case rules, including scripts without case.
use super::Token;
use std::ops::Range;

#[derive(Clone, Copy, Default)]
pub(super) struct NamingLints {
    pub camel: bool,
    pub upper: bool,
}

pub(super) struct Violation {
    pub range: Range<usize>,
    pub message: String,
}

fn char_has_case(c: char) -> bool {
    !c.to_lowercase().eq(c.to_uppercase()) || c.is_uppercase() || c.is_lowercase()
}

fn changes_when_titlecased(c: char) -> bool {
    // `char::to_titlecase` is unstable; uppercase is the titlecase for every
    // cased letter except the four digraph letters, handled explicitly.
    let title: Vec<char> = match c {
        'Ǆ' | 'ǅ' | 'ǆ' => vec!['ǅ'],
        'Ǉ' | 'ǈ' | 'ǉ' => vec!['ǈ'],
        'Ǌ' | 'ǋ' | 'ǌ' => vec!['ǋ'],
        'Ǳ' | 'ǲ' | 'ǳ' => vec!['ǲ'],
        _ => c.to_uppercase().collect(),
    };
    title != [c]
}

fn not_camel_case(s: &str) -> bool {
    let mut last = '\0';
    s.chars().any(|snd| {
        let fst = std::mem::replace(&mut last, snd);
        match (fst, snd) {
            ('_', '_') => true,
            ('_', _) if char_has_case(snd) => true,
            (_, '_') if char_has_case(fst) => true,
            _ => snd.is_uppercase() && changes_when_titlecased(snd),
        }
    })
}

pub(super) fn is_upper_camel_case(name: &str) -> bool {
    let name = name.trim_matches('_');
    let Some(first) = name.chars().next() else {
        return true;
    };
    !(changes_when_titlecased(first) || not_camel_case(name))
}

pub(super) fn is_upper_case(name: &str) -> bool {
    name.chars().all(|c| c.to_uppercase().eq([c]))
}

fn ident(text: &str) -> Option<&str> {
    let name = text.strip_prefix("r#").unwrap_or(text);
    let first = name.chars().next()?;
    ((first == '_' || first.is_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_alphanumeric()))
    .then_some(name)
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

struct Scan<'a> {
    text: &'a str,
    tokens: &'a [Token],
    lints: NamingLints,
}

impl Scan<'_> {
    fn t(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |t| t.text(self.text))
    }
    fn name(&self, i: usize) -> Option<&str> {
        let raw = self.t(i);
        let name = ident(raw)?;
        (raw.starts_with("r#") || !KEYWORDS.contains(&name)).then_some(name)
    }
    fn camel(&self, i: usize, sort: &str) -> Result<(), Violation> {
        match self.name(i) {
            Some(name) if self.lints.camel && !is_upper_camel_case(name) => Err(Violation {
                range: self.tokens[i].range.clone(),
                message: format!("{sort} `{name}` should have an upper camel case name"),
            }),
            _ => Ok(()),
        }
    }
    fn upper(&self, i: usize, sort: &str) -> Result<(), Violation> {
        match self.name(i) {
            Some(name) if self.lints.upper && name != "_" && !is_upper_case(name) => {
                Err(Violation {
                    range: self.tokens[i].range.clone(),
                    message: format!("{sort} `{name}` should have an upper case name"),
                })
            }
            _ => Ok(()),
        }
    }
    /// Generic parameter list starting at `<` token `open`; returns the index after `>`.
    fn generics(&self, open: usize, end: usize) -> Result<usize, Violation> {
        let mut depth = 0usize;
        let mut i = open;
        let mut param_start = true;
        while i < end {
            let t = self.t(i);
            match t {
                "<" => {
                    depth += 1;
                    param_start = depth == 1;
                    i += 1;
                    continue;
                }
                ">" if i == 0 || !matches!(self.t(i - 1), "-" | "=") => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        return Ok(i);
                    }
                    continue;
                }
                "," if depth == 1 => {
                    param_start = true;
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if param_start && depth == 1 {
                if t == "const" {
                    self.upper(i + 1, "const parameter")?;
                } else if !t.starts_with('\'') && !t.starts_with('#') {
                    self.camel(i, "type parameter")?;
                }
            }
            param_start = false;
            i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
        }
        Ok(i)
    }
    fn variants(&self, open: usize) -> Result<(), Violation> {
        let Some(close) = self.tokens[open].close else {
            return Ok(());
        };
        let mut i = open + 1;
        let mut start = true;
        while i < close {
            let t = self.t(i);
            if t == "," {
                start = true;
                i += 1;
                continue;
            }
            if start && t == "#" {
                // `#[attr]` on a variant: skip the bracket group.
                i += 1;
                if self.t(i) == "[" {
                    i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
                }
                continue;
            }
            if start && t.starts_with("//") || start && t.starts_with("/*") {
                i += 1;
                continue;
            }
            if start {
                self.camel(i, "variant")?;
                start = false;
            }
            i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
        }
        Ok(())
    }
    fn run(&self, start: usize, end: usize) -> Result<(), Violation> {
        let mut i = start;
        while i < end {
            let prev = if i > start { self.t(i - 1) } else { "" };
            match self.t(i) {
                "struct" | "enum" | "union" | "trait" | "type" if prev != "::" => {
                    let kw = self.t(i);
                    let after = self.t(i + 2);
                    let is_item = match kw {
                        "union" => matches!(after, "{" | "<" | "where"),
                        "type" => matches!(after, "=" | ";" | ":" | "<" | "where"),
                        _ => true,
                    };
                    if is_item && self.name(i + 1).is_some() {
                        let sort = if kw == "trait" { "trait" } else { "type" };
                        self.camel(i + 1, sort)?;
                        let mut j = i + 2;
                        if self.t(j) == "<" {
                            j = self.generics(j, end)?;
                        }
                        if kw == "enum" {
                            while j < end && self.t(j) != "{" {
                                j += 1;
                            }
                            if j < end {
                                self.variants(j)?;
                            }
                        }
                        i += 2;
                        continue;
                    }
                }
                "fn" if self.name(i + 1).is_some() && self.t(i + 2) == "<" => {
                    i = self.generics(i + 2, end)?;
                    continue;
                }
                "impl" if self.t(i + 1) == "<" => {
                    i = self.generics(i + 1, end)?;
                    continue;
                }
                "static" if prev != "'" => {
                    let at = if self.t(i + 1) == "mut" { i + 2 } else { i + 1 };
                    if self.t(at + 1) == ":" {
                        self.upper(at, "static variable")?;
                    }
                }
                "const" if self.t(i + 2) == ":" => {
                    self.upper(i + 1, "constant")?;
                }
                _ => {}
            }
            i += 1;
        }
        Ok(())
    }
}

/// Check tokens `start..end` for naming lint violations.
pub(super) fn check(
    text: &str,
    tokens: &[Token],
    start: usize,
    end: usize,
    lints: NamingLints,
) -> Result<(), Violation> {
    if !lints.camel && !lints.upper {
        return Ok(());
    }
    Scan {
        text,
        tokens,
        lints,
    }
    .run(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_case_rules() {
        for ok in [
            "X",
            "_X",
            "X_",
            "__",
            "Θχ",
            "ヒa",
            "_ヒb",
            "ΦΙΛΟΣ",
            "Ab1",
            "A1_2",
        ] {
            assert!(is_upper_camel_case(ok), "{ok}");
        }
        for bad in [
            "χa",
            "__χa",
            "对__否",
            "ヒ__χ",
            "Hello_你好",
            "Hello_World",
            "a",
            "ΦΙΛΟΣ_Σ",
        ] {
            assert!(!is_upper_camel_case(bad), "{bad}");
        }
        for ok in ["ラ", "__密__封__线__内__禁__止__答__题__", "FOO_1"] {
            assert!(is_upper_case(ok), "{ok}");
        }
        for bad in ["τεχ", "ძალა_ერთობაშია", "ǋ", "foo"] {
            assert!(!is_upper_case(bad), "{bad}");
        }
    }
}
