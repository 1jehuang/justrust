//! `match_like_matches_macro`: a `match`, or an `if let .. else`, whose arms give `true` and
//! `false`, which `matches!` says alone.
//!
//! Follows clippy's `clippy_lints/src/matches/match_like_matches.rs` and the checks before it
//! in `clippy_lints/src/matches/mod.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0). A `Some(_)` arm is left to
//! `redundant_pattern_matching` as clippy leaves it when that lint is not allowed, which is
//! taken to be always.

use syntax::{
    AstNode,
    ast::{self, HasAttrs as _},
};

use super::utils;
use crate::{
    check::Cx,
    diag::{Applicability, SuggestionStyle},
    msrvs,
};

const LINT: &str = "match_like_matches_macro";

pub(super) fn check_match(cx: &mut Cx<'_, '_, '_>, expr: &ast::MatchExpr) {
    if !cx.on(LINT) || !cx.conf().meets(msrvs::MATCHES_MACRO) {
        return;
    }
    let (Some(range), Some(scrutinee), Some(list)) =
        (cx.range(expr.syntax()), expr.expr(), expr.match_arm_list())
    else {
        return;
    };
    let arms: Vec<ast::MatchArm> = list.arms().collect();
    // clippy's `walk_intra_arm_text` turns away a match with an arm under `#[cfg]`.
    if arms
        .iter()
        .any(|arm| arm.attrs().any(|it| it.path().is_some_and(|it| it.syntax().text() == "cfg")))
    {
        return;
    }
    let Some((last, without_last)) = arms.split_last() else { return };
    let Some((first, middle)) = without_last.split_first() else { return };
    if utils::contains_comment(expr.syntax(), expr.syntax().text_range())
        || !is_bool(cx, &ast::Expr::MatchExpr(expr.clone()))
    {
        return;
    }
    let (Some(b0), Some(b1)) = (
        first.expr().and_then(|it| find_bool_lit(&it)),
        last.expr().and_then(|it| find_bool_lit(&it)),
    ) else {
        return;
    };
    if b0 == b1 {
        return;
    }
    let first_guard_ok = first.guard().is_none_or(|guard| {
        !guard.syntax().descendants().any(|it| ast::LetExpr::can_cast(it.kind()))
    });
    let shape = (middle.is_empty() && first_guard_ok)
        || without_last.iter().all(|arm| {
            arm.attrs().next().is_none()
                && arm.guard().is_none()
                && arm.expr().and_then(|it| find_bool_lit(&it)) == Some(b0)
        });
    if !shape || !matches!(last.pat(), Some(ast::Pat::WildcardPat(_))) {
        return;
    }
    if without_last.iter().any(|arm| arm.pat().is_some_and(|it| is_some_wild(&it))) {
        return;
    }
    let mut pats = Vec::new();
    for arm in without_last {
        let Some(pat) = arm.pat().and_then(|it| cx.range(it.syntax())) else { return };
        pats.push(cx.snippet(pat));
    }
    let mut pat_and_guard = pats.join(" | ");
    if let Some(guard) = first.guard() {
        let Some(condition) = guard.condition().and_then(|it| cx.range(it.syntax())) else {
            return;
        };
        pat_and_guard = format!("{pat_and_guard} if {}", cx.snippet(condition));
    }
    let Some(snippet) = scrutinee_snippet(cx, &scrutinee) else { return };
    // An arm under a `cfg` that is off is gone before clippy sees the match.
    let applicability = Applicability::MaybeIncorrect;
    let span = cx.span(range);
    let code = format!("{}matches!({snippet}, {pat_and_guard})", if b0 { "" } else { "!" });
    cx.lint(LINT, expr.syntax(), span.clone(), "match expression looks like `matches!` macro")
        .suggestion_with_style(
            vec![(span, code)],
            "use `matches!` directly",
            applicability,
            SuggestionStyle::ShowAlways,
        );
}

pub(super) fn check_if_let(cx: &mut Cx<'_, '_, '_>, expr: &ast::IfExpr) {
    if !cx.on(LINT) || !cx.conf().meets(msrvs::MATCHES_MACRO) {
        return;
    }
    let (
        Some(range),
        Some(ast::Expr::LetExpr(let_expr)),
        Some(then),
        Some(ast::ElseBranch::Block(otherwise)),
    ) = (cx.range(expr.syntax()), expr.condition(), expr.then_branch(), expr.else_branch())
    else {
        return;
    };
    let (Some(pat), Some(scrutinee)) = (let_expr.pat(), let_expr.expr()) else { return };
    if utils::contains_comment(expr.syntax(), expr.syntax().text_range())
        || !is_bool(cx, &ast::Expr::IfExpr(expr.clone()))
    {
        return;
    }
    let (Some(b0), Some(b1)) = (
        find_bool_lit(&ast::Expr::BlockExpr(then)),
        find_bool_lit(&ast::Expr::BlockExpr(otherwise)),
    ) else {
        return;
    };
    if b0 == b1 || is_some_wild(&pat) {
        return;
    }
    let applicability = Applicability::MaybeIncorrect;
    let Some(pat) = cx.range(pat.syntax()) else { return };
    let pat = cx.snippet(pat);
    let Some(snippet) = scrutinee_snippet(cx, &scrutinee) else { return };
    let span = cx.span(range);
    let code = format!("{}matches!({snippet}, {pat})", if b0 { "" } else { "!" });
    cx.lint(
        LINT,
        expr.syntax(),
        span.clone(),
        "`if let .. else` expression looks like `matches!` macro",
    )
    .suggestion_with_style(
        vec![(span, code)],
        "use `matches!` directly",
        applicability,
        SuggestionStyle::ShowAlways,
    );
}

fn is_bool(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    cx.ty(expr).is_some_and(|it| it.is_bool())
}

/// The scrutinee as written, a borrow of a reference taken off (clippy's #6503).
fn scrutinee_snippet(cx: &Cx<'_, '_, '_>, scrutinee: &ast::Expr) -> Option<String> {
    let mut expr = scrutinee.clone();
    if let ast::Expr::RefExpr(borrow) = scrutinee
        && borrow.raw_token().is_none()
        && let Some(inner) = borrow.expr()
        && cx.ty(&inner).is_some_and(|it| it.is_reference())
    {
        expr = inner;
    }
    Some(cx.snippet(cx.range(expr.syntax())?))
}

/// clippy's `find_bool_lit`: a `bool` literal, or a block of nothing else.
fn find_bool_lit(expr: &ast::Expr) -> Option<bool> {
    let expr = match expr {
        ast::Expr::BlockExpr(block) if block.modifier().is_none() => {
            let list = block.stmt_list()?;
            if list.statements().next().is_some() {
                return None;
            }
            list.tail_expr()?
        }
        _ => expr.clone(),
    };
    let ast::Expr::Literal(literal) = expr else { return None };
    match literal.kind() {
        ast::LiteralKind::Bool(it) => Some(it),
        _ => None,
    }
}

/// clippy's `is_some_wild`: `Some(_, ..)`.
fn is_some_wild(pat: &ast::Pat) -> bool {
    let ast::Pat::TupleStructPat(pat) = pat else { return false };
    let first_wild = matches!(pat.fields().next(), Some(ast::Pat::WildcardPat(_)));
    let name = pat.path().and_then(|it| it.first_segment()).and_then(|it| it.name_ref());
    first_wild && name.is_some_and(|it| it.text() == "Some")
}
