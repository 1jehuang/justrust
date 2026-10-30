//! The format macros' arguments as rustc's `FormatArgs` has them: each placeholder of the
//! template with the trait it formats with and the arguments it takes.
//!
//! Follows `rustc_parse_format` and `rustc_builtin_macros/src/format.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0), for what the lints read.

use hir::HasSource as _;
use syntax::{
    AstNode, SyntaxNode, TextRange,
    ast::{self, HasAttrs as _, HasName as _},
};

use crate::check::Cx;

/// The std macros clippy counts as format macros (`FORMAT_MACRO_DIAG_ITEMS`).
const MACROS: &[&str] = &[
    "assert_eq",
    "assert",
    "assert_ne",
    "panic",
    "debug_assert_eq",
    "debug_assert",
    "debug_assert_ne",
    "eprint",
    "eprintln",
    "format_args",
    "format",
    "print",
    "println",
    "todo",
    "unimplemented",
    "write",
    "writeln",
];

/// The trait a placeholder formats with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Trait {
    Display,
    Debug,
    /// `x`, `X`, `o`, `b`, `e`, `E` or `p`.
    Other,
}

/// What a placeholder or its width or precision takes: an argument by index into all the
/// arguments written, or one captured from the scope by name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Arg {
    Index(usize),
    Captured(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Placeholder {
    pub(crate) arg: Arg,
    pub(crate) format_trait: Trait,
    pub(crate) width: Option<Arg>,
    pub(crate) precision: Option<Arg>,
}

/// A format macro call written in the file: its name, its placeholders and the arguments it
/// was handed, as its expansion has them.
pub(crate) struct FormatCall {
    pub(crate) name: String,
    /// Whether it is one of std's macros, not one marked `#[clippy::format_args]`.
    pub(crate) std: bool,
    pub(crate) placeholders: Vec<Placeholder>,
    pub(crate) args: Vec<ast::Expr>,
    /// The template, its escapes read.
    pub(crate) template: String,
    /// Where the template's literal is written.
    pub(crate) template_range: TextRange,
}

/// `call` as a format macro call, if it is one of std's or one its definition marks
/// `#[clippy::format_args]` (as `log`'s are), as clippy's `is_format_macro` has it, and its
/// template is a string literal written in it.
pub(crate) fn of(cx: &Cx<'_, '_, '_>, call: &ast::MacroCall) -> Option<FormatCall> {
    let (name, std) = match cx.std_macro(call) {
        Some(name) => (MACROS.contains(&name.as_str()).then_some(name)?, true),
        None => (marked_format_args(cx, call)?, false),
    };
    let input = cx.range(call.token_tree()?.syntax())?;
    let format_args = format_args_in(cx, call, input)?;
    let ast::Expr::Literal(literal) = format_args.template()? else { return None };
    let template_range = cx.original_range(literal.syntax())?;
    let ast::LiteralKind::String(template) = literal.kind() else { return None };
    let template = template.value().ok()?;
    let mut args = Vec::new();
    let mut names = Vec::new();
    for arg in format_args.args() {
        let expr = arg.expr()?;
        if let Some(name) = arg.name() {
            names.push((name.text().to_owned(), args.len()));
        }
        args.push(expr);
    }
    let placeholders = parse(&template, &names)?;
    Some(FormatCall {
        name,
        std,
        placeholders,
        args,
        template: template.into_owned(),
        template_range,
    })
}

/// The name of the macro `call` calls, if its definition has `#[clippy::format_args]`.
fn marked_format_args(cx: &Cx<'_, '_, '_>, call: &ast::MacroCall) -> Option<String> {
    let db = cx.db();
    let mac = cx.sema().resolve_macro_call(call)?;
    let source = mac.source(db)?.value;
    let marked = source
        .attrs()
        .any(|attr| attr.path().is_some_and(|path| path.syntax().text() == "clippy::format_args"));
    marked.then(|| mac.name(db).as_str().to_owned())
}

/// The `format_args!` call `call` expands to, at some depth, whose template is written in
/// `input`.
fn format_args_in(
    cx: &Cx<'_, '_, '_>,
    call: &ast::MacroCall,
    input: TextRange,
) -> Option<ast::FormatArgsExpr> {
    const MOST: usize = 16;
    let mut pending: Vec<SyntaxNode> = vec![cx.sema().expand_macro_call(call)?.value];
    let mut expanded = 0;
    while let Some(root) = pending.pop() {
        for node in root.descendants() {
            if let Some(format_args) = ast::FormatArgsExpr::cast(node.clone()) {
                let written = format_args.template().and_then(|it| cx.original_range(it.syntax()));
                if written.is_some_and(|it| input.contains_range(it)) {
                    return Some(format_args);
                }
            } else if let Some(inner) = ast::MacroCall::cast(node)
                && expanded < MOST
                && let Some(expansion) = cx.sema().expand_macro_call(&inner)
            {
                expanded += 1;
                pending.push(expansion.value);
            }
        }
    }
    None
}

/// The placeholders of `template`, its named arguments `names` given with their indices;
/// `None` if rustc would not read it.
pub(crate) fn parse(template: &str, names: &[(String, usize)]) -> Option<Vec<Placeholder>> {
    let mut parser = Parser { chars: template.chars().collect(), at: 0, next: 0, names };
    let mut placeholders = Vec::new();
    while let Some(c) = parser.bump() {
        match c {
            '{' if parser.eat('{') => {}
            '{' => placeholders.push(parser.placeholder()?),
            '}' if parser.eat('}') => {}
            '}' => return None,
            _ => {}
        }
    }
    Some(placeholders)
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    /// The index the next implicit argument takes.
    next: usize,
    names: &'a [(String, usize)],
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        let eaten = self.peek() == Some(c);
        if eaten {
            self.at += 1;
        }
        eaten
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn integer(&mut self) -> Option<usize> {
        let start = self.at;
        while self.peek().is_some_and(|it| it.is_ascii_digit()) {
            self.at += 1;
        }
        (self.at > start)
            .then(|| self.chars[start..self.at].iter().collect::<String>().parse().ok())?
    }

    fn word(&mut self) -> Option<String> {
        let start = self.at;
        if !self.peek().is_some_and(|it| it == '_' || it.is_alphabetic()) {
            return None;
        }
        while self.peek().is_some_and(|it| it == '_' || it.is_alphanumeric()) {
            self.at += 1;
        }
        Some(self.chars[start..self.at].iter().collect())
    }

    fn named(&self, name: String) -> Arg {
        match self.names.iter().find(|(it, _)| *it == name) {
            Some((_, index)) => Arg::Index(*index),
            None => Arg::Captured(name),
        }
    }

    /// A placeholder, after its `{`.
    fn placeholder(&mut self) -> Option<Placeholder> {
        self.skip_whitespace();
        let explicit = if let Some(index) = self.integer() {
            Some(Arg::Index(index))
        } else {
            self.word().map(|it| self.named(it))
        };
        self.skip_whitespace();
        let mut width = None;
        let mut precision = None;
        let mut star = false;
        let mut format_trait = Trait::Display;
        if self.eat(':') {
            if self.peek_at(1).is_some_and(|it| matches!(it, '<' | '^' | '>')) {
                self.at += 2;
            } else if self.peek().is_some_and(|it| matches!(it, '<' | '^' | '>')) {
                self.at += 1;
            }
            if !self.eat('+') {
                self.eat('-');
            }
            self.eat('#');
            if self.peek() == Some('0') && self.peek_at(1) == Some('$') {
                width = self.count();
            } else {
                self.eat('0');
                width = self.count();
            }
            if self.eat('.') {
                if self.eat('*') {
                    star = true;
                } else {
                    precision = self.count();
                }
            }
            let start = self.at;
            while self.peek().is_some_and(|it| it != '}') {
                self.at += 1;
            }
            let ty: String = self.chars[start..self.at].iter().collect();
            format_trait = match ty.trim() {
                "" => Trait::Display,
                "?" | "x?" | "X?" => Trait::Debug,
                "x" | "X" | "o" | "b" | "e" | "E" | "p" => Trait::Other,
                _ => return None,
            };
        }
        if !self.eat('}') {
            return None;
        }
        // `.*` takes the next argument for the precision, before the value's.
        if star {
            precision = Some(Arg::Index(self.next));
            self.next += 1;
        }
        let arg = explicit.unwrap_or_else(|| {
            self.next += 1;
            Arg::Index(self.next - 1)
        });
        Some(Placeholder { arg, format_trait, width, precision })
    }

    /// A width or precision: a number, or an argument followed by `$`. A literal number takes
    /// no argument.
    fn count(&mut self) -> Option<Arg> {
        let start = self.at;
        if let Some(index) = self.integer() {
            if self.eat('$') {
                return Some(Arg::Index(index));
            }
            return None;
        }
        if let Some(name) = self.word() {
            if self.eat('$') {
                return Some(self.named(name));
            }
            self.at = start;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Arg, Placeholder, Trait, parse};

    fn at(index: usize, format_trait: Trait) -> Placeholder {
        Placeholder { arg: Arg::Index(index), format_trait, width: None, precision: None }
    }

    #[test]
    fn placeholders_as_rustc_reads_them() {
        assert_eq!(
            parse("{{}} {} {:?} {x}", &[]),
            Some(vec![
                at(0, Trait::Display),
                at(1, Trait::Debug),
                Placeholder { arg: Arg::Captured("x".into()), ..at(0, Trait::Display) },
            ])
        );
        assert_eq!(
            parse("{0:1$.2$} {:016x?} {:.*} {n:>w$}", &[("n".into(), 5)]),
            Some(vec![
                Placeholder {
                    width: Some(Arg::Index(1)),
                    precision: Some(Arg::Index(2)),
                    ..at(0, Trait::Display)
                },
                at(0, Trait::Debug),
                Placeholder { precision: Some(Arg::Index(1)), ..at(2, Trait::Display) },
                Placeholder { width: Some(Arg::Captured("w".into())), ..at(5, Trait::Display) },
            ])
        );
        assert_eq!(
            parse("{:p} {:#x} {:08.3e}", &[]),
            Some(vec![at(0, Trait::Other), at(1, Trait::Other), at(2, Trait::Other),])
        );
        assert_eq!(parse("}", &[]), None);
    }
}
