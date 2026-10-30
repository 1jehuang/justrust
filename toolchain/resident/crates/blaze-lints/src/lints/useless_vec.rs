//! `useless_vec`: `vec![..]` where an array, or a borrow of one, would do as well.
//!
//! Follows clippy's `clippy_lints/src/useless_vec.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0). Not ported: a length that is
//! a constant's name rather than a literal (clippy evaluates it), and a `vec![..]` a macro
//! hands on twice, which clippy leaves alone when one of its uses needs a `Vec`; here each is
//! judged alone.

use syntax::{
    AstNode, NodeOrToken, SyntaxKind, SyntaxNode,
    ast::{self, HasArgList as _, HasAttrs as _},
};

use super::utils;
use crate::{check::Cx, diag::Applicability, msrvs};

const LINT: &str = "useless_vec";

/// clippy's `VEC_METHODS_SHADOWING_SLICE_METHODS`: methods of `Vec` that slices have too.
const SLICE_METHODS: [&str; 3] = ["as_ptr", "is_empty", "len"];

/// What clippy suggests for a `vec![..]`.
#[derive(Clone, Copy)]
enum Suggested {
    /// `&[..]` or `&mut [..]`, for `&vec![..]` or `&mut vec![..]`.
    Slice { mutable: bool },
    /// `[..]`.
    Array,
}

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, call: &ast::MacroCall) {
    if !cx.on(LINT) || cx.std_macro(call).as_deref() != Some("vec") {
        return;
    }
    let Some(call_expr) = call.syntax().parent().and_then(ast::MacroExpr::cast) else { return };
    let call_expr = ast::Expr::MacroExpr(call_expr);
    // clippy looks at `&vec![..]` whole: the `vec![..]` in it is a `Vec` where it is.
    let (expr, suggested) =
        match past_parens(call_expr.syntax()).parent().and_then(ast::RefExpr::cast) {
            Some(borrow) if borrow.raw_token().is_none() => {
                if !cx.is_written(borrow.syntax()) {
                    return;
                }
                let mutable = borrow.mut_token().is_some();
                (ast::Expr::RefExpr(borrow), Suggested::Slice { mutable })
            }
            Some(_) => return,
            None => (call_expr, Suggested::Array),
        };
    if cx.conf().bool("allow-useless-vec-in-tests", false) && in_test(expr.syntax()) {
        return;
    }
    if requires_vec(cx, &expr, matches!(suggested, Suggested::Array)) {
        return;
    }
    let Some(tt) = call.token_tree() else { return };
    let args = cx.macro_args(call);
    let limit = u64::try_from(cx.conf().int("too-large-for-stack", 200)).unwrap_or(0);
    let (inner, len) = if repeats(tt.syntax()) {
        let [(elem_at, elem), (len_at, len)] = &args[..] else { return };
        let copy = cx.ty(elem).is_some_and(|it| it.is_copy(cx.db()));
        let Some(length) = int_literal(len) else { return };
        if !copy || size_of(cx, elem).checked_mul(length).is_none_or(|it| it > limit) {
            return;
        }
        (cx.snippet(*elem_at), Some(cx.snippet(*len_at)))
    } else {
        if args.len() != elements(tt.syntax()) {
            return;
        }
        let length = args.len() as u64;
        if size_of(cx, &expr).checked_mul(length).is_none_or(|it| it > limit) {
            return;
        }
        let inner = match (args.first(), args.last()) {
            (Some((first, _)), Some((last, _))) => cx.snippet(first.cover(*last)),
            _ => String::new(),
        };
        (inner, None)
    };
    let len = len.map_or_else(String::new, |it| format!("; {it}"));
    let (snippet, desc) = match suggested {
        Suggested::Slice { mutable: true } => (format!("&mut [{inner}{len}]"), "a slice"),
        Suggested::Slice { mutable: false } => (format!("&[{inner}{len}]"), "a slice"),
        Suggested::Array => (format!("[{inner}{len}]"), "an array"),
    };
    let Some(range) = cx.range(expr.syntax()) else { return };
    // A suggestion that would drop a comment written in the call is not applied by itself.
    let source = cx.sema().parse(cx.file);
    let applicability = if utils::contains_comment(source.syntax(), range) {
        Applicability::Unspecified
    } else {
        Applicability::MachineApplicable
    };
    let span = cx.span(range);
    cx.lint(LINT, expr.syntax(), span.clone(), "useless use of `vec!`").suggestion(
        span,
        format!("you can use {desc} directly"),
        snippet,
        applicability,
    );
}

