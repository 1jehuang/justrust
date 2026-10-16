//! Native, deliberately restricted Rust frontend. No external compiler is invoked.
//!
//! Syntax diagnostics have one-based line/column positions. The span-free public
//! AST means semantic diagnostics identify functions and names instead of spans.
//! Explicit semicolons discard expression values, while omitted-semicolon
//! blocklike statements require unit. While loops do not establish return flow.
//! Numeric literals support primitive integers through 64 bits.
//! Inline modules, named nonempty braced structs, nonempty unit/tuple/braced enums, ordinary visibility, and
//! simple module-level imports have separate type/value namespaces. Named
//! structs and enums are nominal and non-Copy. Match admits scalar literals,
//! top-level variants with irrefutable payload patterns and whole-value
//! bindings/wildcards with finite exhaustiveness. Nested enum patterns require
//! a single-variant enum. Tuple-variant rest patterns remain unsupported.
//! Move checking belongs to CFG analysis, not
//! this semantic checker. External modules, restricted
//! visibility, reexports, grouped/glob imports, and block-local items are not
//! supported. An executable requires a declared root main, not an imported alias.
use super::ir;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
}
/// Syntax-only result. Resolve this once before publishing callable signatures.
#[derive(Clone, Debug)]
pub struct ParsedProgram {
    pub program: Program,
    items: HashMap<String, Vec<Item>>,
}
#[derive(Clone, Debug)]
struct Item {
    public: bool,
    kind: ItemKind,
}
#[derive(Clone, Debug)]
enum ItemKind {
    Module,
    Function,
    Struct,
    Enum,
    Variant(ir::VariantShape),
    Import(String),
}

fn qualify(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_owned()
    } else {
        format!("{module}::{name}")
    }
}
fn parent(path: &str) -> &str {
    path.rsplit_once("::").map_or("", |(p, _)| p)
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub regions: RegionDecls,
    pub params: Vec<(String, Type)>,
    pub result: Type,
    pub body: Block,
}
#[derive(Clone, Debug, Default)]
pub struct RegionDecls {
    pub params: Vec<String>,
    pub outlives: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: String,
    pub public: bool,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: String,
    pub public: bool,
    pub variants: Vec<VariantDecl>,
}
#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: String,
    pub shape: ir::VariantShape,
    pub fields: Vec<FieldDecl>,
}
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Type,
    pub public: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldSelector {
    Deref,
    ConstantIndex(usize),
    Tuple(usize),
    Named(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Ref {
        kind: ir::BorrowKind,
        region: Option<String>,
        referent: Box<Type>,
    },
    Array(Box<Type>, usize),
    Int(ir::IntTy),
    Bool,
    Unit,
    Tuple(Vec<Type>),
    Named(String),
    /// Semantic-only never type, not an admitted source type annotation.
    Never,
    /// Unannotated locals only, resolved independently during function checking.
    Infer,
}
#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}
#[derive(Clone, Debug)]
pub struct Place {
    pub local: String,
    pub fields: Vec<FieldSelector>,
}
#[derive(Clone, Debug)]
pub enum Pattern {
    Variant {
        name: String,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Pattern)>,
        rest: bool,
    },
    Path(String),
    Bool(bool),
    Int {
        literal: IntegerLiteral,
        negative: bool,
    },
    Wildcard,
    Binding {
        name: String,
        mutable: bool,
    },
    Tuple(Vec<Pattern>),
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
        rest: bool,
    },
}
#[derive(Clone, Debug)]
pub enum Stmt {
    Declare {
        name: String,
        ty: Type,
        mutable: bool,
    },
    Let {
        name: String,
        mutable: bool,
        ty: Type,
        value: Expr,
    },
    LetPattern {
        pattern: Pattern,
        ty: Type,
        value: Expr,
    },
    Assign {
        name: String,
        value: Expr,
    },
    AssignIndex {
        array: Place,
        index: Expr,
        value: Expr,
    },
    AssignOp {
        place: Place,
        op: BinaryOp,
        value: Expr,
    },
    AssignIndexOp {
        array: Place,
        index: Expr,
        op: BinaryOp,
        value: Expr,
    },
    AssignPlace {
        place: Place,
        value: Expr,
    },
    Expr(Expr),
    /// A blocklike statement without an explicit value-discarding semicolon.
    ImplicitExpr(Expr),
    Return(Option<Expr>),
    While {
        condition: Expr,
        body: Block,
    },
}
/// Source literal magnitude, before contextual signedness and range checking.
#[derive(Clone, Debug)]
pub struct IntegerLiteral {
    pub magnitude: u64,
    pub suffix: Option<ir::IntTy>,
}

fn integer_type(name: &str) -> Option<ir::IntTy> {
    use ir::IntTy::*;
    Some(match name {
        "i8" => I8,
        "i16" => I16,
        "i32" => I32,
        "i64" => I64,
        "isize" => Isize,
        "u8" => U8,
        "u16" => U16,
        "u32" => U32,
        "u64" => U64,
        "usize" => Usize,
        _ => return None,
    })
}

fn parse_integer_literal(text: &str) -> Result<IntegerLiteral, String> {
    let (radix, start) = if text.starts_with("0x") {
        (16, 2)
    } else if text.starts_with("0o") {
        (8, 2)
    } else if text.starts_with("0b") {
        (2, 2)
    } else {
        (10, 0)
    };
    let bytes = text.as_bytes();
    let mut at = start;
    let mut magnitude = 0u64;
    let mut digit_seen = false;
    while let Some(&b) = bytes.get(at) {
        if b == b'_' {
            at += 1;
            continue;
        }
        let Some(digit) = (b as char).to_digit(radix) else {
            break;
        };
        magnitude = magnitude
            .checked_mul(u64::from(radix))
            .and_then(|n| n.checked_add(u64::from(digit)))
            .ok_or("integer literal exceeds u64")?;
        digit_seen = true;
        at += 1;
    }
    if !digit_seen {
        return Err("integer literal requires a digit".into());
    }
    let suffix = if at == text.len() {
        None
    } else {
        Some(integer_type(&text[at..]).ok_or("unsupported or invalid integer literal suffix")?)
    };
    Ok(IntegerLiteral { magnitude, suffix })
}

fn resolve_integer(
    literal: &IntegerLiteral,
    negative: bool,
    expected: Option<&ir::Ty>,
) -> Result<ir::IntValue, String> {
    let contextual = match expected {
        Some(ir::Ty::Int(ty)) => Some(*ty),
        _ => None,
    };
    let ty = literal.suffix.or(contextual).unwrap_or(ir::IntTy::I32);
    if literal.suffix.is_some() && contextual.is_some_and(|t| t != ty) {
        return Err("integer literal suffix does not match expected integer type".into());
    }
    if negative && !ty.is_signed() {
        return Err("unsigned integer negation is invalid".into());
    }
    let limit = if ty.is_signed() {
        let sign = 1u64 << (ty.bits() - 1);
        if negative { sign } else { sign - 1 }
    } else {
        ty.mask()
    };
    if literal.magnitude > limit {
        return Err(format!(
            "integer literal out of range for {}; backward numeric inference is unsupported",
            ty.name()
        ));
    }
    let bits = if negative {
        0u64.wrapping_sub(literal.magnitude) & ty.mask()
    } else {
        literal.magnitude
    };
    let value = ir::IntValue { ty, bits };
    value.validate()?;
    Ok(value)
}

