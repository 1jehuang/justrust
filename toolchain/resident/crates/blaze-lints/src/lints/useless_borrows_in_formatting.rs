//! `useless_borrows_in_formatting`: a format macro's argument borrowed when what it borrows
//! formats the same.
//!
//! Follows clippy's `clippy_lints/src/format_args.rs` (`check_useless_borrows_in_formatting`)
//! at rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use rustc_hash::FxHashSet;
use syntax::{AstNode, TextRange, ast};

use super::format::{self, Arg, Trait};
use crate::{check::Cx, diag::Applicability};

const LINT: &str = "useless_borrows_in_formatting";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, call: &ast::MacroCall) {
    if !cx.on(LINT) {
        return;
    }
    let Some(format) = format::of(cx, call) else { return };
    let (Some(display), Some(debug), Some(sized)) = (
        cx.std_trait(&["core", "fmt", "Display"]),
        cx.std_trait(&["core", "fmt", "Debug"]),
        cx.lang_trait(hir::LangItem::Sized),
    ) else {
        return;
    };
    // The same argument taken twice is one diagnostic, as rustc emits a diagnostic once.
    let mut seen: FxHashSet<TextRange> = FxHashSet::default();
    for placeholder in &format.placeholders {
        let fmt_trait = match placeholder.format_trait {
            Trait::Display => display,
            Trait::Debug => debug,
            Trait::Other => continue,
        };
        for arg in
            [Some(&placeholder.arg), placeholder.width.as_ref(), placeholder.precision.as_ref()]
        {
            let Some(Arg::Index(index)) = arg else { continue };
            let Some(expr) = format.args.get(*index) else { continue };
            let Some((range, peeled, message)) = peel(cx, expr, fmt_trait, sized) else { continue };
            if !seen.insert(range) {
                continue;
            }
            let span = cx.span(range);
            cx.lint(
                LINT,
                call.syntax(),
                span.clone(),
                format!("redundant reference in `{}!` argument", format.name),
            )
            .suggestion(span, message, peeled, Applicability::MachineApplicable);
        }
    }
}

/// Where `arg` is written, what it is with the borrows that change nothing peeled, and the
/// help saying which.
fn peel(
    cx: &Cx<'_, '_, '_>,
    arg: &ast::Expr,
    fmt_trait: hir::Trait,
    sized: hir::Trait,
) -> Option<(TextRange, String, &'static str)> {
    let range = cx.original_range(arg.syntax())?;
    let db = cx.db();
    let mut only_mutable = true;
    let mut has_mutable = false;
    let mut peeled = arg.clone();
    // rustc's HIR has no parentheses: `&(x)` is a borrow of `x` written `(x)`.
    while let ast::Expr::RefExpr(borrow) = strip_parens(&peeled)
        && borrow.raw_token().is_none()
        && let Some(inner) = borrow.expr()
        && let Some(ty) = cx.ty(&strip_parens(&inner))
        && ty.impls_trait(db, sized, &[])
        && ty.impls_trait(db, fmt_trait, &[])
    {
        let mutable = borrow.mut_token().is_some();
        only_mutable &= mutable;
        has_mutable |= mutable;
        peeled = inner;
    }
    if peeled.syntax() == arg.syntax() {
        return None;
    }
    let peeled = cx.snippet(cx.original_range(peeled.syntax())?);
    let message = if only_mutable {
        "remove the redundant `&mut`"
    } else if has_mutable {
        "remove the redundant `&`/`&mut`"
    } else {
        "remove the redundant `&`"
    };
    Some((range, peeled, message))
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
