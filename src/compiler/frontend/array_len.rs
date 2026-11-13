//! Parse-time evaluation of array length expressions (`[T; N * 2]`).
//!
//! Lengths are typed and checked like stock const evaluation: integer
//! literals take their type from suffixes or context (defaulting to `i32`),
//! arithmetic overflow, division by zero, oversized shifts and failing
//! `ilog`/`pow` calls are E0080, and a final type other than `usize` is
//! E0308. Admitted operands are integer and bool literals, integer `const`
//! items of the same source buffer, primitive `MAX`/`MIN`/`BITS`,
//! `size_of`/`align_of` of primitive, pointer, array and `NonNull` types,
//! casts between integers and from bool, comparisons and the logical
//! operators, and the `ilog10`/`ilog2`/`pow` methods.
//!
//! Name binding is conservative: a const item is admitted only when its name
//! is unique in the buffer (and is not also a const generic parameter), and a
//! primitive or layout path is admitted only when no source item in the
//! buffer could shadow it. Anything else is `Unsupported` and falls back to
//! the explicit diagnostic in the caller.
use super::*;
use std::collections::HashMap;

/// A candidate const item: token ranges of its declared type and initializer.
#[derive(Clone)]
pub(super) struct ConstItem {
    ty: std::ops::Range<usize>,
    init: std::ops::Range<usize>,
}

/// Candidate const items keyed by name. `None` marks a name that is
/// ambiguous or has no evaluable initializer and must never be evaluated.
pub(super) type ConstTable = HashMap<String, Option<ConstItem>>;

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
        let mut item = None;
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
                            item = Some(ConstItem {
                                ty: at + 3..cursor,
                                init: start..end,
                            });
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
        // `const N: usize` generic parameters have no initializer; callers
        // reject lengths naming an in-scope const parameter separately.
        if item.is_none() {
            at += 2;
            continue;
        }
        match table.get(&key) {
            Some(_) => {
                table.insert(key, None);
            }
            None => {
                table.insert(key, item);
            }
        }
        at += 2;
    }
    table
}

/// Why a length was not evaluated.
pub(super) enum Failure {
    /// Outside the admitted subset; the caller reports it as unsupported.
    Unsupported,
    /// A definite language error (E0080, E0308, ...) with its message.
    Invalid(String),
}

type R<T> = Result<T, Failure>;

fn invalid<T>(message: String) -> R<T> {
    Err(Failure::Invalid(message))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Int(ir::IntTy),
    Bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl Op {
    fn parse(text: &str) -> Option<(u8, Op)> {
        Some(match text {
            "||" => (1, Op::Or),
            "&&" => (2, Op::And),
            "==" => (3, Op::Eq),
            "!=" => (3, Op::Ne),
            "<" => (3, Op::Lt),
            "<=" => (3, Op::Le),
            ">" => (3, Op::Gt),
            ">=" => (3, Op::Ge),
            "|" => (4, Op::BitOr),
            "^" => (5, Op::BitXor),
            "&" => (6, Op::BitAnd),
            "<<" => (7, Op::Shl),
            ">>" => (7, Op::Shr),
            "+" => (8, Op::Add),
            "-" => (8, Op::Sub),
            "*" => (9, Op::Mul),
            "/" => (9, Op::Div),
            "%" => (9, Op::Rem),
            _ => return None,
        })
    }
    fn spelling(self) -> &'static str {
        match self {
            Op::Or => "||",
            Op::And => "&&",
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::BitOr => "|",
            Op::BitXor => "^",
            Op::BitAnd => "&",
            Op::Shl => "<<",
            Op::Shr => ">>",
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Rem => "%",
        }
    }
    fn comparison(self) -> bool {
        matches!(self, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge)
    }
}

#[derive(Clone, Copy)]
enum Method {
    Ilog10,
    Ilog2,
    Pow,
}

enum CExpr {
    Lit(u128, Option<ir::IntTy>),
    Bool(bool),
    Const(String),
    /// `u8::MAX`, `i32::MIN`, `u64::BITS`.
    Assoc(ir::IntTy, &'static str),
    /// A computed `size_of`/`align_of` result (always `usize`).
    Layout(u128),
    Method(Box<CExpr>, Method, Option<Box<CExpr>>),
    Neg(Box<CExpr>),
    Not(Box<CExpr>),
    Bin(Op, Box<CExpr>, Box<CExpr>),
    Cast(Box<CExpr>, ir::IntTy),
}

const PRIMITIVES: [&str; 17] = [
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64", "bool", "char", "str",
];

struct Eval<'a> {
    tokens: &'a [Token],
    table: &'a ConstTable,
    active: Vec<String>,
    fuel: usize,
    depth: usize,
}

