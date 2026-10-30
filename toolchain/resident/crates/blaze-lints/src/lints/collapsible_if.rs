//! `collapsible_if`: `if a { if b { .. } }`, which is `if a && b { .. }`.
//!
//! Follows clippy's `clippy_lints/src/collapsible_if.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0). `collapsible_else_if`
//! (pedantic) is not ported, nor the `#[expect]` written on the inner `if` that clippy lets
//! through; code handed to a macro is not linted here.

use syntax::{AstNode, TextRange, TextSize, ast};

use crate::{check::Cx, diag::Applicability, msrvs};

const LINT: &str = "collapsible_if";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::IfExpr) {
    if !cx.on(LINT) || !cx.in_file() || expr.else_branch().is_some() {
        return;
    }
    let (Some(check), Some(then)) = (expr.condition(), expr.then_branch()) else { return };
    if !eligible(cx, &check) || then.label().is_some() || then.modifier().is_some() {
        return;
    }
    let Some(stmts) = then.stmt_list() else { return };
    let Some(inner) = expr_block(&stmts) else { return };
    let ast::Expr::IfExpr(inner_if) = peel_parens(&inner) else { return };
    let Some(check_inner) = inner_if.condition() else { return };
    if inner_if.else_branch().is_some() || !eligible(cx, &check_inner) {
        return;
    }
    let (Some(open), Some(close)) = (stmts.l_curly_token(), stmts.r_curly_token()) else { return };
    let text = cx.text();
    let inner_range = inner.syntax().text_range();
    // Nothing but whitespace before the inner `if`: a comment or an attribute stays.
    let before = &text[usize::from(open.text_range().end())..usize::from(inner_range.start())];
    if !before.trim().is_empty() {
        return;
    }
    let after = &text[usize::from(inner_range.end())..usize::from(close.text_range().start())];
    if contains_cfg(after) {
        return;
    }
    let open_range = with_leading_whitespace(text, open.text_range());
    let close_range = with_leading_whitespace(text, close.text_range());
    let inner_if_start = inner_if.syntax().text_range().start();
    let mut parts = vec![
        (cx.span(open_range), String::new()),
        (cx.span(close_range), String::new()),
        (cx.span(TextRange::at(inner_if_start, TextSize::from(2))), "&&".to_owned()),
    ];
    if inner_if_start > inner_range.start() {
        parts.push((cx.span(TextRange::new(inner_range.start(), inner_if_start)), String::new()));
        let inner_if_end = inner_if.syntax().text_range().end();
        parts.push((cx.span(TextRange::new(inner_if_end, inner_range.end())), String::new()));
    }
    parts.extend(parens_around(cx, &check));
    parts.extend(parens_around(cx, &check_inner));
    let span = cx.span(expr.syntax().text_range());
    cx.lint(LINT, inner_if.syntax(), span, "this `if` statement can be collapsed")
        .multipart_suggestion("collapse nested if block", parts, Applicability::MachineApplicable);
}

/// clippy's `eligible_condition`: not a `let`, unless `let` chains can be written.
fn eligible(cx: &Cx<'_, '_, '_>, cond: &ast::Expr) -> bool {
    !matches!(cond, ast::Expr::LetExpr(_))
        || (cx.edition() >= span::Edition::Edition2024 && cx.conf().meets(msrvs::LET_CHAINS))
}

/// clippy's `expr_block`: the block's tail with no statements, or its one statement ended by
/// `;` with no tail.
fn expr_block(stmts: &ast::StmtList) -> Option<ast::Expr> {
    let statements: Vec<ast::Stmt> = stmts.statements().collect();
    match (&statements[..], stmts.tail_expr()) {
        ([], tail) => tail,
        ([ast::Stmt::ExprStmt(stmt)], None) if stmt.semicolon_token().is_some() => stmt.expr(),
        _ => None,
    }
}

fn peel_parens(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    while let ast::Expr::ParenExpr(paren) = &expr
        && let Some(inner) = paren.expr()
    {
        expr = inner;
    }
    expr
}

/// `(` and `)` around `expr` when it is an `||`, which `&&` would bind tighter than.
fn parens_around(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> Vec<(crate::diag::Span, String)> {
    let ast::Expr::BinExpr(bin) = peel_parens(expr) else { return Vec::new() };
    if bin.op_kind() != Some(ast::BinaryOp::LogicOp(ast::LogicOp::Or)) {
        return Vec::new();
    }
    let range = expr.syntax().text_range();
    vec![
        (cx.span(TextRange::empty(range.start())), "(".to_owned()),
        (cx.span(TextRange::empty(range.end())), ")".to_owned()),
    ]
}

/// clippy's `span_contains_cfg`: `#[cfg` written in `text`.
fn contains_cfg(text: &str) -> bool {
    let squeezed: String = text.chars().filter(|it| !it.is_whitespace()).collect();
    squeezed.contains("#[cfg(") || squeezed.contains("#![cfg(")
}

/// clippy's `with_leading_whitespace`: `range` with the whitespace before it, unless that
/// whitespace starts after a line comment and code follows `range` on its line.
fn with_leading_whitespace(text: &str, range: TextRange) -> TextRange {
    let start = usize::from(range.start());
    let trimmed = text[..start].trim_end().len();
    let line_start = text[..trimmed].rfind('\n').map_or(0, |it| it + 1);
    let crosses_line = text[trimmed..start].contains('\n');
    let after_comment = text[line_start..trimmed].contains("//");
    let rest_of_line = text[usize::from(range.end())..].split('\n').next().unwrap_or("");
    if crosses_line && after_comment && !rest_of_line.trim().is_empty() {
        return range;
    }
    TextRange::new(TextSize::from(trimmed as u32), range.end())
}
