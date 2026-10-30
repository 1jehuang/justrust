//! `needless_return`: `return x` as the last thing a function or closure does, where `x`
//! alone would do.
//!
//! Follows clippy's `clippy_lints/src/returns/needless_return.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0). Not ported: clippy's check
//! that the returned value leaks no temporary with a significant drop.

use syntax::{
    AstNode, SyntaxNode, TextRange, TextSize,
    ast::{self, HasAttrs as _},
};

use crate::{check::Cx, diag::Applicability};

const LINT: &str = "needless_return";

/// clippy's `RetReplacement`: what the `return` gives way to.
#[derive(Clone, PartialEq, Eq)]
enum Replacement {
    Empty,
    Block,
    Unit,
    NeedsPar(String),
    Expr(String),
}

impl Replacement {
    fn help(&self) -> &'static str {
        match self {
            Self::Empty | Self::Expr(_) => "remove `return`",
            Self::Block => "replace `return` with an empty block",
            Self::Unit => "replace `return` with a unit value",
            Self::NeedsPar(_) => "remove `return` and wrap the sequence with parentheses",
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Block => "{}".to_owned(),
            Self::Unit => "()".to_owned(),
            Self::NeedsPar(inner) => format!("({inner})"),
            Self::Expr(inner) => inner.clone(),
        }
    }
}

/// What a needless `return` found is reported with.
struct Found {
    ret: ast::ReturnExpr,
    ret_span: TextRange,
    semis: Vec<TextRange>,
    replacement: Replacement,
}

/// clippy's `check_fn` for a function or a method with a body.
pub(super) fn check_fn(cx: &mut Cx<'_, '_, '_>, function: &ast::Fn) {
    if !cx.on(LINT) || cx.range(function.syntax()).is_none() {
        return;
    }
    let Some(body) = function.body() else { return };
    let mut found = Vec::new();
    check_block(cx, &ast::Expr::BlockExpr(body), Vec::new(), &mut found);
    report(cx, function.syntax(), found);
}

/// clippy's `check_fn` for a closure.
pub(super) fn check_closure(cx: &mut Cx<'_, '_, '_>, closure: &ast::ClosureExpr) {
    if !cx.on(LINT) || cx.range(closure.syntax()).is_none() {
        return;
    }
    let Some(body) = closure.body() else { return };
    let replacement = match &body {
        ast::Expr::ReturnExpr(it) if it.expr().is_none() => Replacement::Block,
        _ => Replacement::Empty,
    };
    let mut found = Vec::new();
    check_final(cx, &body, Vec::new(), replacement, None, &mut found);
    report(cx, closure.syntax(), found);
}

fn report(cx: &mut Cx<'_, '_, '_>, when: &SyntaxNode, found: Vec<Found>) {
    for Found { ret, ret_span, semis, replacement } in found {
        let Some(range) = cx.range(ret.syntax()) else { continue };
        let mut parts = vec![(cx.span(ret_span), replacement.text())];
        parts.extend(semis.into_iter().map(|it| (cx.span(it), String::new())));
        let span = cx.span(range);
        cx.lint_when(LINT, ret.syntax(), when, span, "unneeded `return` statement")
            .multipart_suggestion(replacement.help(), parts, Applicability::MachineApplicable);
    }
}

/// clippy's `check_block_return`: the last expression of `expr`, if it is a block.
fn check_block(
    cx: &Cx<'_, '_, '_>,
    expr: &ast::Expr,
    mut semis: Vec<TextRange>,
    found: &mut Vec<Found>,
) {
    let ast::Expr::BlockExpr(block) = expr else { return };
    if !matches!(
        block.modifier(),
        None | Some(ast::BlockModifier::Unsafe(_) | ast::BlockModifier::Label(_))
    ) {
        return;
    }
    let Some(list) = block.stmt_list() else { return };
    if let Some(tail) = list.tail_expr() {
        check_final(cx, &tail, semis, Replacement::Empty, None, found);
        return;
    }
    let Some(ast::Stmt::ExprStmt(stmt)) = list.statements().last() else { return };
    let (Some(stmt_range), Some(list_range)) = (cx.range(stmt.syntax()), cx.range(list.syntax()))
    else {
        return;
    };
    if contains_cfg(cx, stmt_range.end(), list_range.end()) {
        return;
    }
    let Some(inner) = stmt.expr() else { return };
    if stmt.semicolon_token().is_some()
        && let Some(inner_range) = cx.range(inner.syntax())
    {
        // The semicolon and the spaces before it, which without `return` would make the
        // value a statement.
        let start = inner_range.end();
        let text = cx.snippet(TextRange::new(start, list_range.end()));
        let len: usize =
            text.chars().take_while(|it| *it == ' ' || *it == ';').map(char::len_utf8).sum();
        semis.push(TextRange::at(start, TextSize::try_from(len).unwrap_or_default()));
    }
    check_final(cx, &inner, semis, Replacement::Empty, None, found);
}

