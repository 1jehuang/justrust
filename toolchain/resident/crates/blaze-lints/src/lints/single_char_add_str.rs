//! `single_char_add_str`: `s.push_str("a")` or `s.insert_str(i, "a")`, which `push('a')` and
//! `insert(i, 'a')` say.
//!
//! Follows clippy's `clippy_lints/src/methods/single_char_add_str.rs` and `clippy_utils`'s
//! `str_literal_to_char_literal` at rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0
//! (MIT or Apache-2.0).

use hir::AsAssocItem as _;
use syntax::{
    AstNode,
    ast::{self, HasArgList as _},
};

use super::utils;
use crate::{check::Cx, diag::Applicability};

const LINT: &str = "single_char_add_str";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::Expr) {
    if !cx.on(LINT) {
        return;
    }
    let Some(call) = utils::method_call(cx, expr) else { return };
    let (short_name, arg, index) = match (call.name.as_str(), &call.args[..]) {
        ("push_str", [arg]) => ("push", arg, None),
        ("insert_str", [index, arg]) => ("insert", arg, Some(index)),
        _ => return,
    };
    if !is_string_method(cx, expr) {
        return;
    }
    let (Some(range), Some(receiver)) = (cx.range(expr.syntax()), cx.range(call.receiver.syntax()))
    else {
        return;
    };
    let base = cx.snippet(receiver);
    let index = match index {
        Some(index) => match cx.range(index.syntax()) {
            Some(it) => format!("{}, ", cx.snippet(it)),
            None => return,
        },
        None => String::new(),
    };
    let (message, help, extension) = if let Some(literal) = char_literal(cx, arg) {
        (
            format!("calling `{short_name}_str()` using a single-character string literal"),
            format!("consider using `{short_name}` with a character literal"),
            literal,
        )
    } else if let ast::Expr::RefExpr(borrow) = arg
        && borrow.raw_token().is_none()
        && let Some(ast::Expr::MethodCallExpr(to_string)) = borrow.expr()
        && to_string.name_ref().is_some_and(|it| it.text() == "to_string")
        && to_string.arg_list().is_some_and(|it| it.args().next().is_none())
        && let Some(char_expr) = to_string.receiver()
        && let Some(ty) = cx.ty(&char_expr)
        && let Some(at) = cx.range(char_expr.syntax())
    {
        let is_ref_char = ty.as_reference().is_some_and(|(it, _)| it.is_char());
        if !is_ref_char && !ty.is_char() {
            return;
        }
        let snippet = cx.snippet(at);
        (
            format!("calling `{short_name}_str()` using a single-character converted to string"),
            format!("consider using `{short_name}` without `to_string()`"),
            if is_ref_char { format!("*{snippet}") } else { snippet },
        )
    } else {
        return;
    };
    let span = cx.span(range);
    cx.lint(LINT, expr.syntax(), span.clone(), message).suggestion(
        span,
        help,
        format!("{base}.{short_name}({index}{extension})"),
        Applicability::MachineApplicable,
    );
}

/// Whether the method `expr` calls is `String`'s own.
fn is_string_method(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    let ast::Expr::MethodCallExpr(method) = expr else { return false };
    let db = cx.db();
    let Some(function) = cx.sema().resolve_method_call(method) else { return false };
    let implementing = function.as_assoc_item(db).and_then(|it| it.implementing_ty(db));
    let string = match cx.std_def(&["alloc", "string", "String"]) {
        Some(hir::ModuleDef::Adt(it)) => it,
        _ => return false,
    };
    implementing.and_then(|it| it.as_adt()) == Some(string)
}

/// clippy's `str_literal_to_char_literal`: the character literal a string literal of one
/// character is.
fn char_literal(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> Option<String> {
    let ast::Expr::Literal(literal) = expr else { return None };
    let ast::LiteralKind::String(string) = literal.kind() else { return None };
    if string.value().ok()?.chars().count() != 1 {
        return None;
    }
    let snippet = cx.snippet(cx.range(literal.syntax())?);
    let hashes = snippet.strip_prefix('r').map(|it| it.len() - it.trim_start_matches('#').len());
    let ch = match hashes {
        Some(hashes) => snippet.get(hashes + 2..snippet.len() - 1 - hashes)?,
        None => snippet.get(1..snippet.len() - 1)?,
    };
    Some(format!(
        "'{}'",
        match ch {
            "'" => "\\'",
            r"\" => "\\\\",
            "\\\"" => "\"",
            _ => ch,
        }
    ))
}