/// clippy's `expr_usage_requires_vec`: whether where `expr` is used needs it a `Vec`. `bare`
/// is whether `expr` is the `vec![..]` itself, not a borrow of it.
fn requires_vec(cx: &Cx<'_, '_, '_>, expr: &ast::Expr, bare: bool) -> bool {
    let used = past_parens(expr.syntax());
    let parent = used.parent();
    if let Some(stmt) = parent.clone().and_then(ast::LetStmt::cast)
        && stmt.initializer().is_some_and(|it| it.syntax() == &used)
    {
        match (stmt.ty(), stmt.pat()) {
            (None, Some(ast::Pat::IdentPat(pat))) => {
                let Some(local) = cx.sema().to_def(&pat) else { return true };
                return !only_slice_uses(cx, local, &stmt);
            }
            (Some(_), _) if bare => return true,
            _ => {}
        }
    }
    if let Some(for_expr) = parent.and_then(ast::ForExpr::cast)
        && for_expr.iterable().is_some_and(|it| it.syntax() == &used)
        && cx.conf().meets(msrvs::ARRAY_INTO_ITERATOR)
    {
        return false;
    }
    !adjusts_to_slice(cx, expr)
}

/// Whether each use of `local` after `stmt` is one an array would serve: borrowed as a slice,
/// indexed, or the receiver of a method slices have too.
fn only_slice_uses(cx: &Cx<'_, '_, '_>, local: hir::Local<'_>, stmt: &ast::LetStmt) -> bool {
    let Some(block) = stmt.syntax().parent() else { return false };
    let after = stmt.syntax().text_range().end();
    let mut ok = true;
    utils::each_path(cx, &block, Some(after), &mut |path| {
        if !ok || utils::local_of(cx, path) != Some(local) {
            return;
        }
        ok = slice_use(cx, path);
    });
    ok
}

fn slice_use(cx: &Cx<'_, '_, '_>, path: &ast::PathExpr) -> bool {
    let expr = ast::Expr::PathExpr(path.clone());
    if adjusts_to_slice(cx, &expr) {
        return true;
    }
    let used = past_parens(path.syntax());
    let Some(parent) = used.parent() else { return false };
    if ast::IndexExpr::can_cast(parent.kind()) {
        return true;
    }
    let Some(call) = ast::MethodCallExpr::cast(parent) else { return false };
    let no_args = call.arg_list().is_some_and(|it| it.args().next().is_none());
    let name = call.name_ref().map(|it| it.text().to_owned());
    no_args
        && call.receiver().is_some_and(|it| it.syntax() == &used)
        && name.is_some_and(|it| SLICE_METHODS.contains(&it.as_str()))
}

/// clippy's `adjusts_to_slice`: whether `expr`, as the compiler adjusts it, is a borrowed slice.
fn adjusts_to_slice(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    cx.adjusted_ty(expr).and_then(|it| it.as_reference()).is_some_and(|(it, _)| it.is_slice())
}

/// The size of `expr`'s type as the compiler adjusts it, or 0 when its layout is not known.
fn size_of(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> u64 {
    cx.adjusted_ty(expr).and_then(|it| it.layout(cx.db()).ok()).map_or(0, |it| it.size())
}

/// Whether the arguments of a `vec!` are `elem; len`.
fn repeats(tt: &SyntaxNode) -> bool {
    tt.children_with_tokens().any(|it| it.kind() == SyntaxKind::SEMICOLON)
}

/// How many elements the arguments of a `vec!` list.
fn elements(tt: &SyntaxNode) -> usize {
    let tokens: Vec<_> = tt.children_with_tokens().filter(|it| !it.kind().is_trivia()).collect();
    let inner = tokens.get(1..tokens.len().saturating_sub(1)).unwrap_or_default();
    if inner.is_empty() {
        return 0;
    }
    let commas = inner
        .iter()
        .filter(|it| matches!(it, NodeOrToken::Token(it) if it.kind() == SyntaxKind::COMMA));
    let trailing = inner.last().is_some_and(|it| it.kind() == SyntaxKind::COMMA);
    commas.count() + 1 - usize::from(trailing)
}

fn int_literal(expr: &ast::Expr) -> Option<u64> {
    let ast::Expr::Literal(literal) = expr else { return None };
    let ast::LiteralKind::IntNumber(number) = literal.kind() else { return None };
    u64::try_from(number.value().ok()?).ok()
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

/// clippy's `is_in_test`: in a `#[test]` function or a `#[cfg(test)]` item.
fn in_test(node: &SyntaxNode) -> bool {
    node.ancestors().filter_map(ast::AnyHasAttrs::cast).any(|item| {
        item.attrs().any(|attr| {
            let text = attr.syntax().text().to_string().replace(char::is_whitespace, "");
            text == "#[test]" || text.starts_with("#[cfg(test")
        })
    })
}
