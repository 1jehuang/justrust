//! `needless_borrows_for_generic_args`: `&x` passed where the parameter is a type parameter
//! whose bounds `x` meets itself, so the borrow is only dereferenced again.
//!
//! Follows clippy's `clippy_lints/src/needless_borrows_for_generic_args.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0). Where clippy asks MIR whether
//! a `&mut` referent is used once, this does not lint; bounds on other parameters and on
//! projections, which clippy instantiates with the referent's type too, are not checked, nor
//! are bounds whose arguments are the callee's parameters.

use hir::{GenericParam, PathResolution};
use ide_db::famous_defs::FamousDefs;
use syntax::{
    AstNode, SyntaxNode,
    ast::{self, HasGenericArgs as _},
};

use crate::{check::Cx, diag::Applicability, msrvs};

const LINT: &str = "needless_borrows_for_generic_args";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::Expr) {
    if !cx.on(LINT) || cx.expansion.is_some() {
        return;
    }
    let ast::Expr::RefExpr(_) = expr else { return };
    let Some((function, index)) = callee(cx, expr.syntax()) else { return };
    let count = needless_borrow_count(cx, function, index, expr);
    if count == 0 {
        return;
    }
    let mut peeled = expr.clone();
    for _ in 0..count {
        let ast::Expr::RefExpr(reference) = &peeled else { break };
        let Some(inner) = reference.expr() else { return };
        peeled = inner;
    }
    let (Some(range), Some(inner)) = (cx.range(expr.syntax()), cx.range(peeled.syntax())) else {
        return;
    };
    let snippet = cx.snippet(inner);
    let span = cx.span(range);
    cx.lint(
        LINT,
        expr.syntax(),
        span.clone(),
        "the borrowed expression implements the required traits",
    )
    .suggestion(span, "change this to", snippet, Applicability::MachineApplicable);
}

/// The function `node` is an argument of, and which of its parameters it is: clippy's
/// `ExprUseNode::FnArg` of a path with no generic arguments, or `MethodArg` other than the
/// receiver of a call with none.
fn callee(cx: &Cx<'_, '_, '_>, node: &SyntaxNode) -> Option<(hir::Function, usize)> {
    let mut arg = node.clone();
    while let Some(paren) = arg.parent().and_then(ast::ParenExpr::cast) {
        arg = paren.syntax().clone();
    }
    let list = ast::ArgList::cast(arg.parent()?)?;
    let index = list.args().position(|it| *it.syntax() == arg)?;
    let call = list.syntax().parent()?;
    if let Some(call) = ast::MethodCallExpr::cast(call.clone()) {
        if call.generic_arg_list().is_some() {
            return None;
        }
        let function = cx.sema().resolve_method_call(&call)?;
        return Some((function, index + 1));
    }
    let call = ast::CallExpr::cast(call)?;
    let ast::Expr::PathExpr(path) = call.expr()? else { return None };
    let path = path.path()?;
    if path.segment()?.generic_arg_list().is_some() {
        return None;
    }
    match cx.sema().resolve_path(&path)? {
        PathResolution::Def(hir::ModuleDef::Function(function)) => Some((function, index)),
        _ => None,
    }
}

fn needless_borrow_count(
    cx: &Cx<'_, '_, '_>,
    function: hir::Function,
    index: usize,
    expr: &ast::Expr,
) -> usize {
    let db = cx.db();
    let params = function.assoc_fn_params(db);
    let Some(param) = params.get(index) else { return 0 };
    let Some(type_param) = param.ty().as_type_param(db) else { return 0 };
    let krate = cx.krate();
    let lang = |item| hir::Trait::lang(db, krate, item);
    let trivial = [
        lang(hir::LangItem::Destruct),
        lang(hir::LangItem::Sized),
        lang(hir::LangItem::MetaSized),
        cx.std_trait(&["core", "any", "Any"]),
    ];
    let bounds = type_param.trait_refs(db);
    if bounds.iter().all(|it| trivial.contains(&Some(it.trait_()))) {
        return 0;
    }
    let ref_mut_self = bounds.iter().any(|it| has_ref_mut_self_method(db, it.trait_()));
    // `replace_types`: the parameter is in the signature only where the argument is.
    let generic = GenericParam::TypeParam(type_param);
    let elsewhere = params
        .iter()
        .enumerate()
        .any(|(at, it)| at != index && it.ty().generic_params(db).contains(&generic))
        || function.ret_type(db).generic_params(db).contains(&generic);
    if elsewhere {
        return 0;
    }
    let into_iter = FamousDefs(cx.sema(), krate).core_iter_IntoIterator();
    let drop = lang(hir::LangItem::Drop);
    let holds = |referent: &ast::Expr| -> bool {
        let inner = peel_parens(referent);
        if let ast::Expr::FieldExpr(field) = &inner
            && let Some(base) = field.expr().and_then(|it| cx.ty(&it))
            && drop.is_some_and(|it| base.impls_trait(db, it, &[]))
        {
            return false;
        }
        let Some(ty) = cx.ty(referent) else { return false };
        if !(ty.is_copy(db) || is_call(cx, &inner)) {
            return false;
        }
        if ref_mut_self && !ty.is_mutable_reference() {
            return false;
        }
        bounds.iter().all(|bound| {
            let trait_ = bound.trait_();
            if Some(trait_) == into_iter
                && ty.is_array()
                && !cx.conf().meets(msrvs::ARRAY_INTO_ITERATOR)
            {
                return false;
            }
            let mut args = Vec::new();
            for at in 1.. {
                let Some(arg) = bound.get_type_argument(at) else { break };
                let Some(arg) = arg.without_params(db) else { return false };
                args.push(arg);
            }
            ty.impls_trait(db, trait_, &args)
        })
    };
    let mut count = 0;
    let mut at = expr.clone();
    while let ast::Expr::RefExpr(reference) = &at {
        let Some(referent) = reference.expr() else { break };
        if !holds(&referent) {
            break;
        }
        at = referent;
        count += 1;
    }
    count
}

fn has_ref_mut_self_method(db: &dyn hir::db::HirDatabase, trait_: hir::Trait) -> bool {
    trait_.items(db).into_iter().any(|it| match it {
        hir::AssocItem::Function(function) => {
            function.self_param(db).is_some_and(|it| it.access(db) == hir::Access::Exclusive)
        }
        _ => false,
    })
}

fn peel_parens(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    while let ast::Expr::ParenExpr(paren) = &expr {
        let Some(inner) = paren.expr() else { break };
        expr = inner;
    }
    expr
}

/// Whether `expr` is a call, or a macro call that expands to one, as `format!` does.
fn is_call(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::CallExpr(_) | ast::Expr::MethodCallExpr(_) => true,
        ast::Expr::MacroExpr(mac) => {
            let Some(call) = mac.macro_call() else { return false };
            let Some(expanded) = cx.sema().expand_macro_call(&call) else { return false };
            let root = expanded.value;
            let expr =
                ast::Expr::cast(root.clone()).or_else(|| root.children().find_map(ast::Expr::cast));
            expr.is_some_and(|it| is_call(cx, &peel_parens(&it)))
        }
        _ => false,
    }
}
