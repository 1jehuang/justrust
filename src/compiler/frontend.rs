//! Native, deliberately restricted Rust frontend. No external compiler is invoked.
//!
//! Syntax diagnostics have one-based line/column positions. The span-free public
//! AST means semantic diagnostics identify functions and names instead of spans.
//! Block/if expression statements conservatively require unit even with a
//! semicolon, and while loops do not establish that a function always returns.
//! Numeric literals are decimal i32 only. Unsupported syntax is rejected.
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
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
                } else if matches!(expr, Expr::If { .. } | Expr::Block(_)) {
                    statements.push(Stmt::Expr(expr));
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
            let name = self.ident()?;
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

/// Parse the supported syntax without performing semantic body checks.
pub fn parse(source: &str) -> Result<Program, String> {
    let mut parser = Parser {
        tokens: lex(source)?,
        at: 0,
        nesting: 0,
    };
    let mut functions = Vec::new();
    while !parser.is("") {
        functions.push(parser.function()?);
    }
    let program = Program { functions };
    for function in &program.functions {
        validate_depth(DepthNode::Block(&function.body))
            .map_err(|e| format!("1:1: in function {}: {e}", function.name))?;
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
                Expr::Block(block) => push(DepthNode::Block(block)),
                _ => {}
            },
            DepthNode::Block(block) => {
                for stmt in &block.statements {
                    match stmt {
                        Stmt::Let { value, .. }
                        | Stmt::Assign { value, .. }
                        | Stmt::Expr(value)
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
struct Checker<'a> {
    signatures: &'a Signatures,
    scopes: Vec<HashMap<String, (Type, bool)>>,
    result: Type,
}
// None means the expression cannot complete normally (the subset's never type).
type Flow = Option<Type>;
impl<'a> Checker<'a> {
    fn new(signatures: &'a Signatures, result: Type) -> Self {
        Self {
            signatures,
            scopes: vec![HashMap::new()],
            result,
        }
    }
    fn bind(&mut self, n: &str, t: Type, mutable: bool) -> Result<(), String> {
        let scope = self.scopes.last_mut().unwrap();
        if scope.insert(n.to_owned(), (t, mutable)).is_some() {
            return Err(format!("duplicate parameter {n}"));
        }
        Ok(())
    }
    fn lookup(&self, n: &str) -> Result<(Type, bool), String> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(n).copied())
            .ok_or_else(|| format!("unresolved local {n}"))
    }
    fn require(&self, got: Flow, want: Type) -> Result<(), String> {
        if got.is_some_and(|t| t != want) {
            Err(format!("expected {want:?}, got {got:?}"))
        } else {
            Ok(())
        }
    }
    fn block(&mut self, b: &Block) -> Result<Flow, String> {
        self.scopes.push(HashMap::new());
        let mut live = true;
        for s in &b.statements {
            let flow = match s {
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    value,
                } => {
                    let f = self.expr(value)?;
                    let t = if *ty == Type::Infer {
                        f.unwrap_or(Type::Unit)
                    } else {
                        self.require(f, *ty)?;
                        *ty
                    };
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(name.clone(), (t, *mutable));
                    f.map(|_| Type::Unit)
                }
                Stmt::Assign { name, value } => {
                    let (t, m) = self.lookup(name)?;
                    if !m {
                        return Err(format!("assignment to immutable local {name}"));
                    }
                    let f = self.expr(value)?;
                    self.require(f, t)?;
                    f.map(|_| Type::Unit)
                }
                Stmt::Expr(e) => {
                    let f = self.expr(e)?;
                    if matches!(e, Expr::If { .. } | Expr::Block(_)) {
                        self.require(f, Type::Unit)?;
                    }
                    f.map(|_| Type::Unit)
                }
                Stmt::Return(e) => {
                    let f = if let Some(e) = e {
                        self.expr(e)?
                    } else {
                        Some(Type::Unit)
                    };
                    self.require(f, self.result)?;
                    None
                }
                Stmt::While { condition, body } => {
                    let f = self.expr(condition)?;
                    self.require(f, Type::Bool)?;
                    let bf = self.block(body)?;
                    self.require(bf, Type::Unit)?;
                    f.map(|_| Type::Unit)
                }
            };
            live &= flow.is_some();
        }
        let tail = if let Some(e) = &b.tail {
            self.expr(e)?
        } else {
            Some(Type::Unit)
        };
        self.scopes.pop();
        Ok(if live { tail } else { None })
    }
    fn expr(&mut self, e: &Expr) -> Result<Flow, String> {
        Ok(match e {
            Expr::Int(_) => Some(Type::I32),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Var(n) => Some(self.lookup(n)?.0),
            Expr::Println(_) => Some(Type::Unit),
            Expr::Unary { op, value } => {
                let f = self.expr(value)?;
                let t = match op {
                    UnaryOp::Neg => Type::I32,
                    UnaryOp::Not => Type::Bool,
                };
                self.require(f, t)?;
                f.map(|_| t)
            }
            Expr::Binary { op, left, right } => {
                let l = self.expr(left)?;
                let r = self.expr(right)?;
                let (operand, result) = match op {
                    BinaryOp::And | BinaryOp::Or => (Type::Bool, Type::Bool),
                    BinaryOp::Eq | BinaryOp::Ne => (l.or(r).unwrap_or(Type::Unit), Type::Bool),
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        (Type::I32, Type::Bool)
                    }
                    _ => (Type::I32, Type::I32),
                };
                self.require(l, operand)?;
                self.require(r, operand)?;
                if l.is_none() || (r.is_none() && !matches!(op, BinaryOp::And | BinaryOp::Or)) {
                    None
                } else {
                    Some(result)
                }
            }
            Expr::Call { name, args } => {
                if self.scopes.iter().rev().any(|s| s.contains_key(name)) {
                    return Err(format!("local {name} is not callable"));
                }
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
                let mut live = true;
                for (a, t) in args.iter().zip(params) {
                    let f = self.expr(a)?;
                    self.require(f, *t)?;
                    live &= f.is_some();
                }
                if live { Some(*result) } else { None }
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                let c = self.expr(condition)?;
                self.require(c, Type::Bool)?;
                let t = self.block(then_block)?;
                let e = self.block(else_block)?;
                if let (Some(a), Some(b)) = (t, e)
                    && a != b
                {
                    return Err(format!("if branches differ: {a:?} and {b:?}"));
                }
                if c.is_none() { None } else { t.or(e) }
            }
            Expr::Block(b) => self.block(b)?,
        })
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
    let f = program
        .functions
        .get(index)
        .ok_or_else(|| format!("invalid function index {index}"))?;
    validate_depth(DepthNode::Block(&f.body))?;
    let mut c = Checker::new(signatures, f.result);
    for (n, t) in &f.params {
        c.bind(n, *t, false)
            .map_err(|e| format!("in function {}: {e}", f.name))?;
    }
    let flow = c
        .block(&f.body)
        .map_err(|e| format!("in function {}: {e}", f.name))?;
    c.require(flow, f.result)
        .map_err(|e| format!("in function {}: {e}", f.name))
}
pub fn check(program: &Program) -> Result<(), String> {
    let signatures = signature_table(program)?;
    for i in 0..program.functions.len() {
        check_function_with_signatures(program, i, &signatures)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
