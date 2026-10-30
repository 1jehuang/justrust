//! `byte_char_slices`: `&[b'a', b'b']`, which a byte string says shorter.
//!
//! Follows clippy's `clippy_lints/src/byte_char_slices.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use syntax::{AstNode, ast};

use super::utils;
use crate::{check::Cx, diag::Applicability, sugg::Sugg};

const LINT: &str = "byte_char_slices";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::Expr) {
    if !cx.on(LINT) {
        return;
    }
    let (has_ref, inner) = match expr {
        ast::Expr::RefExpr(borrow)
            if borrow.raw_token().is_none() && borrow.mut_token().is_none() =>
        {
            let Some(inner) = borrow.expr() else { return };
            (true, inner)
        }
        _ => {
            // A borrowed array is linted with its borrow.
            if past_parens(expr)
                .parent()
                .and_then(ast::RefExpr::cast)
                .is_some_and(|it| it.raw_token().is_none() && it.mut_token().is_none())
            {
                return;
            }
            (false, expr.clone())
        }
    };
    let ast::Expr::ArrayExpr(array) = strip_parens(&inner) else { return };
    if array.semicolon_token().is_some() {
        return;
    }
    let Some(range) = cx.range(expr.syntax()) else { return };
    let members: Vec<ast::Expr> = array.exprs().collect();
    let source = cx.sema().parse(cx.file);
    if members.is_empty()
        || utils::contains_comment(source.syntax(), range)
        || cx.snippet(range).contains("#[cfg")
    {
        return;
    }
    let mut bytes = String::from("b\"");
    for member in &members {
        let ast::Expr::Literal(literal) = member else { return };
        if !matches!(literal.kind(), ast::LiteralKind::Byte(_)) {
            return;
        }
        let Some(at) = cx.range(literal.syntax()) else { return };
        let snippet = cx.snippet(at);
        let Some(inner) = snippet.get(2..snippet.len().saturating_sub(1)) else { return };
        bytes.push_str(match inner {
            "\"" => "\\\"",
            "\\'" => "'",
            other => other,
        });
    }
    bytes.push('"');
    let mut sugg = Sugg::NonParen(bytes);
    if !has_ref && !cx.adjusted_ty(expr).is_some_and(|it| is_array_slice(&it)) {
        sugg = sugg.deref();
    }
    let span = cx.span(range);
    cx.lint(LINT, expr.syntax(), span.clone(), "can be more succinctly written as a byte str")
        .suggestion(span, "try", sugg.to_string(), Applicability::MachineApplicable);
}

/// rustc's `Ty::is_array_slice`: a slice, or a reference or pointer to one.
fn is_array_slice(ty: &hir::Type<'_>) -> bool {
    ty.is_slice() || ty.as_reference().is_some_and(|(it, _)| it.is_slice())
}

fn past_parens(expr: &ast::Expr) -> syntax::SyntaxNode {
    let mut node = expr.syntax().clone();
    while let Some(parent) = node.parent()
        && ast::ParenExpr::can_cast(parent.kind())
    {
        node = parent;
    }
    node
}

fn strip_parens(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    while let ast::Expr::ParenExpr(paren) = &expr
        && let Some(inner) = paren.expr()
    {
        expr = inner;
    }
    expr
}
