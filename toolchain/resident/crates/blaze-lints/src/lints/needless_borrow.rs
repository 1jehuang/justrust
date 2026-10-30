//! `needless_borrow`: `&x` where `x` is already a reference and the compiler dereferences the
//! borrow away again.
//!
//! Follows clippy's `clippy_lints/src/dereference.rs` (the `DerefedBorrow` state) at
//! rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0), for a borrow
//! used as a call's or method's argument, a field's value, a `let`'s initializer or what a
//! `return` gives. Not ported: borrows that are a receiver, a callee or have a field read
//! (where clippy asks whether auto-borrow would pick the same method) and chains of `&` and
//! `*`. A `ref` binding to a reference whose every use the compiler
//! dereferences is linted too, as clippy's `check_pat` does.

use hir::{Adjust, AutoBorrow, Mutability};
use syntax::{
    AstNode, SyntaxNode, TextRange,
    ast::{self, HasName as _},
};

use super::utils;
use crate::{check::Cx, diag::Applicability, sugg::has_enclosing_paren};

const LINT: &str = "needless_borrow";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::Expr) {
    if !cx.on(LINT) || cx.expansion.is_some() {
        return;
    }
    let ast::Expr::RefExpr(borrow) = expr else { return };
    if borrow.raw_token().is_some() {
        return;
    }
    let Some(sub) = borrow.expr() else { return };
    if !cx.is_written(sub.syntax()) || is_ref_op(&strip_parens(&sub)) {
        return;
    }
    // The node the borrow is used at, past the parentheses around it.
    let mut used = expr.syntax().clone();
    while let Some(parent) = used.parent()
        && ast::ParenExpr::can_cast(parent.kind())
    {
        used = parent;
    }
    let Some(parent) = used.parent() else { return };
    let in_call = ast::ArgList::can_cast(parent.kind());
    let is_use = in_call
        || ast::RecordExprField::can_cast(parent.kind())
        || ast::LetStmt::can_cast(parent.kind())
        || ast::ReturnExpr::can_cast(parent.kind());
    if !is_use {
        return;
    }
    let Some(adjustments) =
        ast::Expr::cast(used.clone()).and_then(|it| cx.sema().expr_adjustments(&it))
    else {
        return;
    };
    let mut iter = adjustments.iter();
    let mut deref_count = 0usize;
    let next = loop {
        match iter.next() {
            Some(adjust) if !matches!(adjust.kind, Adjust::Deref(_)) => break Some(adjust),
            Some(adjust) if !adjust.target.is_reference() => {
                deref_count += 1;
                break iter.next();
            }
            Some(_) => deref_count += 1,
            None => break None,
        }
    };
    // A mutable reborrow the compiler does not insert by itself keeps one more reference, so
    // that the borrow is not moved.
    let reborrows_mut =
        next.is_some_and(|it| matches!(it.kind, Adjust::Borrow(AutoBorrow::Ref(Mutability::Mut))));
    let required = if reborrows_mut && !reborrow_stable(cx, &used, &parent) { 3 } else { 2 };
    if deref_count < required {
        return;
    }
    // `&*p` of a raw pointer stays, as rustc's `dangerous_implicit_autorefs` wants it.
    if let ast::Expr::PrefixExpr(prefix) = strip_parens(&sub)
        && prefix.op_kind() == Some(ast::UnaryOp::Deref)
        && prefix.expr().and_then(|it| cx.adjusted_ty(&it)).is_some_and(|it| it.is_raw_ptr())
    {
        return;
    }
    let (Some(range), Some(sub_range)) = (cx.range(expr.syntax()), cx.range(sub.syntax())) else {
        return;
    };
    let snip = cx.snippet(sub_range);
    // A method call binds tighter than an operator: a call's arguments are each whole.
    let in_method_call =
        in_call && parent.parent().is_some_and(|it| ast::MethodCallExpr::can_cast(it.kind()));
    let needs_paren = in_method_call
        && matches!(
            sub,
            ast::Expr::PrefixExpr(_)
                | ast::Expr::BinExpr(_)
                | ast::Expr::CastExpr(_)
                | ast::Expr::RangeExpr(_)
        );
    let sugg = if needs_paren && !has_enclosing_paren(&snip) { format!("({snip})") } else { snip };
    let span = cx.span(range);
    cx.lint(
        LINT,
        expr.syntax(),
        span.clone(),
        "this expression creates a reference which is immediately dereferenced by the compiler",
    )
    .suggestion(span, "change this to", sugg, Applicability::MachineApplicable);
}

