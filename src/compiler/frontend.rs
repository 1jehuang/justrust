//! Native, deliberately restricted Rust frontend. No external compiler is invoked.
//!
//! Syntax diagnostics have one-based line/column positions. The span-free public
//! AST means semantic diagnostics identify functions and names instead of spans.
//! Explicit semicolons discard expression values, while omitted-semicolon
//! blocklike statements require unit. While loops do not establish return flow.
//! Numeric literals are decimal i32 only. Unsupported syntax is rejected.
//! Inline modules, public/private functions/modules, and simple module-level
//! imports have separate module/value namespaces. External modules, restricted
//! visibility, reexports, grouped/glob imports, and block-local items are not
//! supported. An executable requires a declared root main, not an imported alias.
use super::ir;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
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
    pub params: Vec<(String, Type)>,
    pub result: Type,
    pub body: Block,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    I32,
    Bool,
    Unit,
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
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: Type,
        value: Expr,
    },
    Assign {
        name: String,
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
#[derive(Clone, Debug)]
pub enum Expr {
    Int(i32),
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
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    line: usize,
    column: usize,
    string: bool,
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
        } else if c.is_ascii_alphabetic() || c == '_' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                advance(&chars, &mut i, &mut line, &mut column);
            }
        } else {
            if c == '\'' {
                return Err(format!(
                    "{line}:{column}: labels, lifetimes and character literals are unsupported"
                ));
            }
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
                        | ('&', '&')
                        | ('|', '|')
                )
            {
                text.push(next);
                advance(&chars, &mut i, &mut line, &mut column);
            }
            if !matches!(
                text.as_str(),
                "(" | ")"
                    | "{"
                    | "}"
                    | ":"
                    | "::"
                    | ";"
                    | ","
                    | "->"
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
        if self.eat("i32") {
            Ok(Type::I32)
        } else if self.eat("bool") {
            Ok(Type::Bool)
        } else if self.eat("(") {
            self.expect(")")?;
            Ok(Type::Unit)
        } else {
            Err(self.error("expected i32, bool or ()"))
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
            matches!(
                (&item.kind, &kind),
                (ItemKind::Module, ItemKind::Module) | (ItemKind::Function, ItemKind::Function)
            )
        }) {
            return Err(self.error(&format!("duplicate item {path}")));
        }
        bindings.push(Item { public, kind });
        Ok(())
    }
    fn function(&mut self) -> Result<Function, String> {
        self.expect("fn")?;
        let name = self.ident()?;
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
        Ok(Function {
            name,
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
        let result = self.block_inner();
        self.nesting -= 1;
        result
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
                let mutable = self.eat("mut");
                let name = self.ident()?;
                // Local inference is performed by each independent function checker.
                let annotation = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect("=")?;
                let value = self.expr(0)?;
                self.expect(";")?;
                let ty = annotation.unwrap_or(Type::Infer);
                statements.push(Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                });
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
                let condition = self.expr(0)?;
                let body = self.block()?;
                statements.push(Stmt::While { condition, body });
            } else if self.at + 1 < self.tokens.len()
                && self.tokens[self.at + 1].text == "="
                && !self.token().string
            {
                let name = self.ident()?;
                self.expect("=")?;
                let value = self.expr(0)?;
                self.expect(";")?;
                statements.push(Stmt::Assign { name, value });
            } else {
                let expr = self.expr(0)?;
                if self.eat(";") {
                    statements.push(Stmt::Expr(expr));
                } else if self.is("}") {
                    tail = Some(Box::new(expr));
                    break;
                } else if matches!(expr, Expr::If { .. } | Expr::Block(_) | Expr::Loop { .. }) {
                    statements.push(Stmt::ImplicitExpr(expr));
                } else {
                    return Err(self.error("expected semicolon or block end"));
                }
            }
        }
        self.expect("}")?;
        Ok(Block { statements, tail })
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
    fn expr_inner(&mut self, min: u8) -> Result<Expr, String> {
        let mut left = if self.eat("-") {
            if self.is("2147483648") {
                self.at += 1;
                Expr::Int(i32::MIN)
            } else {
                Expr::Unary {
                    op: UnaryOp::Neg,
                    value: Box::new(self.expr(7)?),
                }
            }
        } else if self.eat("!") {
            Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(self.expr(7)?),
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
            let value = if matches!(self.token().text.as_str(), ";" | "}" | ")" | "," | "") {
                None
            } else {
                Some(Box::new(self.expr(0)?))
            };
            Expr::Break { value }
        } else if self.eat("continue") {
            Expr::Continue
        } else if self.eat("if") {
            let condition = Box::new(self.expr(0)?);
            let then_block = self.block()?;
            let else_block = if self.eat("else") {
                if self.is("if") {
                    Block {
                        statements: Vec::new(),
                        tail: Some(Box::new(self.expr(0)?)),
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
        } else if self.eat("(") {
            if self.eat(")") {
                Expr::Block(Block {
                    statements: Vec::new(),
                    tail: None,
                })
            } else {
                let e = self.expr(0)?;
                self.expect(")")?;
                e
            }
        } else if !self.token().string
            && self.token().text.starts_with(|c: char| c.is_ascii_digit())
        {
            let n = self
                .token()
                .text
                .parse::<i32>()
                .map_err(|_| self.error("invalid or out-of-range i32 literal"))?;
            self.at += 1;
            Expr::Int(n)
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
                        args.push(self.expr(0)?);
                        if !self.eat(",") || self.is(")") {
                            break;
                        }
                    }
                }
                self.expect(")")?;
                Expr::Call { name, args }
            } else {
                if name.contains("::") || matches!(name.as_str(), "crate" | "self" | "super") {
                    return Err(self.error("qualified paths are supported only as function calls"));
                }
                Expr::Var(name)
            }
        };
        let mut compared = false;
        loop {
            let (precedence, op) = match self.token().text.as_str() {
                "||" => (1, BinaryOp::Or),
                "&&" => (2, BinaryOp::And),
                "==" => (3, BinaryOp::Eq),
                "!=" => (3, BinaryOp::Ne),
                "<" => (3, BinaryOp::Lt),
                "<=" => (3, BinaryOp::Le),
                ">" => (3, BinaryOp::Gt),
                ">=" => (3, BinaryOp::Ge),
                "+" => (5, BinaryOp::Add),
                "-" => (5, BinaryOp::Sub),
                "*" => (6, BinaryOp::Mul),
                "/" => (6, BinaryOp::Div),
                "%" => (6, BinaryOp::Rem),
                _ => break,
            };
            if self.token().string || precedence < min {
                break;
            }
            if precedence == 3 || precedence == 4 {
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
    };
    let mut parsed = ParsedProgram {
        program: Program {
            functions: Vec::new(),
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
                }),
                ItemKind::Function if !module => Some(ResolvedItem {
                    path: path.into(),
                    module: false,
                }),
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
                        if module { "module" } else { "function" }
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
                }
            }
            "self" => {
                at = 1;
                ResolvedItem {
                    path: from.into(),
                    module: true,
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
            if !current.module {
                return Err(format!("{} is not a module in path {path}", current.path));
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
    fn block(
        &mut self,
        block: &mut Block,
        module: &str,
        locals: &mut Vec<std::collections::HashSet<String>>,
    ) -> Result<(), String> {
        locals.push(std::collections::HashSet::new());
        for stmt in &mut block.statements {
            match stmt {
                Stmt::Let { name, value, .. } => {
                    self.expr(value, module, locals)?;
                    locals.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Assign { value, .. }
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
            Expr::Call { name, args } => {
                // Check the source spelling, not its eventual canonical name:
                // `let f=1; crate::f()` must not be shadowed by the local f.
                if !name.contains("::") && locals.iter().rev().any(|s| s.contains(name)) {
                    return Err(format!("local {name} is not callable"));
                }
                let resolved = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                *name = resolved.path;
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
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
    for function in &mut program.functions {
        validate_depth(DepthNode::Block(&function.body))?;
        let mut locals = vec![function.params.iter().map(|(n, _)| n.clone()).collect()];
        resolver
            .block(&mut function.body, parent(&function.name), &mut locals)
            .map_err(|e| format!("in function {}: {e}", function.name))?;
    }
    Ok(program)
}
enum DepthNode<'a> {
    Expr(&'a Expr),
    Block(&'a Block),
}
fn validate_depth(root: DepthNode<'_>) -> Result<(), String> {
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > 128 {
            return Err("AST nesting exceeds limit 128".into());
        }
        let mut push = |node| stack.push((node, depth + 1));
        match node {
            DepthNode::Expr(expr) => match expr {
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
                        Stmt::Let { value, .. }
                        | Stmt::Assign { value, .. }
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
pub type Signatures = HashMap<String, (Vec<Type>, Type)>;
fn signatures(program: &Program) -> Result<Signatures, String> {
    let mut signatures: Signatures = HashMap::new();
    for f in &program.functions {
        if f.result == Type::Infer || f.params.iter().any(|(_, t)| *t == Type::Infer) {
            return Err(format!(
                "in function {}: signature types cannot be inferred",
                f.name
            ));
        }
        if signatures
            .insert(
                f.name.clone(),
                (f.params.iter().map(|(_, t)| *t).collect(), f.result),
            )
            .is_some()
        {
            return Err(format!("duplicate function {}", f.name));
        }
    }
    match signatures.get("main") {
        Some((p, Type::Unit)) if p.is_empty() => {}
        _ => return Err("main must exist, take no arguments and return ()".into()),
    }
    Ok(signatures)
}
struct LoopFrame {
    id: ir::LoopId,
    accepts_value: bool,
    checking_condition: bool,
    break_ty: Option<Type>,
    can_exit: bool,
}
struct Checker<'a> {
    signatures: &'a Signatures,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    locals: Vec<ir::LocalDecl>,
    exprs: Vec<ir::TypedExpr>,
    loops: Vec<LoopFrame>,
    next_loop: usize,
    reachable: bool,
    result: Type,
}
// Semantic type and possible normal completion are independent. In particular,
// an unreachable explicit block tail retains its type, unlike a pure never value.
#[derive(Clone, Copy)]
struct Flow {
    ty: Type,
    completes: bool,
}
impl Flow {
    fn value(ty: Type) -> Self {
        Self {
            ty,
            completes: true,
        }
    }
    fn never() -> Self {
        Self {
            ty: Type::Never,
            completes: false,
        }
    }
}
fn checked_ty(ty: Type) -> Result<ir::Ty, String> {
    match ty {
        Type::I32 => Ok(ir::Ty::I32),
        Type::Bool => Ok(ir::Ty::Bool),
        Type::Unit => Ok(ir::Ty::Unit),
        Type::Never => Ok(ir::Ty::Never),
        Type::Infer => Err("signature types cannot be inferred".into()),
    }
}
fn syntax_ty(ty: ir::Ty) -> Type {
    match ty {
        ir::Ty::I32 => Type::I32,
        ir::Ty::Bool => Type::Bool,
        ir::Ty::Unit => Type::Unit,
        ir::Ty::Never => Type::Never,
    }
}
impl<'a> Checker<'a> {
    fn new(signatures: &'a Signatures, result: Type) -> Self {
        Self {
            signatures,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            exprs: Vec::new(),
            loops: Vec::new(),
            next_loop: 0,
            reachable: true,
            result,
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
    fn local(&mut self, name: &str, ty: Type, mutable: bool) -> Result<ir::LocalId, String> {
        let local = ir::LocalId(self.locals.len());
        self.locals.push(ir::LocalDecl {
            name: name.to_owned(),
            ty: checked_ty(ty)?,
            mutable,
        });
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_owned(), local);
        Ok(local)
    }
    fn bind(&mut self, name: &str, ty: Type) -> Result<ir::LocalId, String> {
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
            ty: syntax_ty(e.ty),
            completes: e.completion == ir::Completion::Returns,
        }
    }
    fn require(&self, got: Flow, want: Type) -> Result<(), String> {
        if got.ty != Type::Never && got.ty != want {
            Err(format!("expected {want:?}, got {:?}", got.ty))
        } else {
            Ok(())
        }
    }
    fn join(&self, a: Type, b: Type, context: &str) -> Result<Type, String> {
        if a == Type::Never {
            Ok(b)
        } else if b == Type::Never || a == b {
            Ok(a)
        } else {
            Err(format!("{context} differ: {a:?} and {b:?}"))
        }
    }
    fn expr_when(&mut self, e: &Expr, reachable: bool) -> Result<ir::ExprId, String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.expr(e);
        self.reachable = previous;
        result
    }
    fn block_when(&mut self, b: &Block, reachable: bool) -> Result<(ir::Block, Flow), String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.block(b);
        self.reachable = previous;
        result
    }
    fn block(&mut self, b: &Block) -> Result<(ir::Block, Flow), String> {
        let entered_reachable = self.reachable;
        self.scopes.push(HashMap::new());
        let mut live = true;
        let mut statements = Vec::with_capacity(b.statements.len());
        for s in &b.statements {
            self.reachable = entered_reachable && live;
            let (statement, completes) = match s {
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                } => {
                    let value = self.expr(value)?;
                    let f = self.flow(value);
                    let t = if *ty == Type::Infer {
                        f.ty
                    } else {
                        self.require(f, *ty)?;
                        *ty
                    };
                    let local = self.local(name, t, *mutable)?;
                    (ir::Stmt::Let { local, value }, f.completes)
                }
                Stmt::Assign { name, value } => {
                    let local = self.lookup(name)?;
                    let declaration = &self.locals[local.0];
                    let t = syntax_ty(declaration.ty);
                    if !declaration.mutable {
                        return Err(format!("assignment to immutable local {name}"));
                    }
                    let value = self.expr(value)?;
                    let f = self.flow(value);
                    self.require(f, t)?;
                    (ir::Stmt::Assign { local, value }, f.completes)
                }
                Stmt::Expr(e) | Stmt::ImplicitExpr(e) => {
                    let value = self.expr(e)?;
                    let f = self.flow(value);
                    if matches!(s, Stmt::ImplicitExpr(_)) {
                        self.require(f, Type::Unit)?;
                    }
                    (ir::Stmt::Expr(value), f.completes)
                }
                Stmt::Return(e) => {
                    let value = e.as_ref().map(|e| self.expr(e)).transpose()?;
                    let f = value.map_or(Flow::value(Type::Unit), |e| self.flow(e));
                    self.require(f, self.result)?;
                    (ir::Stmt::Return(value), false)
                }
                Stmt::While { condition, body } => {
                    // Never skip a condition's target barrier to an outer loop.
                    let id = self.enter_loop(false);
                    let condition = self.expr(condition)?;
                    let f = self.flow(condition);
                    self.require(f, Type::Bool)?;
                    self.loops.last_mut().unwrap().checking_condition = false;
                    let (body, bf) = self.block(body)?;
                    self.require(bf, Type::Unit)?;
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
        let tail = b.tail.as_ref().map(|e| self.expr(e)).transpose()?;
        self.reachable = entered_reachable;
        let mut flow = tail.map_or_else(
            || {
                if live {
                    Flow::value(Type::Unit)
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
        let (kind, flow) = match e {
            Expr::Int(value) => (ir::ExprKind::Int(*value), Flow::value(Type::I32)),
            Expr::Bool(value) => (ir::ExprKind::Bool(*value), Flow::value(Type::Bool)),
            Expr::Var(n) => {
                let local = self.lookup(n)?;
                let ty = syntax_ty(self.locals[local.0].ty);
                (
                    ir::ExprKind::Local(local),
                    Flow {
                        ty,
                        completes: ty != Type::Never,
                    },
                )
            }
            Expr::Println(text) => (ir::ExprKind::Println(text.clone()), Flow::value(Type::Unit)),
            Expr::Unary { op, value } => {
                let value = self.expr(value)?;
                let f = self.flow(value);
                let t = match op {
                    UnaryOp::Neg => Type::I32,
                    UnaryOp::Not => Type::Bool,
                };
                self.require(f, t)?;
                if matches!(op, UnaryOp::Neg) && f.ty == Type::Never {
                    return Err("unary negation of never is unsupported".into());
                }
                let ty = if f.ty == Type::Never { Type::Never } else { t };
                (
                    ir::ExprKind::Unary { op: *op, value },
                    Flow {
                        ty,
                        completes: f.completes,
                    },
                )
            }
            Expr::Binary { op, left, right } => {
                let left = self.expr(left)?;
                let right = self.expr_when(right, self.reachable && self.flow(left).completes)?;
                let l = self.flow(left);
                let r = self.flow(right);
                let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
                let (operand, result) = match op {
                    BinaryOp::And | BinaryOp::Or => (Type::Bool, Type::Bool),
                    BinaryOp::Eq | BinaryOp::Ne => {
                        (self.join(l.ty, r.ty, "equality operands")?, Type::Bool)
                    }
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        (Type::I32, Type::Bool)
                    }
                    _ => (Type::I32, Type::I32),
                };
                self.require(l, operand)?;
                self.require(r, operand)?;
                // Trait-based scalar operators do not gain a never coercion
                // merely because evaluating an operand cannot complete.
                if !logical && (l.ty == Type::Never || r.ty == Type::Never) {
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
                let (params, result) = self
                    .signatures
                    .get(name)
                    .ok_or_else(|| format!("unresolved function {name}"))?;
                if params.len() != args.len() {
                    return Err(format!(
                        "wrong arity for {name}: expected {}, got {}",
                        params.len(),
                        args.len()
                    ));
                }
                let signature = ir::Signature {
                    params: params
                        .iter()
                        .copied()
                        .map(checked_ty)
                        .collect::<Result<_, _>>()?,
                    result: checked_ty(*result)?,
                };
                let mut completes = true;
                let mut arguments = Vec::with_capacity(args.len());
                for (a, t) in args.iter().zip(params) {
                    let value = self.expr_when(a, self.reachable && completes)?;
                    let f = self.flow(value);
                    self.require(f, *t)?;
                    completes &= f.completes;
                    arguments.push(value);
                }
                (
                    ir::ExprKind::Call {
                        name: name.clone(),
                        args: arguments,
                        signature,
                    },
                    Flow {
                        ty: *result,
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
                self.require(c, Type::Bool)?;
                let reachable = self.reachable && c.completes;
                let (then_block, t) = self.block_when(then_block, reachable)?;
                let (else_block, e) = self.block_when(else_block, reachable)?;
                let ty = self.join(t.ty, e.ty, "if branches")?;
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
                self.require(body_flow, Type::Unit)?;
                let frame = self.loops.pop().unwrap();
                let flow = Flow {
                    ty: frame.break_ty.unwrap_or(Type::Never),
                    completes: frame.can_exit,
                };
                (ir::ExprKind::Loop { id, body }, flow)
            }
            Expr::Break { value } => {
                let target = self.loop_target("break")?;
                let value = value.as_ref().map(|e| self.expr(e)).transpose()?;
                let flow = value.map_or(Flow::value(Type::Unit), |e| self.flow(e));
                let frame = self.loops.last().unwrap();
                let previous_break_ty = frame.break_ty;
                if !frame.accepts_value && value.is_some() {
                    return Err("break with a value is only allowed in loop, not while".into());
                }
                // Check every lexical break and the operand before contributing.
                // A typed but unreachable tail is not the never type.
                if self.reachable && flow.completes {
                    self.loops.last_mut().unwrap().can_exit = true;
                }
                if flow.ty != Type::Never {
                    let previous = previous_break_ty.unwrap_or(Type::Never);
                    let ty = self.join(previous, flow.ty, "break values")?;
                    self.loops.last_mut().unwrap().break_ty = Some(ty);
                }
                (ir::ExprKind::Break { target, value }, Flow::never())
            }
            Expr::Continue => {
                let target = self.loop_target("continue")?;
                (ir::ExprKind::Continue { target }, Flow::never())
            }
            Expr::Block(b) => {
                let (block, flow) = self.block(b)?;
                (ir::ExprKind::Block(block), flow)
            }
        };
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: checked_ty(flow.ty)?,
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
/// Validate signatures, lexical scopes, types, mutability, calls and return flow.
pub fn signature_table(program: &Program) -> Result<Signatures, String> {
    let table = signatures(program)?;
    for f in &program.functions {
        let mut names = std::collections::HashSet::new();
        for (name, _) in &f.params {
            if !names.insert(name) {
                return Err(format!(
                    "in function {}: duplicate parameter {name}",
                    f.name
                ));
            }
        }
    }
    Ok(table)
}
pub fn check_signatures(program: &Program) -> Result<(), String> {
    signature_table(program).map(|_| ())
}
pub fn check_function(program: &Program, index: usize) -> Result<(), String> {
    let signatures = signature_table(program)?;
    check_function_with_signatures(program, index, &signatures)
}
/// `signatures` must be the validated table for this same immutable program.
pub fn check_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<(), String> {
    type_function_with_signatures(program, index, signatures).map(|_| ())
}
/// Check and lower one body using the validated signatures for this program.
/// The returned arena owns all resolved locals, expression types and call facts.
pub fn type_function_with_signatures(
    program: &Program,
    index: usize,
    signatures: &Signatures,
) -> Result<ir::CheckedFunction, String> {
    let f = program
        .functions
        .get(index)
        .ok_or_else(|| format!("invalid function index {index}"))?;
    validate_depth(DepthNode::Block(&f.body))?;
    let mut c = Checker::new(signatures, f.result);
    let mut params = Vec::with_capacity(f.params.len());
    for (n, t) in &f.params {
        params.push(
            c.bind(n, *t)
                .map_err(|e| format!("in function {}: {e}", f.name))?,
        );
    }
    let (body, flow) = c
        .block(&f.body)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    c.require(flow, f.result)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    Ok(ir::CheckedFunction {
        name: f.name.clone(),
        params,
        result: checked_ty(f.result).map_err(|e| format!("in function {}: {e}", f.name))?,
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

    fn typed(source: &str, index: usize) -> ir::CheckedFunction {
        let program = parse(source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, index, &signatures).unwrap();
        checked.validate().unwrap();
        checked
    }

    #[test]
    fn loop_control_has_distinct_resolved_targets_and_scalar_types() {
        let f = typed(
            "fn main(){ let x=loop { while true { continue; break; } loop { break; } break 3; }; let y=loop { break true; }; }",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty).collect::<Vec<_>>(),
            vec![ir::Ty::I32, ir::Ty::Bool]
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
        assert_eq!(root.flow(), Some(ir::Ty::I32));
        let f = typed(
            "fn f()->i32 { loop { break { return 3; }; } } fn main(){}",
            0,
        );
        assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
        let f = typed("fn f()->i32 { loop { continue; break 3; } } fn main(){}", 0);
        assert_eq!(f.exprs[f.body.tail.unwrap().0].ty, ir::Ty::I32);
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
        assert_eq!(value.ty, ir::Ty::I32);
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
            f.locals.iter().map(|l| l.ty).collect::<Vec<_>>(),
            vec![ir::Ty::I32, ir::Ty::I32, ir::Ty::Bool, ir::Ty::I32]
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
        assert_eq!(call.flow(), Some(ir::Ty::I32));
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
        assert_eq!(signature.result, ir::Ty::I32);
        assert_eq!(f.exprs[args[0].0].ty, ir::Ty::Bool);
        assert_eq!(f.locals[0].ty, ir::Ty::I32);
    }

    #[test]
    fn divergence_and_short_circuit_keep_existing_completion() {
        let f = typed(
            "fn f()->bool { let x=false && { return true; }; let y=true || { return false; }; let z={ return x; }; } fn main(){}",
            0,
        );
        assert_eq!(
            f.locals.iter().map(|l| l.ty).collect::<Vec<_>>(),
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
            "fn main(){ return; let x=1; x=2; }",
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
