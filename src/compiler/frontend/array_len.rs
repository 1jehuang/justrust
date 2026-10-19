//! Parse-time evaluation of array length expressions (`[T; N * 2]`).
//!
//! Lengths may name integer `const` items of the same source buffer. Only
//! items whose name is unique in the buffer (and is not also a const generic
//! parameter) are admitted, so a length can never silently bind a shadowed or
//! differently scoped declaration. Anything else falls back to the explicit
//! unsupported diagnostic in the caller.
use super::*;
use std::collections::HashMap;

/// Token index range of each candidate const item initializer, keyed by name.
/// `None` marks an ambiguous name that must never be evaluated.
pub(super) type ConstTable = HashMap<String, Option<std::ops::Range<usize>>>;

pub(super) fn scan(tokens: &[Token]) -> ConstTable {
    let mut table = ConstTable::new();
    let mut at = 0;
    while at + 2 < tokens.len() {
        let t = &tokens[at];
        let is_const = t.kind == ParserTokenKind::Ident && t.text == "const";
        let name = &tokens[at + 1];
        if !is_const || name.kind != ParserTokenKind::Ident || tokens[at + 2].text != ":" {
            at += 1;
            continue;
        }
        let mut cursor = at + 3;
        let mut depth = 0i32;
        let mut range = None;
        while cursor < tokens.len() {
            let tok = &tokens[cursor];
            if tok.kind == ParserTokenKind::Punct || tok.kind == ParserTokenKind::Ident {
                match tok.text.as_str() {
                    "(" | "[" => depth += 1,
                    ")" | "]" if depth > 0 => depth -= 1,
                    "=" if depth == 0 => {
                        let start = cursor + 1;
                        let mut end = start;
                        let mut d = 0i32;
                        while end < tokens.len() {
                            match tokens[end].text.as_str() {
                                "(" | "[" | "{" => d += 1,
                                ")" | "]" | "}" => d -= 1,
                                ";" if d == 0 => break,
                                _ => {}
                            }
                            if d < 0 {
                                break;
                            }
                            end += 1;
                        }
                        if tokens.get(end).is_some_and(|t| t.text == ";") {
                            range = Some(start..end);
                        }
                        break;
                    }
                    "," | ">" | ")" | "]" | "{" | ";" | ">>" if depth == 0 => break,
                    _ => {}
                }
            }
            if tok.kind == ParserTokenKind::Eof {
                break;
            }
            cursor += 1;
        }
        let key = name.text.clone();
        match table.get(&key) {
            Some(_) => {
                table.insert(key, None);
            }
            None => {
                table.insert(key, range);
            }
        }
        at += 2;
    }
    table
}

struct Eval<'a> {
    tokens: &'a [Token],
    table: &'a ConstTable,
    active: Vec<String>,
    fuel: usize,
}

impl Eval<'_> {
    fn expr(&mut self, range: std::ops::Range<usize>) -> Option<u128> {
        let mut at = range.start;
        let value = self.binary(&mut at, range.end, 0)?;
        (at == range.end).then_some(value)
    }
    fn prec(op: &str) -> Option<u8> {
        Some(match op {
            "|" => 1,
            "^" => 2,
            "&" => 3,
            "<<" | ">>" => 4,
            "+" | "-" => 5,
            "*" | "/" | "%" => 6,
            _ => return None,
        })
    }
    fn binary(&mut self, at: &mut usize, end: usize, min: u8) -> Option<u128> {
        let mut left = self.unary(at, end)?;
        while *at < end {
            let tok = &self.tokens[*at];
            if tok.kind != ParserTokenKind::Punct {
                break;
            }
            let Some(p) = Self::prec(&tok.text) else {
                break;
            };
            if p < min {
                break;
            }
            let op = tok.text.clone();
            *at += 1;
            let right = self.binary(at, end, p + 1)?;
            left = match op.as_str() {
                "|" => left | right,
                "^" => left ^ right,
                "&" => left & right,
                "<<" => left.checked_shl(u32::try_from(right).ok()?)?,
                ">>" => left.checked_shr(u32::try_from(right).ok()?)?,
                "+" => left.checked_add(right)?,
                "-" => left.checked_sub(right)?,
                "*" => left.checked_mul(right)?,
                "/" => left.checked_div(right)?,
                "%" => left.checked_rem(right)?,
                _ => return None,
            };
        }
        Some(left)
    }
    fn unary(&mut self, at: &mut usize, end: usize) -> Option<u128> {
        self.fuel = self.fuel.checked_sub(1)?;
        if *at >= end {
            return None;
        }
        let tok = &self.tokens[*at];
        let value = match tok.kind {
            ParserTokenKind::Number => {
                let literal = parse_integer_literal(&tok.text).ok()?;
                *at += 1;
                literal.magnitude
            }
            ParserTokenKind::Punct if tok.text == "(" => {
                *at += 1;
                let v = self.binary(at, end, 0)?;
                if self.tokens.get(*at)?.text != ")" {
                    return None;
                }
                *at += 1;
                v
            }
            ParserTokenKind::Ident => {
                let name = tok.text.clone();
                *at += 1;
                if self.active.contains(&name) || self.active.len() > 64 {
                    return None;
                }
                let range = self.table.get(&name)?.clone()?;
                self.active.push(name);
                let v = self.expr(range);
                self.active.pop();
                v?
            }
            _ => return None,
        };
        // `as usize` style casts are admitted only when lossless.
        if *at + 1 < end && self.tokens[*at].text == "as" {
            let ty = integer_type(&self.tokens[*at + 1].text)?;
            if ty.is_signed() || value > ty.mask() {
                return None;
            }
            *at += 2;
        }
        Some(value)
    }
}

/// Evaluate `tokens[range]` as a non-negative integer constant expression.
pub(super) fn evaluate(
    tokens: &[Token],
    table: &ConstTable,
    range: std::ops::Range<usize>,
) -> Option<u128> {
    Eval {
        tokens,
        table,
        active: Vec::new(),
        fuel: 4096,
    }
    .expr(range)
}

/// Index one past the array-length expression starting at `start`: the
/// position of the closing `]` at depth zero.
pub(super) fn length_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (index, tok) in tokens.iter().enumerate().skip(start) {
        if tok.kind == ParserTokenKind::Eof {
            return None;
        }
        if tok.kind != ParserTokenKind::Punct {
            continue;
        }
        match tok.text.as_str() {
            "(" | "[" | "{" => depth += 1,
            "]" if depth == 0 => return Some(index),
            ")" | "]" | "}" => depth -= 1,
            ";" | "," if depth == 0 => return None,
            _ => {}
        }
    }
    None
}