/// clippy's `TyCoercionStability::is_reborrow_stable` for where the borrow is used: whether
/// the type declared there is a reference, which the compiler reborrows into by itself.
fn reborrow_stable(cx: &Cx<'_, '_, '_>, used: &SyntaxNode, parent: &SyntaxNode) -> bool {
    let db = cx.db();
    if let Some(args) = ast::ArgList::cast(parent.clone()) {
        let Some(index) = args.args().position(|it| it.syntax() == used) else { return false };
        let declared = match args.syntax().parent().and_then(ast::Expr::cast) {
            Some(ast::Expr::MethodCallExpr(call)) => cx
                .sema()
                .resolve_method_call(&call)
                .and_then(|it| it.params_without_self(db).into_iter().nth(index))
                .map(|it| it.ty().clone()),
            Some(ast::Expr::CallExpr(call)) => {
                let Some(callable) =
                    call.expr().and_then(|it| cx.sema().resolve_expr_as_callable(&it))
                else {
                    return false;
                };
                match callable.kind() {
                    hir::CallableKind::Function(function) => function
                        .params_without_self(db)
                        .into_iter()
                        .nth(index)
                        .map(|it| it.ty().clone()),
                    _ => callable.params().into_iter().nth(index).map(|it| it.ty().clone()),
                }
            }
            _ => None,
        };
        return declared.is_some_and(|it| it.is_reference());
    }
    if let Some(field) = ast::RecordExprField::cast(parent.clone()) {
        return cx
            .sema()
            .resolve_record_field(&field)
            .is_some_and(|(field, ..)| field.ty(db).is_reference());
    }
    if let Some(stmt) = ast::LetStmt::cast(parent.clone()) {
        return matches!(stmt.ty(), Some(ast::Type::RefType(_)));
    }
    if ast::ReturnExpr::can_cast(parent.kind()) {
        let function = parent.ancestors().find_map(ast::Fn::cast);
        return function
            .and_then(|it| cx.sema().to_def(&it))
            .is_some_and(|it| it.ret_type(db).is_reference());
    }
    false
}

/// clippy's `try_parse_ref_op`: a borrow or a dereference.
fn is_ref_op(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::RefExpr(it) => it.raw_token().is_none(),
        ast::Expr::PrefixExpr(it) => it.op_kind() == Some(ast::UnaryOp::Deref),
        _ => false,
    }
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