impl Eval<'_> {
    fn tick(&mut self) -> R<()> {
        self.fuel = self.fuel.checked_sub(1).ok_or(Failure::Unsupported)?;
        Ok(())
    }

    fn text(&self, at: usize) -> &str {
        self.tokens.get(at).map_or("", |t| t.text.as_str())
    }

    /// True when some source item in the buffer declares `name` as a type,
    /// module, function or macro and could shadow a built-in meaning.
    fn shadowed(&self, name: &str) -> bool {
        let mut in_use = false;
        for (index, token) in self.tokens.iter().enumerate() {
            match token.text.as_str() {
                "use" if token.kind == ParserTokenKind::Ident => in_use = true,
                ";" => in_use = false,
                _ => {}
            }
            if index == 0 || token.text != name || token.kind != ParserTokenKind::Ident {
                continue;
            }
            let before = &self.tokens[index - 1];
            if before.kind != ParserTokenKind::Ident {
                continue;
            }
            match before.text.as_str() {
                "struct" | "enum" | "union" | "type" | "mod" | "trait" | "fn" | "crate" => {
                    return true;
                }
                // A `use` rename (`use x as u8;`), not a cast.
                "as" if in_use => return true,
                _ => {}
            }
        }
        false
    }

    fn declared_static(&self, name: &str) -> bool {
        self.tokens.windows(3).any(|w| {
            w[0].text == "static"
                && (w[1].text == name || (w[1].text == "mut" && w[2].text == name))
        })
    }

    // ---- parsing -------------------------------------------------------

    fn parse_range(&mut self, range: std::ops::Range<usize>) -> R<CExpr> {
        let mut at = range.start;
        let expr = self.binary(&mut at, range.end, 0)?;
        if at != range.end {
            return Err(Failure::Unsupported);
        }
        Ok(expr)
    }

    fn binary(&mut self, at: &mut usize, end: usize, min: u8) -> R<CExpr> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Failure::Unsupported);
        }
        let mut left = self.unary(at, end)?;
        while *at < end {
            self.tick()?;
            let tok = &self.tokens[*at];
            if tok.kind == ParserTokenKind::Ident && tok.text == "as" {
                if min > 10 {
                    break;
                }
                let target = integer_type(self.text(*at + 1)).ok_or(Failure::Unsupported)?;
                if self.shadowed(self.text(*at + 1)) {
                    return Err(Failure::Unsupported);
                }
                *at += 2;
                left = CExpr::Cast(Box::new(left), target);
                continue;
            }
            if tok.kind != ParserTokenKind::Punct {
                break;
            }
            let Some((precedence, op)) = Op::parse(&tok.text) else {
                break;
            };
            if precedence < min {
                break;
            }
            *at += 1;
            // Comparisons are non-associative; others are left-associative.
            let right = self.binary(at, end, precedence + 1)?;
            if op.comparison() && *at < end {
                if let Some((3, _)) = Op::parse(self.text(*at)) {
                    return Err(Failure::Unsupported);
                }
            }
            left = CExpr::Bin(op, Box::new(left), Box::new(right));
        }
        self.depth -= 1;
        Ok(left)
    }

    fn unary(&mut self, at: &mut usize, end: usize) -> R<CExpr> {
        self.tick()?;
        if *at >= end {
            return Err(Failure::Unsupported);
        }
        let tok = &self.tokens[*at];
        let mut value = match (&tok.kind, tok.text.as_str()) {
            (ParserTokenKind::Punct, "-") => {
                *at += 1;
                return Ok(CExpr::Neg(Box::new(self.unary(at, end)?)));
            }
            (ParserTokenKind::Punct, "!") => {
                *at += 1;
                return Ok(CExpr::Not(Box::new(self.unary(at, end)?)));
            }
            (ParserTokenKind::Number, text) => {
                let literal = parse_integer_literal(text).map_err(|_| Failure::Unsupported)?;
                *at += 1;
                CExpr::Lit(literal.magnitude, literal.suffix)
            }
            (ParserTokenKind::Punct, "(") => {
                *at += 1;
                let inner = self.binary(at, end, 0)?;
                if *at >= end || self.text(*at) != ")" {
                    return Err(Failure::Unsupported);
                }
                *at += 1;
                inner
            }
            (ParserTokenKind::Ident, "true") => {
                *at += 1;
                CExpr::Bool(true)
            }
            (ParserTokenKind::Ident, "false") => {
                *at += 1;
                CExpr::Bool(false)
            }
            (ParserTokenKind::Ident, _) => self.path_expr(at, end)?,
            _ => return Err(Failure::Unsupported),
        };
        // Postfix method calls bind tighter than unary operators and casts.
        while *at + 3 < end && self.text(*at) == "." {
            let method = match self.text(*at + 1) {
                "ilog10" => Method::Ilog10,
                "ilog2" => Method::Ilog2,
                "pow" => Method::Pow,
                _ => return Err(Failure::Unsupported),
            };
            if self.text(*at + 2) != "(" {
                return Err(Failure::Unsupported);
            }
            *at += 3;
            let argument = if matches!(method, Method::Pow) {
                Some(Box::new(self.binary(at, end, 0)?))
            } else {
                None
            };
            if *at >= end || self.text(*at) != ")" {
                return Err(Failure::Unsupported);
            }
            *at += 1;
            value = CExpr::Method(Box::new(value), method, argument);
        }
        Ok(value)
    }

    fn path_expr(&mut self, at: &mut usize, end: usize) -> R<CExpr> {
        let mut segments = vec![self.text(*at).to_string()];
        *at += 1;
        while *at + 1 < end
            && self.text(*at) == "::"
            && self.tokens[*at + 1].kind == ParserTokenKind::Ident
        {
            segments.push(self.text(*at + 1).to_string());
            *at += 2;
        }
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        match segments.as_slice() {
            [name] if *name != "size_of" && *name != "align_of" => {
                if PRIMITIVES.contains(name) {
                    return Err(Failure::Unsupported);
                }
                Ok(CExpr::Const((*name).to_string()))
            }
            [ty, item] if matches!(*item, "MAX" | "MIN" | "BITS") => {
                let ty = integer_type(ty).ok_or(Failure::Unsupported)?;
                if self.shadowed(segments[0]) {
                    return Err(Failure::Unsupported);
                }
                Ok(CExpr::Assoc(
                    ty,
                    match *item {
                        "MAX" => "MAX",
                        "MIN" => "MIN",
                        _ => "BITS",
                    },
                ))
            }
            _ => {
                let align = match segments.as_slice() {
                    [.., "size_of"] => false,
                    [.., "align_of"] => true,
                    _ => return Err(Failure::Unsupported),
                };
                let prefix = &segments[..segments.len() - 1];
                let admitted = match prefix {
                    // The 2024 prelude name, unless imported from elsewhere.
                    [] => self.only_imports_from("mem", segments[0]),
                    ["mem"] => {
                        // `mem` must name core's module through an import.
                        !self.shadowed("mem") && self.only_imports_from_root("mem")
                    }
                    ["core" | "std", "mem"] => !self.shadowed(prefix[0]),
                    _ => false,
                };
                if !admitted
                    || self.shadowed(segments[segments.len() - 1])
                    || self.text(*at) != "::"
                    || self.text(*at + 1) != "<"
                {
                    return Err(Failure::Unsupported);
                }
                *at += 2;
                let mut pending = 0usize;
                let (size, alignment) = self.layout(at, end, &mut pending)?;
                match (pending, self.text(*at)) {
                    (0, ">") => *at += 1,
                    (1, _) => {}
                    (0, ">>") => {
                        return Err(Failure::Unsupported);
                    }
                    _ => return Err(Failure::Unsupported),
                }
                if self.text(*at) != "(" || self.text(*at + 1) != ")" || *at + 2 > end {
                    return Err(Failure::Unsupported);
                }
                *at += 2;
                Ok(CExpr::Layout(if align { alignment } else { size }))
            }
        }
    }

    /// Size and alignment of a written type. `pending` counts `>` tokens
    /// already consumed from a split `>>` that close enclosing generics.
    fn layout(&mut self, at: &mut usize, end: usize, pending: &mut usize) -> R<(u128, u128)> {
        self.tick()?;
        self.depth += 1;
        if self.depth > 64 || *at >= end {
            return Err(Failure::Unsupported);
        }
        let text = self.text(*at).to_string();
        let result = match text.as_str() {
            "*" => {
                if !matches!(self.text(*at + 1), "const" | "mut") {
                    return Err(Failure::Unsupported);
                }
                *at += 2;
                self.layout(at, end, pending)?;
                (8, 8)
            }
            "&" => {
                *at += 1;
                if self.tokens.get(*at).map(|t| &t.kind) == Some(&ParserTokenKind::Lifetime) {
                    *at += 1;
                }
                if self.text(*at) == "mut" {
                    *at += 1;
                }
                // Unsized referents (`str`, slices) are fat; not admitted.
                if self.text(*at) == "str" {
                    return Err(Failure::Unsupported);
                }
                if self.text(*at) == "[" {
                    let mut probe = *at + 1;
                    let mut p = 0;
                    self.layout(&mut probe, end, &mut p)?;
                    if self.text(probe) != ";" {
                        return Err(Failure::Unsupported);
                    }
                }
                self.layout(at, end, pending)?;
                (8, 8)
            }
            "[" => {
                *at += 1;
                let mut inner = 0;
                let (size, alignment) = self.layout(at, end, &mut inner)?;
                if inner != 0 || self.text(*at) != ";" {
                    return Err(Failure::Unsupported);
                }
                *at += 1;
                let close = length_end(self.tokens, *at).ok_or(Failure::Unsupported)?;
                if close >= end {
                    return Err(Failure::Unsupported);
                }
                let count = self.usize_expr(*at..close)?;
                *at = close + 1;
                let size = size.checked_mul(count).ok_or(Failure::Unsupported)?;
                (size, alignment)
            }
            "(" if self.text(*at + 1) == ")" => {
                *at += 2;
                (0, 1)
            }
            _ => {
                let mut segments = vec![text.clone()];
                let mut cursor = *at + 1;
                while self.text(cursor) == "::"
                    && self
                        .tokens
                        .get(cursor + 1)
                        .is_some_and(|t| t.kind == ParserTokenKind::Ident)
                {
                    segments.push(self.text(cursor + 1).to_string());
                    cursor += 2;
                }
                let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
                match segments.as_slice() {
                    [name] if !self.shadowed(name) => {
                        let scalar = match *name {
                            "bool" => Some((1, 1)),
                            "char" | "f32" => Some((4, 4)),
                            "f64" => Some((8, 8)),
                            other => integer_type(other).map(|ty| {
                                let n = u128::from(ty.bits() / 8);
                                (n, n)
                            }),
                        };
                        if let Some(scalar) = scalar {
                            *at = cursor;
                            scalar
                        } else if *name == "NonNull" && self.non_null_imported() {
                            *at = cursor;
                            self.non_null_argument(at, end, pending)?
                        } else {
                            return Err(Failure::Unsupported);
                        }
                    }
                    ["ptr", "NonNull"] | ["core" | "std", "ptr", "NonNull"]
                        if !self.shadowed("NonNull")
                            && !self.shadowed(segments[0])
                            && (segments.len() == 3 || self.ptr_imported()) =>
                    {
                        *at = cursor;
                        self.non_null_argument(at, end, pending)?
                    }
                    _ => return Err(Failure::Unsupported),
                }
            }
        };
        self.depth -= 1;
        Ok(result)
    }

    fn non_null_argument(
        &mut self,
        at: &mut usize,
        end: usize,
        pending: &mut usize,
    ) -> R<(u128, u128)> {
        if self.text(*at) != "<" {
            return Err(Failure::Unsupported);
        }
        *at += 1;
        let mut inner = 0;
        self.layout(at, end, &mut inner)?;
        if inner == 1 {
            // The pointee's `>>` also closed this `NonNull<...>`.
        } else if inner != 0 {
            return Err(Failure::Unsupported);
        } else {
            match self.text(*at) {
                ">" => *at += 1,
                ">>" => {
                    *at += 1;
                    *pending += 1;
                }
                _ => return Err(Failure::Unsupported),
            }
        }
        if *pending > 1 {
            return Err(Failure::Unsupported);
        }
        Ok((8, 8))
    }

    /// Every path or group occurrence of `name` is a `core::mem`/`std::mem`
    /// import: `mem::name` or inside a `mem::{...}` group.
    fn only_imports_from(&self, module: &str, name: &str) -> bool {
        let mut group = false;
        for (index, token) in self.tokens.iter().enumerate() {
            match token.text.as_str() {
                "{" => {
                    group = index >= 4
                        && self.text(index - 3) == "::"
                        && matches!(self.text(index - 4), "core" | "std")
                        && index >= 2
                        && self.text(index - 1) == "::"
                        && self.text(index - 2) == module;
                }
                "}" | ";" => group = false,
                _ if token.text == name && index > 0 => {
                    let before = self.text(index - 1);
                    let ok = match before {
                        "::" => {
                            index >= 4
                                && self.text(index - 2) == module
                                && self.text(index - 3) == "::"
                                && matches!(self.text(index - 4), "core" | "std")
                        }
                        "{" | "," => group,
                        _ => true,
                    };
                    if !ok {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// Every `::module` path occurrence is rooted at `core` or `std`.
    fn only_imports_from_root(&self, module: &str) -> bool {
        self.tokens.iter().enumerate().all(|(index, token)| {
            token.text != module
                || index < 2
                || self.text(index - 1) != "::"
                || matches!(self.text(index - 2), "core" | "std")
        }) && self.tokens.windows(3).any(|w| {
            matches!(w[0].text.as_str(), "core" | "std") && w[1].text == "::" && w[2].text == module
        })
    }

    fn non_null_imported(&self) -> bool {
        self.only_imports_from("ptr", "NonNull")
            && self.only_imports_from_root("ptr")
            && self
                .tokens
                .windows(2)
                .any(|w| w[0].text == "ptr" && matches!(w[1].text.as_str(), "::"))
    }

    fn ptr_imported(&self) -> bool {
        !self.shadowed("ptr") && self.only_imports_from_root("ptr")
    }

    // ---- typing and evaluation ----------------------------------------

    fn usize_expr(&mut self, range: std::ops::Range<usize>) -> R<u128> {
        let expr = self.parse_range(range)?;
        let usize = Ty::Int(ir::IntTy::Usize);
        match self.ty(&expr)? {
            None => {}
            Some(found) if found == usize => {}
            Some(found) => {
                return invalid(format!(
                    "E0308: mismatched types: expected `usize`, found `{}`",
                    show(found)
                ));
            }
        }
        self.eval(&expr, usize)
    }

    fn const_item(&mut self, name: &str) -> R<(Ty, ConstItem)> {
        let item = self
            .table
            .get(name)
            .cloned()
            .flatten()
            .ok_or(Failure::Unsupported)?;
        if item.ty.len() != 1 || self.shadowed(name) || self.declared_static(name) {
            return Err(Failure::Unsupported);
        }
        let spelled = self.text(item.ty.start);
        if self.shadowed(spelled) {
            return Err(Failure::Unsupported);
        }
        let ty = match spelled {
            "bool" => Ty::Bool,
            other => Ty::Int(integer_type(other).ok_or(Failure::Unsupported)?),
        };
        Ok((ty, item))
    }

    /// The type of `expr`, or `None` for an unconstrained integer literal.
    fn ty(&mut self, expr: &CExpr) -> R<Option<Ty>> {
        self.tick()?;
        Ok(match expr {
            CExpr::Lit(_, suffix) => suffix.map(Ty::Int),
            CExpr::Bool(_) => Some(Ty::Bool),
            CExpr::Const(name) => Some(self.const_item(name)?.0),
            CExpr::Assoc(_, "BITS") => Some(Ty::Int(ir::IntTy::U32)),
            CExpr::Assoc(ty, _) => Some(Ty::Int(*ty)),
            CExpr::Layout(_) => Some(Ty::Int(ir::IntTy::Usize)),
            CExpr::Method(receiver, method, argument) => {
                let Some(Ty::Int(ty)) = self.ty(receiver)? else {
                    // `{integer}` receivers need inference (E0689).
                    return Err(Failure::Unsupported);
                };
                match method {
                    Method::Ilog10 | Method::Ilog2 => Some(Ty::Int(ir::IntTy::U32)),
                    Method::Pow => {
                        let argument = argument.as_ref().ok_or(Failure::Unsupported)?;
                        match self.ty(argument)? {
                            None | Some(Ty::Int(ir::IntTy::U32)) => {}
                            Some(other) => {
                                return invalid(format!(
                                    "E0308: mismatched types: expected `u32`, found `{}`",
                                    show(other)
                                ));
                            }
                        }
                        Some(Ty::Int(ty))
                    }
                }
            }
            CExpr::Neg(inner) => match self.ty(inner)? {
                Some(Ty::Bool) => {
                    return invalid("E0600: cannot apply unary operator `-` to type `bool`".into());
                }
                Some(Ty::Int(ty)) if !ty.is_signed() => {
                    return invalid(format!(
                        "E0600: cannot apply unary operator `-` to type `{}`",
                        show(Ty::Int(ty))
                    ));
                }
                other => other,
            },
            CExpr::Not(inner) => self.ty(inner)?,
            CExpr::Cast(inner, target) => {
                self.ty(inner)?;
                Some(Ty::Int(*target))
            }
            CExpr::Bin(op, left, right) => {
                let l = self.ty(left)?;
                let r = self.ty(right)?;
                match op {
                    Op::And | Op::Or => {
                        for side in [l, r] {
                            if side != Some(Ty::Bool) {
                                return invalid(format!(
                                    "E0308: mismatched types: expected `bool`, found `{}`",
                                    side.map_or("integer".into(), show)
                                ));
                            }
                        }
                        Some(Ty::Bool)
                    }
                    Op::Shl | Op::Shr => {
                        if l == Some(Ty::Bool) || r == Some(Ty::Bool) {
                            return invalid(format!(
                                "E0369: no implementation for `bool {} _`",
                                op.spelling()
                            ));
                        }
                        l
                    }
                    _ => {
                        let unified = match (l, r) {
                            (Some(a), Some(b)) if a != b => {
                                return invalid(format!(
                                    "E0308: mismatched types: expected `{}`, found `{}`",
                                    show(a),
                                    show(b)
                                ));
                            }
                            (Some(Ty::Bool), None) | (None, Some(Ty::Bool)) => {
                                return invalid(
                                    "E0308: mismatched types: expected `bool`, found integer"
                                        .into(),
                                );
                            }
                            (a, b) => a.or(b),
                        };
                        if op.comparison() {
                            Some(Ty::Bool)
                        } else {
                            if unified == Some(Ty::Bool)
                                && !matches!(op, Op::BitAnd | Op::BitOr | Op::BitXor)
                            {
                                return invalid(format!(
                                    "E0369: no implementation for `bool {} bool`",
                                    op.spelling()
                                ));
                            }
                            unified
                        }
                    }
                }
            }
        })
    }

    /// Evaluate `expr`, using `want` for unconstrained integer literals.
    /// Returns the value's bits truncated to its type width.
    fn eval(&mut self, expr: &CExpr, want: Ty) -> R<u128> {
        self.tick()?;
        self.depth += 1;
        if self.depth > 256 {
            return Err(Failure::Unsupported);
        }
        let value = match expr {
            CExpr::Lit(magnitude, suffix) => {
                let ty = match suffix.map(Ty::Int).unwrap_or(want) {
                    Ty::Int(ty) => ty,
                    Ty::Bool => {
                        return invalid(
                            "E0308: mismatched types: expected `bool`, found integer".into(),
                        );
                    }
                };
                literal(*magnitude, ty, false)?
            }
            CExpr::Bool(value) => u128::from(*value),
            CExpr::Const(name) => {
                let (ty, item) = self.const_item(name)?;
                if self.active.contains(name) || self.active.len() > 64 {
                    return Err(Failure::Unsupported);
                }
                let init = self.parse_range(item.init.clone())?;
                match self.ty(&init)? {
                    None => {}
                    Some(found) if found == ty => {}
                    Some(found) => {
                        return invalid(format!(
                            "E0308: mismatched types: expected `{}`, found `{}`",
                            show(ty),
                            show(found)
                        ));
                    }
                }
                self.active.push(name.clone());
                let value = self.eval(&init, ty);
                self.active.pop();
                value?
            }
            CExpr::Assoc(ty, "BITS") => u128::from(ty.bits()),
            CExpr::Assoc(ty, "MAX") => {
                if ty.is_signed() {
                    ty.mask() >> 1
                } else {
                    ty.mask()
                }
            }
            CExpr::Assoc(ty, _) => {
                if ty.is_signed() {
                    (ty.mask() >> 1) + 1
                } else {
                    0
                }
            }
            CExpr::Layout(value) => {
                if *value > ir::IntTy::Usize.mask() {
                    return Err(Failure::Unsupported);
                }
                *value
            }
            CExpr::Method(receiver, method, argument) => {
                let Some(Ty::Int(ty)) = self.ty(receiver)? else {
                    return Err(Failure::Unsupported);
                };
                let bits = self.eval(receiver, Ty::Int(ty))?;
                if !ty.is_signed() && bits > i128::MAX as u128 {
                    return Err(Failure::Unsupported);
                }
                let value = signed(bits, ty);
                match method {
                    Method::Ilog10 | Method::Ilog2 => {
                        if value <= 0 {
                            return invalid(
                                "E0080: evaluation panicked: argument of integer logarithm must be positive"
                                    .into(),
                            );
                        }
                        let value = value as u128;
                        u128::from(if matches!(method, Method::Ilog10) {
                            value.ilog10()
                        } else {
                            value.ilog2()
                        })
                    }
                    Method::Pow => {
                        let argument = argument.as_ref().ok_or(Failure::Unsupported)?;
                        let exponent = self.eval(argument, Ty::Int(ir::IntTy::U32))?;
                        let mut acc: i128 = 1;
                        for _ in 0..exponent.min(256) {
                            self.tick()?;
                            acc = acc
                                .checked_mul(value)
                                .filter(|v| fits(*v, ty))
                                .ok_or_else(|| {
                                    Failure::Invalid(
                                        "E0080: evaluation panicked: attempt to exponentiate with overflow"
                                            .into(),
                                    )
                                })?;
                            if acc == 0 || acc == 1 {
                                break;
                            }
                        }
                        acc as u128 & ty.mask()
                    }
                }
            }
            CExpr::Neg(inner) => {
                let ty = match self.ty(inner)?.unwrap_or(want) {
                    Ty::Int(ty) => ty,
                    Ty::Bool => {
                        return invalid(
                            "E0600: cannot apply unary operator `-` to type `bool`".into(),
                        );
                    }
                };
                if !ty.is_signed() {
                    return invalid(format!(
                        "E0600: cannot apply unary operator `-` to type `{}`",
                        show(Ty::Int(ty))
                    ));
                }
                if let CExpr::Lit(magnitude, _) = inner.as_ref() {
                    literal(*magnitude, ty, true)?
                } else {
                    let value = signed(self.eval(inner, Ty::Int(ty))?, ty);
                    let negated =
                        value
                            .checked_neg()
                            .filter(|v| fits(*v, ty))
                            .ok_or_else(|| {
                                Failure::Invalid(format!(
                                    "E0080: attempt to negate `{}::MIN`, which would overflow",
                                    show(Ty::Int(ty))
                                ))
                            })?;
                    negated as u128 & ty.mask()
                }
            }
            CExpr::Not(inner) => match self.ty(inner)?.unwrap_or(want) {
                Ty::Bool => u128::from(self.eval(inner, Ty::Bool)? == 0),
                Ty::Int(ty) => !self.eval(inner, Ty::Int(ty))? & ty.mask(),
            },
            CExpr::Cast(inner, target) => {
                let source = match self.ty(inner)? {
                    Some(ty) => ty,
                    // Rustc types a bare literal cast operand as the target.
                    None if matches!(inner.as_ref(), CExpr::Lit(..))
                        || matches!(inner.as_ref(), CExpr::Neg(v) if matches!(v.as_ref(), CExpr::Lit(..))) =>
                    {
                        Ty::Int(*target)
                    }
                    None => Ty::Int(ir::IntTy::I32),
                };
                let bits = self.eval(inner, source)?;
                match source {
                    Ty::Bool => bits,
                    Ty::Int(ty) => signed(bits, ty) as u128 & target.mask(),
                }
            }
            CExpr::Bin(op, left, right) => self.binary_value(*op, left, right, want)?,
        };
        self.depth -= 1;
        Ok(value)
    }

    fn binary_value(&mut self, op: Op, left: &CExpr, right: &CExpr, want: Ty) -> R<u128> {
        let l = self.ty(left)?;
        let r = self.ty(right)?;
        match op {
            Op::And => {
                // Short-circuit like stock evaluation.
                if self.eval(left, Ty::Bool)? == 0 {
                    return Ok(0);
                }
                return self.eval(right, Ty::Bool);
            }
            Op::Or => {
                if self.eval(left, Ty::Bool)? != 0 {
                    return Ok(1);
                }
                return self.eval(right, Ty::Bool);
            }
            Op::Shl | Op::Shr => {
                let Ty::Int(ty) = l.unwrap_or(want) else {
                    return Err(Failure::Unsupported);
                };
                let Ty::Int(shift_ty) = r.unwrap_or(Ty::Int(ir::IntTy::I32)) else {
                    return Err(Failure::Unsupported);
                };
                let value = self.eval(left, Ty::Int(ty))?;
                let amount = signed(self.eval(right, Ty::Int(shift_ty))?, shift_ty);
                if amount < 0 || amount >= i128::from(ty.bits()) {
                    return invalid(format!(
                        "E0080: attempt to shift {} by `{amount}`, which would overflow",
                        if op == Op::Shl { "left" } else { "right" }
                    ));
                }
                let amount = amount as u32;
                return Ok(if op == Op::Shl {
                    (value << amount) & ty.mask()
                } else if ty.is_signed() {
                    (signed(value, ty) >> amount) as u128 & ty.mask()
                } else {
                    value >> amount
                });
            }
            _ => {}
        }
        let ty = match l.or(r) {
            Some(ty) => ty,
            None if op.comparison() => Ty::Int(ir::IntTy::I32),
            None => want,
        };
        let a = self.eval(left, ty)?;
        let b = self.eval(right, ty)?;
        let Ty::Int(int) = ty else {
            return Ok(match op {
                Op::Eq => u128::from(a == b),
                Op::Ne => u128::from(a != b),
                Op::Lt => u128::from(a < b),
                Op::Le => u128::from(a <= b),
                Op::Gt => u128::from(a > b),
                Op::Ge => u128::from(a >= b),
                Op::BitAnd => a & b,
                Op::BitOr => a | b,
                Op::BitXor => a ^ b,
                _ => return Err(Failure::Unsupported),
            });
        };
        if !int.is_signed() && (a > i128::MAX as u128 || b > i128::MAX as u128) {
            // Only u128 reaches here; compare and compute on raw bits.
            return Ok(match op {
                Op::Eq => u128::from(a == b),
                Op::Ne => u128::from(a != b),
                Op::Lt => u128::from(a < b),
                Op::Le => u128::from(a <= b),
                Op::Gt => u128::from(a > b),
                Op::Ge => u128::from(a >= b),
                Op::BitAnd => a & b,
                Op::BitOr => a | b,
                Op::BitXor => a ^ b,
                Op::Add => a.checked_add(b).ok_or(Failure::Unsupported)?,
                Op::Sub => a.checked_sub(b).ok_or(Failure::Unsupported)?,
                Op::Mul => a.checked_mul(b).ok_or(Failure::Unsupported)?,
                Op::Div | Op::Rem if b == 0 => return Err(Failure::Unsupported),
                Op::Div => a / b,
                Op::Rem => a % b,
                _ => return Err(Failure::Unsupported),
            });
        }
        let (sa, sb) = (signed(a, int), signed(b, int));
        let overflow = |verb: &str| {
            Failure::Invalid(format!(
                "E0080: attempt to {verb} `{}_{} {} {}_{}`, which would overflow",
                sa,
                show(ty),
                op.spelling(),
                sb,
                show(ty)
            ))
        };
        Ok(match op {
            Op::Eq => u128::from(sa == sb),
            Op::Ne => u128::from(sa != sb),
            Op::Lt => u128::from(sa < sb),
            Op::Le => u128::from(sa <= sb),
            Op::Gt => u128::from(sa > sb),
            Op::Ge => u128::from(sa >= sb),
            Op::BitAnd => a & b,
            Op::BitOr => a | b,
            Op::BitXor => a ^ b,
            Op::Div | Op::Rem if sb == 0 => {
                return invalid(format!(
                    "E0080: attempt to {} `{sa}_{}` {} zero",
                    if op == Op::Div {
                        "divide"
                    } else {
                        "calculate the remainder of"
                    },
                    show(ty),
                    if op == Op::Div {
                        "by"
                    } else {
                        "with a divisor of"
                    }
                ));
            }
            _ => {
                let result = {
                    match op {
                        Op::Add => sa.checked_add(sb),
                        Op::Sub => sa.checked_sub(sb),
                        Op::Mul => sa.checked_mul(sb),
                        Op::Div => sa.checked_div(sb),
                        Op::Rem => sa.checked_rem(sb),
                        _ => return Err(Failure::Unsupported),
                    }
                };
                let result = result
                    .filter(|v| fits(*v, int))
                    .ok_or_else(|| overflow("compute"))?;
                result as u128 & int.mask()
            }
        })
    }
}

fn show(ty: Ty) -> String {
    match ty {
        Ty::Bool => "bool".into(),
        Ty::Int(ty) => format!("{ty:?}").to_lowercase(),
    }
}

/// The value of `bits` (truncated to `ty`) as a signed integer. `u128`
/// values above `i128::MAX` are never produced through this path.
fn signed(bits: u128, ty: ir::IntTy) -> i128 {
    if ty.is_signed() {
        let shift = 128 - ty.bits();
        ((bits << shift) as i128) >> shift
    } else {
        bits as i128
    }
}

fn fits(value: i128, ty: ir::IntTy) -> bool {
    if ty.is_signed() {
        let bound = 1i128 << (ty.bits() - 1).min(126);
        if ty.bits() == 128 {
            return true;
        }
        value >= -bound && value < bound
    } else {
        value >= 0 && (ty.bits() == 128 || value as u128 <= ty.mask())
    }
}

/// A literal of `magnitude` (negated when `negative`) typed `ty`.
fn literal(magnitude: u128, ty: ir::IntTy, negative: bool) -> R<u128> {
    let limit = if ty.is_signed() {
        let sign = 1u128 << (ty.bits() - 1);
        if negative { sign } else { sign - 1 }
    } else {
        ty.mask()
    };
    if magnitude > limit {
        // `overflowing_literals` is a lint that may be allowed; leave it to
        // the ordinary literal checks rather than guessing its level here.
        return Err(Failure::Unsupported);
    }
    Ok(if negative {
        magnitude.wrapping_neg() & ty.mask()
    } else {
        magnitude
    })
}

/// Evaluate `tokens[range]` as a `usize` array length.
pub(super) fn evaluate(
    tokens: &[Token],
    table: &ConstTable,
    range: std::ops::Range<usize>,
) -> Result<u128, Failure> {
    let mut eval = Eval {
        tokens,
        table,
        active: Vec::new(),
        fuel: 4096,
        depth: 0,
    };
    eval.usize_expr(range)
}

/// Evaluate a scalar const item initializer `tokens[range]` of the declared
/// type spelled by the single token `ty`. `Ok(None)` when `ty` is not an
/// integer or `bool`. The result is the value's bits truncated to `ty`.
pub(super) fn evaluate_item(
    tokens: &[Token],
    table: &ConstTable,
    ty: &str,
    range: std::ops::Range<usize>,
) -> Result<Option<(Option<ir::IntTy>, u128)>, Failure> {
    let mut eval = Eval {
        tokens,
        table,
        active: Vec::new(),
        fuel: 4096,
        depth: 0,
    };
    if eval.shadowed(ty) {
        return Err(Failure::Unsupported);
    }
    let declared = match ty {
        "bool" => Ty::Bool,
        other => match integer_type(other) {
            Some(int) => Ty::Int(int),
            None => return Ok(None),
        },
    };
    let expr = eval.parse_range(range)?;
    match eval.ty(&expr)? {
        None if declared != Ty::Bool => {}
        Some(found) if found == declared => {}
        Some(found) => {
            return invalid(format!(
                "E0308: mismatched types: expected `{}`, found `{}`",
                show(declared),
                show(found)
            ));
        }
        None => return invalid("E0308: mismatched types: expected `bool`, found integer".into()),
    }
    let value = eval.eval(&expr, declared)?;
    Ok(Some((
        match declared {
            Ty::Int(int) => Some(int),
            Ty::Bool => None,
        },
        value,
    )))
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

impl Parser<'_, '_> {
    /// Fold a scalar const item initializer spanning tokens `start..end`
    /// (excluding `;`) into a literal when the typed evaluator computes it,
    /// so operations the scalar graph CTFE path cannot express (casts,
    /// layout queries, `ilog10`, comparisons of constants) still get a
    /// checked value. Definite evaluation errors are reported here even for
    /// unused constants, as stock rustc does. Otherwise the original
    /// initializer is kept for the ordinary paths.
    pub(super) fn fold_scalar_const_initializer(
        &self,
        ty: &Type,
        start: Option<usize>,
        initializer: Expr,
    ) -> Result<Expr, ParseError> {
        let spelled = match ty {
            Type::Bool => "bool",
            Type::Int(int) if *int != ir::IntTy::Isize => int_spelling(*int),
            _ => return Ok(initializer),
        };
        if super::root_resolution::scalar_simple(&initializer)
            || self.purpose == ParserPurpose::InactiveSyntax
        {
            return Ok(initializer);
        }
        let (Some(start), Some(end)) = (start, self.cursor.index()) else {
            return Ok(initializer);
        };
        let tokens: &[Token] = self.input;
        let table = scan(tokens);
        match evaluate_item(tokens, &table, spelled, start..end) {
            Ok(Some((Some(int), bits))) => {
                let negative = int.is_signed() && (bits >> (int.bits() - 1)) & 1 == 1;
                let magnitude = if negative {
                    bits.wrapping_neg() & int.mask()
                } else {
                    bits
                };
                let literal = Expr::Integer(IntegerLiteral {
                    magnitude,
                    suffix: Some(int),
                });
                Ok(if negative {
                    Expr::Unary {
                        op: UnaryOp::Neg,
                        value: Box::new(literal),
                        site: None,
                    }
                } else {
                    literal
                })
            }
            Ok(Some((None, bits))) => Ok(Expr::Bool(bits != 0)),
            Ok(None) | Err(Failure::Unsupported) => Ok(initializer),
            Err(Failure::Invalid(message)) => Err(self.error(&message)),
        }
    }
}

fn int_spelling(ty: ir::IntTy) -> &'static str {
    match ty {
        ir::IntTy::I8 => "i8",
        ir::IntTy::I16 => "i16",
        ir::IntTy::I32 => "i32",
        ir::IntTy::I64 => "i64",
        ir::IntTy::I128 => "i128",
        ir::IntTy::Isize => "isize",
        ir::IntTy::U8 => "u8",
        ir::IntTy::U16 => "u16",
        ir::IntTy::U32 => "u32",
        ir::IntTy::U64 => "u64",
        ir::IntTy::U128 => "u128",
        ir::IntTy::Usize => "usize",
    }
}