/// clippy's `check_final_expr`.
fn check_final(
    cx: &Cx<'_, '_, '_>,
    expr: &ast::Expr,
    semis: Vec<TextRange>,
    replacement: Replacement,
    match_ty: Option<hir::Type<'_>>,
    found: &mut Vec<Found>,
) {
    match expr {
        ast::Expr::ReturnExpr(ret) => {
            let Some(range) = cx.range(ret.syntax()) else { return };
            let inner = ret.expr();
            let ret_span = if inner.is_none() && replacement == Replacement::Empty {
                extend_to_previous_non_ws(cx, range)
            } else {
                range
            };
            let replacement = match &inner {
                Some(inner) => {
                    let Some(at) = cx.range(inner.syntax()) else { return };
                    let snippet = cx.snippet(at);
                    if needs_parentheses(inner) {
                        Replacement::NeedsPar(snippet)
                    } else {
                        Replacement::Expr(snippet)
                    }
                }
                None => match match_ty {
                    Some(ty) if ty.is_unit() || ty.is_tuple() => Replacement::Unit,
                    Some(_) => return,
                    None => replacement,
                },
            };
            // An attribute on the `return` keeps it an expression of its own.
            if ret.attrs().next().is_some() {
                return;
            }
            found.push(Found { ret: ret.clone(), ret_span, semis, replacement });
        }
        ast::Expr::IfExpr(if_expr) => {
            if let Some(then) = if_expr.then_branch() {
                check_block(cx, &ast::Expr::BlockExpr(then), semis.clone(), found);
            }
            match if_expr.else_branch() {
                Some(ast::ElseBranch::Block(block)) => {
                    check_final(
                        cx,
                        &ast::Expr::BlockExpr(block),
                        semis,
                        Replacement::Empty,
                        match_ty,
                        found,
                    );
                }
                Some(ast::ElseBranch::IfExpr(inner)) => {
                    check_final(
                        cx,
                        &ast::Expr::IfExpr(inner),
                        semis,
                        Replacement::Empty,
                        match_ty,
                        found,
                    );
                }
                None => {}
            }
        }
        ast::Expr::MatchExpr(match_expr) => {
            let ty = cx.ty(expr);
            for arm in match_expr.match_arm_list().into_iter().flat_map(|it| it.arms()) {
                if let Some(body) = arm.expr() {
                    check_final(cx, &body, semis.clone(), Replacement::Unit, ty.clone(), found);
                }
            }
        }
        _ => check_block(cx, expr, semis, found),
    }
}

/// clippy's `binary_expr_needs_parentheses`: an operator whose left operand starts with a
/// block, which as a statement would end there.
fn needs_parentheses(expr: &ast::Expr) -> bool {
    fn contains_block(expr: &ast::Expr, is_operand: bool) -> bool {
        match expr {
            ast::Expr::BinExpr(bin) => bin.lhs().is_some_and(|it| contains_block(&it, true)),
            ast::Expr::CastExpr(cast) => cast.expr().is_some_and(|it| contains_block(&it, true)),
            _ if is_block_like(expr) => is_operand,
            _ => false,
        }
    }
    contains_block(expr, false)
}

fn is_block_like(expr: &ast::Expr) -> bool {
    matches!(
        expr,
        ast::Expr::BlockExpr(_)
            | ast::Expr::IfExpr(_)
            | ast::Expr::LoopExpr(_)
            | ast::Expr::MatchExpr(_)
            | ast::Expr::WhileExpr(_)
            | ast::Expr::ForExpr(_)
    )
}

/// clippy's `extend_span_to_previous_non_ws`.
fn extend_to_previous_non_ws(cx: &Cx<'_, '_, '_>, range: TextRange) -> TextRange {
    let before = &cx.text()[..usize::from(range.start())];
    match before.bytes().rposition(|c| !b" \t\n".contains(&c)) {
        Some(at) => TextRange::new(TextSize::try_from(at + 1).unwrap_or_default(), range.end()),
        None => range,
    }
}

/// clippy's `span_contains_cfg`: whether a `#[cfg(..)]` is written in `start..end`.
fn contains_cfg(cx: &Cx<'_, '_, '_>, start: TextSize, end: TextSize) -> bool {
    let text = cx.snippet(TextRange::new(start, end));
    let compact: String = text.chars().filter(|it| !it.is_whitespace()).collect();
    compact.contains("#[cfg(") || compact.contains("#![cfg(")
}