/// clippy's `check_pat` and `check_local_usage`: `ref x` where `x` is bound to a `&&T`, used
/// only where the compiler dereferences it twice, as a field's base or dereferenced.
pub(super) fn check_ref_pat(cx: &mut Cx<'_, '_, '_>, pat: &ast::IdentPat) {
    if !cx.on(LINT) || pat.ref_token().is_none() || pat.mut_token().is_some() {
        return;
    }
    let db = cx.db();
    let Some(local) = cx.sema().to_def(pat) else { return };
    let ty = local.ty(db);
    let Some((inner, _)) = ty.as_reference() else { return };
    if !matches!(inner.as_reference(), Some((_, Mutability::Shared))) {
        return;
    }
    let Some(body) = utils::enclosing_body(pat.syntax()) else { return };
    // The bindings of `local`, more than one in alternatives of an or-pattern.
    let bindings: Vec<ast::IdentPat> = body_and_params(&body)
        .flat_map(|it| it.descendants().filter_map(ast::IdentPat::cast).collect::<Vec<_>>())
        .filter(|it| cx.sema().to_def(it) == Some(local))
        .collect();
    if bindings.first().is_some_and(|it| it.syntax() != pat.syntax()) {
        return;
    }
    let mut replacements: Vec<(TextRange, String)> = Vec::new();
    let mut spans = Vec::new();
    for binding in &bindings {
        let (Some(range), Some(name)) = (cx.range(binding.syntax()), binding.name()) else {
            return;
        };
        let Some(name) = cx.range(name.syntax()) else { return };
        spans.push(range);
        replacements.push((range, cx.snippet(name)));
    }
    let mut cancelled = false;
    let mut always_deref = true;
    utils::each_path(cx, &body, None, &mut |path| {
        if cancelled || utils::local_of(cx, path) != Some(local) {
            return;
        }
        let expr = ast::Expr::PathExpr(path.clone());
        let adjustments = cx.sema().expr_adjustments(&expr).unwrap_or_default();
        if matches!(&adjustments[..], [first, second, ..] if matches!(first.kind, Adjust::Deref(_)) && matches!(second.kind, Adjust::Deref(_)))
        {
            return;
        }
        let used = past_parens(path.syntax());
        match used.parent().and_then(parent_expr) {
            Some(ast::Expr::FieldExpr(_)) => {}
            Some(ast::Expr::PrefixExpr(prefix))
                if prefix.op_kind() == Some(ast::UnaryOp::Deref) =>
            {
                match (cx.range(prefix.syntax()), cx.range(path.syntax())) {
                    (Some(at), Some(name)) => replacements.push((at, cx.snippet(name))),
                    _ => cancelled = true,
                }
            }
            Some(parent) if cx.range(parent.syntax()).is_some() => {
                if unambiguous(&parent) {
                    cancelled = true;
                } else {
                    always_deref = false;
                }
            }
            _ if cx.range(path.syntax()).is_some() => always_deref = false,
            _ => cancelled = true,
        }
    });
    // A binding some use needs as a double reference is `ref_binding_to_reference`'s.
    if cancelled || !always_deref {
        return;
    }
    let mut span = crate::diag::MultiSpan::default();
    span.primary = spans.iter().map(|it| cx.span(*it)).collect();
    let parts = replacements.into_iter().map(|(at, text)| (cx.span(at), text)).collect();
    cx.lint_after(
        LINT,
        pat.syntax(),
        &body,
        span,
        "this pattern creates a reference to a reference",
    )
    .multipart_suggestion("try", parts, Applicability::MachineApplicable);
}

/// The body and, for a closure or a function, its parameters: where its bindings are.
fn body_and_params(body: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    let params = body.parent().and_then(|owner| {
        ast::ClosureExpr::cast(owner.clone())
            .and_then(|it| it.param_list())
            .or_else(|| ast::Fn::cast(owner).and_then(|it| it.param_list()))
    });
    params.map(|it| it.syntax().clone()).into_iter().chain([body.clone()])
}

/// The expression rustc's HIR has as `node`'s parent, past what is not an expression there.
fn parent_expr(node: SyntaxNode) -> Option<ast::Expr> {
    let node = if ast::ArgList::can_cast(node.kind()) { node.parent()? } else { node };
    ast::Expr::cast(node)
}

/// Whether rustc's `ExprPrecedence` of `expr` is `Unambiguous`: an operand needs no
/// parentheses around it.
fn unambiguous(expr: &ast::Expr) -> bool {
    !matches!(
        expr,
        ast::Expr::BinExpr(_)
            | ast::Expr::CastExpr(_)
            | ast::Expr::PrefixExpr(_)
            | ast::Expr::RefExpr(_)
            | ast::Expr::ReturnExpr(_)
            | ast::Expr::BreakExpr(_)
            | ast::Expr::ContinueExpr(_)
            | ast::Expr::YieldExpr(_)
            | ast::Expr::BecomeExpr(_)
            | ast::Expr::ClosureExpr(_)
            | ast::Expr::LetExpr(_)
    )
}

fn past_parens(node: &SyntaxNode) -> SyntaxNode {
    let mut node = node.clone();
    while let Some(parent) = node.parent()
        && ast::ParenExpr::can_cast(parent.kind())
    {
        node = parent;
    }
    node
}