impl Expr {
    /// Construct a source signed literal for hand-built frontend fixtures.
    pub fn i32(value: i32) -> Self {
        let literal = Self::Integer(IntegerLiteral {
            magnitude: u64::from(value.unsigned_abs()),
            suffix: Some(ir::IntTy::I32),
        });
        if value < 0 {
            Self::Unary {
                op: UnaryOp::Neg,
                value: Box::new(literal),
            }
        } else {
            literal
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Cast {
        value: Box<Expr>,
        target: Type,
    },
    Borrow {
        kind: ir::BorrowKind,
        place: Place,
    },
    Deref {
        value: Box<Expr>,
    },
    Integer(IntegerLiteral),
    DynamicIndex {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Array(Vec<Expr>),
    Repeat {
        value: Box<Expr>,
        len: usize,
    },
    Index {
        base: Box<Expr>,
        index: usize,
    },
    EnumVariant {
        name: String,
        shape: ir::VariantShape,
        fields: Vec<(FieldSelector, Expr)>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Tuple(Vec<Expr>),
    Struct {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    Field {
        base: Box<Expr>,
        field: FieldSelector,
    },

    Bool(bool),
    Var(String),
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    If {
        condition: Box<Expr>,
        then_block: Block,
        else_block: Block,
    },
    Block(Block),
    Loop {
        body: Block,
    },
    Break {
        value: Option<Box<Expr>>,
    },
    Continue,
    Println(String),
}
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
}
#[derive(Clone, Copy, Debug)]
pub enum UnaryOp {
    Neg,
    Not,
}
#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    line: usize,
    column: usize,
    string: bool,
}
fn compound_operator(token: &Token) -> Option<BinaryOp> {
    if token.string {
        return None;
    }
    Some(match token.text.as_str() {
        "+=" => BinaryOp::Add,
        "-=" => BinaryOp::Sub,
        "*=" => BinaryOp::Mul,
        "/=" => BinaryOp::Div,
        "%=" => BinaryOp::Rem,
        "&=" => BinaryOp::BitAnd,
        "|=" => BinaryOp::BitOr,
        "^=" => BinaryOp::BitXor,
        "<<=" => BinaryOp::Shl,
        ">>=" => BinaryOp::Shr,
        _ => return None,
    })
}
fn lex(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let (mut i, mut line, mut column) = (0, 1, 1);
    let mut tokens = Vec::new();
    fn advance(chars: &[char], i: &mut usize, line: &mut usize, column: &mut usize) {
        if chars[*i] == '\n' {
            *line += 1;
            *column = 1;
        } else {
            *column += 1;
        }
        *i += 1;
    }
    while i < chars.len() {
        let c = chars[i];
        if matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\r'
                | '\u{000b}'
                | '\u{000c}'
                | '\u{0085}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            advance(&chars, &mut i, &mut line, &mut column);
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                advance(&chars, &mut i, &mut line, &mut column);
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = (line, column);
            let mut depth = 1;
            advance(&chars, &mut i, &mut line, &mut column);
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    advance(&chars, &mut i, &mut line, &mut column);
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    advance(&chars, &mut i, &mut line, &mut column);
                }
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if depth != 0 {
                return Err(format!("{}:{}: unterminated comment", start.0, start.1));
            }
            continue;
        }
        let (sl, sc) = (line, column);
        let mut text = String::new();
        let string = c == '"';
        if string {
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && chars[i] != '"' {
                let mut ch = chars[i];
                if ch == '\r' {
                    if chars.get(i + 1) != Some(&'\n') {
                        return Err(format!("{line}:{column}: bare carriage return in string"));
                    }
                    advance(&chars, &mut i, &mut line, &mut column);
                    ch = '\n';
                }
                if ch == '\\' {
                    advance(&chars, &mut i, &mut line, &mut column);
                    ch = match chars.get(i) {
                        Some('n') => '\n',
                        Some('r') => '\r',
                        Some('t') => '\t',
                        Some('0') => '\0',
                        Some('"') => '"',
                        Some('\'') => '\'',
                        Some('\\') => '\\',
                        Some('x') => {
                            advance(&chars, &mut i, &mut line, &mut column);
                            let hi =
                                chars.get(i).and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                    format!("{line}:{column}: expected two hex digits")
                                })?;
                            advance(&chars, &mut i, &mut line, &mut column);
                            let lo =
                                chars.get(i).and_then(|c| c.to_digit(16)).ok_or_else(|| {
                                    format!("{line}:{column}: expected two hex digits")
                                })?;
                            let value = hi * 16 + lo;
                            if value > 127 {
                                return Err(format!(
                                    "{line}:{column}: hex string escape must be ASCII"
                                ));
                            }
                            char::from_u32(value).unwrap()
                        }
                        Some('u') => {
                            advance(&chars, &mut i, &mut line, &mut column);
                            if chars.get(i) != Some(&'{') {
                                return Err(format!(
                                    "{line}:{column}: expected unicode escape brace"
                                ));
                            }
                            advance(&chars, &mut i, &mut line, &mut column);
                            let mut value = 0u32;
                            let mut digits = 0;
                            while let Some(&c) = chars.get(i) {
                                if c == '}' {
                                    break;
                                }
                                if c == '_' && digits > 0 {
                                    advance(&chars, &mut i, &mut line, &mut column);
                                    continue;
                                }
                                let digit = c.to_digit(16).ok_or_else(|| {
                                    format!("{line}:{column}: invalid unicode escape")
                                })?;
                                digits += 1;
                                if digits > 6 {
                                    return Err(format!(
                                        "{line}:{column}: unicode escape exceeds six digits"
                                    ));
                                }
                                value = value * 16 + digit;
                                advance(&chars, &mut i, &mut line, &mut column);
                            }
                            if digits == 0 || chars.get(i) != Some(&'}') {
                                return Err(format!(
                                    "{line}:{column}: unterminated unicode escape"
                                ));
                            }
                            char::from_u32(value)
                                .ok_or_else(|| format!("{line}:{column}: invalid unicode scalar"))?
                        }
                        _ => return Err(format!("{line}:{column}: unsupported string escape")),
                    };
                }
                text.push(ch);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if i == chars.len() {
                return Err(format!("{sl}:{sc}: unterminated string"));
            }
            advance(&chars, &mut i, &mut line, &mut column);
        } else if c == '\'' {
            text.push(c);
            advance(&chars, &mut i, &mut line, &mut column);
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if text.len() == 1 || chars.get(i) == Some(&'\'') {
                return Err(format!("{sl}:{sc}: character literals are unsupported"));
            }
        } else if c.is_ascii_alphabetic() || c == '_' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
            }
        } else {
            if !c.is_ascii() {
                return Err(format!(
                    "{line}:{column}: non-ASCII identifiers are unsupported"
                ));
            }
            text.push(c);
            advance(&chars, &mut i, &mut line, &mut column);
            if let Some(&next) = chars.get(i)
                && matches!(
                    (c, next),
                    ('-', '>')
                        | (':', ':')
                        | ('=', '=')
                        | ('!', '=')
                        | ('<', '=')
                        | ('>', '=')
                        | ('<', '<')
                        | ('>', '>')
                        | ('&', '&')
                        | ('|', '|')
                        | ('=', '>')
                        | ('%', '=')
                        | ('+', '=')
                        | ('-', '=')
                        | ('*', '=')
                        | ('/', '=')
                        | ('&', '=')
                        | ('|', '=')
                        | ('^', '=')
                        | ('.', '.')
                )
            {
                text.push(next);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if matches!(text.as_str(), "<<" | ">>") && chars.get(i) == Some(&'=') {
                text.push('=');
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if !matches!(
                text.as_str(),
                "(" | ")"
                    | "{"
                    | "}"
                    | "["
                    | "]"
                    | ":"
                    | "::"
                    | ";"
                    | ","
                    | "."
                    | ".."
                    | "->"
                    | "=>"
                    | "+"
                    | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "="
                    | "!"
                    | "=="
                    | "!="
                    | "<"
                    | "<="
                    | ">"
                    | ">="
                    | "<<"
                    | ">>"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "&="
                    | "|="
                    | "^="
                    | "<<="
                    | ">>="
                    | "&"
                    | "|"
                    | "^"
                    | "&&"
                    | "||"
            ) {
                return Err(format!("{sl}:{sc}: unsupported token {text:?}"));
            }
        }
        tokens.push(Token {
            text,
            line: sl,
            column: sc,
            string,
        });
    }
    tokens.push(Token {
        text: String::new(),
        line,
        column,
        string: false,
    });
    Ok(tokens)
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    nesting: usize,
    allow_struct: bool,
}
impl Parser {
    fn token(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn is(&self, s: &str) -> bool {
        !self.token().string && self.token().text == s
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn error(&self, message: &str) -> String {
        format!(
            "{}:{}: {message}, found {:?}",
            self.token().line,
            self.token().column,
            self.token().text
        )
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {s:?}")))
        }
    }
    fn ident(&mut self) -> Result<String, String> {
        let t = self.token();
        let reserved = [
            "fn", "let", "mut", "if", "else", "while", "return", "true", "false", "i32", "bool",
            "pub", "const", "static", "loop", "break", "continue", "for", "in", "match", "struct",
            "enum", "use", "mod", "impl", "trait", "self", "Self", "as", "async", "await", "move",
            "unsafe", "where", "type", "ref", "extern", "crate", "super", "dyn", "abstract",
            "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized",
            "virtual", "yield", "try", "gen",
        ];
        if !t.string
            && t.text != "_"
            && t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && !reserved.contains(&t.text.as_str())
        {
            let name = t.text.clone();
            self.at += 1;
            Ok(name)
        } else {
            Err(self.error("expected ASCII identifier"))
        }
    }
    fn ty(&mut self) -> Result<Type, String> {
        if self.nesting >= 128 {
            return Err(self.error("type nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.ty_inner();
        self.nesting -= 1;
        result
    }
    fn ty_inner(&mut self) -> Result<Type, String> {
        if self.token().string {
            return Err(self.error("quoted tokens are not types"));
        }
        if self.is("&&") {
            return Err(self.error("nested reference types are unsupported in B1"));
        }
        if self.is("_") || self.is("!") {
            return Err(self.error("inferred and never source type annotations are unsupported"));
        }
        if self.eat("&") {
            let region = if self.token().text.starts_with('\'') {
                Some(self.lifetime()?)
            } else {
                None
            };
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            return Ok(Type::Ref {
                kind,
                region,
                referent: Box::new(self.ty()?),
            });
        }
        if self.eat("[") {
            let element = self.ty()?;
            self.expect(";")?;
            let len = self.array_number("array length")?;
            self.expect("]")?;
            Ok(Type::Array(Box::new(element), len))
        } else if let Some(ty) = integer_type(&self.token().text) {
            self.at += 1;
            Ok(Type::Int(ty))
        } else if self.eat("bool") {
            Ok(Type::Bool)
        } else if self.eat("(") {
            if self.eat(")") {
                return Ok(Type::Unit);
            }
            let first = self.ty()?;
            if !self.eat(",") {
                self.expect(")")?;
                return Ok(first);
            }
            let mut fields = vec![first];
            while !self.is(")") {
                fields.push(self.ty()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            Ok(Type::Tuple(fields))
        } else {
            self.path().map(Type::Named)
        }
    }
    fn path(&mut self) -> Result<String, String> {
        let mut parts = Vec::new();
        if self.is("crate") || self.is("self") || self.is("super") {
            let first = self.token().text.clone();
            self.at += 1;
            parts.push(first.clone());
            if first == "super" {
                while self.is("::")
                    && self
                        .tokens
                        .get(self.at + 1)
                        .is_some_and(|t| !t.string && t.text == "super")
                {
                    self.at += 2;
                    parts.push("super".into());
                }
            }
        } else {
            parts.push(self.ident()?);
        }
        while self.eat("::") {
            if self.is("*") || self.is("{") {
                return Err(self.error("glob and grouped paths are unsupported"));
            }
            parts.push(self.ident()?);
        }
        Ok(parts.join("::"))
    }
    fn array_number(&mut self, context: &str) -> Result<usize, String> {
        let token = self.token();
        if token.string {
            return Err(self.error(&format!("{context} requires an integer literal")));
        }
        let literal = parse_integer_literal(&token.text).map_err(|e| self.error(&e))?;
        let value = resolve_integer(&literal, false, Some(&ir::Ty::Int(ir::IntTy::Usize)))
            .map_err(|e| self.error(&e))?;
        if value.bits > ir::MAX_ARRAY_LEN as u64 {
            return Err(self.error("array length/index exceeds supported limit"));
        }
        let number = value.bits as usize;
        self.at += 1;
        if !self.is("]") {
            return Err(self.error(
                "const-expression array lengths/indices are unsupported; use a decimal literal",
            ));
        }
        Ok(number)
    }
    fn items(&mut self, module: &str, parsed: &mut ParsedProgram) -> Result<(), String> {
        if self.nesting >= 128 {
            return Err(self.error("module nesting exceeds limit 128"));
        }
        self.nesting += 1;
        while !self.is("") && !self.is("}") {
            let public = self.eat("pub");
            if public && self.is("(") {
                return Err(self.error("restricted pub visibility is unsupported"));
            }
            let (name, kind) = if self.eat("mod") {
                let name = self.ident()?;
                let full = qualify(module, &name);
                if self.eat(";") {
                    return Err(self.error("external module files are unsupported; use inline mod"));
                }
                self.expect("{")?;
                // Register before parsing descendants, including empty modules.
                self.insert_item(parsed, &full, public, ItemKind::Module)?;
                self.items(&full, parsed)?;
                self.expect("}")?;
                continue;
            } else if self.eat("use") {
                if public {
                    return Err(self.error("pub use reexports are unsupported"));
                }
                let path = self.path()?;
                let name = if self.eat("as") {
                    self.ident()?
                } else {
                    let name = path.rsplit("::").next().unwrap();
                    if matches!(name, "crate" | "self" | "super") {
                        return Err(self.error("module keyword imports require an explicit alias"));
                    }
                    name.to_owned()
                };
                self.expect(";")?;
                (name, ItemKind::Import(path))
            } else if self.eat("enum") {
                let name = self.ident()?;
                if self.is("<") {
                    return Err(self.error("generic enums are unsupported"));
                }
                self.expect("{")?;
                let mut variants: Vec<VariantDecl> = Vec::new();
                while !self.is("}") {
                    let variant = self.ident()?;
                    let mut fields = Vec::new();
                    let shape = if self.eat("(") {
                        while !self.is(")") {
                            if fields.len() >= 4096 {
                                return Err(self.error("variant field count exceeds limit 4096"));
                            }
                            if self.is("pub") {
                                return Err(self
                                    .error("enum variant fields cannot have explicit visibility"));
                            }
                            fields.push(FieldDecl {
                                name: fields.len().to_string(),
                                ty: self.ty()?,
                                public: true,
                            });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect(")")?;
                        ir::VariantShape::Tuple
                    } else if self.eat("{") {
                        while !self.is("}") {
                            if fields.len() >= 4096 {
                                return Err(self.error("variant field count exceeds limit 4096"));
                            }
                            if self.is("pub") {
                                return Err(self
                                    .error("enum variant fields cannot have explicit visibility"));
                            }
                            let name = self.ident()?;
                            self.expect(":")?;
                            fields.push(FieldDecl {
                                name,
                                ty: self.ty()?,
                                public: true,
                            });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        ir::VariantShape::Struct
                    } else {
                        ir::VariantShape::Unit
                    };
                    if self.is("=") {
                        return Err(self.error("explicit enum discriminants are unsupported"));
                    }
                    if variants.iter().any(|v| v.name == variant) {
                        return Err(self.error("duplicate enum variant"));
                    }
                    if variants.len() >= 4096 {
                        return Err(self.error("enum variant count exceeds limit 4096"));
                    }
                    self.insert_item(
                        parsed,
                        &qualify(&qualify(module, &name), &variant),
                        true,
                        ItemKind::Variant(shape),
                    )?;
                    variants.push(VariantDecl {
                        name: variant,
                        shape,
                        fields,
                    });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                if variants.is_empty() {
                    return Err(self.error("empty enums are unsupported"));
                }
                parsed.program.enums.push(EnumDecl {
                    name: qualify(module, &name),
                    public,
                    variants,
                });
                (name, ItemKind::Enum)
            } else if self.eat("struct") {
                let name = self.ident()?;
                self.expect("{")?;
                let mut fields = Vec::new();
                while !self.is("}") {
                    let public = self.eat("pub");
                    if public && self.is("(") {
                        return Err(self.error("restricted field visibility is unsupported"));
                    }
                    let name = self.ident()?;
                    self.expect(":")?;
                    fields.push(FieldDecl {
                        name,
                        ty: self.ty()?,
                        public,
                    });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                if fields.is_empty() {
                    return Err(self.error("empty structs are unsupported"));
                }
                parsed.program.structs.push(StructDecl {
                    name: qualify(module, &name),
                    public,
                    fields,
                });
                (name, ItemKind::Struct)
            } else if self.is("fn") {
                let mut function = self.function()?;
                let name = function.name.clone();
                function.name = qualify(module, &name);
                parsed.program.functions.push(function);
                (name, ItemKind::Function)
            } else {
                return Err(self.error("expected fn, inline mod or simple use item"));
            };
            self.insert_item(parsed, &qualify(module, &name), public, kind)?;
        }
        self.nesting -= 1;
        Ok(())
    }
    fn insert_item(
        &self,
        parsed: &mut ParsedProgram,
        path: &str,
        public: bool,
        kind: ItemKind,
    ) -> Result<(), String> {
        let bindings = parsed.items.entry(path.to_owned()).or_default();
        if bindings.iter().any(|item| {
            if matches!(kind, ItemKind::Enum)
                && matches!(
                    item.kind,
                    ItemKind::Module | ItemKind::Struct | ItemKind::Enum
                )
                || matches!(item.kind, ItemKind::Enum)
                    && matches!(kind, ItemKind::Module | ItemKind::Struct)
            {
                return true;
            }
            matches!(
                (&item.kind, &kind),
                (ItemKind::Module, ItemKind::Module)
                    | (ItemKind::Module, ItemKind::Struct)
                    | (ItemKind::Struct, ItemKind::Module)
                    | (ItemKind::Struct, ItemKind::Struct)
                    | (ItemKind::Function, ItemKind::Function)
            )
        }) {
            return Err(self.error(&format!("duplicate item {path}")));
        }
        bindings.push(Item { public, kind });
        Ok(())
    }
    fn lifetime(&mut self) -> Result<String, String> {
        if self.token().string {
            return Err(self.error("expected named lifetime, not a string"));
        }
        let text = self.token().text.clone();
        let name = text
            .strip_prefix('\'')
            .ok_or_else(|| self.error("expected named lifetime"))?;
        if name == "static" || name == "_" {
            return Err(self.error("static and inferred lifetimes are unsupported"));
        }
        if [
            "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
            "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "struct", "super", "trait", "true", "type", "unsafe", "use",
            "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final",
            "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
        ]
        .contains(&name)
        {
            return Err(self.error("keyword lifetime names are unsupported"));
        }
        if !name
            .as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            return Err(self.error("expected named ASCII lifetime"));
        }
        self.at += 1;
        Ok(text)
    }
    fn region_bounds(&mut self, long: &str, regions: &mut RegionDecls) -> Result<(), String> {
        if self.eat(":") {
            loop {
                let short = self.lifetime()?;
                if regions.outlives.len() >= 4096 {
                    return Err(self.error("lifetime bounds exceed supported limit 4096"));
                }
                regions.outlives.push((long.to_owned(), short));
                if !self.eat("+") {
                    break;
                }
            }
        }
        Ok(())
    }
    fn function(&mut self) -> Result<Function, String> {
        self.expect("fn")?;
        let name = self.ident()?;
        let mut regions = RegionDecls::default();
        if self.eat("<") {
            if self.is(">") {
                return Err(self.error("empty generic parameter lists are unsupported"));
            }
            loop {
                if regions.params.len() >= 64 {
                    return Err(self.error("declared lifetimes exceed supported limit 64"));
                }
                if !self.token().text.starts_with('\'') {
                    return Err(self.error("type and const generic parameters are unsupported"));
                }
                let region = self.lifetime()?;
                regions.params.push(region.clone());
                self.region_bounds(&region, &mut regions)?;
                if !self.eat(",") || self.is(">") {
                    break;
                }
            }
            self.expect(">")?;
        }
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.is(")") {
            loop {
                let n = self.ident()?;
                self.expect(":")?;
                params.push((n, self.ty()?));
                if !self.eat(",") || self.is(")") {
                    break;
                }
            }
        }
        self.expect(")")?;
        let result = if self.eat("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        if self.eat("where") {
            loop {
                let long = self.lifetime()?;
                if !self.is(":") {
                    return Err(self.error("only lifetime outlives where bounds are supported"));
                }
                self.region_bounds(&long, &mut regions)?;
                if !self.eat(",") || self.is("{") {
                    break;
                }
            }
        }
        Ok(Function {
            name,
            regions,
            params,
            result,
            body: self.block()?,
        })
    }
    fn block(&mut self) -> Result<Block, String> {
        if self.nesting >= 128 {
            return Err(self.error("syntax nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let previous = self.allow_struct;
        self.allow_struct = true;
        let result = self.block_inner();
        self.allow_struct = previous;
        self.nesting -= 1;
        result
    }
    fn pattern(&mut self) -> Result<Pattern, String> {
        if self.nesting >= 128 {
            return Err(self.error("pattern nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.pattern_inner();
        self.nesting -= 1;
        result
    }
    fn pattern_inner(&mut self) -> Result<Pattern, String> {
        if self.token().string {
            return Err(self.error("string literal patterns are unsupported"));
        }
        if self.is("&") || self.is("&&") {
            return Err(self.error("reference patterns and match ergonomics are unsupported in B1"));
        }
        if self.eat("true") {
            return Ok(Pattern::Bool(true));
        }
        if self.eat("false") {
            return Ok(Pattern::Bool(false));
        }
        if self.is("-") || self.token().text.starts_with(|c: char| c.is_ascii_digit()) {
            let negative = self.eat("-");
            if self.token().string {
                return Err(self.error("expected an integer literal after minus"));
            }
            let literal = parse_integer_literal(&self.token().text).map_err(|e| self.error(&e))?;
            self.at += 1;
            return Ok(Pattern::Int { literal, negative });
        }
        if self.eat("_") {
            return Ok(Pattern::Wildcard);
        }
        if self.eat("(") {
            if self.eat(")") {
                return Ok(Pattern::Tuple(Vec::new()));
            }
            let first = self.pattern()?;
            if !self.eat(",") {
                self.expect(")")?;
                return Ok(first);
            }
            let mut fields = vec![first];
            while !self.is(")") {
                if self.is("..") {
                    return Err(self.error("tuple rest patterns are unsupported"));
                }
                fields.push(self.pattern()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            return Ok(Pattern::Tuple(fields));
        }
        if self.eat("mut") {
            return Ok(Pattern::Binding {
                name: self.ident()?,
                mutable: true,
            });
        }
        let name = self.path()?;
        if self.eat("(") {
            let mut fields = Vec::new();
            while !self.is(")") {
                if self.is("..") {
                    return Err(self.error("tuple variant rest patterns are unsupported"));
                }
                fields.push((FieldSelector::Tuple(fields.len()), self.pattern()?));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            return Ok(Pattern::Variant {
                name,
                shape: ir::VariantShape::Tuple,
                fields,
                rest: false,
            });
        }
        if !self.eat("{") {
            if name.contains("::") || matches!(name.as_str(), "self" | "super" | "crate") {
                return Ok(Pattern::Path(name));
            }
            return Ok(Pattern::Binding {
                name,
                mutable: false,
            });
        }
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.is("}") {
            if self.eat("..") {
                rest = true;
                if !self.is("}") {
                    return Err(self.error("struct rest pattern must be last"));
                }
                break;
            }
            let mutable = self.eat("mut");
            let field = self.ident()?;
            let pattern = if self.eat(":") {
                if mutable {
                    return Err(self.error("mut must apply to a binding, not a field selector"));
                }
                self.pattern()?
            } else {
                Pattern::Binding {
                    name: field.clone(),
                    mutable,
                }
            };
            fields.push((field, pattern));
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(Pattern::Struct { name, fields, rest })
    }
    fn compound_statement(&mut self, target: Expr, op: BinaryOp) -> Result<Stmt, String> {
        let value = self.expr(0)?;
        self.expect(";")?;
        if let Expr::DynamicIndex { base, index } = target {
            return Ok(Stmt::AssignIndexOp {
                array: self.assignment_place(*base)?,
                index: *index,
                op,
                value,
            });
        }
        Ok(Stmt::AssignOp {
            place: self.assignment_place(target)?,
            op,
            value,
        })
    }
    fn block_inner(&mut self) -> Result<Block, String> {
        self.expect("{")?;
        let mut statements = Vec::new();
        let mut tail = None;
        while !self.is("}") {
            if self.is("") {
                return Err(self.error("unterminated block"));
            }
            if self.eat(";") {
                continue;
            }
            if self.is("mod") || self.is("use") || self.is("fn") || self.is("pub") {
                return Err(self.error("block-local items are unsupported"));
            }
            if self.eat("let") {
                let pattern = self.pattern()?;
                if matches!(pattern, Pattern::Bool(_) | Pattern::Int { .. }) {
                    return Err(self.error("refutable literal pattern in let binding"));
                }
                // Local inference is performed by each independent function checker.
                let annotation = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                if self.eat(";") {
                    let Pattern::Binding { name, mutable } = pattern else {
                        return Err(self.error("deferred pattern declarations are unsupported"));
                    };
                    let ty = annotation.ok_or_else(||self.error("untyped deferred declarations are unsupported; add a concrete type annotation"))?;
                    statements.push(Stmt::Declare { name, ty, mutable });
                    continue;
                }
                self.expect("=")?;
                let value = self.expr(0)?;
                self.expect(";")?;
                let ty = annotation.unwrap_or(Type::Infer);
                if let Pattern::Binding { name, mutable } = pattern {
                    statements.push(Stmt::Let {
                        name,
                        mutable,
                        ty,
                        value,
                    });
                } else {
                    statements.push(Stmt::LetPattern { pattern, ty, value });
                }
            } else if self.eat("return") {
                let value = if self.is(";") || self.is("}") {
                    None
                } else {
                    Some(self.expr(0)?)
                };
                if !self.eat(";") && !self.is("}") {
                    return Err(self.error("expected semicolon or block end after return"));
                }
                statements.push(Stmt::Return(value));
            } else if self.eat("while") {
                let condition = self.condition()?;
                let body = self.block()?;
                statements.push(Stmt::While { condition, body });
            } else {
                let expr = self.statement_expr()?;
                if let Some(op) = compound_operator(self.token()) {
                    self.at += 1;
                    statements.push(self.compound_statement(expr, op)?);
                } else if self.eat("=") {
                    let value = self.expr(0)?;
                    self.expect(";")?;
                    if let Expr::DynamicIndex { base, index } = expr {
                        let array = self.assignment_place(*base)?;
                        statements.push(Stmt::AssignIndex {
                            array,
                            index: *index,
                            value,
                        });
                        continue;
                    }
                    let place = self.assignment_place(expr)?;
                    if place.fields.is_empty() {
                        statements.push(Stmt::Assign {
                            name: place.local,
                            value,
                        });
                    } else {
                        statements.push(Stmt::AssignPlace { place, value });
                    }
                } else if self.eat(";") {
                    statements.push(Stmt::Expr(expr));
                } else if self.is("}") {
                    tail = Some(Box::new(expr));
                    break;
                } else if matches!(
                    expr,
                    Expr::If { .. } | Expr::Block(_) | Expr::Loop { .. } | Expr::Match { .. }
                ) {
                    statements.push(Stmt::ImplicitExpr(expr));
                } else {
                    return Err(self.error("expected semicolon or block end"));
                }
            }
        }
        self.expect("}")?;
        Ok(Block { statements, tail })
    }
    fn assignment_place(&self, mut expr: Expr) -> Result<Place, String> {
        let mut fields = Vec::new();
        loop {
            match expr {
                Expr::Var(local) => { fields.reverse(); return Ok(Place { local, fields }); }
                Expr::Deref {value}=>{fields.push(FieldSelector::Deref);expr = *value;}
                Expr::Field { base, field } => { fields.push(field); expr = *base; }
                Expr::Index { base,index } => {fields.push(FieldSelector::ConstantIndex(index));expr = *base;}
                _ => return Err(self.error("assignment requires a local-rooted place; destructuring and temporary assignment are unsupported")),
            }
        }
    }
    fn nested_expr(&mut self) -> Result<Expr, String> {
        let previous = self.allow_struct;
        self.allow_struct = true;
        let result = self.expr(0);
        self.allow_struct = previous;
        result
    }
    fn condition(&mut self) -> Result<Expr, String> {
        let previous = self.allow_struct;
        self.allow_struct = false;
        let result = self.expr(0);
        self.allow_struct = previous;
        result
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        if self.nesting >= 128 {
            return Err(self.error("syntax nesting exceeds limit 128"));
        }
        self.nesting += 1;
        let result = self.expr_inner(min);
        self.nesting -= 1;
        result
    }
    fn statement_expr(&mut self) -> Result<Expr, String> {
        if self.nesting >= 128 {
            return Err(self.error("syntax nesting exceeds limit128"));
        }
        self.nesting += 1;
        let result = self.expr_inner_mode(0, true);
        self.nesting -= 1;
        result
    }
    fn cast_after_as(&mut self, value: Expr) -> Result<Expr, String> {
        self.expect("as")?;
        if self.is("(") {
            return Err(self.error("parenthesized cast target types are unsupported"));
        }
        let target = self.ty()?;
        if self.is("<") || self.is("<<") {
            return Err(self.error("parenthesize the cast before a less-than comparison; otherwise '<' starts generic arguments"));
        }
        if self.is(".") || self.is("[") || self.is("(") {
            return Err(self.error(
                "a cast cannot be followed directly by a postfix operation; parenthesize the cast",
            ));
        }
        let expr = Expr::Cast {
            value: Box::new(value),
            target,
        };
        validate_depth(DepthNode::Expr(&expr)).map_err(|e| self.error(&e))?;
        Ok(expr)
    }
    fn expr_inner(&mut self, min: u8) -> Result<Expr, String> {
        self.expr_inner_mode(min, false)
    }
    fn expr_inner_mode(&mut self, min: u8, statement: bool) -> Result<Expr, String> {
        if !self.token().string && self.token().text.starts_with('\'') {
            return Err(self.error("labels and lifetime expressions are unsupported"));
        }
        let statement_blocklike =
            statement && matches!(self.token().text.as_str(), "if" | "match" | "loop" | "{");
        if self.is("&&") {
            return Err(self.error("nested reference borrows are unsupported in B1"));
        }
        let mut left = if self.eat("&") {
            let kind = if self.eat("mut") {
                ir::BorrowKind::Mut
            } else {
                ir::BorrowKind::Shared
            };
            let value = self.expr(11)?;
            let place=self.assignment_place(value).map_err(|_|self.error("borrowing requires a local-root static place; rvalues and runtime indexed borrows are unsupported"))?;
            Expr::Borrow { kind, place }
        } else if self.eat("*") {
            Expr::Deref {
                value: Box::new(self.expr(11)?),
            }
        } else if self.eat("-") {
            Expr::Unary {
                op: UnaryOp::Neg,
                value: Box::new(self.expr(11)?),
            }
        } else if self.eat("!") {
            Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(self.expr(11)?),
            }
        } else if self.eat("true") {
            Expr::Bool(true)
        } else if self.eat("false") {
            Expr::Bool(false)
        } else if self.eat("loop") {
            Expr::Loop {
                body: self.block()?,
            }
        } else if self.eat("break") {
            let value = if matches!(self.token().text.as_str(), ";" | "}" | ")" | "]" | "," | "") {
                None
            } else {
                Some(Box::new(self.expr(0)?))
            };
            Expr::Break { value }
        } else if self.eat("continue") {
            Expr::Continue
        } else if self.eat("match") {
            let scrutinee = Box::new(self.condition()?);
            self.expect("{")?;
            let mut arms = Vec::new();
            while !self.is("}") {
                if arms.len() >= 4096 {
                    return Err(self.error("match arm count exceeds limit 4096"));
                }
                let pattern = self.pattern()?;
                if self.is("if") || self.is("|") || self.is("@") {
                    return Err(
                        self.error("match guards, or-patterns and @ patterns are unsupported")
                    );
                }
                self.expect("=>")?;
                let direct_return = self.eat("return");
                let body = if direct_return {
                    let value = if self.is(",") || self.is("}") {
                        None
                    } else {
                        Some(self.nested_expr()?)
                    };
                    Expr::Block(Block {
                        statements: vec![Stmt::Return(value)],
                        tail: None,
                    })
                } else {
                    self.nested_expr()?
                };
                let blocklike = !direct_return
                    && matches!(
                        body,
                        Expr::Block(_) | Expr::If { .. } | Expr::Loop { .. } | Expr::Match { .. }
                    );
                arms.push(MatchArm { pattern, body });
                if !self.eat(",") && !self.is("}") && !blocklike {
                    return Err(self.error("expected comma after match arm"));
                }
            }
            self.expect("}")?;
            Expr::Match { scrutinee, arms }
        } else if self.eat("if") {
            let condition = Box::new(self.condition()?);
            let then_block = self.block()?;
            let else_block = if self.eat("else") {
                if self.is("if") {
                    Block {
                        statements: Vec::new(),
                        tail: Some(Box::new(if statement {
                            self.statement_expr()?
                        } else {
                            self.expr(0)?
                        })),
                    }
                } else {
                    self.block()?
                }
            } else {
                Block {
                    statements: Vec::new(),
                    tail: None,
                }
            };
            Expr::If {
                condition,
                then_block,
                else_block,
            }
        } else if self.is("{") {
            Expr::Block(self.block()?)
        } else if self.eat("[") {
            if self.eat("]") {
                Expr::Array(Vec::new())
            } else {
                let first = self.nested_expr()?;
                if self.eat(";") {
                    let len = self.array_number("array repeat length")?;
                    self.expect("]")?;
                    Expr::Repeat {
                        value: Box::new(first),
                        len,
                    }
                } else {
                    let mut values = vec![first];
                    while self.eat(",") && !self.is("]") {
                        if values.len() >= ir::MAX_ARRAY_LEN {
                            return Err(self.error("array literal exceeds supported length"));
                        }
                        values.push(self.nested_expr()?);
                    }
                    self.expect("]")?;
                    Expr::Array(values)
                }
            }
        } else if self.eat("(") {
            if self.eat(")") {
                Expr::Block(Block {
                    statements: Vec::new(),
                    tail: None,
                })
            } else {
                let first = self.nested_expr()?;
                if self.eat(",") {
                    let mut fields = vec![first];
                    while !self.is(")") {
                        fields.push(self.nested_expr()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    Expr::Tuple(fields)
                } else {
                    self.expect(")")?;
                    first
                }
            }
        } else if !self.token().string
            && self.token().text.starts_with(|c: char| c.is_ascii_digit())
        {
            let literal = parse_integer_literal(&self.token().text).map_err(|e| self.error(&e))?;
            self.at += 1;
            Expr::Integer(literal)
        } else {
            let name = self.path()?;
            if self.eat("!") {
                if name != "println" {
                    return Err(self.error("only println! is supported"));
                }
                self.expect("(")?;
                if !self.token().string {
                    return Err(self.error("println! requires one literal string"));
                }
                let text = self.token().text.clone();
                self.at += 1;
                let text = literal_format(&text).map_err(|e| self.error(&e))?;
                self.eat(",");
                self.expect(")")?;
                Expr::Println(text)
            } else if self.eat("(") {
                let mut args = Vec::new();
                if !self.is(")") {
                    loop {
                        args.push(self.nested_expr()?);
                        if !self.eat(",") || self.is(")") {
                            break;
                        }
                    }
                }
                self.expect(")")?;
                Expr::Call { name, args }
            } else if self.allow_struct && self.eat("{") {
                let mut fields = Vec::new();
                while !self.is("}") {
                    let field = self.ident()?;
                    let value = if self.eat(":") {
                        self.expr(0)?
                    } else {
                        Expr::Var(field.clone())
                    };
                    fields.push((field, value));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                Expr::Struct { name, fields }
            } else {
                Expr::Var(name)
            }
        };
        let mut compared = false;
        loop {
            if statement_blocklike
                && matches!(
                    left,
                    Expr::If { .. } | Expr::Match { .. } | Expr::Loop { .. } | Expr::Block(_)
                )
                && self.is("[")
            {
                break;
            }
            if self.eat("[") {
                let literal = self.token().text.clone();
                let constant = (!self.token().string
                    && self.tokens.get(self.at + 1).is_some_and(|t| t.text == "]"))
                .then(|| parse_integer_literal(&literal).ok())
                .flatten()
                .filter(|literal| literal.suffix.is_none());
                if let Some(literal) = constant {
                    let value =
                        resolve_integer(&literal, false, Some(&ir::Ty::Int(ir::IntTy::Usize)))
                            .map_err(|e| self.error(&e))?;
                    if value.bits > ir::MAX_ARRAY_LEN as u64 {
                        return Err(self.error("array index exceeds supported limit"));
                    }
                    let index = value.bits as usize;
                    self.at += 1;
                    self.expect("]")?;
                    left = Expr::Index {
                        base: Box::new(left),
                        index,
                    };
                } else {
                    let index = self.nested_expr()?;
                    self.expect("]")?;
                    left = Expr::DynamicIndex {
                        base: Box::new(left),
                        index: Box::new(index),
                    };
                }
                validate_depth(DepthNode::Expr(&left)).map_err(|e| self.error(&e))?;
                continue;
            }
            if self.eat(".") {
                let field = if self.token().text.starts_with(|c: char| c.is_ascii_digit()) {
                    let token = self.token();
                    if token.string
                        || !token.text.bytes().all(|c| c.is_ascii_digit())
                        || (token.text.len() > 1 && token.text.starts_with('0'))
                    {
                        return Err(self.error(
                            "tuple field index must be a canonical unsigned decimal integer",
                        ));
                    }
                    let index = token
                        .text
                        .parse::<usize>()
                        .map_err(|_| self.error("tuple field index is out of range"))?;
                    self.at += 1;
                    FieldSelector::Tuple(index)
                } else {
                    FieldSelector::Named(self.ident()?)
                };
                left = Expr::Field {
                    base: Box::new(left),
                    field,
                };
                validate_depth(DepthNode::Expr(&left)).map_err(|e| self.error(&e))?;
                continue;
            }
            if statement_blocklike
                && matches!(
                    left,
                    Expr::If { .. } | Expr::Match { .. } | Expr::Loop { .. } | Expr::Block(_)
                )
            {
                break;
            }
            if self.is("as") {
                if min > 10 {
                    break;
                }
                left = self.cast_after_as(left)?;
                continue;
            }
            let (precedence, op) = match self.token().text.as_str() {
                "||" => (1, BinaryOp::Or),
                "&&" => (2, BinaryOp::And),
                "==" => (3, BinaryOp::Eq),
                "!=" => (3, BinaryOp::Ne),
                "<" => (3, BinaryOp::Lt),
                "<=" => (3, BinaryOp::Le),
                ">" => (3, BinaryOp::Gt),
                ">=" => (3, BinaryOp::Ge),
                "|" => (4, BinaryOp::BitOr),
                "^" => (5, BinaryOp::BitXor),
                "&" => (6, BinaryOp::BitAnd),
                "<<" => (7, BinaryOp::Shl),
                ">>" => (7, BinaryOp::Shr),
                "+" => (8, BinaryOp::Add),
                "-" => (8, BinaryOp::Sub),
                "*" => (9, BinaryOp::Mul),
                "/" => (9, BinaryOp::Div),
                "%" => (9, BinaryOp::Rem),
                _ => break,
            };
            if self.token().string || precedence < min {
                break;
            }
            if matches!(
                op,
                BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr
            ) && self
                .tokens
                .get(self.at + 1)
                .is_some_and(|token| !token.string && token.text == "=")
            {
                return Err(self.error("compound bitwise/shift assignments are unsupported"));
            }
            if precedence == 3 {
                if compared {
                    return Err(self.error("chained comparisons require parentheses"));
                }
                compared = true;
            } else if precedence < 3 {
                compared = false;
            }
            self.at += 1;
            let right = self.expr(precedence + 1)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
            validate_depth(DepthNode::Expr(&left)).map_err(|e| self.error(&e))?;
        }
        Ok(left)
    }
}
fn literal_format(text: &str) -> Result<String, String> {
    let mut chars = text.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if (c == '{' || c == '}') && chars.next() != Some(c) {
            return Err("println! formatting placeholders are unsupported".into());
        }
        out.push(c);
    }
    Ok(out)
}

/// Parse syntax only, retaining imports and visibility for the signatures phase.
pub fn parse_unresolved(source: &str) -> Result<ParsedProgram, String> {
    let mut parser = Parser {
        tokens: lex(source)?,
        at: 0,
        nesting: 0,
        allow_struct: true,
    };
    let mut parsed = ParsedProgram {
        program: Program {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
        },
        items: HashMap::new(),
    };
    parser.items("", &mut parsed)?;
    if !parser.is("") {
        return Err(parser.error("unexpected module closing brace"));
    }
    for function in &parsed.program.functions {
        validate_depth(DepthNode::Block(&function.body))
            .map_err(|e| format!("1:1: in function {}: {e}", function.name))?;
    }
    Ok(parsed)
}
/// Convenience frontend for callers without separate parse/signatures phases.
/// Performs name resolution, but not function body type checks.
pub fn parse(source: &str) -> Result<Program, String> {
    resolve(&parse_unresolved(source)?)
}

#[derive(Clone, Debug)]
struct ResolvedItem {
    path: String,
    module: bool,
    kind: ResolvedKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedKind {
    Module,
    Function,
    Struct,
    Enum,
    Variant,
}
struct Resolver<'a> {
    items: &'a HashMap<String, Vec<Item>>,
    imports: HashMap<(String, usize, bool), Option<ResolvedItem>>,
    active: std::collections::HashSet<(String, usize, bool)>,
}
impl Resolver<'_> {
    fn item(
        &mut self,
        path: &str,
        requester: &str,
        module: bool,
        visible_only: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let Some(bindings) = self.items.get(path) else {
            return Ok(None);
        };
        let owner = parent(path);
        let descendant = owner.is_empty()
            || requester == owner
            || requester
                .strip_prefix(owner)
                .is_some_and(|s| s.starts_with("::"));
        let mut result = None;
        for (index, item) in bindings.iter().enumerate() {
            let resolved = match &item.kind {
                ItemKind::Module if module => Some(ResolvedItem {
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }),
                ItemKind::Function if !module => Some(ResolvedItem {
                    path: path.into(),
                    module: false,
                    kind: ResolvedKind::Function,
                }),
                ItemKind::Struct if module => Some(ResolvedItem {
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Struct,
                }),
                ItemKind::Enum if module => Some(ResolvedItem {
                    path: path.into(),
                    module: true,
                    kind: ResolvedKind::Enum,
                }),
                ItemKind::Variant(shape) if module || *shape != ir::VariantShape::Struct => {
                    Some(ResolvedItem {
                        path: path.into(),
                        module,
                        kind: ResolvedKind::Variant,
                    })
                }
                ItemKind::Import(_) => self.import(path, index, module)?,
                _ => None,
            };
            if let Some(resolved) = resolved {
                if !item.public && !descendant {
                    if visible_only {
                        continue;
                    }
                    return Err(format!(
                        "private item {path} is inaccessible from module {requester}"
                    ));
                }
                if result.replace(resolved).is_some() {
                    return Err(format!(
                        "duplicate item {path} in {} namespace",
                        if module { "type" } else { "value" }
                    ));
                }
            }
        }
        Ok(result)
    }
    fn import(
        &mut self,
        path: &str,
        index: usize,
        module: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let key = (path.to_owned(), index, module);
        if let Some(resolved) = self.imports.get(&key) {
            return Ok(resolved.clone());
        }
        if self.active.len() >= 128 {
            return Err("import resolution nesting exceeds limit 128".into());
        }
        if !self.active.insert(key.clone()) {
            // This import may bind only the other namespace. Treat an active
            // edge as absent until its concrete namespace is established. A
            // cycle without a concrete item remains unresolved and is rejected.
            return Ok(None);
        }
        let ItemKind::Import(target) = &self.items[path][index].kind else {
            unreachable!()
        };
        let resolved = self.path(&target.clone(), parent(path), module, true)?;
        self.active.remove(&key);
        self.imports.insert(key, resolved.clone());
        Ok(resolved)
    }
    fn path(
        &mut self,
        path: &str,
        from: &str,
        module: bool,
        importing: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let parts: Vec<_> = path.split("::").collect();
        let mut at = 0;
        let mut current = match parts[0] {
            "crate" => {
                at = 1;
                ResolvedItem {
                    path: String::new(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            "self" => {
                at = 1;
                ResolvedItem {
                    path: from.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            "super" => {
                let mut module = from;
                while parts.get(at) == Some(&"super") {
                    if module.is_empty() {
                        return Err(format!("super path {path} escapes crate root"));
                    }
                    module = parent(module);
                    at += 1;
                }
                ResolvedItem {
                    path: module.into(),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            }
            name => {
                at += 1;
                let Some(item) = self.item(
                    &qualify(from, name),
                    from,
                    if parts.len() == 1 { module } else { true },
                    importing && parts.len() == 1,
                )?
                else {
                    return Ok(None);
                };
                item
            }
        };
        while let Some(name) = parts.get(at) {
            if !matches!(current.kind, ResolvedKind::Module | ResolvedKind::Enum) {
                return Err(format!("{} is not a module in path {path}", current.path));
            }
            // Enum variants reserve type bindings too, but are neither usable
            // source types nor traversable nested module scopes.
            if current.kind == ResolvedKind::Enum && at + 1 != parts.len() {
                return Ok(None);
            }
            let Some(item) = self.item(
                &qualify(&current.path, name),
                from,
                if at + 1 == parts.len() { module } else { true },
                importing && at + 1 == parts.len(),
            )?
            else {
                return Ok(None);
            };
            current = item;
            at += 1;
        }
        Ok((current.module == module).then_some(current))
    }
    fn ty(&mut self, ty: &mut Type, module: &str) -> Result<(), String> {
        match ty {
            Type::Ref { referent, .. } => self.ty(referent, module)?,
            Type::Array(element, _) => self.ty(element, module)?,
            Type::Named(name) => {
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved type {name}"))?;
                if !matches!(item.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                    return Err(format!("{} is a module, not a nominal type", item.path));
                }
                *name = item.path;
            }
            Type::Tuple(fields) => {
                for ty in fields {
                    self.ty(ty, module)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pattern(
        &mut self,
        pattern: &mut Pattern,
        module: &str,
        names: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        match pattern {
            Pattern::Variant { name, fields, .. } => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved enum variant {name}"))?;
                if item.kind != ResolvedKind::Variant {
                    return Err(format!("{} is not a variant", item.path));
                }
                *name = item.path;
                for (_, pattern) in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
            Pattern::Wildcard => {}
            Pattern::Bool(_) | Pattern::Int { .. } => {}
            Pattern::Path(name) => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved variant pattern {name}"))?;
                if item.kind != ResolvedKind::Variant {
                    return Err(format!("{} is not an enum variant pattern", item.path));
                }
                *name = item.path;
            }
            Pattern::Binding { name, mutable } => {
                if let Some(item) = self.path(name, module, false, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    if *mutable {
                        return Err(format!("binding {name} cannot shadow an enum variant"));
                    }
                    *pattern = Pattern::Path(item.path);
                    return Ok(());
                }
                if !names.insert(name.clone()) {
                    return Err(format!("duplicate pattern binding {name}"));
                }
            }
            Pattern::Tuple(fields) => {
                for pattern in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
            Pattern::Struct { name, fields, rest } => {
                if let Some(item) = self.path(name, module, true, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    for (_, pattern) in fields.iter_mut() {
                        self.pattern(pattern, module, names)?;
                    }
                    *pattern = Pattern::Variant {
                        name: item.path,
                        shape: ir::VariantShape::Struct,
                        fields: fields
                            .iter()
                            .map(|(n, p)| (FieldSelector::Named(n.clone()), p.clone()))
                            .collect(),
                        rest: *rest,
                    };
                    return Ok(());
                }
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved struct pattern {name}"))?;
                if item.kind != ResolvedKind::Struct {
                    return Err(format!("{} is not a struct pattern", item.path));
                }
                *name = item.path;
                for (_, pattern) in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
        }
        Ok(())
    }
    fn block(
        &mut self,
        block: &mut Block,
        module: &str,
        locals: &mut Vec<std::collections::HashSet<String>>,
    ) -> Result<(), String> {
        locals.push(std::collections::HashSet::new());
        for stmt in &mut block.statements {
            if let Stmt::Let {
                name,
                mutable,
                ty,
                value,
            } = stmt
                && let Some(item) = self.path(name, module, false, false)?
                && item.kind == ResolvedKind::Variant
            {
                if *mutable {
                    return Err(format!("binding {name} cannot shadow an enum variant"));
                }
                *stmt = Stmt::LetPattern {
                    pattern: Pattern::Path(item.path),
                    ty: ty.clone(),
                    value: value.clone(),
                };
            }
            match stmt {
                Stmt::Declare { name, ty, .. } => {
                    self.ty(ty, module)?;
                    if let Some(item) = self.path(name, module, false, false)?
                        && item.kind == ResolvedKind::Variant
                    {
                        return Err(format!(
                            "enum variant patterns in deferred declarations are unsupported: {name}"
                        ));
                    }
                    locals.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    locals.last_mut().unwrap().insert(name.clone());
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    let mut names = std::collections::HashSet::new();
                    self.pattern(pattern, module, &mut names)?;
                    locals.last_mut().unwrap().extend(names);
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    self.expr(value, module, locals)?;
                    self.expr(index, module, locals)?;
                }
                Stmt::Assign { value, .. }
                | Stmt::AssignOp { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::Expr(value)
                | Stmt::ImplicitExpr(value)
                | Stmt::Return(Some(value)) => {
                    self.expr(value, module, locals)?;
                }
                Stmt::While { condition, body } => {
                    self.expr(condition, module, locals)?;
                    self.block(body, module, locals)?;
                }
                Stmt::Return(None) => {}
            }
        }
        if let Some(tail) = &mut block.tail {
            self.expr(tail, module, locals)?;
        }
        locals.pop();
        Ok(())
    }
    fn expr(
        &mut self,
        expr: &mut Expr,
        module: &str,
        locals: &mut Vec<std::collections::HashSet<String>>,
    ) -> Result<(), String> {
        match expr {
            Expr::Array(elements) => {
                for e in elements {
                    self.expr(e, module, locals)?;
                }
            }
            Expr::Cast { value, target } => {
                self.expr(value, module, locals)?;
                self.ty(target, module)?;
            }
            Expr::Deref { value } => self.expr(value, module, locals)?,
            Expr::Borrow { .. } => {}
            Expr::Repeat { value, .. } => self.expr(value, module, locals)?,
            Expr::DynamicIndex { base, index } => {
                self.expr(base, module, locals)?;
                self.expr(index, module, locals)?;
            }
            Expr::Index { base, .. } => self.expr(base, module, locals)?,
            Expr::Var(name) => {
                if !name.contains("::") && locals.iter().rev().any(|scope| scope.contains(name)) {
                    return Ok(());
                }
                if let Some(item) = self.path(name, module, false, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    *expr = Expr::EnumVariant {
                        name: item.path,
                        shape: ir::VariantShape::Unit,
                        fields: Vec::new(),
                    };
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee, module, locals)?;
                for arm in arms {
                    let mut names = std::collections::HashSet::new();
                    self.pattern(&mut arm.pattern, module, &mut names)?;
                    locals.push(names);
                    self.expr(&mut arm.body, module, locals)?;
                    locals.pop();
                }
            }
            Expr::Call { name, args } => {
                // Check the source spelling, not its eventual canonical name:
                // `let f=1; crate::f()` must not be shadowed by the local f.
                if !name.contains("::") && locals.iter().rev().any(|s| s.contains(name)) {
                    return Err(format!("local {name} is not callable"));
                }
                let resolved = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                if resolved.kind == ResolvedKind::Variant {
                    for arg in args.iter_mut() {
                        self.expr(arg, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: resolved.path,
                        shape: ir::VariantShape::Tuple,
                        fields: args
                            .iter()
                            .enumerate()
                            .map(|(i, e)| (FieldSelector::Tuple(i), e.clone()))
                            .collect(),
                    };
                    return Ok(());
                }
                if resolved.kind != ResolvedKind::Function {
                    return Err(format!("{} is not callable", resolved.path));
                }
                *name = resolved.path;
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::Tuple(fields) => {
                for field in fields {
                    self.expr(field, module, locals)?;
                }
            }
            Expr::Struct { name, fields } => {
                if let Some(item) = self.path(name, module, true, false)?
                    && item.kind == ResolvedKind::Variant
                {
                    for (_, value) in fields.iter_mut() {
                        self.expr(value, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: item.path,
                        shape: ir::VariantShape::Struct,
                        fields: fields
                            .iter()
                            .map(|(n, e)| (FieldSelector::Named(n.clone()), e.clone()))
                            .collect(),
                    };
                    return Ok(());
                }
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| format!("unresolved struct {name}"))?;
                if item.kind != ResolvedKind::Struct {
                    return Err(format!("{} is not a struct", item.path));
                }
                *name = item.path;
                for (_, value) in fields {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::Field { base, .. } => self.expr(base, module, locals)?,
            Expr::Unary { value, .. } => self.expr(value, module, locals)?,
            Expr::Binary { left, right, .. } => {
                self.expr(left, module, locals)?;
                self.expr(right, module, locals)?;
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(condition, module, locals)?;
                self.block(then_block, module, locals)?;
                self.block(else_block, module, locals)?;
            }
            Expr::Block(block) | Expr::Loop { body: block } => self.block(block, module, locals)?,
            Expr::Break { value: Some(value) } => self.expr(value, module, locals)?,
            _ => {}
        }
        Ok(())
    }
}

/// Resolve imports, visibility and all call paths once in the name-resolution
/// graph operation, before publishing signatures.
/// The result is backend-ready: flattened canonical function names, with root
/// `main` unchanged. Public AST construction remains supported as resolved IR.
/// Unqualified names belong to the current module only, not parent modules.
pub fn resolve(parsed: &ParsedProgram) -> Result<Program, String> {
    let mut resolver = Resolver {
        items: &parsed.items,
        imports: HashMap::new(),
        active: std::collections::HashSet::new(),
    };
    // Validate unused imports too, deterministically, including cycles/privacy.
    let mut paths: Vec<_> = parsed.items.keys().cloned().collect();
    paths.sort();
    for path in paths {
        for (index, item) in parsed.items[&path].iter().enumerate() {
            if matches!(item.kind, ItemKind::Import(_)) {
                let m = resolver.import(&path, index, true)?;
                let f = resolver.import(&path, index, false)?;
                if m.is_none() && f.is_none() {
                    return Err(format!("unresolved or cyclic import {path}"));
                }
            }
        }
        resolver.item(&path, parent(&path), true, false)?;
        resolver.item(&path, parent(&path), false, false)?;
    }
    let mut program = parsed.program.clone();
    for enumeration in &mut program.enums {
        for variant in &mut enumeration.variants {
            for field in &mut variant.fields {
                validate_depth(DepthNode::Type(&field.ty))?;
                resolver.ty(&mut field.ty, parent(&enumeration.name))?;
            }
        }
    }
    for structure in &mut program.structs {
        for field in &mut structure.fields {
            validate_depth(DepthNode::Type(&field.ty))?;
            resolver.ty(&mut field.ty, parent(&structure.name))?;
        }
    }
    for function in &mut program.functions {
        resolver.ty(&mut function.result, parent(&function.name))?;
        for (name, ty) in &mut function.params {
            resolver.ty(ty, parent(&function.name))?;
            if let Some(item) = resolver.path(name, parent(&function.name), false, false)?
                && item.kind == ResolvedKind::Variant
            {
                return Err(format!(
                    "enum variant patterns in function parameters are unsupported: {name}"
                ));
            }
        }
        validate_depth(DepthNode::Block(&function.body))?;
        let mut locals = vec![function.params.iter().map(|(n, _)| n.clone()).collect()];
        resolver
            .block(&mut function.body, parent(&function.name), &mut locals)
            .map_err(|e| format!("in function {}: {e}", function.name))?;
    }
    Ok(program)
}
enum DepthNode<'a> {
    Type(&'a Type),
    Pattern(&'a Pattern),
    Expr(&'a Expr),
    Block(&'a Block),
}
fn validate_depth(root: DepthNode<'_>) -> Result<(), String> {
    let mut stack = vec![(root, 0usize)];
    let mut pattern_nodes = 0usize;
    while let Some((node, depth)) = stack.pop() {
        if depth > 128 {
            return Err("AST nesting exceeds limit 128".into());
        }
        let mut push = |node| stack.push((node, depth + 1));
        match node {
            DepthNode::Type(Type::Ref { referent, .. }) => push(DepthNode::Type(referent)),
            DepthNode::Type(Type::Array(element, _)) => push(DepthNode::Type(element)),
            DepthNode::Type(Type::Tuple(fields)) => {
                for field in fields {
                    push(DepthNode::Type(field));
                }
            }
            DepthNode::Type(_) => {}
            DepthNode::Pattern(pattern) => {
                pattern_nodes += 1;
                if pattern_nodes > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("pattern node count exceeds limit".into());
                }
                match pattern {
                    Pattern::Variant { fields, .. } => {
                        for (_, p) in fields {
                            push(DepthNode::Pattern(p));
                        }
                    }
                    Pattern::Tuple(fields) => {
                        for pattern in fields {
                            push(DepthNode::Pattern(pattern));
                        }
                    }
                    Pattern::Struct { fields, .. } => {
                        for (_, pattern) in fields {
                            push(DepthNode::Pattern(pattern));
                        }
                    }
                    _ => {}
                }
            }
            DepthNode::Expr(expr) => match expr {
                Expr::Cast { value, target } => {
                    push(DepthNode::Expr(value));
                    push(DepthNode::Type(target));
                }
                Expr::Array(elements) => {
                    for e in elements {
                        push(DepthNode::Expr(e));
                    }
                }
                Expr::Deref { value } => push(DepthNode::Expr(value)),
                Expr::Repeat { value, .. } => push(DepthNode::Expr(value)),
                Expr::DynamicIndex { base, index } => {
                    push(DepthNode::Expr(base));
                    push(DepthNode::Expr(index));
                }
                Expr::Index { base, .. } => push(DepthNode::Expr(base)),
                Expr::EnumVariant { fields, .. } => {
                    for (_, e) in fields {
                        push(DepthNode::Expr(e));
                    }
                }
                Expr::Match { scrutinee, arms } => {
                    push(DepthNode::Expr(scrutinee));
                    for arm in arms {
                        push(DepthNode::Pattern(&arm.pattern));
                        push(DepthNode::Expr(&arm.body));
                    }
                }
                Expr::Tuple(fields) => {
                    for field in fields {
                        push(DepthNode::Expr(field));
                    }
                }
                Expr::Struct { fields, .. } => {
                    for (_, value) in fields {
                        push(DepthNode::Expr(value));
                    }
                }
                Expr::Field { base, .. } => push(DepthNode::Expr(base)),
                Expr::Unary { value, .. } => push(DepthNode::Expr(value)),
                Expr::Binary { left, right, .. } => {
                    push(DepthNode::Expr(left));
                    push(DepthNode::Expr(right));
                }
                Expr::Call { args, .. } => {
                    for arg in args {
                        push(DepthNode::Expr(arg));
                    }
                }
                Expr::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    push(DepthNode::Expr(condition));
                    push(DepthNode::Block(then_block));
                    push(DepthNode::Block(else_block));
                }
                Expr::Block(block) | Expr::Loop { body: block } => push(DepthNode::Block(block)),
                Expr::Break { value: Some(value) } => push(DepthNode::Expr(value)),
                _ => {}
            },
            DepthNode::Block(block) => {
                for stmt in &block.statements {
                    match stmt {
                        Stmt::Declare { ty, .. } => push(DepthNode::Type(ty)),
                        Stmt::Let { value, ty, .. } => {
                            push(DepthNode::Expr(value));
                            push(DepthNode::Type(ty));
                        }
                        Stmt::LetPattern { pattern, ty, value } => {
                            push(DepthNode::Pattern(pattern));
                            push(DepthNode::Type(ty));
                            push(DepthNode::Expr(value));
                        }
                        Stmt::AssignIndex { index, value, .. }
                        | Stmt::AssignIndexOp { index, value, .. } => {
                            push(DepthNode::Expr(value));
                            push(DepthNode::Expr(index));
                        }
                        Stmt::Assign { value, .. }
                        | Stmt::AssignOp { value, .. }
                        | Stmt::AssignPlace { value, .. }
                        | Stmt::Expr(value)
                        | Stmt::ImplicitExpr(value)
                        | Stmt::Return(Some(value)) => push(DepthNode::Expr(value)),
                        Stmt::While { condition, body } => {
                            push(DepthNode::Expr(condition));
                            push(DepthNode::Block(body));
                        }
                        Stmt::Return(None) => {}
                    }
                }
                if let Some(tail) = &block.tail {
                    push(DepthNode::Expr(tail));
                }
            }
        }
    }
    Ok(())
}
/// One immutable semantic environment shared by every checked function.
#[derive(Clone, Debug)]
pub struct TypeEnv {
    pub structs: HashMap<String, ir::StructTy>,
    pub enums: HashMap<String, ir::EnumTy>,
    pub functions: HashMap<String, ir::Signature>,
}
pub type Signatures = TypeEnv;

fn visible_from(owner: &str, requester: &str) -> bool {
    owner.is_empty()
        || requester == owner
        || requester
            .strip_prefix(owner)
            .is_some_and(|s| s.starts_with("::"))
}

/// Validate every declaration, including unused types, before creating any Arc DAG.
pub fn type_env(program: &Program) -> Result<TypeEnv, String> {
    enum Decl<'a> {
        Struct(&'a StructDecl),
        Enum(&'a EnumDecl),
    }
    impl Decl<'_> {
        fn name(&self) -> &str {
            match self {
                Self::Struct(d) => &d.name,
                Self::Enum(d) => &d.name,
            }
        }
        fn public(&self) -> bool {
            match self {
                Self::Struct(d) => d.public,
                Self::Enum(d) => d.public,
            }
        }
        fn fields(&self) -> Vec<&FieldDecl> {
            match self {
                Self::Struct(d) => d.fields.iter().collect(),
                Self::Enum(d) => d.variants.iter().flat_map(|v| v.fields.iter()).collect(),
            }
        }
    }
    let all: Vec<_> = program
        .structs
        .iter()
        .map(Decl::Struct)
        .chain(program.enums.iter().map(Decl::Enum))
        .collect();
    let mut indices = HashMap::new();
    for (index, decl) in all.iter().enumerate() {
        if indices.insert(decl.name().to_owned(), index).is_some() {
            return Err(format!("duplicate nominal type {}", decl.name()));
        }
        match decl {
            Decl::Struct(d) => {
                if d.fields.is_empty() {
                    return Err(format!("empty struct {} is unsupported", d.name));
                }
            }
            Decl::Enum(d) => {
                if d.variants.is_empty() {
                    return Err("empty enums are unsupported".into());
                }
                if d.variants.len() > 4096 {
                    return Err("enum variant count exceeds limit 4096".into());
                }
                let mut names = std::collections::HashSet::new();
                for variant in &d.variants {
                    if !names.insert(&variant.name) {
                        return Err(format!("duplicate enum variant {}", variant.name));
                    }
                    if variant.fields.len() > 4096 {
                        return Err("variant field count exceeds limit 4096".into());
                    }
                    if variant.shape == ir::VariantShape::Unit && !variant.fields.is_empty() {
                        return Err("unit variant cannot have fields".into());
                    }
                    if variant.shape == ir::VariantShape::Tuple
                        && variant
                            .fields
                            .iter()
                            .enumerate()
                            .any(|(i, f)| f.name != i.to_string())
                    {
                        return Err("tuple variant fields require positional names".into());
                    }
                }
            }
        }
        let groups = match decl {
            Decl::Struct(d) => vec![d.fields.as_slice()],
            Decl::Enum(d) => d.variants.iter().map(|v| v.fields.as_slice()).collect(),
        };
        for fields in groups {
            let mut names = std::collections::HashSet::new();
            for field in fields {
                if !names.insert(&field.name) {
                    return Err(format!("duplicate field {} in {}", field.name, decl.name()));
                }
            }
        }
    }
    fn dependencies(
        ty: &Type,
        indices: &HashMap<String, usize>,
        all: &[Decl<'_>],
        module: &str,
    ) -> Result<Vec<usize>, String> {
        validate_depth(DepthNode::Type(ty))?;
        let mut pending = vec![ty];
        let mut deps = Vec::new();
        while let Some(ty) = pending.pop() {
            match ty {
                Type::Ref{..}=>return Err("reference results and reference-containing aggregate/nested types are unsupported".into()),
                Type::Array(element, len) => {
                    if *len > ir::MAX_ARRAY_LEN {
                        return Err("array length exceeds supported limit".into());
                    }
                    pending.push(element);
                }
                Type::Named(name) => {
                    let index = *indices
                        .get(name)
                        .ok_or_else(|| format!("unknown nominal type {name}"))?;
                    if !all[index].public() && !visible_from(parent(name), module) {
                        return Err(format!("private nominal type {name}"));
                    }
                    deps.push(index);
                }
                Type::Tuple(fields) => {
                    if fields.is_empty() {
                        return Err("empty tuple types must use ()".into());
                    }
                    pending.extend(fields);
                }
                Type::Infer => return Err("signature types cannot be inferred".into()),
                _ => {}
            }
        }
        Ok(deps)
    }
    let mut edges = vec![Vec::new(); all.len()];
    for (index, decl) in all.iter().enumerate() {
        for field in decl.fields() {
            edges[index].extend(dependencies(
                &field.ty,
                &indices,
                &all,
                parent(decl.name()),
            )?);
        }
    }
    let mut functions = std::collections::HashSet::new();
    for function in &program.functions {
        if !functions.insert(&function.name) {
            return Err(format!("duplicate function {}", function.name));
        }
        let mut params = std::collections::HashSet::new();
        for (name, _) in &function.params {
            if !params.insert(name) {
                return Err(format!(
                    "in function {}: duplicate parameter {name}",
                    function.name
                ));
            }
        }
        let result_ty = if let Type::Ref { referent, .. } = &function.result {
            referent.as_ref()
        } else {
            &function.result
        };
        dependencies(result_ty, &indices, &all, parent(&function.name))?;
        for (_, ty) in &function.params {
            validate_depth(DepthNode::Type(ty))?;
            let referent = if let Type::Ref { referent, .. } = ty {
                referent.as_ref()
            } else {
                ty
            };
            dependencies(referent, &indices, &all, parent(&function.name))?;
        }
    }
    if !program
        .functions
        .iter()
        .any(|f| f.name == "main" && f.params.is_empty() && f.result == Type::Unit)
    {
        return Err("main must exist, take no arguments and return ()".into());
    }
    let mut state = vec![0u8; all.len()];
    let mut order = Vec::new();
    for root in 0..all.len() {
        if state[root] != 0 {
            continue;
        }
        let mut pending = vec![(root, false, 0usize)];
        while let Some((index, exit, depth)) = pending.pop() {
            if exit {
                state[index] = 2;
                order.push(index);
                continue;
            }
            if state[index] == 2 {
                continue;
            }
            if state[index] == 1 {
                return Err(format!(
                    "by-value nominal cycle involving {}",
                    all[index].name()
                ));
            }
            if depth > 128 {
                return Err("nominal dependency nesting exceeds limit 128".into());
            }
            state[index] = 1;
            pending.push((index, true, depth));
            for &dep in edges[index].iter().rev() {
                pending.push((dep, false, depth + 1));
            }
        }
    }
    fn height(ty: &Type, indices: &HashMap<String, usize>, heights: &[usize]) -> usize {
        match ty {
            Type::Array(element, _) => 1 + height(element, indices, heights),
            Type::Named(n) => heights[indices[n]],
            Type::Tuple(f) => {
                1 + f
                    .iter()
                    .map(|t| height(t, indices, heights))
                    .max()
                    .unwrap_or(0)
            }
            _ => 1,
        }
    }
    let mut heights = vec![0usize; all.len()];
    for &index in &order {
        heights[index] = 1 + all[index]
            .fields()
            .iter()
            .map(|f| height(&f.ty, &indices, &heights))
            .max()
            .unwrap_or(0);
        if heights[index] > 128 {
            return Err("expanded nominal type nesting exceeds limit 128".into());
        }
    }
    // No nominal Arc is allocated until every declaration and the entire mixed
    // by-value graph has been validated, including unused declarations.
    let snapshot = ir::SnapshotId::allocate()?;
    let mut env = TypeEnv {
        structs: HashMap::new(),
        enums: HashMap::new(),
        functions: HashMap::new(),
    };
    for index in order {
        let fields = |fields: &[FieldDecl],
                      env: &TypeEnv|
         -> Result<std::sync::Arc<[ir::FieldDef]>, String> {
            fields
                .iter()
                .map(|f| {
                    Ok(ir::FieldDef {
                        name: f.name.clone(),
                        ty: checked_ty(&f.ty, env)?,
                        public: f.public,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
                .map(Into::into)
        };
        let id = ir::DefId { snapshot, index };
        match &all[index] {
            Decl::Struct(d) => {
                let ty = ir::StructTy::new(ir::StructDef {
                    id,
                    name: d.name.clone(),
                    module: parent(&d.name).into(),
                    public: d.public,
                    fields: fields(&d.fields, &env)?,
                });
                env.structs.insert(d.name.clone(), ty);
            }
            Decl::Enum(d) => {
                let variants = d
                    .variants
                    .iter()
                    .map(|v| {
                        Ok(ir::VariantDef {
                            name: v.name.clone(),
                            shape: v.shape,
                            fields: fields(&v.fields, &env)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let ty = ir::EnumTy::new(ir::EnumDef {
                    id,
                    name: d.name.clone(),
                    module: parent(&d.name).into(),
                    public: d.public,
                    variants: variants.into(),
                });
                env.enums.insert(d.name.clone(), ty);
            }
        }
    }
    for function in &program.functions {
        let params = function
            .params
            .iter()
            .map(|(_, ty)| checked_ty(ty, &env))
            .collect::<Result<Vec<_>, _>>()?;
        let result = checked_ty(&function.result, &env)?;
        let ref_contract = canonical_ref_contract(function, &params, &result)?;
        let signature = ir::Signature {
            params,
            result,
            ref_contract,
        };
        env.functions.insert(function.name.clone(), signature);
    }
    Ok(env)
}
struct LoopFrame {
    id: ir::LoopId,
    accepts_value: bool,
    checking_condition: bool,
    break_ty: Option<ir::Ty>,
    can_exit: bool,
}
struct PatternLocal {
    name: String,
    ty: ir::Ty,
    mutable: bool,
    projection: Vec<ir::Projection>,
}
type ResolvedVariantPattern<'p> = (ir::EnumTy, usize, Vec<(usize, &'p Pattern)>);

const COUNT_EVAL_WORK: usize = 1_000_000;
#[derive(Clone, Copy, Debug)]
enum CountValue {
    Known(ir::IntValue),
    Dynamic,
    Unsupported,
}
#[derive(Clone, Copy)]
enum CountMemo {
    Unseen,
    Visiting,
    Done(CountValue),
}
struct CountEvaluator<'a> {
    exprs: &'a [ir::TypedExpr],
    memo: Vec<CountMemo>,
    heights: Vec<usize>,
    work: usize,
}
impl<'a> CountEvaluator<'a> {
    fn new(exprs: &'a [ir::TypedExpr]) -> Result<Self, String> {
        let bytes = exprs
            .len()
            .checked_mul(std::mem::size_of::<CountMemo>() + std::mem::size_of::<usize>())
            .ok_or("shift count memo allocation overflow")?;
        if exprs.len() > COUNT_EVAL_WORK || bytes > 64 * 1024 * 1024 {
            return Err("shift count memo resource limit exceeded".into());
        }
        Ok(Self {
            exprs,
            memo: vec![CountMemo::Unseen; exprs.len()],
            heights: vec![0; exprs.len()],
            work: 0,
        })
    }
    fn charge(&mut self) -> Result<(), String> {
        self.work += 1;
        if self.work > COUNT_EVAL_WORK {
            return Err("shift count evaluation work limit exceeded".into());
        }
        Ok(())
    }
    fn value(&self, id: ir::ExprId) -> Result<CountValue, String> {
        match self.memo.get(id.0) {
            Some(CountMemo::Done(value)) => Ok(*value),
            _ => Err("invalid shift count evaluation dependency".into()),
        }
    }
    fn evaluate(&mut self, root: ir::ExprId) -> Result<CountValue, String> {
        let mut pending = vec![(root, false, 0usize)];
        while let Some((id, ready, depth)) = pending.pop() {
            self.charge()?;
            if depth > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            let expr = self
                .exprs
                .get(id.0)
                .ok_or("invalid shift count expression ID")?;
            if !ready {
                match self.memo[id.0] {
                    CountMemo::Done(_) => {
                        if depth + self.heights[id.0] > 128 {
                            return Err("shift count evaluation depth limit exceeded".into());
                        }
                        continue;
                    }
                    CountMemo::Visiting => return Err("cycle in shift count expression".into()),
                    CountMemo::Unseen => {}
                }
                // A typed integer that cannot return has no constant value.
                if expr.completion == ir::Completion::Never {
                    self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                    continue;
                }
                let children = match &expr.kind {
                    ir::ExprKind::Int(value) => {
                        value.validate()?;
                        if expr.ty != ir::Ty::Int(value.ty) {
                            return Err("shift count constant type mismatch".into());
                        }
                        self.memo[id.0] = CountMemo::Done(CountValue::Known(*value));
                        continue;
                    }
                    ir::ExprKind::Local(_)
                    | ir::ExprKind::Call { .. }
                    | ir::ExprKind::Index { .. }
                    | ir::ExprKind::DynamicIndex { .. }
                    | ir::ExprKind::Deref { .. }
                    | ir::ExprKind::Field { .. } => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    ir::ExprKind::Unary { value, .. } | ir::ExprKind::IntCast { value, .. } => {
                        [Some(*value), None]
                    }
                    ir::ExprKind::Binary { left, right, .. } => [Some(*left), Some(*right)],
                    ir::ExprKind::Block(block) if block.statements.is_empty() => [block.tail, None],
                    ir::ExprKind::Block(_) => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Dynamic);
                        continue;
                    }
                    _ => {
                        self.memo[id.0] = CountMemo::Done(CountValue::Unsupported);
                        continue;
                    }
                };
                self.memo[id.0] = CountMemo::Visiting;
                pending.push((id, true, depth));
                for child in children.into_iter().flatten().rev() {
                    pending.push((child, false, depth + 1));
                }
                continue;
            }
            let result = match &expr.kind {
                ir::ExprKind::Block(block) => block
                    .tail
                    .map(|id| self.value(id))
                    .transpose()?
                    .unwrap_or(CountValue::Unsupported),
                ir::ExprKind::IntCast { value, to } => match self.value(*value)? {
                    CountValue::Known(value) => CountValue::Known(ir::IntValue {
                        ty: *to,
                        bits: count_integer(value) as u64 & to.mask(),
                    }),
                    other => other,
                },
                ir::ExprKind::Unary { op, value } => match self.value(*value)? {
                    CountValue::Known(value) => {
                        let bits = match op {
                            UnaryOp::Not => !value.bits & value.ty.mask(),
                            UnaryOp::Neg => {
                                if !value.ty.is_signed() {
                                    return Err("unsigned negation in shift count".into());
                                }
                                count_checked_signed(-count_integer(value), value.ty)?
                            }
                        };
                        CountValue::Known(ir::IntValue { ty: value.ty, bits })
                    }
                    other => other,
                },
                ir::ExprKind::Binary { op, left, right } => {
                    let lhs = self.value(*left)?;
                    let rhs = self.value(*right)?;
                    if matches!(op, BinaryOp::Shl | BinaryOp::Shr)
                        && let CountValue::Known(value) = rhs
                    {
                        let ir::Ty::Int(ty) = self.exprs[left.0].ty else {
                            return Err("noninteger nested shift".into());
                        };
                        count_shift_amount(value, ty)?;
                    }
                    match (lhs, rhs) {
                        (CountValue::Known(lhs), CountValue::Known(rhs)) => {
                            count_binary(*op, lhs, rhs)?
                        }
                        (CountValue::Unsupported, _) | (_, CountValue::Unsupported) => {
                            CountValue::Unsupported
                        }
                        _ => CountValue::Dynamic,
                    }
                }
                _ => return Err("invalid shift count evaluation state".into()),
            };
            if let CountValue::Known(value) = result
                && expr.ty != ir::Ty::Int(value.ty)
            {
                return Err("shift count result type mismatch".into());
            }
            let height = match &expr.kind {
                ir::ExprKind::Block(block) => {
                    block.tail.map(|id| self.heights[id.0] + 1).unwrap_or(0)
                }
                ir::ExprKind::IntCast { value, .. } | ir::ExprKind::Unary { value, .. } => {
                    self.heights[value.0] + 1
                }
                ir::ExprKind::Binary { left, right, .. } => {
                    self.heights[left.0].max(self.heights[right.0]) + 1
                }
                _ => 0,
            };
            if depth + height > 128 {
                return Err("shift count evaluation depth limit exceeded".into());
            }
            self.heights[id.0] = height;
            self.memo[id.0] = CountMemo::Done(result);
        }
        self.value(root)
    }
}
fn count_integer(value: ir::IntValue) -> i128 {
    let bits = value.ty.bits();
    if value.ty.is_signed() && value.bits & (1u64 << (bits - 1)) != 0 {
        value.bits as i128 - (1i128 << bits)
    } else {
        value.bits as i128
    }
}
fn count_checked_signed(value: i128, ty: ir::IntTy) -> Result<u64, String> {
    let limit = 1i128 << (ty.bits() - 1);
    if value < -limit || value >= limit {
        return Err("arithmetic overflow in static shift count".into());
    }
    Ok(value as u64 & ty.mask())
}
fn count_shift_amount(value: ir::IntValue, lhs: ir::IntTy) -> Result<u32, String> {
    value.validate()?;
    let amount = count_integer(value);
    if amount < 0 || amount >= lhs.bits() as i128 {
        return Err(format!(
            "shift count {amount} overflows {}-bit integer",
            lhs.bits()
        ));
    }
    Ok(amount as u32)
}
fn count_binary(op: BinaryOp, lhs: ir::IntValue, rhs: ir::IntValue) -> Result<CountValue, String> {
    lhs.validate()?;
    rhs.validate()?;
    let ty = lhs.ty;
    let bits = if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
        let amount = count_shift_amount(rhs, ty)?;
        match op {
            BinaryOp::Shl => (lhs.bits << amount) & ty.mask(),
            _ if ty.is_signed() => (count_integer(lhs) >> amount) as u64 & ty.mask(),
            _ => lhs.bits >> amount,
        }
    } else {
        if lhs.ty != rhs.ty {
            return Err("static shift count integer type mismatch".into());
        }
        match op {
            BinaryOp::BitAnd => lhs.bits & rhs.bits,
            BinaryOp::BitOr => lhs.bits | rhs.bits,
            BinaryOp::BitXor => lhs.bits ^ rhs.bits,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if ty.is_signed() {
                    let a = count_integer(lhs);
                    let b = count_integer(rhs);
                    if matches!(op, BinaryOp::Div | BinaryOp::Rem)
                        && (b == 0 || (a == -(1i128 << (ty.bits() - 1)) && b == -1))
                    {
                        return Err("invalid division in static shift count".into());
                    }
                    let value = match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .ok_or("arithmetic overflow in static shift count")?;
                    count_checked_signed(value, ty)?
                } else {
                    let a = lhs.bits as u128;
                    let b = rhs.bits as u128;
                    let value = match op {
                        BinaryOp::Add => a.checked_add(b),
                        BinaryOp::Sub => a.checked_sub(b),
                        BinaryOp::Mul => a.checked_mul(b),
                        BinaryOp::Div => a.checked_div(b),
                        _ => a.checked_rem(b),
                    }
                    .filter(|v| *v <= ty.mask() as u128)
                    .ok_or("invalid arithmetic in static shift count")?;
                    value as u64
                }
            }
            _ => return Ok(CountValue::Unsupported),
        }
    };
    Ok(CountValue::Known(ir::IntValue { ty, bits }))
}
fn validate_shift_counts(
    exprs: &[ir::TypedExpr],
    queries: &[(ir::IntTy, ir::ExprId)],
) -> Result<(), String> {
    if queries.is_empty()
        && !exprs.iter().any(|e| {
            matches!(
                e.kind,
                ir::ExprKind::Binary {
                    op: BinaryOp::Shl | BinaryOp::Shr,
                    ..
                }
            )
        })
    {
        return Ok(());
    }
    let mut evaluator = CountEvaluator::new(exprs)?;
    if queries.len() > COUNT_EVAL_WORK {
        return Err("compound shift query work limit exceeded".into());
    }
    for &(ty, value) in queries {
        evaluator.charge()?;
        match evaluator.evaluate(value)? {
            CountValue::Known(value) => {
                count_shift_amount(value, ty)?;
            }
            CountValue::Dynamic => {}
            CountValue::Unsupported => {
                return Err("unsupported static-shaped compound shift count expression".into());
            }
        }
    }
    for expr in exprs {
        evaluator.charge()?;
        if let ir::ExprKind::Binary {
            op: BinaryOp::Shl | BinaryOp::Shr,
            left,
            right,
        } = expr.kind
        {
            let ir::Ty::Int(lhs) = exprs.get(left.0).ok_or("invalid shift lhs")?.ty else {
                return Err("noninteger shift lhs".into());
            };
            match evaluator.evaluate(right)? {
                CountValue::Known(value) => {
                    count_shift_amount(value, lhs)?;
                }
                CountValue::Dynamic => {}
                CountValue::Unsupported => {
                    return Err("unsupported static-shaped shift count expression".into());
                }
            }
        }
    }
    Ok(())
}
struct Checker<'a> {
    signatures: &'a Signatures,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    locals: Vec<ir::LocalDecl>,
    exprs: Vec<ir::TypedExpr>,
    loops: Vec<LoopFrame>,
    next_loop: usize,
    reachable: bool,
    result: ir::Ty,
    module: String,
    count_queries: Vec<(ir::IntTy, ir::ExprId)>,
    field_indices: std::cell::RefCell<HashMap<ir::DefId, HashMap<String, usize>>>,
}
#[derive(Clone, Copy)]
enum Expectation<'a> {
    None,
    HasType(&'a ir::Ty),
    CastableToInt(ir::IntTy),
}
impl<'a> Expectation<'a> {
    fn hard(self) -> Option<&'a ir::Ty> {
        match self {
            Self::HasType(ty) => Some(ty),
            _ => None,
        }
    }
    fn literal(self, literal: &IntegerLiteral, negative: bool) -> Result<ir::IntValue, String> {
        let cast_hint = match self {
            Self::CastableToInt(to) if literal.suffix.is_none() => Some(ir::Ty::Int(to)),
            _ => None,
        };
        resolve_integer(literal, negative, self.hard().or(cast_hint.as_ref()))
    }
}
impl<'a> From<Option<&'a ir::Ty>> for Expectation<'a> {
    fn from(ty: Option<&'a ir::Ty>) -> Self {
        ty.map_or(Self::None, Self::HasType)
    }
}

// Semantic type and possible normal completion are independent. In particular,
// an unreachable explicit block tail retains its type, unlike a pure never value.
#[derive(Clone)]
struct Flow {
    ty: ir::Ty,
    completes: bool,
}
impl Flow {
    fn value(ty: ir::Ty) -> Self {
        Self {
            ty,
            completes: true,
        }
    }
    fn never() -> Self {
        Self {
            ty: ir::Ty::Never,
            completes: false,
        }
    }
}
fn canonical_ref_contract(
    function: &Function,
    params: &[ir::Ty],
    result: &ir::Ty,
) -> Result<std::sync::Arc<ir::RefContract>, String> {
    let mut ids = HashMap::new();
    if function.regions.params.len() > 64 || function.regions.outlives.len() > 4096 {
        return Err("declared lifetime resources exceed supported limits".into());
    }
    for (index, name) in function.regions.params.iter().enumerate() {
        if ids
            .insert(name.as_str(), ir::RegionParamId(index as u16))
            .is_some()
        {
            return Err(format!("duplicate lifetime parameter {name}"));
        }
    }
    let id = |name: &str| {
        ids.get(name)
            .copied()
            .ok_or_else(|| format!("undeclared lifetime {name}"))
    };
    let mut edges = function
        .regions
        .outlives
        .iter()
        .map(|(a, b)| Ok((id(a)?, id(b)?)))
        .collect::<Result<Vec<_>, String>>()?;
    edges.sort_unstable();
    edges.dedup();
    for (_, ty) in &function.params {
        if let Type::Ref {
            region: Some(name), ..
        } = ty
        {
            id(name)?;
        }
    }
    if let Type::Ref {
        region: Some(name), ..
    } = &function.result
    {
        id(name)?;
    }
    if !matches!(result, ir::Ty::Ref { .. }) {
        if !ids.is_empty()
            || function.params.iter().any(|(_, t)| {
                matches!(
                    t,
                    Type::Ref {
                        region: Some(_),
                        ..
                    }
                )
            })
        {
            return Err(
                "named lifetime contracts on no-reference-result functions are unsupported".into(),
            );
        }
        return Ok(std::sync::Arc::new(ir::RefContract {
            param_regions: vec![None; params.len()].into(),
            ..Default::default()
        }));
    }
    let Type::Ref {
        kind: result_kind,
        region: output,
        ..
    } = &function.result
    else {
        return Err("reference result requires a source reference type".into());
    };
    let mut region_count = ids.len();
    let mut input_positions = 0usize;
    let mut input_region = None;
    let mut param_regions = Vec::with_capacity(params.len());
    for (_, ty) in &function.params {
        let region = match ty {
            Type::Ref { region, .. } => {
                let region = if let Some(name) = region {
                    id(name)?
                } else {
                    if region_count >= ir::MAX_REGION_PARAMS {
                        return Err("combined explicit and anonymous lifetime regions exceed supported limit64".into());
                    }
                    let region = ir::RegionParamId(region_count as u16);
                    region_count += 1;
                    region
                };
                input_positions += 1;
                input_region = Some(region);
                Some(region)
            }
            _ => None,
        };
        param_regions.push(region);
    }
    let result_region = if let Some(output) = output {
        id(output)?
    } else if input_positions == 1 {
        input_region.ok_or("missing input lifetime for elided reference output")?
    } else {
        return Err("missing lifetime specifier: elided reference output requires exactly one input reference lifetime position".into());
    };
    let mut contract = ir::RefContract {
        region_count: region_count as u16,
        param_regions: param_regions.into(),
        result_region: Some(result_region),
        outlives: edges.into(),
        returns: ir::ReturnContract::NoReference,
    };
    let candidates = contract.eligible_input_paths(params, result)?.into();
    contract.returns = match result_kind {
        ir::BorrowKind::Shared => ir::ReturnContract::SharedInputPaths(candidates),
        ir::BorrowKind::Mut => ir::ReturnContract::MutInputPaths(candidates),
    };
    contract.validate(params, result)?;
    Ok(std::sync::Arc::new(contract))
}
fn checked_local_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    if matches!(
        ty,
        Type::Ref {
            region: Some(_),
            ..
        }
    ) {
        return Err("named lifetime annotations on locals are unsupported; use an elided local reference annotation".into());
    }
    checked_ty(ty, env)
}
fn checked_ty(ty: &Type, env: &TypeEnv) -> Result<ir::Ty, String> {
    checked_ty_at(ty, env, true)
}
fn checked_ty_at(ty: &Type, env: &TypeEnv, allow_ref: bool) -> Result<ir::Ty, String> {
    match ty {
        Type::Ref { kind, referent, .. } => {
            if !allow_ref || matches!(referent.as_ref(), Type::Never | Type::Infer) {
                return Err("nested or aggregate reference types are unsupported in B1".into());
            }
            Ok(ir::Ty::Ref {
                kind: *kind,
                referent: std::sync::Arc::new(checked_ty_at(referent, env, false)?),
            })
        }
        Type::Array(element, len) => {
            if *len > ir::MAX_ARRAY_LEN {
                return Err("array length exceeds supported limit".into());
            }
            Ok(ir::Ty::Array {
                element: std::sync::Arc::new(checked_ty_at(element, env, false)?),
                len: *len,
            })
        }
        Type::Int(ty) => Ok(ir::Ty::Int(*ty)),
        Type::Bool => Ok(ir::Ty::Bool),
        Type::Unit => Ok(ir::Ty::Unit),
        Type::Never => Ok(ir::Ty::Never),
        Type::Tuple(fields) if !fields.is_empty() => fields
            .iter()
            .map(|ty| checked_ty_at(ty, env, false))
            .collect::<Result<Vec<_>, _>>()
            .map(|fields| ir::Ty::Tuple(fields.into())),
        Type::Tuple(_) => Err("empty tuple types must use ()".into()),
        Type::Named(name) => env
            .structs
            .get(name)
            .cloned()
            .map(ir::Ty::Struct)
            .or_else(|| env.enums.get(name).cloned().map(ir::Ty::Enum))
            .ok_or_else(|| format!("unknown nominal type {name}")),
        Type::Infer => Err("signature types cannot be inferred".into()),
    }
}
fn inhabited(ty: &ir::Ty) -> bool {
    let mut work = vec![ty];
    let mut seen = std::collections::HashSet::new();
    while let Some(ty) = work.pop() {
        match ty {
            ir::Ty::Array { element, len } => {
                if *len > 0 {
                    work.push(element);
                }
            }
            ir::Ty::Never => return false,
            ir::Ty::Tuple(fields) => work.extend(fields.iter()),
            ir::Ty::Struct(ty) if seen.insert(ty.def().id) => {
                work.extend(ty.def().fields.iter().map(|field| &field.ty))
            }
            _ => {}
        }
    }
    true
}
fn coerces(got: &ir::Ty, want: &ir::Ty) -> bool {
    if *got == ir::Ty::Never || got == want {
        return true;
    }
    match (got, want) {
        (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) if na == nb => coerces(a, b),
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(a, b)| coerces(a, b))
        }
        _ => false,
    }
}
impl<'a> Checker<'a> {
    fn new(signatures: &'a Signatures, result: ir::Ty, module: String) -> Self {
        Self {
            signatures,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            exprs: Vec::new(),
            loops: Vec::new(),
            next_loop: 0,
            reachable: true,
            result,
            module,
            count_queries: Vec::new(),
            field_indices: std::cell::RefCell::new(HashMap::new()),
        }
    }
    fn enter_loop(&mut self, accepts_value: bool) -> ir::LoopId {
        let id = ir::LoopId(self.next_loop);
        self.next_loop += 1;
        self.loops.push(LoopFrame {
            id,
            accepts_value,
            checking_condition: !accepts_value,
            break_ty: None,
            can_exit: false,
        });
        id
    }
    fn loop_target(&self, operation: &str) -> Result<ir::LoopId, String> {
        let frame = self
            .loops
            .last()
            .ok_or_else(|| format!("{operation} outside a loop"))?;
        if frame.checking_condition {
            return Err(format!(
                "{operation} in a while condition is unsupported (E0590)"
            ));
        }
        Ok(frame.id)
    }
    fn local(&mut self, name: &str, ty: ir::Ty, mutable: bool) -> Result<ir::LocalId, String> {
        let local = ir::LocalId(self.locals.len());
        self.locals.push(ir::LocalDecl {
            name: name.to_owned(),
            ty,
            mutable,
        });
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_owned(), local);
        Ok(local)
    }
    fn bind(&mut self, name: &str, ty: ir::Ty) -> Result<ir::LocalId, String> {
        if self.scopes.last().unwrap().contains_key(name) {
            return Err(format!("duplicate parameter {name}"));
        }
        self.local(name, ty, false)
    }
    fn lookup(&self, n: &str) -> Result<ir::LocalId, String> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(n).copied())
            .ok_or_else(|| format!("unresolved local {n}"))
    }
    fn flow(&self, id: ir::ExprId) -> Flow {
        let e = &self.exprs[id.0];
        Flow {
            ty: e.ty.clone(),
            completes: e.completion == ir::Completion::Returns,
        }
    }
    fn require(&self, got: &Flow, want: &ir::Ty) -> Result<(), String> {
        if matches!((&got.ty,want),(ir::Ty::Ref{kind:ir::BorrowKind::Mut,referent:a},ir::Ty::Ref{kind:ir::BorrowKind::Shared,referent:b}) if a==b)
        {
            return Err("implicit mutable-to-shared reference coercion/reborrowing is unsupported; use an explicit &*r reborrow".into());
        }
        if !coerces(&got.ty, want) {
            Err(format!("expected {want:?}, got {:?}", got.ty))
        } else {
            Ok(())
        }
    }
    fn join(&self, a: &ir::Ty, b: &ir::Ty, context: &str) -> Result<ir::Ty, String> {
        if *a == ir::Ty::Never {
            return Ok(b.clone());
        }
        if *b == ir::Ty::Never || a == b {
            return Ok(a.clone());
        }
        if let (
            ir::Ty::Array {
                element: a,
                len: na,
            },
            ir::Ty::Array {
                element: b,
                len: nb,
            },
        ) = (a, b)
            && na == nb
        {
            return Ok(ir::Ty::Array {
                element: std::sync::Arc::new(self.join(a, b, context)?),
                len: *na,
            });
        }
        if let (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) = (a, b)
            && a.len() == b.len()
        {
            return a
                .iter()
                .zip(b.iter())
                .map(|(a, b)| self.join(a, b, context))
                .collect::<Result<Vec<_>, _>>()
                .map(|fields| ir::Ty::Tuple(fields.into()));
        }
        Err(format!("{context} differ: {a:?} and {b:?}"))
    }
    fn accessible(&self, ty: &ir::Ty) -> Result<(), String> {
        let mut work = vec![ty];
        while let Some(ty) = work.pop() {
            match ty {
                ir::Ty::Ref { referent, .. } => work.push(referent),
                ir::Ty::Array { element, .. } => work.push(element),
                ir::Ty::Tuple(fields) => work.extend(fields.iter()),
                ir::Ty::Enum(ty)
                    if !ty.def().public && !visible_from(&ty.def().module, &self.module) =>
                {
                    return Err(format!(
                        "private type {} is inaccessible from module {}",
                        ty.def().name,
                        self.module
                    ));
                }
                ir::Ty::Struct(ty)
                    if !ty.def().public && !visible_from(&ty.def().module, &self.module) =>
                {
                    return Err(format!(
                        "private type {} is inaccessible from module {}",
                        ty.def().name,
                        self.module
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn checked_place(&self, place: &Place) -> Result<(ir::Place, ir::Ty, bool), String> {
        if place.fields.len() > 128 {
            return Err("place projection nesting exceeds limit128".into());
        }
        let local = self.lookup(&place.local)?;
        let declaration = &self.locals[local.0];
        let mut ty = declaration.ty.clone();
        let mut writable = declaration.mutable;
        let mut derefs = 0;
        let mut projection = Vec::with_capacity(place.fields.len());
        for selector in &place.fields {
            match selector {
                FieldSelector::Deref => {
                    derefs += 1;
                    if derefs > 1 {
                        return Err("multiple dereferences in one B1 place are unsupported".into());
                    }
                    let ir::Ty::Ref { kind, referent } = &ty else {
                        return Err("dereference requires a reference".into());
                    };
                    writable = *kind == ir::BorrowKind::Mut;
                    ty = referent.as_ref().clone();
                    projection.push(ir::Projection::Deref);
                }
                FieldSelector::ConstantIndex(index) => {
                    ty = self.index_type(&ty, *index)?;
                    projection.push(ir::Projection::ConstantIndex(*index));
                }
                _ => {
                    if let ir::Ty::Ref { kind, referent } = &ty {
                        if derefs != 0 {
                            return Err(
                                "multiple dereferences in one field place are unsupported".into()
                            );
                        }
                        if place.fields.len() >= 128 {
                            return Err(
                                "normalized place projection nesting exceeds limit128".into()
                            );
                        }
                        derefs += 1;
                        writable = *kind == ir::BorrowKind::Mut;
                        ty = referent.as_ref().clone();
                        projection.push(ir::Projection::Deref);
                    }
                    let (index, field) = self.field_type(&ty, selector)?;
                    ty = field;
                    projection.push(ir::Projection::Field(index));
                }
            }
        }
        self.accessible(&ty)?;
        Ok((ir::Place { local, projection }, ty, writable))
    }
    fn reject_aggregate_ref(ty: &ir::Ty) -> Result<(), String> {
        let mut work = vec![ty];
        while let Some(ty) = work.pop() {
            match ty {
                ir::Ty::Ref { .. } => {
                    return Err(
                        "reference aggregate elements and nested references are unsupported in B1"
                            .into(),
                    );
                }
                ir::Ty::Array { element, .. } => work.push(element),
                ir::Ty::Tuple(fields) => work.extend(fields.iter()),
                _ => {}
            }
        }
        Ok(())
    }
    fn require_index(&self, flow: &Flow) -> Result<(), String> {
        self.require(flow,&ir::Ty::Int(ir::IntTy::Usize)).map_err(|_|"runtime index requires usize, without implicit i32 conversion; backward numeric inference is unsupported".into())
    }
    fn scalar_hint(&self, e: &Expr) -> Option<ir::Ty> {
        match e {
            Expr::Cast { target, .. } => checked_local_ty(target, self.signatures).ok(),
            Expr::Deref { value } => self.scalar_hint(value).and_then(|ty| {
                if let ir::Ty::Ref { referent, .. } = ty {
                    Some(referent.as_ref().clone())
                } else {
                    None
                }
            }),
            Expr::Integer(literal) => literal.suffix.map(ir::Ty::Int),
            Expr::Var(name) => self
                .lookup(name)
                .ok()
                .map(|id| self.locals[id.0].ty.clone()),
            Expr::Call { name, .. } => self
                .signatures
                .functions
                .get(name)
                .map(|s| s.result.clone()),
            Expr::Unary { value, .. } => self.scalar_hint(value),
            Expr::Binary {
                op:
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::And
                    | BinaryOp::Or,
                ..
            } => Some(ir::Ty::Bool),
            Expr::Binary {
                op: BinaryOp::Shl | BinaryOp::Shr,
                left,
                ..
            } => self.scalar_hint(left),
            Expr::Binary { left, right, .. } => {
                self.scalar_hint(left).or_else(|| self.scalar_hint(right))
            }
            Expr::Field { base, field } => self.scalar_hint(base).and_then(|t| {
                let receiver = if let ir::Ty::Ref { referent, .. } = &t {
                    referent.as_ref()
                } else {
                    &t
                };
                self.field_type(receiver, field).ok().map(|(_, t)| t)
            }),
            Expr::Index { base, .. } | Expr::DynamicIndex { base, .. } => {
                self.scalar_hint(base).and_then(|t| {
                    if let ir::Ty::Array { element, .. } = t {
                        Some(element.as_ref().clone())
                    } else {
                        None
                    }
                })
            }
            Expr::Block(b) => b.tail.as_ref().and_then(|e| self.scalar_hint(e)),
            _ => None,
        }
    }
    fn deref_source(&self, mut id: ir::ExprId) -> bool {
        loop {
            match &self.exprs[id.0].kind {
                ir::ExprKind::Deref { .. } => return true,
                ir::ExprKind::Index { base, .. }
                | ir::ExprKind::Field { base, .. }
                | ir::ExprKind::DynamicIndex { base, .. } => id = *base,
                _ => return false,
            }
        }
    }
    fn dynamic_place(e: &Expr) -> bool {
        match e {
            Expr::DynamicIndex { .. } => true,
            Expr::Index { base, .. } | Expr::Field { base, .. } => Self::dynamic_place(base),
            _ => false,
        }
    }
    fn index_type(&self, ty: &ir::Ty, index: usize) -> Result<ir::Ty, String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let ir::Ty::Array { element, len } = ty else {
            return Err("indexing requires an array".into());
        };
        if index >= *len {
            return Err(format!(
                "array index {index} out of bounds for length {len}"
            ));
        }
        self.accessible(element)?;
        Ok(element.as_ref().clone())
    }
    fn field_type(&self, ty: &ir::Ty, field: &FieldSelector) -> Result<(usize, ir::Ty), String> {
        if matches!(ty, ir::Ty::Ref { .. }) {
            return Err("implicit reference autodereference is unsupported in B1; use an explicit (*r) place".into());
        }
        let (index, projected) = match (ty, field) {
            (ir::Ty::Tuple(fields), FieldSelector::Tuple(index)) => fields
                .get(*index)
                .cloned()
                .map(|ty| (*index, ty))
                .ok_or_else(|| {
                    format!(
                        "tuple field {index} is out of bounds for {} fields",
                        fields.len()
                    )
                }),
            (ir::Ty::Struct(ty), FieldSelector::Named(name)) => {
                let index = {
                    let mut cache = self.field_indices.borrow_mut();
                    let indices = cache.entry(ty.def().id).or_insert_with(|| {
                        ty.def()
                            .fields
                            .iter()
                            .enumerate()
                            .map(|(index, field)| (field.name.clone(), index))
                            .collect()
                    });
                    *indices
                        .get(name)
                        .ok_or_else(|| format!("unknown field {name} in {}", ty.def().name))?
                };
                let field = &ty.def().fields[index];
                if !field.public && !visible_from(&ty.def().module, &self.module) {
                    return Err(format!("private field {name} in {}", ty.def().name));
                }
                Ok((index, field.ty.clone()))
            }
            _ => Err(format!("invalid field projection {field:?} on {ty:?}")),
        }?;
        self.accessible(&projected)?;
        Ok((index, projected))
    }
    /// Recover only concrete pattern shape, without a general inference engine.
    fn pattern_ty(&self, pattern: &Pattern, ty: &ir::Ty) -> Result<ir::Ty, String> {
        if matches!(ty, ir::Ty::Ref { .. })
            && !matches!(pattern, Pattern::Wildcard | Pattern::Binding { .. })
        {
            return Err(
                "reference patterns and implicit match ergonomics are unsupported in B1".into(),
            );
        }
        match pattern {
            Pattern::Path(name) | Pattern::Variant { name,.. } => {
                let (enumeration, _) = self.enum_variant(name)?;
                let expected = ir::Ty::Enum(enumeration);
                if !coerces(ty,&expected) { return Err(format!("enum pattern {name} does not match {ty:?}")); }
                Ok(expected)
            }
            Pattern::Bool(_) | Pattern::Int { .. } => Err("refutable literal pattern in let binding".into()),
            Pattern::Wildcard | Pattern::Binding { .. } => Ok(ty.clone()),
            Pattern::Tuple(patterns) => match ty {
                ir::Ty::Unit if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never if patterns.is_empty() => Ok(ir::Ty::Unit),
                ir::Ty::Never => patterns.iter().map(|pattern| {
                    if matches!(pattern, Pattern::Binding { .. } | Pattern::Wildcard) {
                        Err("unannotated never tuple pattern inference is unsupported; add a whole-pattern type annotation".into())
                    } else { self.pattern_ty(pattern, &ir::Ty::Never) }
                }).collect::<Result<Vec<_>, String>>().map(|fields| ir::Ty::Tuple(fields.into())),
                ir::Ty::Tuple(fields) if fields.len() == patterns.len() => patterns.iter().zip(fields.iter()).map(|(pattern, ty)| self.pattern_ty(pattern, ty)).collect::<Result<Vec<_>, _>>().map(|fields| ir::Ty::Tuple(fields.into())),
                _ => Err(format!("tuple pattern with {} fields does not match {ty:?}", patterns.len())),
            },
            Pattern::Struct { name, .. } => {
                let expected = self.signatures.structs.get(name).cloned().map(ir::Ty::Struct).ok_or_else(|| format!("unknown struct pattern {name}"))?;
                self.accessible(&expected)?;
                if !coerces(ty, &expected) { return Err(format!("struct pattern {name} does not match {ty:?}")); }
                Ok(expected)
            }
        }
    }
    fn pattern_locals(
        &self,
        pattern: &Pattern,
        ty: &ir::Ty,
        path: &mut Vec<ir::Projection>,
        names: &mut std::collections::HashSet<String>,
        pending: &mut Vec<PatternLocal>,
        work: &mut usize,
    ) -> Result<(), String> {
        if path.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern projection depth exceeds limit".into());
        }
        match pattern {
            Pattern::Wildcard => {}
            Pattern::Path(_) | Pattern::Variant { .. } => {
                let (enumeration, variant, fields) = self.variant_pattern(pattern, ty)?;
                if enumeration.def().variants.len() != 1 {
                    return Err(
                        "refutable enum patterns in irrefutable positions are unsupported".into(),
                    );
                }
                for (index, pattern) in fields {
                    path.push(ir::Projection::Downcast(ir::VariantId {
                        owner: enumeration.def().id,
                        index: variant,
                    }));
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(
                        pattern,
                        &enumeration.def().variants[variant].fields[index].ty,
                        path,
                        names,
                        pending,
                        work,
                    )?;
                    path.pop();
                    path.pop();
                }
            }
            Pattern::Bool(_) | Pattern::Int { .. } => {
                return Err("nested refutable literal patterns are unsupported".into());
            }
            Pattern::Binding { name, mutable } => {
                if name == "_" {
                    return Err("underscore is a wildcard, not a binding name".into());
                }
                if !names.insert(name.clone()) {
                    return Err(format!("duplicate pattern binding {name}"));
                }
                self.accessible(ty)?;
                if pending.len() >= ir::MAX_PATTERN_BINDINGS {
                    return Err("pattern binding count exceeds limit".into());
                }
                *work = work
                    .checked_add(1 + path.len())
                    .ok_or("pattern projection work overflow")?;
                if *work > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("pattern projection work exceeds limit".into());
                }
                pending.push(PatternLocal {
                    name: name.clone(),
                    ty: ty.clone(),
                    mutable: *mutable,
                    projection: path.clone(),
                });
            }
            Pattern::Tuple(patterns) => {
                if patterns.is_empty() && *ty == ir::Ty::Unit {
                    return Ok(());
                }
                let ir::Ty::Tuple(fields) = ty else {
                    return Err(format!("tuple pattern does not match {ty:?}"));
                };
                if fields.len() != patterns.len() {
                    return Err("tuple pattern arity mismatch".into());
                }
                for (index, (pattern, ty)) in patterns.iter().zip(fields.iter()).enumerate() {
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, ty, path, names, pending, work)?;
                    path.pop();
                }
            }
            Pattern::Struct { name, fields, rest } => {
                let expected = self.pattern_ty(pattern, ty)?;
                if &expected != ty {
                    return Err(
                        "nested never struct pattern requires a concrete annotated shape".into(),
                    );
                }
                let ir::Ty::Struct(structure) = ty else {
                    return Err(format!("expected struct pattern {name}"));
                };
                let mut selected = std::collections::HashSet::new();
                for (field, pattern) in fields {
                    let (index, field_ty) =
                        self.field_type(ty, &FieldSelector::Named(field.clone()))?;
                    if !selected.insert(index) {
                        return Err(format!("duplicate struct pattern field {field}"));
                    }
                    path.push(ir::Projection::Field(index));
                    self.pattern_locals(pattern, &field_ty, path, names, pending, work)?;
                    path.pop();
                }
                if !rest && selected.len() != structure.def().fields.len() {
                    return Err(format!(
                        "missing fields in struct pattern {name}; use .. to omit fields"
                    ));
                }
            }
        }
        Ok(())
    }
    fn expr_expected_when(
        &mut self,
        e: &Expr,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.expr_expected(e, expected);
        self.reachable = previous;
        result
    }
    fn enum_variant(&self, name: &str) -> Result<(ir::EnumTy, usize), String> {
        let (owner, variant) = name
            .rsplit_once("::")
            .ok_or_else(|| format!("unresolved enum variant {name}"))?;
        let ty = self
            .signatures
            .enums
            .get(owner)
            .cloned()
            .ok_or_else(|| format!("unknown enum {owner}"))?;
        self.accessible(&ir::Ty::Enum(ty.clone()))?;
        let index = ty
            .def()
            .variants
            .iter()
            .position(|definition| definition.name == variant)
            .ok_or_else(|| format!("unknown enum variant {name}"))?;
        Ok((ty, index))
    }
    fn variant_pattern<'p>(
        &self,
        pattern: &'p Pattern,
        expected: &ir::Ty,
    ) -> Result<ResolvedVariantPattern<'p>, String> {
        let (name, shape, fields, rest) = match pattern {
            Pattern::Path(name) => (name, ir::VariantShape::Unit, &[][..], false),
            Pattern::Variant {
                name,
                shape,
                fields,
                rest,
            } => (name, *shape, fields.as_slice(), *rest),
            _ => return Err("expected enum variant pattern".into()),
        };
        let (ty, variant) = self.enum_variant(name)?;
        if *expected != ir::Ty::Enum(ty.clone()) {
            return Err(format!(
                "variant pattern {name} does not match {expected:?}"
            ));
        }
        let definition = &ty.def().variants[variant];
        if shape != definition.shape
            && !(definition.shape == ir::VariantShape::Unit
                && shape == ir::VariantShape::Struct
                && fields.is_empty())
        {
            return Err(format!(
                "variant pattern {name} has wrong constructor shape"
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut selected = Vec::new();
        for (selector, pattern) in fields {
            let index = match selector {
                FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => *i,
                FieldSelector::Named(n) => definition
                    .fields
                    .iter()
                    .position(|f| &f.name == n)
                    .ok_or_else(|| format!("unknown variant field {n}"))?,
                _ => return Err("invalid variant field selector".into()),
            };
            let field = definition
                .fields
                .get(index)
                .ok_or("variant pattern arity mismatch")?;
            if !seen.insert(index) {
                return Err("duplicate variant pattern field".into());
            }
            self.accessible(&field.ty)?;
            selected.push((index, pattern));
        }
        if !rest && seen.len() != definition.fields.len() {
            return Err("missing variant pattern fields".into());
        }
        Ok((ty, variant, selected))
    }
    fn compound_value(
        &mut self,
        op: BinaryOp,
        ty: &ir::Ty,
        value: &Expr,
    ) -> Result<(ir::ExprId, Flow), String> {
        let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
        let bitwise = matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor);
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr
        ) {
            return Err("invalid compound assignment operator".into());
        }
        if !matches!(ty, ir::Ty::Int(_)) && !(bitwise && *ty == ir::Ty::Bool) {
            return Err(
                "compound assignment requires primitive integer or bitwise Bool destination".into(),
            );
        }
        let value = self.expr_expected(value, if shift { None } else { Some(ty) })?;
        let flow = self.flow(value);
        if shift {
            if flow.ty == ir::Ty::Never {
                return Err("compound shift on semantic Never is unsupported".into());
            }
            if !matches!(flow.ty, ir::Ty::Int(_)) {
                return Err("compound shift count must be an integer".into());
            }
            let ir::Ty::Int(ty) = ty else {
                return Err("compound shift destination must be integer".into());
            };
            if self.count_queries.len() >= COUNT_EVAL_WORK {
                return Err("compound shift query work limit exceeded".into());
            }
            self.count_queries.push((*ty, value));
        } else {
            if flow.ty == ir::Ty::Never {
                return Err("compound operator on semantic Never is unsupported".into());
            }
            self.require(&flow, ty)?;
        }
        Ok((value, flow))
    }
    fn block_when(&mut self, b: &Block, reachable: bool) -> Result<(ir::Block, Flow), String> {
        self.block_expected_when(b, reachable, None)
    }
    fn block_expected_when(
        &mut self,
        b: &Block,
        reachable: bool,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::Block, Flow), String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.block_expected(b, expected);
        self.reachable = previous;
        result
    }
    fn block(&mut self, b: &Block) -> Result<(ir::Block, Flow), String> {
        self.block_expected(b, None)
    }
    fn block_expected(
        &mut self,
        b: &Block,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::Block, Flow), String> {
        self.block_with_expectation(b, expected.into())
    }
    fn block_with_expectation(
        &mut self,
        b: &Block,
        expectation: Expectation<'_>,
    ) -> Result<(ir::Block, Flow), String> {
        let entered_reachable = self.reachable;
        self.scopes.push(HashMap::new());
        let mut live = true;
        let mut statements = Vec::with_capacity(b.statements.len());
        for s in &b.statements {
            self.reachable = entered_reachable && live;
            let (statement, completes) = match s {
                Stmt::Declare { name, ty, mutable } => {
                    if matches!(ty, Type::Infer | Type::Never) {
                        return Err(
                            "deferred declarations require a concrete supported source type".into(),
                        );
                    }
                    let ty = checked_local_ty(ty, self.signatures)?;
                    self.accessible(&ty)?;
                    let local = self.local(name, ty, *mutable)?;
                    (ir::Stmt::Declare { local }, true)
                }
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                } => {
                    let annotation = if *ty == Type::Infer {
                        None
                    } else {
                        Some(checked_local_ty(ty, self.signatures)?)
                    };
                    let value = self.expr_expected(value, annotation.as_ref())?;
                    let f = self.flow(value);
                    let t = if *ty == Type::Infer {
                        f.ty.clone()
                    } else {
                        let ty = checked_local_ty(ty, self.signatures)?;
                        self.accessible(&ty)?;
                        self.require(&f, &ty)?;
                        ty
                    };
                    let local = self.local(name, t, *mutable)?;
                    (ir::Stmt::Let { local, value }, f.completes)
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    // Resolve/check every initializer against the complete old scope.
                    let annotation = if *ty == Type::Infer {
                        None
                    } else {
                        Some(checked_local_ty(ty, self.signatures)?)
                    };
                    let direct_dynamic = matches!(value, Expr::DynamicIndex { .. });
                    let direct_wildcard = matches!(pattern, Pattern::Wildcard);
                    let value = self.expr_expected(value, annotation.as_ref())?;
                    if self.deref_source(value)
                        && matches!(pattern, Pattern::Path(_) | Pattern::Variant { .. })
                    {
                        return Err(
                            "enum downcasts after a dereference are unsupported in B1".into()
                        );
                    }
                    let flow = self.flow(value);
                    if direct_dynamic && !flow.ty.is_copy() && !direct_wildcard {
                        return Err("non-Copy runtime indexed pattern extraction is unsupported; only direct let _ inspection is admitted".into());
                    }
                    let seed = if *ty == Type::Infer {
                        flow.ty.clone()
                    } else {
                        let ty = checked_local_ty(ty, self.signatures)?;
                        self.accessible(&ty)?;
                        self.require(&flow, &ty)?;
                        ty
                    };
                    let ty = self.pattern_ty(pattern, &seed)?;
                    self.require(&flow, &ty)?;
                    self.accessible(&ty)?;
                    let mut pending = Vec::new();
                    self.pattern_locals(
                        pattern,
                        &ty,
                        &mut Vec::new(),
                        &mut std::collections::HashSet::new(),
                        &mut pending,
                        &mut 0usize,
                    )?;
                    // No partially installed binder set can affect another binder.
                    let mut bindings = Vec::with_capacity(pending.len());
                    for binding in pending {
                        let local = self.local(&binding.name, binding.ty, binding.mutable)?;
                        bindings.push(ir::PatternBinding {
                            local,
                            projection: binding.projection,
                        });
                    }
                    (
                        ir::Stmt::LetPattern {
                            value,
                            ty,
                            bindings,
                        },
                        flow.completes,
                    )
                }
                Stmt::Assign { name, value } => {
                    let local = self.lookup(name)?;
                    let declaration = &self.locals[local.0];
                    let t = declaration.ty.clone();
                    let value = self.expr_expected(value, Some(&t))?;
                    let f = self.flow(value);
                    self.require(&f, &t)?;
                    (ir::Stmt::Assign { local, value }, f.completes)
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                } => {
                    let (array, ty, writable) = self.checked_place(array)?;
                    if array.projection.contains(&ir::Projection::Deref) {
                        return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                    }
                    if !writable {
                        return Err("runtime index assignment requires a mutable place".into());
                    }
                    let ir::Ty::Array { element, .. } = &ty else {
                        return Err("runtime index assignment requires an array".into());
                    };
                    let value = self.expr_expected(value, Some(element))?;
                    let flow = self.flow(value);
                    self.require(&flow, element)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::Stmt::AssignIndex {
                            array,
                            index,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignOp { place, op, value } => {
                    let (place, ty, _) = self.checked_place(place)?;
                    let (value, flow) = self.compound_value(*op, &ty, value)?;
                    (
                        ir::Stmt::AssignOp {
                            place,
                            op: *op,
                            value,
                        },
                        flow.completes,
                    )
                }
                Stmt::AssignIndexOp {
                    array,
                    index,
                    op,
                    value,
                } => {
                    let (array, ty, _) = self.checked_place(array)?;
                    if array.projection.contains(&ir::Projection::Deref) {
                        return Err("runtime indexed access through references is unsupported; snapshot a Copy referent first".into());
                    }
                    let ir::Ty::Array { element, .. } = ty else {
                        return Err("compound index assignment requires an array".into());
                    };
                    let (value, flow) = self.compound_value(*op, &element, value)?;
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && flow.completes,
                        Some(&ir::Ty::Int(ir::IntTy::Usize)),
                    )?;
                    let index_flow = self.flow(index);
                    self.require_index(&index_flow)?;
                    (
                        ir::Stmt::AssignIndexOp {
                            array,
                            index,
                            op: *op,
                            value,
                        },
                        flow.completes && index_flow.completes,
                    )
                }
                Stmt::AssignPlace { place, value } => {
                    let (place, ty, writable) = self.checked_place(place)?;
                    if !place.projection.is_empty()
                        && !writable
                        && !place.projection.contains(&ir::Projection::Deref)
                    {
                        return Err(
                            "assignment requires a mutable place; shared references cannot write"
                                .into(),
                        );
                    }
                    let value = self.expr_expected(value, Some(&ty))?;
                    let flow = self.flow(value);
                    self.require(&flow, &ty)?;
                    (ir::Stmt::AssignPlace { place, value }, flow.completes)
                }
                Stmt::Expr(e) | Stmt::ImplicitExpr(e) => {
                    let value = self.expr(e)?;
                    let f = self.flow(value);
                    if matches!(s, Stmt::ImplicitExpr(_)) {
                        self.require(&f, &ir::Ty::Unit)?;
                    }
                    (ir::Stmt::Expr(value), f.completes)
                }
                Stmt::Return(e) => {
                    let result_ty = self.result.clone();
                    let value = e
                        .as_ref()
                        .map(|e| self.expr_expected(e, Some(&result_ty)))
                        .transpose()?;
                    let f = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                    self.require(&f, &self.result)?;
                    (ir::Stmt::Return(value), false)
                }
                Stmt::While { condition, body } => {
                    // Never skip a condition's target barrier to an outer loop.
                    let id = self.enter_loop(false);
                    let condition = self.expr(condition)?;
                    let f = self.flow(condition);
                    self.require(&f, &ir::Ty::Bool)?;
                    self.loops.last_mut().unwrap().checking_condition = false;
                    let (body, bf) = self.block(body)?;
                    self.require(&bf, &ir::Ty::Unit)?;
                    self.loops.pop();
                    (
                        ir::Stmt::While {
                            id,
                            condition,
                            body,
                        },
                        f.completes,
                    )
                }
            };
            statements.push(statement);
            live &= completes;
        }
        self.reachable = entered_reachable && live;
        let tail = b
            .tail
            .as_ref()
            .map(|e| self.expr_with_expectation(e, expectation))
            .transpose()?;
        self.reachable = entered_reachable;
        let mut flow = tail.map_or_else(
            || {
                if live {
                    Flow::value(ir::Ty::Unit)
                } else {
                    Flow::never()
                }
            },
            |e| self.flow(e),
        );
        flow.completes &= live;
        self.scopes.pop();
        Ok((ir::Block { statements, tail }, flow))
    }
    fn expr(&mut self, e: &Expr) -> Result<ir::ExprId, String> {
        self.expr_expected(e, None)
    }
    fn expr_expected(&mut self, e: &Expr, expected: Option<&ir::Ty>) -> Result<ir::ExprId, String> {
        self.expr_with_expectation(e, expected.into())
    }
    fn expr_with_expectation(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        if let Expr::Cast { value, target } = e {
            return self.cast_expression(value, target);
        }
        self.expr_without_cast(e, expectation)
    }
    fn cast_expression(&mut self, value: &Expr, target: &Type) -> Result<ir::ExprId, String> {
        let target = checked_local_ty(target, self.signatures)?;
        let ir::Ty::Int(to) = target else {
            return Err("noninteger cast endpoints are unsupported".into());
        };
        let value = self.expr_with_expectation(value, Expectation::CastableToInt(to))?;
        let source = self.flow(value);
        if source.ty == ir::Ty::Never {
            return Err("casts from semantic Never are unsupported".into());
        }
        if !matches!(source.ty, ir::Ty::Int(_)) {
            return Err("noninteger cast endpoints are unsupported".into());
        }
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            kind: ir::ExprKind::IntCast { value, to },
            ty: ir::Ty::Int(to),
            completion: if source.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
        });
        Ok(id)
    }
    fn expr_without_cast(
        &mut self,
        e: &Expr,
        expectation: Expectation<'_>,
    ) -> Result<ir::ExprId, String> {
        // A castable hint is not a hard result constraint or reference coercion site.
        let expected = expectation.hard();
        if let Some(ir::Ty::Ref { kind, referent }) = expected {
            if let Expr::Var(name) = e {
                let local = self.lookup(name)?;
                if let ir::Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: actual,
                } = &self.locals[local.0].ty
                    && actual == referent
                {
                    // Reborrow the original carrier before any value read can move it.
                    return self.expr_expected(
                        &Expr::Borrow {
                            kind: *kind,
                            place: Place {
                                local: name.clone(),
                                fields: vec![FieldSelector::Deref],
                            },
                        },
                        expected,
                    );
                }
            }
            if matches!(e, Expr::Loop { .. }) {
                return Err("reference expected-context propagation through loop results is unsupported; use an explicit borrow/reborrow".into());
            }
        }
        let (kind, flow) = match e {
            Expr::Cast { .. } => return Err("cast bypassed expression dispatcher".into()),
            Expr::Borrow { kind, place } => {
                let (place, referent, _writable) = self.checked_place(place).map_err(|error|format!("B1 borrow requires an existing local-root static place; borrowed rvalues/non-local targets are unsupported: {error}"))?;
                Self::reject_aggregate_ref(&referent)?;
                (
                    ir::ExprKind::Borrow { kind: *kind, place },
                    Flow::value(ir::Ty::Ref {
                        kind: *kind,
                        referent: std::sync::Arc::new(referent),
                    }),
                )
            }
            Expr::Deref { value } => {
                if !matches!(value.as_ref(), Expr::Var(_) | Expr::Call { .. }) {
                    return Err("non-local-root dereference carriers are unsupported except genuine reference call results".into());
                }
                let value = self.expr(value)?;
                let flow = self.flow(value);
                let ir::Ty::Ref { referent, .. } = &flow.ty else {
                    return Err("dereference requires a reference".into());
                };
                let ty = referent.as_ref().clone();
                self.accessible(&ty)?;
                (
                    ir::ExprKind::Deref { value },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::Array(elements) => {
                if elements.len() > ir::MAX_ARRAY_LEN {
                    return Err("array literal exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                if elements.is_empty() && hint.is_none() {
                    return Err("empty array literal inference is unsupported; provide a concrete expected array type".into());
                }
                let mut element = hint.cloned().unwrap_or(ir::Ty::Never);
                let mut values = Vec::new();
                let mut completes = true;
                for expression in elements {
                    let value =
                        self.expr_expected_when(expression, self.reachable && completes, hint)?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    if let Some(expected) = hint {
                        self.require(&flow, expected)?;
                    } else {
                        element = self.join(&element, &flow.ty, "array elements")?;
                    }
                    values.push(value);
                }
                (
                    ir::ExprKind::Array(values),
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: elements.len(),
                        },
                        completes,
                    },
                )
            }
            Expr::Repeat { value, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err("array repeat exceeds supported length".into());
                }
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                let value = self.expr_expected(value, hint)?;
                let flow = self.flow(value);
                if let Some(expected) = hint {
                    self.require(&flow, expected)?;
                }
                let element = hint.cloned().unwrap_or(flow.ty);
                if *len > 1 && !element.is_copy() {
                    return Err("array repeat of more than one element requires Copy".into());
                }
                (
                    ir::ExprKind::Repeat { value, len: *len },
                    Flow {
                        ty: ir::Ty::Array {
                            element: std::sync::Arc::new(element),
                            len: *len,
                        },
                        completes: flow.completes,
                    },
                )
            }
            Expr::DynamicIndex { base, index } => {
                if Self::dynamic_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = self.expr(base)?;
                if self.deref_source(base) {
                    return Err("runtime indexed access through references is unsupported in B1; snapshot a Copy referent first".into());
                }
                let source = self.flow(base);
                let ir::Ty::Array { element, .. } = &source.ty else {
                    return Err("runtime indexing requires an array".into());
                };
                let ty = element.as_ref().clone();
                self.accessible(&ty)?;
                let index = self.expr_expected_when(
                    index,
                    self.reachable && source.completes,
                    Some(&ir::Ty::Int(ir::IntTy::Usize)),
                )?;
                let index_flow = self.flow(index);
                self.require_index(&index_flow)?;
                (
                    ir::ExprKind::DynamicIndex { base, index },
                    Flow {
                        ty,
                        completes: source.completes && index_flow.completes,
                    },
                )
            }
            Expr::Index { base, index } => {
                if Self::dynamic_place(base) {
                    return Err("nested/chained runtime indexed place bases are unsupported; snapshot the row first".into());
                }
                let base = self.expr(base)?;
                let flow = self.flow(base);
                let ty = self.index_type(&flow.ty, *index)?;
                (
                    ir::ExprKind::Index {
                        base,
                        index: *index,
                    },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::EnumVariant {
                name,
                shape,
                fields,
            } => {
                let (ty, variant) = self.enum_variant(name)?;
                let definition = &ty.def().variants[variant];
                if *shape != definition.shape
                    && !(definition.shape == ir::VariantShape::Unit
                        && *shape == ir::VariantShape::Struct
                        && fields.is_empty())
                {
                    if *shape == ir::VariantShape::Unit
                        && definition.shape == ir::VariantShape::Tuple
                    {
                        return Err("first-class tuple variant constructors are unsupported".into());
                    }
                    return Err(format!("variant constructor {name} has wrong shape"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::new();
                let mut completes = true;
                for (selector, value) in fields {
                    let index = match selector {
                        FieldSelector::Tuple(i) if definition.shape == ir::VariantShape::Tuple => {
                            *i
                        }
                        FieldSelector::Named(n) => definition
                            .fields
                            .iter()
                            .position(|f| &f.name == n)
                            .ok_or_else(|| format!("unknown variant field {n}"))?,
                        _ => return Err("invalid variant constructor selector".into()),
                    };
                    let field = definition
                        .fields
                        .get(index)
                        .ok_or("variant constructor arity mismatch")?;
                    if !seen.insert(index) {
                        return Err("duplicate variant constructor field".into());
                    }
                    self.accessible(&field.ty)?;
                    let value = self.expr_expected_when(
                        value,
                        self.reachable && completes,
                        Some(&field.ty),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &field.ty)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != definition.fields.len() {
                    return Err("missing variant constructor fields".into());
                }
                (
                    ir::ExprKind::EnumVariant {
                        ty: ty.clone(),
                        variant,
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Enum(ty),
                        completes,
                    },
                )
            }
            Expr::Match { scrutinee, arms } => {
                if arms.len() > 4096 {
                    return Err("match arm count exceeds limit 4096".into());
                }
                let scrutinee = self.expr(scrutinee)?;
                if self.deref_source(scrutinee)
                    && arms.iter().any(|arm| {
                        matches!(arm.pattern, Pattern::Path(_) | Pattern::Variant { .. })
                    })
                {
                    return Err("enum downcasts after a dereference are unsupported in B1".into());
                }
                let source = self.flow(scrutinee);
                let mut domain = source.ty.clone();
                // A divergent operand can coerce to the concrete type named by
                // a pattern without inventing a general inference variable.
                if domain == ir::Ty::Never {
                    for arm in arms {
                        let hint = match &arm.pattern {
                            Pattern::Path(name) | Pattern::Variant { name, .. } => {
                                Some(ir::Ty::Enum(self.enum_variant(name)?.0))
                            }
                            Pattern::Bool(_) => Some(ir::Ty::Bool),
                            Pattern::Int { literal, .. } => {
                                Some(ir::Ty::Int(literal.suffix.unwrap_or(ir::IntTy::I32)))
                            }
                            Pattern::Tuple(fields) if fields.is_empty() => Some(ir::Ty::Unit),
                            _ => None,
                        };
                        if let Some(hint) = hint {
                            domain = hint;
                            break;
                        }
                    }
                }
                self.accessible(&domain)?;
                if matches!(domain, ir::Ty::Ref { .. })
                    && arms.iter().any(|arm| {
                        !matches!(arm.pattern, Pattern::Wildcard | Pattern::Binding { .. })
                    })
                {
                    return Err(
                        "reference patterns and implicit match ergonomics are unsupported in B1"
                            .into(),
                    );
                }
                let mut catchall = false;
                let mut covered = std::collections::HashSet::<u64>::new();
                let mut checked = Vec::with_capacity(arms.len());
                let mut result = ir::Ty::Never;
                let mut completes = false;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    let (pattern, key) = match &arm.pattern {
                        Pattern::Wildcard => (ir::MatchPattern::Wildcard, None),
                        Pattern::Binding { name, mutable } => {
                            let local = self.local(name, domain.clone(), *mutable)?;
                            (ir::MatchPattern::Binding(local), None)
                        }
                        Pattern::Bool(value) => {
                            if domain != ir::Ty::Bool {
                                return Err(format!(
                                    "bool match pattern does not match {domain:?}"
                                ));
                            }
                            (ir::MatchPattern::Bool(*value), Some(u64::from(*value)))
                        }
                        Pattern::Int { literal, negative } => {
                            if !matches!(domain, ir::Ty::Int(_)) {
                                return Err(format!(
                                    "integer match pattern does not match {domain:?}"
                                ));
                            }
                            let value = resolve_integer(literal, *negative, Some(&domain))?;
                            (ir::MatchPattern::Int(value), Some(value.bits))
                        }
                        Pattern::Tuple(fields) if fields.is_empty() => {
                            if domain != ir::Ty::Unit {
                                return Err(format!(
                                    "unit match pattern does not match {domain:?}"
                                ));
                            }
                            (ir::MatchPattern::Unit, Some(0))
                        }
                        Pattern::Path(_) | Pattern::Variant { .. } => {
                            let (ty, variant, fields) =
                                self.variant_pattern(&arm.pattern, &domain)?;
                            let mut names = std::collections::HashSet::new();
                            let mut pending = Vec::new();
                            let mut work = 0;
                            let mut bindings = Vec::new();
                            for (index, pattern) in fields {
                                let mut path = vec![
                                    ir::Projection::Downcast(ir::VariantId {
                                        owner: ty.def().id,
                                        index: variant,
                                    }),
                                    ir::Projection::Field(index),
                                ];
                                self.pattern_locals(
                                    pattern,
                                    &ty.def().variants[variant].fields[index].ty,
                                    &mut path,
                                    &mut names,
                                    &mut pending,
                                    &mut work,
                                )?;
                            }
                            for binding in pending {
                                let local =
                                    self.local(&binding.name, binding.ty, binding.mutable)?;
                                bindings.push(ir::PatternBinding {
                                    local,
                                    projection: binding.projection,
                                });
                            }
                            (
                                ir::MatchPattern::EnumVariant {
                                    ty,
                                    variant,
                                    bindings,
                                },
                                Some(variant as u64),
                            )
                        }
                        Pattern::Tuple(_) | Pattern::Struct { .. } => {
                            return Err(
                                "aggregate destructuring match patterns are unsupported".into()
                            );
                        }
                    };
                    let fully_covered = match &domain {
                        ir::Ty::Bool => covered.len() == 2,
                        ir::Ty::Unit => covered.len() == 1,
                        ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                        ir::Ty::Int(ty) => {
                            ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                        }
                        _ => false,
                    };
                    let useful = !catchall
                        && !fully_covered
                        && key.is_none_or(|key| !covered.contains(&key));
                    let body = self.expr_expected_when(
                        &arm.body,
                        self.reachable && source.completes && useful,
                        expected,
                    )?;
                    self.scopes.pop();
                    let arm_flow = self.flow(body);
                    // Even unreachable arms impose their source type obligation.
                    result = self.join(&result, &arm_flow.ty, "match arms")?;
                    completes |= useful && arm_flow.completes;
                    if let Some(key) = key {
                        covered.insert(key);
                    } else {
                        catchall = true;
                    }
                    checked.push(ir::MatchArm { pattern, body });
                }
                let exhaustive = catchall
                    || match &domain {
                        ir::Ty::Never => true,
                        ir::Ty::Bool => covered.len() == 2,
                        ir::Ty::Unit => covered.len() == 1,
                        ir::Ty::Enum(ty) => covered.len() == ty.def().variants.len(),
                        ir::Ty::Int(ty) => {
                            ty.bits() < 64 && covered.len() as u64 == (1u64 << ty.bits())
                        }
                        _ => false,
                    };
                if !exhaustive {
                    return Err(format!("non-exhaustive match on {domain:?}"));
                }
                (
                    ir::ExprKind::Match {
                        scrutinee,
                        arms: checked,
                    },
                    Flow {
                        ty: result,
                        completes: source.completes && completes,
                    },
                )
            }
            Expr::Tuple(fields) => {
                if fields.is_empty() {
                    return Err("empty tuples must use ()".into());
                }
                let mut values = Vec::with_capacity(fields.len());
                let mut types = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (index, field) in fields.iter().enumerate() {
                    let hint = match expected {
                        Some(ir::Ty::Tuple(types)) => types.get(index),
                        _ => None,
                    };
                    let value =
                        self.expr_expected_when(field, self.reachable && completes, hint)?;
                    let flow = self.flow(value);
                    completes &= flow.completes;
                    types.push(flow.ty);
                    values.push(value);
                }
                (
                    ir::ExprKind::Tuple(values),
                    Flow {
                        ty: ir::Ty::Tuple(types.into()),
                        completes,
                    },
                )
            }
            Expr::Struct { name, fields } => {
                let ty = self
                    .signatures
                    .structs
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown struct {name}"))?;
                if !ty.def().public && !visible_from(&ty.def().module, &self.module) {
                    return Err(format!("private struct {name}"));
                }
                let mut seen = std::collections::HashSet::new();
                let mut values = Vec::with_capacity(fields.len());
                let mut completes = true;
                for (name, value) in fields {
                    let (index, expected) = self.field_type(
                        &ir::Ty::Struct(ty.clone()),
                        &FieldSelector::Named(name.clone()),
                    )?;
                    if !seen.insert(index) {
                        return Err(format!("duplicate field {name}"));
                    }
                    let value = self.expr_expected_when(
                        value,
                        self.reachable && completes,
                        Some(&expected),
                    )?;
                    let flow = self.flow(value);
                    self.require(&flow, &expected)?;
                    completes &= flow.completes;
                    values.push((index, value));
                }
                if seen.len() != ty.def().fields.len() {
                    return Err(format!("missing fields in struct {}", ty.def().name));
                }
                (
                    ir::ExprKind::Struct {
                        ty: ty.clone(),
                        fields: values,
                    },
                    Flow {
                        ty: ir::Ty::Struct(ty),
                        completes,
                    },
                )
            }
            Expr::Field { base, field } => {
                let receiver = base.as_ref();
                let mut base = self.expr(base)?;
                let mut flow = self.flow(base);
                if let ir::Ty::Ref { referent, .. } = &flow.ty {
                    if !matches!(receiver, Expr::Var(_) | Expr::Call { .. }) {
                        return Err("complex reference field receivers are unsupported; use a local or genuine call result".into());
                    }
                    let ty = referent.as_ref().clone();
                    self.accessible(&ty)?;
                    let deref = ir::ExprId(self.exprs.len());
                    self.exprs.push(ir::TypedExpr {
                        ty: ty.clone(),
                        completion: self.exprs[base.0].completion,
                        kind: ir::ExprKind::Deref { value: base },
                    });
                    base = deref;
                    flow.ty = ty;
                }
                let (index, ty) = self.field_type(&flow.ty, field)?;
                (
                    ir::ExprKind::Field { base, index },
                    Flow {
                        ty,
                        completes: flow.completes,
                    },
                )
            }
            Expr::Integer(literal) => {
                let value = expectation.literal(literal, false)?;
                (ir::ExprKind::Int(value), Flow::value(ir::Ty::Int(value.ty)))
            }
            Expr::Bool(value) => (ir::ExprKind::Bool(*value), Flow::value(ir::Ty::Bool)),
            Expr::Var(n) => {
                let local = self.lookup(n)?;
                let ty = self.locals[local.0].ty.clone();
                (
                    ir::ExprKind::Local(local),
                    Flow {
                        completes: inhabited(&ty),
                        ty,
                    },
                )
            }
            Expr::Println(text) => (
                ir::ExprKind::Println(text.clone()),
                Flow::value(ir::Ty::Unit),
            ),
            Expr::Unary { op, value } => {
                if matches!(op, UnaryOp::Neg)
                    && let Expr::Integer(literal) = value.as_ref()
                {
                    let value = expectation.literal(literal, true)?;
                    let id = ir::ExprId(self.exprs.len());
                    self.exprs.push(ir::TypedExpr {
                        kind: ir::ExprKind::Int(value),
                        ty: ir::Ty::Int(value.ty),
                        completion: ir::Completion::Returns,
                    });
                    return Ok(id);
                }
                let hint = expected
                    .filter(|t| matches!(t, ir::Ty::Int(_) | ir::Ty::Bool))
                    .cloned()
                    .or_else(|| self.scalar_hint(value));
                let value = if matches!(expectation, Expectation::CastableToInt(_)) {
                    self.expr_with_expectation(value, expectation)?
                } else {
                    self.expr_expected(value, hint.as_ref())?
                };
                let f = self.flow(value);
                let t = match op {
                    UnaryOp::Neg => match &f.ty {
                        ir::Ty::Int(ty) if ty.is_signed() => f.ty.clone(),
                        ir::Ty::Never => {
                            return Err("unary negation of never is unsupported".into());
                        }
                        _ => return Err("unary negation requires a signed integer".into()),
                    },
                    UnaryOp::Not if matches!(f.ty, ir::Ty::Int(_)) => f.ty.clone(),
                    UnaryOp::Not => ir::Ty::Bool,
                };
                self.require(&f, &t)?;
                if matches!(op, UnaryOp::Neg) && f.ty == ir::Ty::Never {
                    return Err("unary negation of never is unsupported".into());
                }
                let ty = if f.ty == ir::Ty::Never {
                    ir::Ty::Never
                } else {
                    t
                };
                (
                    ir::ExprKind::Unary { op: *op, value },
                    Flow {
                        ty,
                        completes: f.completes,
                    },
                )
            }
            Expr::Binary { op, left, right } => {
                let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
                let shifting = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
                let hint = if logical {
                    Some(ir::Ty::Bool)
                } else {
                    expected
                        .filter(|t| matches!(t, ir::Ty::Int(_)))
                        .cloned()
                        .or_else(|| self.scalar_hint(left))
                        .or_else(|| {
                            if shifting {
                                None
                            } else {
                                self.scalar_hint(right)
                            }
                        })
                };
                let left = self.expr_expected(left, hint.as_ref())?;
                let left_flow = self.flow(left);
                let right_hint = if shifting {
                    None
                } else {
                    hint.or_else(|| {
                        if matches!(left_flow.ty, ir::Ty::Int(_) | ir::Ty::Bool) {
                            Some(left_flow.ty.clone())
                        } else {
                            None
                        }
                    })
                };
                let right = self.expr_expected_when(
                    right,
                    self.reachable && left_flow.completes,
                    right_hint.as_ref(),
                )?;
                let l = self.flow(left);
                let r = self.flow(right);
                if matches!(l.ty, ir::Ty::Ref { .. }) || matches!(r.ty, ir::Ty::Ref { .. }) {
                    return Err("reference comparisons and arithmetic are unsupported in B1".into());
                }
                if matches!(l.ty, ir::Ty::Tuple(_) | ir::Ty::Struct(_))
                    || matches!(r.ty, ir::Ty::Tuple(_) | ir::Ty::Struct(_))
                {
                    return Err("aggregate operators and comparisons are unsupported".into());
                }
                let numeric = if matches!(l.ty, ir::Ty::Int(_)) {
                    l.ty.clone()
                } else if matches!(r.ty, ir::Ty::Int(_)) {
                    r.ty.clone()
                } else {
                    ir::Ty::Int(ir::IntTy::I32)
                };
                let (operand, result) = match op {
                    BinaryOp::And | BinaryOp::Or => (ir::Ty::Bool, ir::Ty::Bool),
                    BinaryOp::Shl | BinaryOp::Shr => {
                        if l.ty == ir::Ty::Never || r.ty == ir::Ty::Never {
                            return Err("shift operator on semantic Never is unsupported".into());
                        }
                        if !matches!(l.ty, ir::Ty::Int(_)) || !matches!(r.ty, ir::Ty::Int(_)) {
                            return Err("shift operands must both be integers".into());
                        }
                        (l.ty.clone(), l.ty.clone())
                    }
                    BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                        let operand = self.join(&l.ty, &r.ty, "bitwise operands")?;
                        if !matches!(operand, ir::Ty::Int(_) | ir::Ty::Bool) {
                            return Err(
                                "bitwise operands require identical integer types or Bool".into()
                            );
                        }
                        (operand.clone(), operand)
                    }
                    BinaryOp::Eq | BinaryOp::Ne => {
                        (self.join(&l.ty, &r.ty, "equality operands")?, ir::Ty::Bool)
                    }
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        (numeric.clone(), ir::Ty::Bool)
                    }
                    _ => (numeric.clone(), numeric),
                };
                self.require(&l, &operand)?;
                if !shifting {
                    self.require(&r, &operand)?;
                }
                // Trait-based scalar operators do not gain a never coercion
                // merely because evaluating an operand cannot complete.
                if !logical && (l.ty == ir::Ty::Never || r.ty == ir::Ty::Never) {
                    return Err("binary operator on never is unsupported".into());
                }
                let completes = l.completes && (logical || r.completes);
                (
                    ir::ExprKind::Binary {
                        op: *op,
                        left,
                        right,
                    },
                    Flow {
                        ty: result,
                        completes,
                    },
                )
            }
            Expr::Call { name, args } => {
                let signature = self
                    .signatures
                    .functions
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                for ty in signature
                    .params
                    .iter()
                    .chain(std::iter::once(&signature.result))
                {
                    self.accessible(ty)?;
                }
                if signature.params.len() != args.len() {
                    return Err(format!(
                        "wrong arity for {name}: expected {}, got {}",
                        signature.params.len(),
                        args.len()
                    ));
                }
                let mut completes = true;
                let mut arguments = Vec::with_capacity(args.len());
                for (a, t) in args.iter().zip(&signature.params) {
                    let value = self.expr_expected_when(a, self.reachable && completes, Some(t))?;
                    let f = self.flow(value);
                    self.require(&f, t)?;
                    completes &= f.completes;
                    arguments.push(value);
                }
                let result_ty = signature.result.clone();
                (
                    ir::ExprKind::Call {
                        name: name.clone(),
                        args: arguments,
                        signature,
                    },
                    Flow {
                        ty: result_ty,
                        completes,
                    },
                )
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.expr(condition)?;
                let c = self.flow(condition);
                self.require(&c, &ir::Ty::Bool)?;
                let reachable = self.reachable && c.completes;
                let (then_block, t) = self.block_expected_when(then_block, reachable, expected)?;
                let (else_block, e) = self.block_expected_when(else_block, reachable, expected)?;
                let ty = self.join(&t.ty, &e.ty, "if branches")?;
                let completes = c.completes && (t.completes || e.completes);
                (
                    ir::ExprKind::If {
                        condition,
                        then_block,
                        else_block,
                    },
                    Flow { ty, completes },
                )
            }
            Expr::Loop { body } => {
                let id = self.enter_loop(true);
                // A loop has its own intrinsic completion facts even when the
                // enclosing expression is unreachable. Only its own body paths
                // determine whether one of its breaks can complete.
                let (body, body_flow) = self.block_when(body, true)?;
                self.require(&body_flow, &ir::Ty::Unit)?;
                let frame = self.loops.pop().unwrap();
                let flow = Flow {
                    ty: frame.break_ty.unwrap_or(ir::Ty::Never),
                    completes: frame.can_exit,
                };
                (ir::ExprKind::Loop { id, body }, flow)
            }
            Expr::Break { value } => {
                let target = self.loop_target("break")?;
                let value = value.as_ref().map(|e| self.expr(e)).transpose()?;
                let flow = value.map_or(Flow::value(ir::Ty::Unit), |e| self.flow(e));
                let frame = self.loops.last().unwrap();
                let previous_break_ty = frame.break_ty.clone();
                if !frame.accepts_value && value.is_some() {
                    return Err("break with a value is only allowed in loop, not while".into());
                }
                // Check every lexical break and the operand before contributing.
                // A typed but unreachable tail is not the never type.
                if self.reachable && flow.completes {
                    self.loops.last_mut().unwrap().can_exit = true;
                }
                if flow.ty != ir::Ty::Never {
                    let previous = previous_break_ty.unwrap_or(ir::Ty::Never);
                    let ty = self.join(&previous, &flow.ty, "break values")?;
                    self.loops.last_mut().unwrap().break_ty = Some(ty);
                }
                (ir::ExprKind::Break { target, value }, Flow::never())
            }
            Expr::Continue => {
                let target = self.loop_target("continue")?;
                (ir::ExprKind::Continue { target }, Flow::never())
            }
            Expr::Block(b) => {
                let (block, flow) = self.block_with_expectation(b, expectation)?;
                (ir::ExprKind::Block(block), flow)
            }
        };
        self.accessible(&flow.ty)?;
        if matches!(flow.ty, ir::Ty::Array { .. } | ir::Ty::Tuple(_)) {
            Self::reject_aggregate_ref(&flow.ty)?;
        }
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: flow.ty,
            completion: if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind,
        });
        Ok(id)
    }
}
/// Validate all declarations and publish one shared nominal type environment.
pub fn signature_table(program: &Program) -> Result<Signatures, String> {
    type_env(program)
}
pub fn check_signatures(program: &Program) -> Result<(), String> {
    signature_table(program).map(|_| ())
}
pub fn check_function(program: &Program, index: usize) -> Result<(), String> {
    let signatures = signature_table(program)?;
    check_function_with_signatures(program, index, &signatures)
}
/// `signatures` must be the validated table for this same immutable program.
/// Direct semantic checking includes CFG initialization/move analysis once.
/// Graph callers use the pure type_function_with_env operation instead.
pub fn check_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<(), String> {
    let typed = type_function_with_signatures(program, index, signatures)?;
    let lowered = super::cfg::lower(&typed)?;
    let analysis = super::ownership::analyze(&lowered);
    if !analysis.passed() {
        return Err(format!(
            "{}: initialization/move analysis (complete={}): {:?}",
            lowered.name, analysis.complete, analysis.diagnostics
        ));
    }
    Ok(())
}
/// Check and lower one body using the validated signatures for this program.
/// The returned arena owns all resolved locals, expression types and call facts.
pub fn type_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<ir::CheckedFunction, String> {
    type_function_with_env(program, index, signatures)
}
/// Check one body with the same nominal environment used for all signatures.
pub fn type_function_with_env(
    program: &Program,
    index: usize,
    signatures: &TypeEnv,
) -> Result<ir::CheckedFunction, String> {
    let f = program
        .functions
        .get(index)
        .ok_or_else(|| format!("invalid function index {index}"))?;
    validate_depth(DepthNode::Block(&f.body))?;
    let signature = signatures
        .functions
        .get(&f.name)
        .ok_or_else(|| format!("missing signature for {}", f.name))?;
    if signature.params.len() != f.params.len() {
        return Err(format!("signature parameter count mismatch for {}", f.name));
    }
    let mut c = Checker::new(signatures, signature.result.clone(), parent(&f.name).into());
    let mut params = Vec::with_capacity(f.params.len());
    for ((n, _), t) in f.params.iter().zip(&signature.params) {
        params.push(
            c.bind(n, t.clone())
                .map_err(|e| format!("in function {}: {e}", f.name))?,
        );
    }
    let (body, flow) = c
        .block_expected(&f.body, Some(&signature.result))
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    c.require(&flow, &c.result)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    validate_shift_counts(&c.exprs, &c.count_queries)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    Ok(ir::CheckedFunction {
        name: f.name.clone(),
        ref_contract: signature.ref_contract.clone(),
        params,
        result: c.result,
        locals: c.locals,
        exprs: c.exprs,
        body,
    })
}
pub fn check(program: &Program) -> Result<(), String> {
    let signatures = signature_table(program)?;
    for i in 0..program.functions.len() {
        check_function_with_signatures(program, i, &signatures)?;
    }
    Ok(())
}

#[cfg(test)]
mod typed_tests {
    use super::*;

    #[test]
    fn compound_statement_tokens_types_and_expression_boundaries() {
        for name in [
            "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize",
        ] {
            let source = format!(
                "fn main(){{let mut x:{name}=4;x+=1;x-=1;x*=2;x/=2;x%=3;x&=3;x|=1;x^=2;x<<=1u64;x>>=1i8;}}"
            );
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
            assert_eq!(
                f.body
                    .statements
                    .iter()
                    .filter(|s| matches!(s, ir::Stmt::AssignOp { .. }))
                    .count(),
                10
            );
        }
        check(&parse("fn main(){let mut b=true;b&=false;b|=true;b^=true;let mut a=[1u8];let i=0usize;a[i]+=2;}").unwrap()).unwrap();
        for source in [
            "fn main(){let mut x=1;let y=(x+=1);}",
            "fn main(){let mut x=1;x \"+=\" 1;}",
            "fn main(){let mut x=1u8;x+=1u16;}",
            "fn main(){let mut x=true;x+=false;}",
            "fn main(){let mut x=1;x<<=true;}",
            "fn main(){let mut x=(1,2);x+=x;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let mut p = parse("fn main(){let mut x=1;x+=2;}").unwrap();
        let Stmt::AssignOp { op, .. } = &mut p.functions[0].body.statements[1] else {
            panic!("compound root place")
        };
        *op = BinaryOp::Eq;
        assert!(check(&p).unwrap_err().contains("operator"));
    }

    #[test]
    fn compound_current_place_rhs_initialization_and_permission_checks() {
        for source in [
            "fn main(){let mut x:i32;x+={x=10;1};}",
            "fn main(){let mut x=1;let mut y=10;let mut p=&mut x;*p+={p=&mut y;2};}",
            "fn main(){let mut a:[i32;1];let i=0usize;a[i]+={a=[10];2};}",
            "fn main(){let mut a=[1];a[{a=[10];0usize}]+={a=[3];2};}",
            "fn main(){let mut x=1;return;x+=1;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let mut x:i32;x+=1;}",
            "fn main(){let x=1;x+=1;}",
            "fn main(){let x=1;let r=&x;*r+=1;}",
            "fn main(){let a=[1];let i=0usize;a[i]+=1;}",
            "fn main(){let mut x=1;let r=&x;x+=1;let y=*r;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p =
            parse("fn rhs()->i32{1}fn idx()->usize{0}fn main(){let mut a=[1];a[idx()]+=rhs();}")
                .unwrap();
        let f = type_function_with_signatures(&p, 2, &signature_table(&p).unwrap()).unwrap();
        assert_eq!(
            f.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        assert!(
            f.body
                .statements
                .iter()
                .any(|s| matches!(s, ir::Stmt::AssignIndexOp { .. }))
        );
    }

    #[test]
    fn compound_shift_queries_share_count_memo_and_check_dead_statements() {
        for source in [
            "fn main(){let mut x=1u8;x<<=8u64;}",
            "fn main(){let mut x=1u8;x>>=-1i16;}",
            "fn main(){let mut a=[1u8];let i=0usize;a[i]<<=3u16+5u16;}",
            "fn main(){let mut x=1u8;return;x<<=8u64;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p = parse("fn main(){let mut x=1u8;let n=8u64;x<<=n;x>>={x=3;1u32};let y=1u8<<1u64;}")
            .unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        let ir::Stmt::AssignOp { value, .. } = f.body.statements[2] else {
            panic!("compound shift")
        };
        assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::U64));
        let literal = ir::TypedExpr {
            kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
            ty: ir::Ty::Int(ir::IntTy::I32),
            completion: ir::Completion::Returns,
        };
        let queries = vec![(ir::IntTy::U8, ir::ExprId(0)); COUNT_EVAL_WORK / 2 + 1];
        assert!(
            validate_shift_counts(&[literal], &queries)
                .unwrap_err()
                .contains("work")
        );
    }

    #[test]
    fn shifts_keep_independent_integer_operands_and_parser_precedence() {
        let names = [
            "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize",
        ];
        for lhs in names {
            for rhs in names {
                let source = format!("fn f(x:{lhs},n:{rhs})->{lhs}{{(x<<n)>>n}}fn main(){{}}");
                let p = parse(&source).unwrap();
                check(&p).unwrap();
                let f =
                    type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
                for expr in &f.exprs {
                    if let ir::ExprKind::Binary {
                        op: BinaryOp::Shl | BinaryOp::Shr,
                        left,
                        right,
                    } = expr.kind
                    {
                        assert_eq!(expr.ty, f.exprs[left.0].ty);
                        assert_eq!(f.exprs[right.0].ty, f.locals[f.params[1].0].ty);
                    }
                }
            }
        }
        let p = parse("fn f()->u8{1<<1u64}fn g(x:u8)->u8{x<<1}fn main(){let x=1<<1u64;let y=(1i16 as u8)<<1;let z=4i32 as u8>>1;let p=1 | 2 & 3 << 4+1*2;}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let g = type_function_with_signatures(&p, 1, &env).unwrap();
        let ir::ExprKind::Binary { right, .. } = g.exprs[g.body.tail.unwrap().0].kind else {
            panic!("shift")
        };
        assert_eq!(g.exprs[right.0].ty, ir::Ty::Int(ir::IntTy::I32));
        let main = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(main.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
        for source in [
            "fn main(){let x=true<<1;}",
            "fn main(){let x=1>>false;}",
            "fn main(){let x=1i32 as u8<<1;}",
            "fn main(){let x=1 \"<<\" 1;}",
        ] {
            assert!(parse(source).and_then(|p| check(&p)).is_err());
        }
        for op in ["<<=", ">>="] {
            check(&parse(&format!("fn main(){{let mut x=1;x{op}1;}} ")).unwrap()).unwrap();
        }
    }

    #[test]
    fn shifts_static_count_grammar_and_runtime_leaves_are_distinct() {
        for count in [
            "8u8",
            "-1i16",
            "(3u16+5u16)",
            "(1u64<<3u8)",
            "(!0u8)",
            "((-1i8) as u64)",
            "{8u32}",
            "(255u8+1u8)",
            "(1u8/0u8)",
            "((-128i8)/(-1i8))",
        ] {
            let source = format!("fn main(){{let x=1u8<<({count});}}");
            assert!(
                parse(&source).and_then(|p| check(&p)).is_err(),
                "accepted {count}"
            );
        }
        check(&parse("fn count()->u64{64}fn f(x:u8,n:u64)->u8{x<<n}fn main(){let n=8u64;let x=1u8<<n;let y=1u64>>count();let z=1u8<<((3u16^1u16) as u64);let a=1u64<<((18446744073709551615u64*0u64) as u8);}").unwrap()).unwrap();
        assert!(
            parse("fn main(){let x=1u8<<(if true{1}else{2});}")
                .and_then(|p| check(&p))
                .unwrap_err()
                .contains("unsupported")
        );
        check(&parse("fn main(){let mut bits=3u8;let snapshot=bits<<{bits=9u8;1u32};}").unwrap())
            .unwrap();
        let p =
            parse("fn dead()->u8{1u8<<({return 7u8;1u32})}fn once(n:u8)->u8{1u8<<n}fn main(){}")
                .unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        assert_eq!(
            f.exprs[f.body.tail.unwrap().0].completion,
            ir::Completion::Never
        );
    }

    #[test]
    fn shift_count_memo_bounds_cycles_and_never_values() {
        let literal = ir::TypedExpr {
            kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
            ty: ir::Ty::Int(ir::IntTy::I32),
            completion: ir::Completion::Returns,
        };
        let mut arena = vec![literal.clone()];
        for index in 0..100 {
            arena.push(ir::TypedExpr {
                kind: ir::ExprKind::Binary {
                    op: BinaryOp::BitOr,
                    left: ir::ExprId(index),
                    right: ir::ExprId(index),
                },
                ..literal.clone()
            });
        }
        let mut eval = CountEvaluator::new(&arena).unwrap();
        assert!(matches!(
            eval.evaluate(ir::ExprId(100)).unwrap(),
            CountValue::Known(_)
        ));
        assert!(eval.work < 1000);
        eval.work = COUNT_EVAL_WORK;
        assert!(eval.evaluate(ir::ExprId(0)).unwrap_err().contains("work"));
        let mut deep = vec![literal.clone()];
        for index in 0..130 {
            deep.push(ir::TypedExpr {
                kind: ir::ExprKind::IntCast {
                    value: ir::ExprId(index),
                    to: ir::IntTy::I32,
                },
                ..literal.clone()
            });
        }
        let mut eval = CountEvaluator::new(&deep).unwrap();
        eval.evaluate(ir::ExprId(100)).unwrap();
        assert!(
            eval.evaluate(ir::ExprId(130))
                .unwrap_err()
                .contains("depth")
        );
        let cyclic = [ir::TypedExpr {
            kind: ir::ExprKind::IntCast {
                value: ir::ExprId(0),
                to: ir::IntTy::I32,
            },
            ..literal.clone()
        }];
        assert!(
            CountEvaluator::new(&cyclic)
                .unwrap()
                .evaluate(ir::ExprId(0))
                .unwrap_err()
                .contains("cycle")
        );
        let never = [ir::TypedExpr {
            completion: ir::Completion::Never,
            ..literal
        }];
        assert!(matches!(
            CountEvaluator::new(&never)
                .unwrap()
                .evaluate(ir::ExprId(0))
                .unwrap(),
            CountValue::Dynamic
        ));
    }

    #[test]
    fn bitwise_exact_integer_and_boolean_types_preserve_operand_hints() {
        for name in [
            "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize",
        ] {
            let source = format!("fn f(x:{name})->{name}{{(x & 3) | (1 ^ x)}}fn main(){{}}");
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
            for expr in &f.exprs {
                if matches!(
                    expr.kind,
                    ir::ExprKind::Binary {
                        op: BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor,
                        ..
                    }
                ) {
                    assert_eq!(expr.ty, f.result);
                }
            }
        }
        check(&parse("fn f(x:bool)->bool{(x & true) | (false ^ x)}fn main(){let x=(1i8<2i8)&(3i16<4i16);let y=(200+100)|1;let z=((200+100)|1) as u8;}").unwrap()).unwrap();
        for source in [
            "fn main(){let x=1u8 & 2u16;}",
            "fn main(){let x=true | 1;}",
            "fn main(){let x=() ^ ();}",
            "fn main(){let x=(1,2) & (3,4);}",
            "fn main(){let x=1;let r=&x;let z=r|r;}",
            "fn main(){let x:u8=256 | 0;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn bitwise_pratt_precedence_and_compound_boundaries() {
        let p = parse("fn main(){let x=1 | 2 ^ 3 & 4 + 5 * 6i16 as i32;}").unwrap();
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::BitOr,
                    right,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("OR precedence")
        };
        let Expr::Binary {
            op: BinaryOp::BitXor,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("XOR precedence")
        };
        let Expr::Binary {
            op: BinaryOp::BitAnd,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("AND precedence")
        };
        let Expr::Binary {
            op: BinaryOp::Add,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("addition precedence")
        };
        let Expr::Binary {
            op: BinaryOp::Mul,
            right,
            ..
        } = right.as_ref()
        else {
            panic!("multiplication precedence")
        };
        assert!(matches!(right.as_ref(), Expr::Cast { .. }));
        check(&parse("fn main(){let x=1i32;let r=&x;let a=*r & -1 | !0 ^ 3;let b=1|2|3;let c=1|2==3;let d=true || false & true && false;}").unwrap()).unwrap();
        for op in ["&=", "|=", "^="] {
            let source = format!("fn main(){{let mut x=1;x{op}2;}}");
            check(&parse(&source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let x=1 \"|\" 2;}",
            "fn main(){let x=1 \"^\" 2;}",
            "fn main(){let x=1<2<3;}",
        ] {
            assert!(parse(source).is_err());
        }
    }

    #[test]
    fn bitwise_eager_completion_and_once_only_operands() {
        let p = parse("fn source()->bool{true}fn f()->bool{source() & source()}fn dead()->bool{false & {return true;false}}fn main(){}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let f = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            f.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        let dead = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(
            dead.exprs[dead.body.tail.unwrap().0].completion,
            ir::Completion::Never
        );
        // An eager RHS read is checked even when the left Boolean decides the value.
        assert!(check(&parse("fn main(){let x:bool;let y=false & x;}").unwrap()).is_err());
        assert!(check(&parse("fn main(){let x:bool;let y=true | x;}").unwrap()).is_err());
    }

    #[test]
    fn cast_soft_expectations_preserve_concrete_sources_and_drop_at_operators() {
        use ir::IntTy::*;
        for (body, source_ty) in [
            ("300i32 as u8", I32),
            ("{let x=300;x} as u8", I32),
            ("(200+100) as u8", I32),
            ("(if true{300}else{301}) as u8", I32),
            ("(match true{true=>300,false=>301}) as u8", I32),
            ("(loop{break 300;}) as u8", I32),
            ("!0 as u8", U8),
            ("-128i8 as u8", I8),
        ] {
            // Parenthesize the outer operand when the body begins a naked block.
            let body = if body.starts_with('{') {
                format!("({body})")
            } else {
                body.to_string()
            };
            let source = format!("fn f()->u8{{{body}}}fn main(){{}}");
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let env = signature_table(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &env).unwrap();
            let ir::ExprKind::IntCast { value, to } = f.exprs[f.body.tail.unwrap().0].kind else {
                panic!("not a cast: {source}")
            };
            assert_eq!(to, U8);
            assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(source_ty), "{source}");
        }
        for source in [
            "fn main(){let x=300 as u8;}",
            "fn main(){let x=({300}) as u8;}",
            "fn main(){let x=(-1) as u8;}",
            "fn main(){let x=300 as u8 as u16;}",
            "fn main(){let x=(1u8+300i32) as u16;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        check(&parse("fn wide()->u64{18446744073709551615 as u64}fn f()->u16{(-1i8) as u16}fn main(){let x:u16=300i32 as u8 as u16;}").unwrap()).unwrap();
    }

    #[test]
    fn cast_parser_preserves_precedence_statement_boundaries_and_typed_tokens() {
        let p=parse("fn main(){let x=-1i16 as u8*2;let y=1i32 as u8 as u16;let z=1+2i32 as u8;let b=(1i32 as u8)<2;}").unwrap();
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::Mul,
                    left,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("multiplication precedence")
        };
        assert!(
            matches!(left.as_ref(),Expr::Cast{value,..} if matches!(value.as_ref(),Expr::Unary{op:UnaryOp::Neg,..}))
        );
        let Stmt::Let {
            value: Expr::Cast { value, .. },
            ..
        } = &p.functions[0].body.statements[1]
        else {
            panic!("cast chain")
        };
        assert!(matches!(value.as_ref(), Expr::Cast { .. }));
        let Stmt::Let {
            value:
                Expr::Binary {
                    op: BinaryOp::Add,
                    right,
                    ..
                },
            ..
        } = &p.functions[0].body.statements[2]
        else {
            panic!("addition precedence")
        };
        assert!(matches!(right.as_ref(), Expr::Cast { .. }));
        for source in [
            "fn main(){let x=1 \"as\" u8;}",
            "fn main(){let x=1 as \"u8\";}",
            "fn main(){let x:\"u8\"=1;}",
            "fn main(){let x=1i32 as u8<2;}",
            "fn main(){let x=1i32 as u8.0;}",
            "fn main(){let x=1i32 as u8[0];}",
            "fn main(){let x=1i32 as u8();}",
            "fn f()->u8{{1} as u8}fn main(){}",
            "fn f()->u8{if true{1}else{2} as u8}fn main(){}",
            "fn f()->u8{match true{_=>1} as u8}fn main(){}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
        assert!(
            parse("fn main(){let x=1i32 as (u8);}")
                .unwrap_err()
                .contains("unsupported")
        );
        check(&parse("fn f()->u8{({1}) as u8}fn g()->u8{(if true{1}else{2}) as u8}fn main(){let b=(1i32 as u8)<2;}").unwrap()).unwrap();
    }

    #[test]
    fn cast_completion_and_once_only_operands_do_not_create_values_or_origins() {
        let p=parse("fn source()->i64{300}fn once()->u8{source() as u8}fn typed_dead()->u8{({return 1u8;300i32}) as u8}fn read(r:&i16)->u8{*r as u8}fn main(){}").unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let once = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            once.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        let dead = type_function_with_signatures(&p, 2, &env).unwrap();
        let cast = &dead.exprs[dead.body.tail.unwrap().0];
        assert_eq!(cast.ty, ir::Ty::Int(ir::IntTy::U8));
        assert_eq!(cast.completion, ir::Completion::Never);
        let ir::ExprKind::IntCast { value, .. } = cast.kind else {
            panic!("expected typed dead cast")
        };
        assert_eq!(dead.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(dead.exprs[value.0].completion, ir::Completion::Never);
        for source in [
            "fn main(){let x=(loop{}) as u8;}",
            "fn main(){let x=true as u8;}",
            "fn main(){let x=1i32;let r=&x;let n=r as usize;}",
        ] {
            assert!(
                parse(source)
                    .and_then(|p| check(&p))
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
        let within = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(120));
        check(&parse(&within).unwrap()).unwrap();
        let source = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(140));
        assert!(parse(&source).unwrap_err().contains("nesting"));
    }

    #[test]
    fn integer_literal_bounds_contexts_and_grouped_minima_are_exact() {
        use ir::IntTy::*;
        for ty in [I8, I16, I32, I64, Isize, U8, U16, U32, U64, Usize] {
            let max = if ty.is_signed() {
                (1u64 << (ty.bits() - 1)) - 1
            } else {
                ty.mask()
            };
            let source = format!(
                "fn f()->{}{{{max}}}fn main(){{let x:{}={max};let y={max}{};let z=1+y;let n=!y;}}",
                ty.name(),
                ty.name(),
                ty.name()
            );
            check(&parse(&source).unwrap()).unwrap();
            let value = resolve_integer(
                &IntegerLiteral {
                    magnitude: max,
                    suffix: Some(ty),
                },
                false,
                None,
            )
            .unwrap();
            assert_eq!(value, ir::IntValue { ty, bits: max });
            if max < u64::MAX {
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: max + 1,
                            suffix: Some(ty)
                        },
                        false,
                        None
                    )
                    .is_err()
                );
            }
            if ty.is_signed() {
                let magnitude = 1u64 << (ty.bits() - 1);
                let source = format!(
                    "fn f()->{}{{-((({magnitude}{})))}}fn main(){{let x:{}=-(({magnitude}));}}",
                    ty.name(),
                    ty.name(),
                    ty.name()
                );
                let program = parse(&source).unwrap();
                check(&program).unwrap();
                let env = signature_table(&program).unwrap();
                let function = type_function_with_signatures(&program, 0, &env).unwrap();
                assert!(
                    matches!(function.exprs[function.body.tail.unwrap().0].kind,ir::ExprKind::Int(ir::IntValue{ty:actual,bits}) if actual==ty&&bits==magnitude)
                );
                assert!(
                    check(
                        &parse(&format!(
                            "fn main(){{let x=-{{{magnitude}{}}};}}",
                            ty.name()
                        ))
                        .unwrap()
                    )
                    .is_err()
                );
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: magnitude + 1,
                            suffix: Some(ty)
                        },
                        true,
                        None
                    )
                    .is_err()
                );
            } else {
                assert!(
                    resolve_integer(
                        &IntegerLiteral {
                            magnitude: 0,
                            suffix: Some(ty)
                        },
                        true,
                        None
                    )
                    .is_err()
                );
            }
        }
        check(&parse("fn main(){let x:i64=-2147483648;let a:u16=1+2u16;let b=2u16+1;let x:[u8;0x2_usize]=[0b1_u8,0o7_u8];let y=[0xff_u8;0b10];let v=y[0x0];let b=(1i8<2i8)==(3i16<4i16);}").unwrap()).unwrap();
        for literal in [
            "0xff_u8",
            "0o177_i16",
            "0b_1010__u32",
            "18_446_744_073_709_551_615u64",
        ] {
            let literal = parse_integer_literal(literal).unwrap();
            resolve_integer(&literal, false, None).unwrap();
        }
        for literal in [
            "0x_",
            "0b2",
            "0o8",
            "18446744073709551616u64",
            "0x1_0000_0000_0000_0000u64",
        ] {
            assert!(
                parse_integer_literal(literal).is_err(),
                "accepted {literal}"
            );
        }
        for source in [
            "fn main(){let x:u8=1i8;}",
            "fn main(){let x:i64=1isize;}",
            "fn main(){let a=[1];let i=0u64;let x=a[i];}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn integer_patterns_preserve_exact_bits_and_bounded_exhaustiveness() {
        check(&parse("fn f(x:i8)->i32{match x{-128i8=>1,127=>2,_=>3}}fn g(x:u64)->i32{match x{18446744073709551615u64=>1,_=>2}}fn main(){}").unwrap()).unwrap();
        for ty in ["u8", "i8"] {
            let range = if ty == "i8" { -128..128 } else { 0..256 };
            let arms = range.map(|n| format!("{n}=>(),")).collect::<String>();
            check(&parse(&format!("fn f(x:{ty}){{match x{{{arms}}};}}fn main(){{}}")).unwrap())
                .unwrap();
        }
        for source in [
            "fn main(){match 1u8{1i8=>(),_=>()};}",
            "fn main(){match 1u8{256=>(),_=>()};}",
            "fn main(){match 1i8{-129=>(),_=>()};}",
            "fn main(){match 1u64{0=>(),1=>()};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
        assert!(parse("fn main(){match 1i8{\"1\"=>(),_=>()};}").is_err());
        let p = parse("fn main(){match 1u8{1=>(),0x01u8=>(),_=>()};}").unwrap();
        check(&p).unwrap();
    }

    #[test]
    fn e3_mixed_contracts_preserve_formal_kinds_and_real_shared_return_reborrows() {
        let p=parse("fn weaken<'a>(r:&'a mut i32)->&'a i32{r}fn mix<'a>(s:&'a i32,m:&'a mut i32,b:bool)->&'a i32{if b{s}else{m}}fn mut_target<'a>(s:&'a i32,m:&'a mut i32)->&'a mut i32{m}fn no_mut_target<'a>(s:&'a i32)->&'a mut i32{loop{}}fn pin<'a>(m:&'a mut bool,s:&'a i32)->&'a i32{s}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        let empty = |param| ir::InputPath {
            param,
            projection: vec![].into(),
        };
        assert_eq!(
            env.functions["mix"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(vec![empty(0), empty(1)].into())
        );
        assert_eq!(
            env.functions["mut_target"].ref_contract.returns,
            ir::ReturnContract::MutInputPaths(vec![empty(1)].into())
        );
        assert_eq!(
            env.functions["no_mut_target"].ref_contract.returns,
            ir::ReturnContract::MutInputPaths(vec![].into())
        );
        assert_eq!(
            env.functions["pin"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(vec![empty(1)].into())
        );
        assert_eq!(
            env.functions["pin"].ref_contract.param_regions.as_ref(),
            &[Some(ir::RegionParamId(0)), Some(ir::RegionParamId(0))]
        );
        assert!(matches!(
            env.functions["weaken"].params[0],
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                ..
            }
        ));
        let weaken = type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(
            matches!(&weaken.exprs[weaken.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place} if place.local==weaken.params[0]&&place.projection==vec![ir::Projection::Deref])
        );
        assert!(
            !weaken
                .exprs
                .iter()
                .any(|e| matches!(e.kind,ir::ExprKind::Local(local) if local==weaken.params[0]))
        );
        for index in 0..p.functions.len() {
            if p.functions[index].name == "no_mut_target" {
                assert!(
                    check_function_with_signatures(&p, index, &env)
                        .unwrap_err()
                        .contains("unsupported")
                );
            } else {
                check_function_with_signatures(&p, index, &env).unwrap();
            }
        }
    }

    #[test]
    fn e3_all_input_positions_regions_and_deferred_adjustments_remain_enforced() {
        let p = parse("fn ambiguous<'a>(s:&'a i32,m:&'a mut bool)->&i32{s}fn main(){}").unwrap();
        assert!(
            signature_table(&p)
                .unwrap_err()
                .contains("missing lifetime specifier")
        );
        let p = parse("fn rigid<'a>(s:&bool,m:&mut i32)->&'a i32{&*m}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["rigid"].ref_contract.param_regions.as_ref(),
            &[Some(ir::RegionParamId(1)), Some(ir::RegionParamId(2))]
        );
        assert_eq!(
            env.functions["rigid"].ref_contract.result_region,
            Some(ir::RegionParamId(0))
        );
        assert!(check_function_with_signatures(&p, 0, &env).is_err());
        let names = (0..63)
            .map(|i| format!("'r{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(&format!(
            "fn cap<{names}>(other:&bool,r:&mut i32)->&'r0 mut i32{{loop{{}}}}fn main(){{}}"
        ))
        .unwrap();
        assert!(signature_table(&p).unwrap_err().contains("limit64"));
        let p = parse("fn invalid<'a>(s:&'a i32)->&'a mut i32{s}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert!(check_function_with_signatures(&p, 0, &env).is_err());
        for source in [
            "fn deferred<'a>(m:&'a mut i32)->&'a i32{&mut *m}fn main(){}",
            "struct S{x:i32}fn deferred<'a>(m:&'a mut S)->&'a i32{&mut m.x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            assert!(
                type_function_with_signatures(&p, 0, &env)
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
    }

    #[test]
    fn e2_implicit_and_explicit_fields_have_identical_ir_and_current_carrier_places() {
        let p=parse("struct S{x:i32,y:i32}fn implicit(r:&mut S){let n=r.x;r.x=n+1;let q=&mut r.y;*q=4;}fn explicit(r:&mut S){let n=(*r).x;(*r).x=n+1;let q=&mut (*r).y;*q=4;}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 0, &env).unwrap();
        check_function_with_signatures(&p, 1, &env).unwrap();
        let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
        let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            format!("{:?}", implicit.exprs),
            format!("{:?}", explicit.exprs)
        );
        assert_eq!(
            format!("{:?}", implicit.body),
            format!("{:?}", explicit.body)
        );
        let p=parse("struct S{x:i32}fn implicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;r.x={r=&mut y;3};}fn explicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;(*r).x={r=&mut y;3};}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 0, &env).unwrap();
        check_function_with_signatures(&p, 1, &env).unwrap();
        let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
        let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            format!("{:?}", implicit.exprs),
            format!("{:?}", explicit.exprs)
        );
        assert_eq!(
            format!("{:?}", implicit.body),
            format!("{:?}", explicit.body)
        );
        assert!(implicit.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
    }

    #[test]
    fn e2_field_reads_use_one_genuine_call_temp_and_usize_peer_hints() {
        let p=parse("struct Wide{u:usize}fn get<'a>(r:&'a Wide)->&'a Wide{r}fn get_mut<'a>(r:&'a mut Wide)->&'a mut Wide{r}fn main(){let mut s=Wide{u:4294967296usize};let r=&s;let sum=1+r.u;let bigger=r.u>1;let x=get(&s).u;let y=get_mut(&mut s).u;let mut tuple=(1,2);let t=&mut tuple;t.0=3;let n=t.1;}").unwrap();
        let env = signature_table(&p).unwrap();
        check_function_with_signatures(&p, 2, &env).unwrap();
        let main = type_function_with_signatures(&p, 2, &env).unwrap();
        assert_eq!(
            main.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            2
        );
        assert!(
            main.exprs
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    ir::ExprKind::Int(ir::IntValue {
                        ty: ir::IntTy::Usize,
                        bits: 1
                    })
                ))
                .count()
                >= 2
        );
        for expr in &main.exprs {
            if let ir::ExprKind::Field { base, .. } = expr.kind {
                assert!(matches!(
                    main.exprs[base.0].kind,
                    ir::ExprKind::Deref { .. }
                ));
            }
        }
    }

    #[test]
    fn e2_field_normalization_preserves_authority_privacy_and_deferred_boundaries() {
        for source in [
            "struct S{x:i32}fn main(){let mut s=S{x:1};let r=&s;r.x=2;}",
            "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let q=&mut r.x;}",
            "struct T{x:i32}struct S{child:T}fn main(){let s=S{child:T{x:1}};let r=&s;let moved=r.child;}",
            "mod m{pub struct S{x:i32}pub fn make()->S{S{x:1}}}fn main(){let s=m::make();let r=&s;let n=r.x;}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        for source in [
            "struct S{a:[i32;1]}fn main(){let s=S{a:[1]};let r=&s;let i=0usize;let _=r.a[i];}",
            "struct S{a:[i32;1]}fn main(){let mut s=S{a:[1]};let r=&mut s;let i=0usize;r.a[i]=2;}",
            "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;match r.e{E::A=>()};}",
            "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;let E::A=r.e;}",
            "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let n=({r}).x;}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            assert!(
                type_function_with_signatures(&p, p.functions.len() - 1, &env)
                    .unwrap_err()
                    .contains("unsupported"),
                "lost boundary {source}"
            );
        }
        check(&parse("struct S{x:i32}fn main(){let s=S{x:1};let r=&s;return;r.x=2;}").unwrap())
            .unwrap();
    }

    #[test]
    fn e1_elision_matches_explicit_contracts_and_keeps_anonymous_inputs_rigid() {
        let p=parse("fn elided(x:&i32,n:i32)->&i32{x}fn explicit<'a>(x:&'a i32,n:i32)->&'a i32{x}fn mut_elided(x:&mut i32)->&mut i32{x}fn mut_explicit<'a>(x:&'a mut i32)->&'a mut i32{x}fn unused_names<'a,'b>(x:&i32)->&i32{x}fn rigid<'a>(x:&i32,y:&i32)->&'a i32{x}fn repeated<'a>(x:&'a i32,y:&'a bool)->&'a i32{x}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["elided"].ref_contract,
            env.functions["explicit"].ref_contract
        );
        assert_eq!(
            env.functions["mut_elided"].ref_contract,
            env.functions["mut_explicit"].ref_contract
        );
        let unused = &env.functions["unused_names"].ref_contract;
        assert_eq!(unused.region_count, 3);
        assert_eq!(unused.param_regions.as_ref(), &[Some(ir::RegionParamId(2))]);
        assert_eq!(unused.result_region, Some(ir::RegionParamId(2)));
        let rigid = &env.functions["rigid"].ref_contract;
        assert_eq!(
            rigid.param_regions.as_ref(),
            &[Some(ir::RegionParamId(1)), Some(ir::RegionParamId(2))]
        );
        assert_eq!(rigid.result_region, Some(ir::RegionParamId(0)));
        assert!(rigid.outlives.is_empty());
        assert_eq!(
            rigid.returns,
            ir::ReturnContract::SharedInputPaths(vec![].into())
        );
        type_function_with_signatures(&p, 5, &env).unwrap();
        assert!(check_function_with_signatures(&p, 5, &env).is_err());
        for index in [0, 1, 2, 3, 4, 6] {
            check_function_with_signatures(&p, index, &env).unwrap();
        }
    }

    #[test]
    fn e1_output_elision_counts_reference_positions_not_distinct_region_ids() {
        for source in [
            "fn missing()->&i32{loop{}}fn main(){}",
            "fn ambiguous(x:&i32,y:&i32)->&i32{x}fn main(){}",
            "fn repeated<'a>(x:&'a i32,y:&'a i32)->&i32{x}fn main(){}",
            "fn repeated<'a>(x:&'a mut i32,y:&'a mut i32)->&mut i32{x}fn main(){}",
            "fn different<'a>(x:&'a i32,y:&'a bool)->&i32{x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            assert!(
                signature_table(&p)
                    .unwrap_err()
                    .contains("missing lifetime specifier"),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn e1_combined_region_cap_does_not_limit_erased_no_reference_parameters() {
        let names = |count: usize| {
            (0..count)
                .map(|i| format!("'r{i}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let p = parse(&format!(
            "fn at_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
            names(63)
        ))
        .unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(env.functions["at_limit"].ref_contract.region_count, 64);
        assert_eq!(
            env.functions["at_limit"].ref_contract.result_region,
            Some(ir::RegionParamId(63))
        );
        let p = parse(&format!(
            "fn over_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
            names(64)
        ))
        .unwrap();
        assert!(signature_table(&p).unwrap_err().contains("limit64"));
        let params = (0..4096)
            .map(|i| format!("p{i}:&i32"))
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(&format!("fn erased({params}){{}}fn main(){{}}")).unwrap();
        let env = signature_table(&p).unwrap();
        let contract = &env.functions["erased"].ref_contract;
        assert_eq!(contract.region_count, 0);
        assert_eq!(contract.param_regions.len(), 4096);
        assert!(contract.param_regions.iter().all(Option::is_none));
        assert_eq!(contract.returns, ir::ReturnContract::NoReference);
    }

    #[test]
    fn projected_contract_paths_keep_source_privacy_and_argument_layouts() {
        let p=parse("mod m{pub struct S{nested:([i32;1],)}pub fn get<'a>(p:&'a S)->&'a i32{&(*p).nested.0[0]}pub fn get_mut<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).nested.0[0]}pub fn make()->S{S{nested:([7],)}}}fn choose<'a>(flag:bool,a:&'a(i32,bool),b:&'a(bool,i32))->&'a i32{if flag{&(*a).0}else{&(*b).1}}fn main(){let mut s=m::make();let r=m::get(&s);let n=*r;let q=m::get_mut(&mut s);*q=9;}").unwrap();
        let env = signature_table(&p).unwrap();
        let nested = ir::InputPath {
            param: 0,
            projection: vec![
                ir::Projection::Field(0),
                ir::Projection::Field(0),
                ir::Projection::ConstantIndex(0),
            ]
            .into(),
        };
        assert_eq!(
            env.functions["m::get"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(vec![nested.clone()].into())
        );
        assert_eq!(
            env.functions["m::get_mut"].ref_contract.returns,
            ir::ReturnContract::MutInputPaths(vec![nested].into())
        );
        assert_eq!(
            env.functions["choose"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(
                vec![
                    ir::InputPath {
                        param: 1,
                        projection: vec![ir::Projection::Field(0)].into()
                    },
                    ir::InputPath {
                        param: 2,
                        projection: vec![ir::Projection::Field(1)].into()
                    },
                ]
                .into()
            )
        );
        for index in 0..p.functions.len() {
            type_function_with_signatures(&p, index, &env).unwrap();
        }
        let invalid=parse("mod m{pub struct S{x:i32}pub fn make()->S{S{x:7}}}fn main(){let s=m::make();let r=&s.x;}").unwrap();
        let env = signature_table(&invalid).unwrap();
        assert!(
            type_function_with_signatures(&invalid, 1, &env)
                .unwrap_err()
                .contains("private")
        );
    }

    #[test]
    fn b3b_mutable_contracts_keep_complete_original_indices_and_return_reborrows() {
        let p=parse("fn pick<'a>(flag:bool,s:&'a mut bool,x:&'a mut i32,y:&'a mut i32)->&'a mut i32{if flag{x}else{y}}fn moved<'a>(r:&'a mut i32)->&'a mut i32{let moved=r;moved}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["pick"].ref_contract.returns,
            ir::ReturnContract::MutInputPaths(
                vec![
                    ir::InputPath {
                        param: 2,
                        projection: vec![].into()
                    },
                    ir::InputPath {
                        param: 3,
                        projection: vec![].into()
                    }
                ]
                .into()
            )
        );
        let pick = type_function_with_signatures(&p, 0, &env).unwrap();
        assert_eq!(
            pick.exprs
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Mut,
                        ..
                    }
                ))
                .count(),
            2
        );
        let moved = type_function_with_signatures(&p, 1, &env).unwrap();
        let ir::Stmt::Let { local, value } = moved.body.statements[0] else {
            panic!("expected genuine untyped handle let")
        };
        assert!(matches!(moved.exprs[value.0].kind,ir::ExprKind::Local(id) if id==moved.params[0]));
        assert!(
            matches!(&moved.exprs[moved.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Mut,place} if place.local==local && place.projection==vec![ir::Projection::Deref])
        );
        assert!(std::sync::Arc::ptr_eq(
            &moved.ref_contract,
            &env.functions["moved"].ref_contract
        ));
    }

    #[test]
    fn b3b_mutable_call_temp_copy_reads_preserve_computed_place_boundaries() {
        let p=parse("struct S{x:i32}fn id<'a>(r:&'a mut S)->&'a mut S{r}fn main(){let mut s=S{x:7};let n=(*id(&mut s)).x;}").unwrap();
        let env = signature_table(&p).unwrap();
        let main = type_function_with_signatures(&p, 1, &env).unwrap();
        assert_eq!(
            main.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        assert!(
            main.exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
        );
        for source in [
            "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;*id(&mut x)=2;}",
            "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;let q=&mut *id(&mut x);}",
        ] {
            assert!(parse(source).unwrap_err().contains("unsupported"));
        }
        for source in [
            "fn bad<'a>(x:&'a i32)->&'a mut i32{loop{}}fn main(){}",
            "fn bad<'a>(x:&'a mut i32,y:&'a i32)->&'a mut i32{x}fn main(){}",
            "fn bad<'a>(x:&'a mut i32)->&'a i32{&*x}fn main(){}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            if source.contains("{loop{}}") {
                assert!(
                    check_function_with_signatures(&p, 0, &env)
                        .unwrap_err()
                        .contains("unsupported")
                );
            } else {
                check_function_with_signatures(&p, 0, &env).unwrap();
            }
        }
    }

    #[test]
    fn b3a_contracts_are_alpha_canonical_and_keep_original_parameter_indices() {
        let source = "fn pick<'a>(flag:bool,s:&'a bool,x:&'a i32,y:&'a i32)->&'a i32{if flag{x}else{y}}fn renamed<'z>(flag:bool,s:&'z bool,x:&'z i32,y:&'z i32)->&'z i32{if flag{x}else{y}}fn main(){}";
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        let first = &env.functions["pick"];
        assert_eq!(first.ref_contract, env.functions["renamed"].ref_contract);
        assert_eq!(
            first.ref_contract.param_regions.as_ref(),
            &[
                None,
                Some(ir::RegionParamId(0)),
                Some(ir::RegionParamId(0)),
                Some(ir::RegionParamId(0))
            ]
        );
        assert_eq!(
            first.ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(
                vec![
                    ir::InputPath {
                        param: 2,
                        projection: vec![].into()
                    },
                    ir::InputPath {
                        param: 3,
                        projection: vec![].into()
                    }
                ]
                .into()
            )
        );
        let checked = type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first.ref_contract,
            &checked.ref_contract
        ));
        assert_eq!(env.functions["main"].ref_contract.param_regions.len(), 0);
    }

    #[test]
    fn b3a_outlives_bounds_and_call_result_dereference_are_structural() {
        let p=parse("struct S{x:i32}fn id<'a>(x:&'a S)->&'a S{x}fn short<'a,'b>(x:&'a i32)->&'b i32 where 'a:'b{x}fn missing<'a,'b>(x:&'a i32)->&'b i32{x}fn main(){let s=S{x:7};let n=(*id(&s)).x;}").unwrap();
        let env = signature_table(&p).unwrap();
        assert_eq!(
            env.functions["short"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(
                vec![ir::InputPath {
                    param: 0,
                    projection: vec![].into()
                }]
                .into()
            )
        );
        assert_eq!(
            env.functions["missing"].ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(vec![].into())
        );
        let checked = type_function_with_signatures(&p, 3, &env).unwrap();
        assert_eq!(
            checked
                .exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        assert!(
            checked
                .exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
        );
        // Missing universal outlives relations are analyzer obligations, not type inference.
        type_function_with_signatures(&p, 2, &env).unwrap();
    }

    #[test]
    fn b3a_rejects_unbound_duplicate_and_deferred_lifetime_forms() {
        for source in [
            "fn f<'a,'a>(x:&'a i32)->&'a i32{x}fn main(){}",
            "fn f<'a>(x:&'b i32)->&'a i32{x}fn main(){}",
            "fn f<'a:'b>(x:&'a i32)->&'a i32{x}fn main(){}",
            "fn f<'a>(x:&'a i32){}fn main(){}",
            "fn f<'fn>(x:&'fn i32)->&'fn i32{x}fn main(){}",
        ] {
            assert!(
                parse(source).and_then(|p| signature_table(&p)).is_err(),
                "accepted {source}"
            );
        }
        let p = parse("fn f<'a>(x:&'a i32)->&'a i32{let q:&'a i32=x;q}fn main(){}").unwrap();
        let env = signature_table(&p).unwrap();
        assert!(
            type_function_with_signatures(&p, 0, &env)
                .unwrap_err()
                .contains("unsupported")
        );
    }

    #[test]
    fn b2_expected_reference_sites_reborrow_the_carrier_without_a_prior_move() {
        let p=parse("fn read(r:&i32)->i32{*r}fn bump(r:&mut i32){*r=*r+1;}fn main(){let mut x=1;let mut r=&mut x;bump(r);bump(r);read(r);let t:&mut i32=r;*t=3;let mut y=2;let s=&mut y;r=s;let a:&i32={r};let b:&i32=if true{r}else{r};let c:&i32=match true{_=>r};let moved=r;}").unwrap();
        let env = signature_table(&p).unwrap();
        let typed = type_function_with_signatures(&p, 2, &env).unwrap();
        let carrier = typed.locals.iter().position(|l| l.name == "r").unwrap();
        let reborrows=typed.exprs.iter().filter(|e|matches!(&e.kind,ir::ExprKind::Borrow{place,..} if place.local==ir::LocalId(carrier)&&place.projection==vec![ir::Projection::Deref])).count();
        assert!(
            reborrows >= 7,
            "missing call/let/assignment/control-context reborrows: {reborrows}"
        );
        let moved = typed
            .body
            .statements
            .iter()
            .find_map(|s| {
                if let ir::Stmt::Let { local, value } = s
                    && typed.locals[local.0].name == "moved"
                {
                    Some(*value)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            matches!(typed.exprs[moved.0].kind,ir::ExprKind::Local(local) if local==ir::LocalId(carrier))
        );
        for i in 0..2 {
            type_function_with_signatures(&p, i, &env).unwrap();
        }
    }

    #[test]
    fn b2_noescape_restrictions_hold_for_unused_declarations_and_explicit_borrows() {
        for source in [
            "fn unused(r:&[&i32;0]){}fn main(){}",
            "fn unused(r:(&i32,)){}fn main(){}",
            "fn unused(r:&&i32){}fn main(){}",
            "fn unused()->&i32{loop{}}fn main(){}",
            "struct S{r:&i32}fn main(){}",
            "fn read(r:&i32){}fn main(){let mut x=1;read(&mut x);}",
            "fn main(){let mut x=1;let r=&mut x;let q:&mut i32=loop{break r;};}",
        ] {
            assert!(
                parse(source)
                    .and_then(|p| {
                        let env = signature_table(&p)?;
                        type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
                    })
                    .is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn blocklike_statements_do_not_consume_a_following_deref_as_multiplication() {
        for source in [
            "fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r}fn main(){let n=choose(true);}",
            "fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}fn main(){let n=run();}",
            "fn main(){let x=1;let r=&x;if true{}else if false{}else{} *r;}",
            "fn main(){let x={1}*2;({1})*2;let a=({[1]})[0];}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
    }

    #[test]
    fn b1_types_explicit_borrows_reborrows_and_static_deref_projections() {
        let program=parse("struct S{x:i32,y:i32}fn main(){let mut s=S{x:1,y:2};let r:&mut S=&mut s;(*r).x=3;let c=(*r).x;let p=&mut (*r).y;*p=4;let q=&*r;let z=(*q).x;let mut a=[1,2];let ar=&mut a;(*ar)[0]=3;let n=(*ar)[0];let owner=1;let sh=&owner;let copied=sh;let shared_s=S{x:7,y:8};let shared_r=&shared_s;let shared_n=(*shared_r).x;let moved=r;}").unwrap();
        let env = signature_table(&program).unwrap();
        let typed = type_function_with_signatures(&program, 0, &env).unwrap();
        assert!(typed.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
        assert!(typed.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::ConstantIndex(0)])));
        assert!(typed.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place} if place.projection==vec![ir::Projection::Deref])));
        for source in [
            "fn main(){let mut x=1;let r=&mut x;let moved=r;let _=*r;match *r{_=>()};}",
            "fn main(){let r:&i32;let _=*r;match *r{_=>()};}",
            "fn main(){let x=1;return;let _=&mut x;}",
            "fn main(){let x=1;let r=&x;return;*r=2;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
    }

    #[test]
    fn b1_rejects_forbidden_reference_locations_authority_and_implicit_coercions() {
        for source in [
            "fn main(){let x=1;let _=&mut x;}",
            "fn main(){let x=1;let r=&x;*r=2;}",
            "fn main(){let mut x=1;let r=&x;let p=&mut *r;}",
        ] {
            let p = parse(source).unwrap();
            let env = signature_table(&p).unwrap();
            type_function_with_signatures(&p, 0, &env).unwrap();
            assert!(check(&p).is_err(), "analysis accepted {source}");
        }
        for source in [
            "fn unused()->&i32{loop{}}fn main(){}",
            "struct S{x:(&i32,)}fn main(){}",
            "enum E{V([&i32;0])}fn main(){}",
            "fn main(){let x=1;let r=&x;let a=[r;0];}",
            "fn main(){let x=1;let r=&x;let t=(r,);}",
            "fn main(){let x=1;let r=&x;let nested=&r;}",
            "fn main(){let r=&1;}",
            "fn main(){let x=1;let a=&x;let b=&x;let eq=a==b;}",
            "fn main(){let a=[1];let i=0usize;let p=&a[i];}",
            "fn main(){let a=[1];let p=&a;let i=0usize;let n=(*p)[i];}",
        ] {
            let result = parse(source).and_then(|p| {
                let env = signature_table(&p)?;
                type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
            });
            assert!(result.is_err(), "accepted {source}");
        }
    }

    #[test]
    fn usize_retains_full_width_and_uses_forward_and_binary_peer_contexts() {
        let p=parse("fn id(x:usize)->usize{x}fn high()->usize{18446744073709551615}fn main(){let u:usize=4294967296;let s=18446744073709551615usize;let b=1<u;let c=u>1;let r:usize=1+2;let q=1+u;let not=!0usize;let x=id(4294967296);let a:[usize;2]=[1,2];let mut b=[1,2];let i:usize=0;b[i]=3;let n=b[i];let min=-2147483648i32;}").unwrap();
        let env = signature_table(&p).unwrap();
        let typed = type_function_with_signatures(&p, 2, &env).unwrap();
        assert!(typed.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Int(ir::IntValue {
                ty: ir::IntTy::Usize,
                bits: 18446744073709551615
            })
        )));
        assert!(
            typed
                .exprs
                .iter()
                .any(|e| matches!(e.kind, ir::ExprKind::DynamicIndex { .. }))
        );
        assert!(
            typed
                .body
                .statements
                .iter()
                .any(|s| matches!(s, ir::Stmt::AssignIndex { .. }))
        );
        assert_eq!(
            type_function_with_signatures(&p, 1, &env).unwrap().result,
            ir::Ty::Int(ir::IntTy::Usize)
        );
    }

    #[test]
    fn usize_does_not_convert_i32_or_erase_runtime_index_pattern_boundaries() {
        for source in [
            "fn main(){let x=0usize;match x{0=>(),_=>()};}",
            "fn main(){match 0usize{0usize=>(),_=>()};}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let a=[1];let i:i32=0;let x=a[i];}",
            "fn main(){let x:usize=1i32;}",
            "fn main(){let x:i32=1usize;}",
            "fn main(){let x:usize=-1;}",
            "fn main(){let x=18446744073709551616usize;}",
            "fn main(){let a=[[1]];let i=0usize;let x=a[i][0];}",
            "struct S{x:i32}fn main(){let a=[S{x:1}];let i=0usize;let S{x:_}=a[i];}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn arrays_contextualize_empty_literals_and_keep_constant_index_projection() {
        let source = "struct S{x:[i32;0]}enum E{V([i32;0])}fn make()->[i32;0]{[]}fn explicit()->[i32;0]{return [];}fn take(x:[i32;0]){}fn main(){let z:[i32;0]=[];take([]);let s=S{x:[]};let e=E::V([]);let nested:[[i32;0];2]=[[],[]];let mut a=[1,2];a[0]=3;let n=a[0];let t:([i32;0],)=([],);let q:[i32;0]=if true{[]}else{[]};let r:[i32;0]=match true{_=>[]};}";
        let program = parse(source).unwrap();
        check(&program).unwrap();
        let function = type_function_with_signatures(
            &program,
            program.functions.len() - 1,
            &signature_table(&program).unwrap(),
        )
        .unwrap();
        assert!(function.body.statements.iter().any(|s|matches!(s,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::ConstantIndex(0)])));
    }

    #[test]
    fn arrays_reject_bounds_types_repeat_moves_and_unsupported_inference() {
        for source in [
            "fn main(){let a=[1,2];let n=a[2];}",
            "fn main(){let a=[1,true];}",
            "fn main(){let a:[i32;1]=[];}",
            "struct S{x:i32}fn main(){let a=[S{x:1};2];}",
            "struct S{x:i32}fn main(){let a=[S{x:1}];let s=a[0];}",
            "fn main(){let a=[1;4097];}",
            "fn main(){let a=[];}",
            "fn main(){let a=[1];let i=0;let n=a[i];}",
        ] {
            assert!(
                parse(source).and_then(|p| check(&p)).is_err(),
                "accepted {source}"
            );
        }
        check(
            &parse("struct S{x:i32}fn main(){let a=[S{x:1};0];let b=[S{x:2};1];let n=b[0].x;}")
                .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn payload_variants_preserve_constructor_shape_source_order_and_projection_ids() {
        let source = "struct S{x:i32} enum E{Unit,Tuple(S,bool),Named{a:i32,b:S},Zero(),Empty{}} use E::Named as Alias;fn main(){let e=Alias{b:S{x:2},a:1};let n=match e{Alias{b:S{x:n},a:_,..}=>n,E::Tuple(S{x:n},_)=>n,E::Unit=>0,E::Zero()=>0,E::Empty{}=>0};let z=E::Zero();let q=E::Empty{};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        let enumeration = &env.enums["E"];
        assert!(enumeration.has_payload());
        assert_eq!(enumeration.def().variants[3].shape, ir::VariantShape::Tuple);
        assert_eq!(
            enumeration.def().variants[4].shape,
            ir::VariantShape::Struct
        );
        assert!(checked.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::EnumVariant{variant:2,fields,..} if fields.iter().map(|(i,_)|*i).collect::<Vec<_>>()==vec![1,0])));
        let arms = checked
            .exprs
            .iter()
            .find_map(|e| match &e.kind {
                ir::ExprKind::Match { arms, .. } => Some(arms),
                _ => None,
            })
            .unwrap();
        let ir::MatchPattern::EnumVariant { bindings, .. } = &arms[0].pattern else {
            panic!("variant pattern")
        };
        assert_eq!(
            bindings[0].projection,
            vec![
                ir::Projection::Downcast(ir::VariantId {
                    owner: enumeration.def().id,
                    index: 2
                }),
                ir::Projection::Field(1),
                ir::Projection::Field(0)
            ]
        );
        check(&program).unwrap();
    }

    #[test]
    fn singleton_payload_let_and_nested_irrefutable_patterns_are_typed() {
        let source = "enum Inner{One(i32)} enum Outer{One{inner:Inner,flag:bool}}fn main(){let Outer::One{inner:Inner::One(mut n),flag:_}=Outer::One{flag:true,inner:Inner::One(3)};n=4;let e=Outer::One{inner:Inner::One(n),flag:false};let n=match e{Outer::One{inner:Inner::One(n),..}=>n};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn mixed_nominal_payload_dag_validates_unused_cycles_fields_and_privacy() {
        for source in [
            "struct S{x:E}enum E{A(S),B}fn main(){}",
            "enum A{V(B)}enum B{V(A)}fn main(){}",
            "enum A{V(Missing)}fn main(){}",
            "enum A{V{x:i32,x:bool}}fn main(){}",
            "mod m{struct H{x:i32}}enum E{V(m::H)}fn main(){}",
            "enum E{V(i32)}fn main(){E::V(true);}",
            "enum E{V{x:i32}}fn main(){E::V{};}",
            "enum E{V(i32)}fn main(){match E::V(1){E::V()=>()};}",
            "enum E{V{i:i32}}fn main(){match E::V{i:1}{E::V{i:x,i:y}=>()};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
        let program =
            parse("enum E{V(S)}struct S{x:i32}fn main(){let E::V(S{x:n})=E::V(S{x:1});}").unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn variant_imports_reserve_type_namespace_without_becoming_types() {
        for variant in ["V", "V(i32)", "V{x:i32}"] {
            for conflict in ["struct A{x:i32}", "enum A{Unit}", "mod A{}"] {
                let source = format!("{conflict} enum E{{{variant}}}use E::V as A;fn main(){{}}");
                assert!(parse(&source).is_err(), "accepted {source}");
            }
            let source = format!("enum E{{{variant}}}use E::V as A;fn f(x:A){{}}fn main(){{}}");
            assert!(parse(&source).is_err(), "variant used as type: {source}");
        }
        check(&parse("fn A()->i32{1}enum E{V{x:i32}}use E::V as A;fn main(){let n=A();let e=A{x:n};match e{A{x:n}=>{}}}").unwrap()).unwrap();
        check(&parse("struct A{x:i32}fn A()->i32{1}fn main(){let n=A();let a=A{x:n};}").unwrap())
            .unwrap();
        check(&parse("enum E{V{x:i32}}use E::V;fn f(V:i32)->i32{V}fn main(){let V:i32;V=1;let n=f(V);let V=2;let n=V;}").unwrap()).unwrap();
    }

    #[test]
    fn typed_deferred_declarations_publish_fresh_uninitialized_locals() {
        let source = "enum E{A,B} struct S{e:E} fn main(){let x=1;let x:i32;let mut y:(i32,bool);let s:S;x=2;y=(x,true);s=S{e:E::A};let x=true;match s.e{E::A=>(),E::B=>()};}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        assert_eq!(checked.locals.len(), 5);
        assert_eq!(checked.locals[0].name, "x");
        assert_eq!(checked.locals[1].name, "x");
        assert!(!checked.locals[1].mutable);
        assert!(checked.locals[2].mutable);
        assert!(matches!(
            checked.body.statements[1],
            ir::Stmt::Declare {
                local: ir::LocalId(1)
            }
        ));
        checked.validate().unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn deferred_declarations_keep_flow_checks_outside_pure_typing() {
        for source in [
            "fn main(){let x:i32;x=1;let y=x;}",
            "fn f(b:bool)->i32{let x:i32;if b{x=1;}else{x=2;}x}fn main(){}",
            "fn main(){let mut x:i32;x=1;x=2;}",
            "fn main(){return;let x=1;x=2;}",
            "fn main(){let x=1;return;x=2;}",
        ] {
            check(&parse(source).unwrap()).unwrap();
        }
        for source in [
            "fn main(){let x:i32;let y=x;}",
            "fn main(){let x:i32;x=1;x=2;}",
            "fn main(){let x=1;x=2;}",
            "fn f(b:bool){let x:i32;if b{x=1;}let y=x;}fn main(){}",
            "fn main(){let x:i32;loop{x=1;}}",
        ] {
            let program = parse(source).unwrap();
            let env = type_env(&program).unwrap();
            type_function_with_env(&program, 0, &env).unwrap();
            assert!(
                check_function_with_signatures(&program, 0, &env).is_err(),
                "accepted {source}"
            );
            assert!(check_function(&program, 0).is_err());
            assert!(check(&program).is_err());
        }
        for source in [
            "fn main(){let x;}",
            "fn main(){let mut x;}",
            "fn main(){let (x,y):(i32,i32);}",
            "fn main(){let _:i32;}",
            "fn main(){let x:_;}",
            "fn main(){let x:!;}",
            "enum E{A}use E::A;fn main(){let A:E;}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn enum_match_types_paths_scopes_and_nominal_identity() {
        let source = "mod m{pub enum E{A,B} pub fn make()->E{E::A}} use m::E as Alias; use Alias::A as First; struct S{e:Alias} fn f(x:Alias)->i32{match x{First=>1,Alias::B=>2}} fn main(){let s=S{e:First};let n=f(s.e);match true{true=>{},false=>{}} match (){()=>{}} let n=match n{-2147483648=>0,x=>x};let _=m::make();}";
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        assert_ne!(env.structs["S"].def().id, env.enums["m::E"].def().id);
        assert!(!ir::Ty::Enum(env.enums["m::E"].clone()).is_copy());
        check(&program).unwrap();
        let checked = type_function_with_env(&program, 1, &env).unwrap();
        assert!(
            checked
                .exprs
                .iter()
                .any(|expr| matches!(&expr.kind,ir::ExprKind::Match{arms,..} if arms.len()==2))
        );
    }

    #[test]
    fn enum_match_exhaustiveness_and_dead_arm_obligations() {
        for source in [
            "enum E{A,B} fn main(){match E::A{E::A=>()};}",
            "fn main(){match true{true=>()};}",
            "fn main(){match 0{0=>()};}",
            "enum E{A} enum F{A} fn main(){match E::A{F::A=>()};}",
            "fn main(){match true{_=>1,true=>false};}",
            "fn main(){match true{_=>(),false=>{let x:bool=1;}};}",
            "fn main(){match 1{x=>x};x;}",
            "enum E{A,B} fn main(){let E::A=E::A;}",
            "fn f()->i32{loop{match true{_=>break 1,true=>break false}}} fn main(){}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        }
        assert!(parse("enum E{A} use E::A; fn main(){match E::A{mut A=>()};}").is_err());
        for source in [
            "enum E{A,B} fn main(){match E::A{E::A=>(),E::A=>(),E::B=>(),_=>()};}",
            "enum E{A} use E::A; fn main(){let A=E::A;match A{A=>()};}",
            "fn f()->i32{match loop{} {}} fn main(){}",
            "enum E{A,B} fn f(e:E)->i32{match e{E::A{}=>return 1,E::B{}=>return 2}} fn main(){let e=E::A{};let n=f(e);}",
            "fn f()->i32{loop {match true{true=>{break 1;},false=>{break 2;},_=>{break 3;}}}} fn main(){}",
        ] {
            check(&parse(source).unwrap()).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            "enum E{A,B}use E::A;fn f(A:E){}fn main(){}",
            "enum E{A}use E::A;fn f(A:E)->E{A}fn main(){}",
        ] {
            assert!(
                parse(source)
                    .unwrap_err()
                    .contains("parameters are unsupported")
            );
        }
    }

    fn typed(source: &str, index: usize) -> ir::CheckedFunction {
        let program = parse(source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, index, &signatures).unwrap();
        checked.validate().unwrap();
        checked
    }

    #[test]
    fn tuple_grouping_and_singleton_syntax_are_distinct() {
        let program = parse("fn f(x:((i32)),y:(bool,))->((i32,bool),()) { ((x,y.0),()) } fn main(){let a=(1);let b=(1,);}").unwrap();
        let f = &program.functions[0];
        assert_eq!(f.params[0].1, Type::Int(ir::IntTy::I32));
        assert_eq!(f.params[1].1, Type::Tuple(vec![Type::Bool]));
        assert_eq!(
            f.result,
            Type::Tuple(vec![
                Type::Tuple(vec![Type::Int(ir::IntTy::I32), Type::Bool]),
                Type::Unit
            ])
        );
        check(&program).unwrap();
        let f = typed("fn main(){let a=(1);let b=(1,);let z=((),((),));}", 0);
        assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(
            f.locals[1].ty,
            ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32)].into())
        );
        assert_eq!(
            f.locals[2].ty,
            ir::Ty::Tuple(vec![ir::Ty::Unit, ir::Ty::Tuple(vec![ir::Ty::Unit].into())].into())
        );
    }

    #[test]
    fn tuple_places_resolve_parenthesized_local_roots_and_shadowing() {
        let f = typed(
            "fn main(){let mut x=((1,true),());((x)).0.1=false;let mut x=(2,);x.0=3;}",
            0,
        );
        let places = f
            .body
            .statements
            .iter()
            .filter_map(|s| match s {
                ir::Stmt::AssignPlace { place, .. } => Some(place),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(places[0].local, ir::LocalId(0));
        assert_eq!(
            places[0].projection,
            vec![ir::Projection::Field(0), ir::Projection::Field(1)]
        );
        assert_eq!(places[1].local, ir::LocalId(1));
        assert_eq!(places[1].projection, vec![ir::Projection::Field(0)]);
        assert_eq!(f.locals[0].name, f.locals[1].name);
    }

    #[test]
    fn tuple_aggregate_flows_preserve_each_actual_type_and_projection_completion() {
        let f = typed("fn f()->i32 {({return 3;},0).1} fn main(){}", 0);
        let projected = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(projected.ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(projected.completion, ir::Completion::Never);
        let ir::ExprKind::Field { base, .. } = projected.kind else {
            panic!("expected projection")
        };
        assert_eq!(
            f.exprs[base.0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Never, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        let f = typed("fn main(){let x:(bool,i32)=({return;},0);}", 0);
        assert_eq!(
            f.locals[0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        let f = typed("fn main(){let x=({return;false},0);}", 0);
        assert_eq!(
            f.locals[0].ty,
            ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
        );
        for source in [
            "fn main(){let x:(i32,i32)=({return;false},0);}",
            "fn main(){let x=if true {({return;false},0)} else {(1,0)};}",
            "fn main(){let x=loop {break ({return;false},0);break (1,0);};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn tuple_calls_and_loop_breaks_keep_structural_signatures() {
        let f = typed(
            "mod m {pub fn f(x:(i32,bool))->(i32,bool){x}} fn main(){let t=loop {break if true {m::f((1,true))} else {(2,false)};};}",
            1,
        );
        let call = f
            .exprs
            .iter()
            .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .unwrap();
        let ty = ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool].into());
        assert_eq!(call.ty, ty);
        let ir::ExprKind::Call {
            name, signature, ..
        } = &call.kind
        else {
            unreachable!()
        };
        assert_eq!(name, "m::f");
        assert_eq!(signature.params, vec![ty.clone()]);
        assert_eq!(signature.result, ty);
        assert_eq!(f.locals[0].ty, ty);
    }

    #[test]
    fn tuple_invalid_fields_assignments_and_operators_are_rejected() {
        for source in [
            "fn main(){let x=(1,);x.0=2;}",
            "fn main(){let mut x=(1,);x.0=true;}",
            "fn main(){let mut x=(1,);x.1=2;}",
            "fn main(){let x=(1,);let y=x.0.0;}",
            "fn main(){let x=().0;}",
            "fn main(){let x=(1,)==(1,);}",
            "fn main(){let x=(1,)<(2,);}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
        for source in [
            "fn main(){(1,).0=2;}",
            "fn main(){let x=(1,).00;}",
            "fn main(){let x=(1,).0usize;}",
            "fn main(){let x=(1,).-1;}",
            "fn main(){let (x,..)=(1,2);}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn tuple_type_nesting_and_nested_inference_are_checked() {
        let nested = format!("{}i32{}", "(".repeat(140), ",)".repeat(140));
        assert!(
            parse(&format!("fn f(x:{nested}){{}}fn main(){{}}"))
                .unwrap_err()
                .contains("nesting")
        );
        let mut program = parse("fn f(x:(i32,)){}fn main(){}").unwrap();
        program.functions[0].params[0].1 = Type::Tuple(vec![Type::Infer]);
        assert!(
            signature_table(&program)
                .unwrap_err()
                .contains("cannot be inferred")
        );
    }

    #[test]
    fn loop_control_has_distinct_resolved_targets_and_scalar_types() {
        let f = typed(
            "fn main(){ let x=loop { while true { continue; break; } loop { break; } break 3; }; let y=loop { break true; }; }",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool]
        );
        let breaks = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Break { target, .. } => {
                    assert_eq!(e.flow(), None);
                    Some(target)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            breaks,
            vec![ir::LoopId(1), ir::LoopId(2), ir::LoopId(0), ir::LoopId(3)]
        );
        assert!(f.exprs.iter().any(|e| matches!(
            e.kind,
            ir::ExprKind::Continue {
                target: ir::LoopId(1)
            }
        )));
    }

    #[test]
    fn divergent_break_operand_contributes_only_its_nested_breaks() {
        let f = typed(
            "fn f()->i32 { loop { break { break 3; }; } } fn main(){}",
            0,
        );
        let root = &f.exprs[f.body.tail.unwrap().0];
        assert!(matches!(root.kind, ir::ExprKind::Loop { .. }));
        assert_eq!(root.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
        let f = typed(
            "fn f()->i32 { loop { break { return 3; }; } } fn main(){}",
            0,
        );
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
        let f = typed("fn f()->i32 { loop { continue; break 3; } } fn main(){}", 0);
        assert_eq!(
            f.exprs[f.body.tail.unwrap().0].ty,
            ir::Ty::Int(ir::IntTy::I32)
        );
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
        let f = typed("fn f()->i32 { loop { continue; } } fn main(){}", 0);
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
    }

    #[test]
    fn typed_unreachable_tails_preserve_types_without_creating_loop_exits() {
        let f = typed(
            "fn f()->i32 { loop { break {return 3; 0}; } } fn main(){}",
            0,
        );
        let value = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(value.ty, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(value.completion, ir::Completion::Never);
        let f = typed("fn f()->i32 { !{return 3;} } fn main(){}", 0);
        let value = &f.exprs[f.body.tail.unwrap().0];
        assert_eq!(value.ty, ir::Ty::Never);
        assert_eq!(value.completion, ir::Completion::Never);
        let f = typed("fn main(){ let x={return; true}; }", 0);
        assert_eq!(f.locals[0].ty, ir::Ty::Bool);
        for source in [
            "fn f()->i32 {return 3; ()} fn main(){}",
            "fn main(){loop {break 1; false}}",
            "fn main(){let x:bool=loop {break {return; 0};};}",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn explicit_semicolons_discard_blocklike_values() {
        let program = parse("fn main(){ {1}; if true {1} else {2}; loop {break 3;}; }").unwrap();
        check(&program).unwrap();
        assert!(
            program.functions[0]
                .body
                .statements
                .iter()
                .all(|s| matches!(s, Stmt::Expr(_)))
        );
        let program = parse("fn main(){ loop {break;} let x=1; }").unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            Stmt::ImplicitExpr(Expr::Loop { .. })
        ));
        check(&program).unwrap();
        for source in [
            "fn main(){ {1} let x=1; }",
            "fn main(){ if true {1} else {2} let x=1; }",
            "fn main(){ loop {break 3;} let x=1; }",
        ] {
            assert!(check(&parse(source).unwrap()).is_err(), "{source}");
        }
    }

    #[test]
    fn loop_obligations_reject_unreachable_conflicts_and_while_values() {
        for source in [
            "fn main(){ loop {break 3; break true;} }",
            "fn main(){ loop {break; break 3;} }",
            "fn main(){ loop {continue; break 3; break true;} }",
            "fn main(){ while true {break ();} }",
            "fn main(){ while true {break {return;};} }",
            "fn main(){ break; }",
            "fn main(){ continue; }",
            "fn main(){ loop {1} }",
        ] {
            let p = parse(source).unwrap();
            assert!(check(&p).is_err(), "{source}");
        }
    }

    #[test]
    fn while_conditions_are_control_target_barriers() {
        for source in [
            "fn main(){ while {break;} {} }",
            "fn main(){ while {continue;} {} }",
            "fn main(){ loop {while {break;} {}} }",
            "fn main(){ loop {while {continue;} {}} }",
        ] {
            let error = check(&parse(source).unwrap()).unwrap_err();
            assert!(error.contains("E0590"), "{error}");
        }
        let f = typed("fn main(){ while {loop {break true;}} {break;} }", 0);
        let ir::Stmt::While { id, .. } = f.body.statements[0] else {
            panic!("expected while")
        };
        assert_eq!(id, ir::LoopId(0));
        let breaks = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Break { target, .. } => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(breaks, vec![ir::LoopId(1), ir::LoopId(0)]);
    }

    #[test]
    fn loop_walkers_resolve_break_value_calls_and_bound_depth() {
        let f = typed(
            "mod m {pub fn f()->i32{3}} fn main(){let x=loop {break m::f();};}",
            1,
        );
        assert!(
            f.exprs
                .iter()
                .any(|e| matches!(&e.kind, ir::ExprKind::Call { name, .. } if name == "m::f"))
        );
        let nested_breaks = "break ".repeat(140);
        let source = format!("fn main(){{{nested_breaks}0;}}");
        assert!(parse(&source).unwrap_err().contains("nesting"));
        assert!(
            parse("fn main(){'outer: loop {break 'outer;}}")
                .unwrap_err()
                .contains("unsupported")
        );
    }

    #[test]
    fn inferred_shadowing_allocates_distinct_typed_locals() {
        let f = typed(
            "fn f(x:i32)->i32 { let x=x+1; let x=true; { let x=3; x; }; if x { 4 } else { 5 } } fn main(){}",
            0,
        );
        assert_eq!(f.params, vec![ir::LocalId(0)]);
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![
                ir::Ty::Int(ir::IntTy::I32),
                ir::Ty::Int(ir::IntTy::I32),
                ir::Ty::Bool,
                ir::Ty::Int(ir::IntTy::I32)
            ]
        );
        assert!(f.locals.iter().all(|l| l.name == "x"));
        let reads = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(3), ir::LocalId(2)]);
    }

    #[test]
    fn assignment_and_initializer_resolve_before_shadowing() {
        let f = typed("fn main(){ let mut x=1; x=x+1; let x=x==2; x; }", 0);
        assert!(f.locals[0].mutable);
        assert!(!f.locals[1].mutable);
        assert_eq!(f.locals[1].ty, ir::Ty::Bool);
        assert!(matches!(
            f.body.statements[1],
            ir::Stmt::Assign {
                local: ir::LocalId(0),
                ..
            }
        ));
        let reads = f
            .exprs
            .iter()
            .filter_map(|e| match e.kind {
                ir::ExprKind::Local(id) => Some(id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(0), ir::LocalId(1)]);
    }

    #[test]
    fn canonical_calls_keep_validated_signature() {
        let f = typed(
            "mod m { pub fn f(x:bool)->i32 { if x { 1 } else { 2 } } } use m::f as g; fn main(){ let n=g(true); }",
            1,
        );
        let call = f
            .exprs
            .iter()
            .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .unwrap();
        assert_eq!(call.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
        let ir::ExprKind::Call {
            name,
            args,
            signature,
        } = &call.kind
        else {
            unreachable!()
        };
        assert_eq!(name, "m::f");
        assert_eq!(signature.params, vec![ir::Ty::Bool]);
        assert_eq!(signature.result, ir::Ty::Int(ir::IntTy::I32));
        assert_eq!(f.exprs[args[0].0].ty, ir::Ty::Bool);
        assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
    }

    #[test]
    fn divergence_and_short_circuit_keep_existing_completion() {
        let f = typed(
            "fn f()->bool { let x=false && { return true; }; let y=true || { return false; }; let z={ return x; }; } fn main(){}",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
            vec![ir::Ty::Bool, ir::Ty::Bool, ir::Ty::Never]
        );
        let binaries = f
            .exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Binary { .. }))
            .collect::<Vec<_>>();
        assert!(binaries.iter().all(|e| e.flow() == Some(ir::Ty::Bool)));
        let never = f
            .exprs
            .iter()
            .filter(|e| e.completion == ir::Completion::Never)
            .collect::<Vec<_>>();
        assert_eq!(never.len(), 3);
        assert!(
            never
                .iter()
                .all(|e| e.ty == ir::Ty::Never && e.flow().is_none())
        );
    }

    #[test]
    fn lowering_and_checking_reject_unreachable_errors_identically() {
        for source in [
            "fn main(){ return; let x=true+1; }",
            "fn main(){ return; let x:bool=1; }",
            "fn main(){ if true { 1 } else { 2 } let x=0; }",
            "fn f()->i32 { while true { return 1; } } fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let signatures = signature_table(&program).unwrap();
            let lowered = type_function_with_signatures(&program, 0, &signatures).unwrap_err();
            assert_eq!(
                check_function_with_signatures(&program, 0, &signatures).unwrap_err(),
                lowered
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_patterns_preserve_old_scope_and_simultaneous_fresh_bindings() {
        let program = parse("fn f()->i32{7} fn main(){let x=1;let ((mut x,),y):((i32,),bool)=((x,),true);x=2;let _=x;let _y=y;let (f,z)=(f(),f());}").unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 1, &env).unwrap();
        checked.validate().unwrap();
        let ir::Stmt::LetPattern {
            value, bindings, ..
        } = &checked.body.statements[1]
        else {
            panic!("expected tuple pattern")
        };
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding
                    .projection
                    .iter()
                    .map(|p| match p {
                        ir::Projection::Field(index) => *index,
                        _ => panic!("unexpected downcast"),
                    })
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![0, 0], vec![1]]
        );
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.local)
                .collect::<Vec<_>>(),
            vec![ir::LocalId(1), ir::LocalId(2)]
        );
        assert!(checked.locals[1].mutable);
        assert_eq!(checked.locals[2].ty, ir::Ty::Bool);
        assert!(
            checked.exprs[..value.0]
                .iter()
                .any(|expr| matches!(expr.kind, ir::ExprKind::Local(ir::LocalId(0))))
        );
        let ir::Stmt::LetPattern { bindings, .. } = &checked.body.statements[3] else {
            panic!("expected wildcard")
        };
        assert!(bindings.is_empty());
        assert_eq!(
            checked
                .locals
                .iter()
                .filter(|local| local.name == "x")
                .count(),
            2
        );
        assert!(
            parse("fn f()->i32{1} fn main(){let (f,z)=(f(),2);f();}")
                .unwrap_err()
                .contains("not callable")
        );
        check(
            &parse(
                "fn main(){let(x,y)=(1,2);let (z,)=(x,);let ()=();let (mut grouped)=z;grouped=y;}",
            )
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn struct_patterns_publish_source_order_paths_and_nominal_types() {
        let program = parse("struct Inner{x:i32} struct S{a:Inner,b:bool} use S as Alias; fn main(){let s=S{a:Inner{x:1},b:true};let Alias{b:mut flag,a:Inner{x:n}}:S=s;flag=false;let Alias{..}=S{a:Inner{x:n},b:flag};}").unwrap();
        let env = type_env(&program).unwrap();
        let checked = type_function_with_env(&program, 0, &env).unwrap();
        checked.validate().unwrap();
        let ir::Stmt::LetPattern { ty, bindings, .. } = &checked.body.statements[1] else {
            panic!("expected struct pattern")
        };
        assert_eq!(*ty, ir::Ty::Struct(env.structs["S"].clone()));
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding
                    .projection
                    .iter()
                    .map(|p| match p {
                        ir::Projection::Field(index) => *index,
                        _ => panic!("unexpected downcast"),
                    })
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![1], vec![0, 0]]
        );
        assert_eq!(checked.locals[bindings[0].local.0].name, "flag");
        assert!(checked.locals[bindings[0].local.0].mutable);
        assert_eq!(
            checked.locals[bindings[1].local.0].ty,
            ir::Ty::Int(ir::IntTy::I32)
        );
    }

    #[test]
    fn pattern_shape_duplicate_annotation_and_privacy_errors_reject_even_when_dead() {
        for source in [
            "fn main(){let(x,x)=(1,2);}",
            "fn main(){let(x,y)=(1,);}",
            "fn main(){let ()=(1,);}",
            "fn main(){let(x,y):(i32,bool)=(1,2);}",
            "fn main(){let(x,y)=(1,true);x=false;}",
            "struct S{x:i32,y:bool} fn main(){let S{x}=S{x:1,y:true};}",
            "struct S{x:i32} fn main(){let S{x:_,x:_}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{missing:_,..}=S{x:1};}",
            "struct S{x:i32} struct T{x:i32} fn main(){let S{x}=T{x:1};}",
            "mod m{pub struct S{private:i32} pub fn make()->S{S{private:1}}} fn main(){let m::S{private:_,..}=m::make();}",
            "mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{hidden:_,..}=m::make();}",
            "fn main(){return;let(x,x)=(1,2);}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
        check(&parse("mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{..}=m::make();}").unwrap()).unwrap();
    }

    #[test]
    fn never_patterns_use_explicit_shape_and_preserve_dead_value_types() {
        for source in [
            "fn main(){let(x,y):(i32,bool)=loop{};}",
            "struct S{x:i32} fn main(){let S{x}=loop{};}",
            "fn main(){let _=loop{};}",
            "fn main(){let ()=loop{};}",
            "struct S{x:i32} fn main(){let(S{x},())=loop{};}",
            "struct S{x:i32} fn main(){let (S{x},)=({return;},);}",
        ] {
            let program = parse(source).unwrap();
            let env = type_env(&program).unwrap();
            type_function_with_env(&program, 0, &env)
                .unwrap()
                .validate()
                .unwrap();
        }
        let error = check(&parse("fn main(){let(x,y)=loop{};}").unwrap()).unwrap_err();
        assert!(error.contains("inference is unsupported"));
        assert!(
            check(&parse("fn main(){let(x,y):(i32,bool)={return;(false,0)};}").unwrap()).is_err()
        );
    }

    #[test]
    fn unsupported_patterns_and_pattern_nesting_are_explicit() {
        for source in [
            "fn main(){let(x,..)=(1,2);}",
            "fn main(){let ref x=1;}",
            "fn main(){let x @ _=1;}",
            "fn main(){let 1=1;}",
            "fn main(){let(mut _)=1;}",
            "fn main(){let(x,y)=(1,2) else {};}",
            "struct S{x:i32} fn main(){let S{..,x}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{..,}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{x,..,}=S{x:1};}",
            "struct S{x:i32} fn main(){let S{. .}=S{x:1};}",
            "fn f((x,y):(i32,i32)){} fn main(){}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
        let source = format!(
            "fn main(){{let {}x{}=1;}}",
            "(".repeat(140),
            ")".repeat(140)
        );
        assert!(parse(&source).unwrap_err().contains("nesting"));
        let mut program = parse("fn main(){let _=1;}").unwrap();
        let mut pattern = Pattern::Wildcard;
        for _ in 0..140 {
            pattern = Pattern::Tuple(vec![pattern]);
        }
        let Stmt::LetPattern {
            pattern: target, ..
        } = &mut program.functions[0].body.statements[0]
        else {
            unreachable!()
        };
        *target = pattern;
        assert!(check(&program).unwrap_err().contains("nesting"));
    }

    #[test]
    fn structs_share_nominal_handles_within_one_environment_only() {
        let program = parse("struct S{x:i32} use S as Alias; fn id(x:Alias)->S{x} fn main(){let x=Alias{x:1};let y=id(x);y.x;}").unwrap();
        let env = type_env(&program).unwrap();
        let ty = ir::Ty::Struct(env.structs["S"].clone());
        assert_eq!(env.functions["id"].params, vec![ty.clone()]);
        assert_eq!(env.functions["id"].result, ty);
        assert!(!ty.is_copy());
        assert!(!ir::Ty::Tuple(vec![ty.clone()].into()).is_copy());
        let separate = type_env(&program).unwrap();
        assert_ne!(env.structs["S"], separate.structs["S"]);
        for index in 0..program.functions.len() {
            type_function_with_env(&program, index, &env).unwrap();
        }
    }

    #[test]
    fn struct_initializers_retain_source_order_and_checked_field_indices() {
        let program =
            parse("struct S{a:i32,b:bool} fn main(){let mut s=S{b:true,a:3};s.a=4;s.b;}").unwrap();
        let env = type_env(&program).unwrap();
        let body = type_function_with_env(&program, 0, &env).unwrap();
        let fields = body
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ir::ExprKind::Struct { fields, .. } => Some(fields),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            fields.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert!(
            body.body.statements.iter().any(
                |stmt| matches!(stmt,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Field(0)])
            )
        );
    }

    #[test]
    fn structs_reject_invalid_unused_declarations_and_nominal_mismatches() {
        for source in [
            "struct A{x:A} fn main(){}",
            "struct A{x:(B,)} struct B{x:A} fn main(){}",
            "struct A{x:Missing} fn main(){}",
            "struct A{x:i32,x:bool} fn main(){}",
            "struct A{} fn main(){}",
            "struct A{x:i32} struct B{x:i32} fn main(){let x:A=B{x:1};}",
            "struct A{x:i32} struct B{x:i32} fn main(){if true{A{x:1}}else{B{x:1}};}",
            "struct A{x:i32} fn main(){A{x:true};}",
            "struct A{x:i32} fn main(){A{};}",
            "struct A{x:i32} fn main(){A{x:1,x:2};}",
            "struct A{x:i32} fn main(){A{x:1,y:2};}",
            "struct A{x:i32} fn main(){A{x:1}==A{x:1};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn struct_privacy_and_type_value_namespaces_are_distinct() {
        let legal = "mod m{pub struct S{x:i32} pub fn S()->i32{7} pub fn make()->S{S{x:1}} mod child{fn read(s:super::S)->i32{s.x}}} use m::S; fn main(){let s=m::make();S();}";
        check(&parse(legal).unwrap()).unwrap();
        for source in [
            "mod m{pub struct S{x:i32}} fn main(){m::S{x:1};}",
            "mod m{pub struct S{x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make().x;}",
            "mod m{struct S{pub x:i32}} fn main(){let x:m::S=loop{};}",
            "mod m{} fn main(){let x:m=loop{};}",
            "mod S{} struct S{x:i32} fn main(){}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn inferred_private_types_are_not_exposed_through_calls_or_public_fields() {
        for source in [
            "mod m{struct S{pub x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make();}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){m::make().inner;}",
            "mod m{struct H{x:i32} pub fn take(x:H){}} fn main(){m::take(loop{});}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner.x=2;}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner=loop{};}",
            "mod m{struct H{pub x:i32} pub struct O{pub inner:H}} fn main(){m::O{inner:loop{}};}",
        ] {
            assert!(
                parse(source).and_then(|program| check(&program)).is_err(),
                "accepted {source}"
            );
        }
    }

    #[test]
    fn struct_conditions_allow_parenthesized_literals_and_nested_blocks() {
        check(&parse("struct S{x:bool} fn test(s:S)->bool{s.x} fn main(){if (S{x:true}).x {} while test(S{x:false}) {} if {let s=S{x:true};s.x} {}}").unwrap()).unwrap();
    }

    #[test]
    fn struct_expanded_depth_is_bounded_independent_of_declaration_order() {
        let mut source = "struct S0{x:i32}".to_owned();
        for index in 1..140 {
            source.push_str(&format!("struct S{index}{{x:S{}}}", index - 1));
        }
        source.push_str("fn main(){}");
        let program = parse(&source).unwrap();
        assert!(type_env(&program).unwrap_err().contains("nesting"));
        let mut source = "struct S0{x:i32}".to_owned();
        for index in 1..40 {
            source.push_str(&format!(
                "struct S{index}{{a:S{},b:S{}}}",
                index - 1,
                index - 1
            ));
        }
        source.push_str("fn f(x:S39){x;} fn main(){}");
        let program = parse(&source).unwrap();
        let env = type_env(&program).unwrap();
        // This fixture intentionally has an exponentially large value layout.
        // Its shared semantic DAG is shallow, independent of backend/analysis
        // resource limits when expanding concrete move paths or storage.
        type_function_with_env(&program, 0, &env).unwrap();
    }

    #[test]
    fn struct_dependency_width_is_not_dependency_depth() {
        let fields = (0..140)
            .map(|i| format!("f{i}:Leaf"))
            .collect::<Vec<_>>()
            .join(",");
        let program = parse(&format!(
            "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{}}"
        ))
        .unwrap();
        let env = type_env(&program).unwrap();
        assert_eq!(env.structs["Wide"].def().fields.len(), 140);
        let values = (0..140)
            .rev()
            .map(|i| format!("f{i}:Leaf{{x:1}}"))
            .collect::<Vec<_>>()
            .join(",");
        let program = parse(&format!(
            "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{Wide{{{values}}};}}"
        ))
        .unwrap();
        check(&program).unwrap();
    }

    #[test]
    fn module_paths_are_canonical_and_main_stays_root() {
        let p = parse(
            "mod a { fn hidden()->i32{1} pub mod b { pub fn run()->i32 {super::hidden()+super::super::root()} } pub fn run()->i32{self::b::run()} } fn root()->i32{2} fn main(){let x=crate::a::run(); let y=a::b::run();}",
        ).unwrap();
        check(&p).unwrap();
        let names: Vec<_> = p.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a::hidden", "a::b::run", "a::run", "root", "main"]);
        let Expr::Binary { left, right, .. } = p.functions[1].body.tail.as_deref().unwrap() else {
            panic!()
        };
        assert!(matches!(left.as_ref(), Expr::Call { name, .. } if name == "a::hidden"));
        assert!(matches!(right.as_ref(), Expr::Call { name, .. } if name == "root"));
    }
    #[test]
    fn syntax_and_resolution_are_separate() {
        let parsed = parse_unresolved("use crate::missing as alias; fn main(){nope();}").unwrap();
        assert_eq!(parsed.program.functions[0].name, "main");
        assert!(resolve(&parsed).unwrap_err().contains("unresolved"));
        let parsed = parse_unresolved("mod a{fn f(){}} fn main(){a::f();}").unwrap();
        assert!(resolve(&parsed).unwrap_err().contains("private"));
        // Body type errors remain independent of declaration/name preparation.
        let parsed = parse_unresolved("fn bad()->i32{true} fn main(){}").unwrap();
        let p = resolve(&parsed).unwrap();
        let signatures = signature_table(&p).unwrap();
        assert!(check_function_with_signatures(&p, 0, &signatures).is_err());
        check_function_with_signatures(&p, 1, &signatures).unwrap();
    }
    #[test]
    fn forward_function_and_module_aliases() {
        good(
            "use next as renamed; use crate::a as next; use renamed::f as call; mod a{pub fn f()->i32{4}} fn main(){let x=call(); let y=renamed::f();}",
        );
        good(
            "mod a { fn f(){} mod child {use super::f as call; pub fn run(){call();}} pub fn run(){child::run();}} fn main(){a::run();}",
        );
        good("mod a{use self as me; pub fn f(){}} fn main(){}");
        good("use crate as root; fn f(){} fn main(){root::f();}");
        bad("use other as one; use one as other; fn main(){}");
        bad("use crate::absent; fn main(){}");
        bad("mod a{fn f(){}} use crate::a::f as unused; fn main(){}");
    }
    #[test]
    fn private_items_accessible_only_from_owner_and_descendants() {
        good(
            "mod a{fn f(){} mod inner{pub fn g(){super::f();}} pub fn run(){inner::g();}} fn main(){a::run();}",
        );
        good(
            "mod a{fn f(){} use self::f as alias; mod b{pub fn f(){super::alias();}} pub fn run(){b::f();}} fn main(){a::run();}",
        );
        bad("mod a{fn f(){}} mod b{pub fn run(){crate::a::f();}} fn main(){b::run();}");
        bad("mod a{mod hidden{pub fn f(){}}} fn main(){a::hidden::f();}");
        bad("mod a{pub fn f(){} use self::f as alias;} fn main(){a::alias();}");
        // Component boundaries, not textual prefixes, establish descendants.
        bad("mod a{fn f(){}} mod ab{pub fn g(){crate::a::f();}} fn main(){}");
    }
    #[test]
    fn module_scopes_do_not_inherit_unqualified_parent_items() {
        bad("fn f(){} mod a{pub fn g(){f();}} fn main(){}");
        bad("mod a{pub fn f(){}} mod b{pub fn g(){a::f();}} fn main(){}");
        good("mod a{pub fn f(){}} mod b{use crate::a; pub fn g(){a::f();}} fn main(){b::g();}");
        bad("fn main(){super::f();}");
        bad("mod a{pub fn f(){super::super::g();}} fn main(){}");
        bad("mod a{fn main(){}}");
    }
    #[test]
    fn source_shadowing_is_checked_before_canonicalization() {
        good("fn f(){} fn main(){let f=1;crate::f();self::f();}");
        good("mod a{pub fn f(){}} use a::f as alias; fn main(){let f=1;alias();}");
        good("mod a{pub fn f(){}} fn main(){let a=1;a::f();}");
        good("fn f()->i32{1} fn main(){let f=f();}");
        bad("fn f(){} fn main(){let f=1;f();}");
        bad("mod a{pub fn f(){}} use a::f as alias; fn main(){let alias=1;alias();}");
        bad("mod a{fn f(){} pub fn g(f:i32){f();}} fn main(){}");
        good("fn f(){} fn main(){{let f=1;}f();}");
    }
    #[test]
    fn separate_namespaces_and_import_collisions() {
        good(
            "mod same{pub fn value()->i32{3}} fn same()->i32{7} fn main(){let x=same()+same::value();}",
        );
        good(
            "mod same{pub fn value()->i32{3}} fn same()->i32{7} use crate::same as alias; fn main(){let x=alias()+alias::value();}",
        );
        good("mod a{pub fn f(){}} use a::f as a; fn main(){a();a::f();}");
        good(
            "mod outer{mod a{pub fn f(){}} use a::f as a; pub fn run(){a();a::f();}} fn main(){outer::run();}",
        );
        good(
            "mod a{pub mod same{pub fn f(){}} fn same(){}} use a::same as alias; fn main(){alias::f();}",
        );
        good("mod a{mod same{} pub fn same(){}} use a::same as alias; fn main(){alias();}");
        bad("mod a{mod same{} fn same(){}} use a::same as alias; fn main(){}");
        good("mod a{} fn f(){} use a as x; use f as x; fn main(){x();}");
        bad("fn f(){} use f as x; use f as x; fn main(){}");
        bad("mod a{} use a as x; mod x{} fn main(){}");
        bad("fn f(){} use f as x; fn x(){} fn main(){}");
    }
    #[test]
    fn unsupported_module_forms_are_rejected_explicitly() {
        for (source, message) in [
            ("mod a; fn main(){}", "external module"),
            ("pub(crate) fn main(){}", "restricted pub"),
            ("use crate::a::*; fn main(){}", "glob and grouped"),
            ("use crate::a::{f}; fn main(){}", "glob and grouped"),
            ("pub use crate::a; fn main(){}", "pub use"),
            ("mod a{} mod a{} fn main(){}", "duplicate item"),
        ] {
            assert!(parse(source).unwrap_err().contains(message), "{source}");
        }
        bad("mod a{} fn main(){a();}");
        bad("fn f(){} fn main(){f::g();}");
        bad("fn main(){use crate::a;}");
        let nested = format!("{}{}fn main(){{}}", "mod a{".repeat(300), "}".repeat(300));
        assert!(parse(&nested).unwrap_err().contains("nesting"));
    }
    fn good(s: &str) {
        let p = parse(s).unwrap();
        check(&p).unwrap();
    }
    fn bad(s: &str) {
        assert!(parse(s).and_then(|p| check(&p)).is_err(), "accepted {s}");
    }
    #[test]
    fn escapes_comparisons_and_reserved() {
        let p = parse(r#"fn main(){println!("\u{1f980}\x21");}"#).unwrap();
        assert!(
            matches!(&p.functions[0].body.statements[0], Stmt::Expr(Expr::Println(s)) if s == "🦀!")
        );
        good("fn main(){let x = (1 < 2) == true; let y = 1<2 && 3<4;}");
        bad("fn main(){let x = true == 1 < 2;}");
        bad("fn try(){} fn main(){}");
        bad("fn main(){let x = 1; let x = true; x=2;}");
        for s in [
            r#"fn main(){println!("\u{d800}");}"#,
            r#"fn main(){println!("\x80");}"#,
            r#"fn main(){println!("\u{}");}"#,
        ] {
            bad(s);
        }
    }
    #[test]
    fn nesting_limits() {
        let nested = format!(
            "fn main(){{let x={}1{};}}",
            "(".repeat(300),
            ")".repeat(300)
        );
        assert!(parse(&nested).unwrap_err().contains("nesting"));
        let chain = format!("fn main(){{let x=1{};}}", "+1".repeat(10000));
        assert!(parse(&chain).unwrap_err().contains("nesting"));
        let blocks = format!("fn main(){{{}{} }}", "{".repeat(300), "}".repeat(300));
        assert!(parse(&blocks).unwrap_err().contains("nesting"));
    }
    #[test]
    fn independent_checks() {
        let p = parse("fn wrong()->i32 {true} fn main(){}").unwrap();
        check_signatures(&p).unwrap();
        assert!(check_function(&p, 0).is_err());
        check_function(&p, 1).unwrap();
        assert!(check_function(&p, 2).is_err());
    }
    #[test]
    fn recursion_and_flow() {
        good(
            "fn fib(n:i32)->i32 { if n < 2 { return n; } fib(n-1)+fib(n-2) } fn main(){ let mut x = fib(4); while x > 0 { x = x - 1; } println!(\"ok\\n{{done}}\"); }",
        );
    }
    #[test]
    fn scopes() {
        good("fn main(){let x=1; let x=x+1; {let x=true; let y=x && !false;} let y=x+1;}");
        bad("fn main(){let x=1; x=2;}");
        bad("fn main(){{let x=1;} let y=x;}");
        bad("fn f(x:i32,x:i32){} fn main(){}");
    }
    #[test]
    fn return_flow() {
        good("fn f(b:bool)->i32 {if b {return 1;} else {return 2;}} fn main(){}");
        bad("fn f(b:bool)->i32 {if b {return 1;}} fn main(){}");
        bad("fn f()->i32 {while true {return 1;}} fn main(){}");
        bad("fn f()->i32 {false && {return 1;};} fn main(){}");
        good("fn f()->i32 {return 1; false;} fn main(){}");
    }
    #[test]
    fn rejects_invalid() {
        for s in [
            "fn main(x:i32){}",
            "fn main()->i32{0}",
            "fn main(){foo();}",
            "fn f(x:i32){} fn main(){f();}",
            "fn main(){let x:bool=1;}",
            "fn main(){let café=1;}",
            "fn main(){println!(\"{}\");}",
            "fn main(){let x=2147483648;}",
            "fn main(){let x=1 < 2 < 3;}",
            "fn main(){} fn main(){}",
            "fn main(){let x=1; x();}",
        ] {
            bad(s);
        }
    }
    #[test]
    fn literals_and_comments() {
        good(
            "/* a /* b */ */ fn main(){let x=-2147483648; let y:()=(); println!(\"a\\t\\\"\\\\\");}",
        );
        bad("fn main(){println!(\"\\q\");}");
        assert!(parse("\nfn main(){@}").unwrap_err().starts_with("2:11:"));
    }
}
