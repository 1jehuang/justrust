//! `bool_assert_comparison`: `assert_eq!` or `assert_ne!` (or their `debug_` forms) with a
//! `true` or `false` on one side, which `assert!` says plainer.
//!
//! Follows clippy's `clippy_lints/src/bool_assert_comparison.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use syntax::{AstNode, TextRange, TextSize, ast};

use crate::{check::Cx, diag::Applicability, sugg::Sugg};

const LINT: &str = "bool_assert_comparison";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, call: &ast::MacroCall) {
    if !cx.on(LINT) {
        return;
    }
    let Some(name) = cx.std_macro(call) else { return };
    let eq = match name.as_str() {
        "assert_eq" | "debug_assert_eq" => true,
        "assert_ne" | "debug_assert_ne" => false,
        _ => return,
    };
    let args = cx.macro_args(call);
    let [(a_range, a), (b_range, b), ..] = &args[..] else { return };
    let literal = |range: &TextRange| match cx.snippet(*range).as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    let (lit_range, value, other, other_range) = match (literal(a_range), literal(b_range)) {
        (Some(value), None) => {
            (TextRange::new(a_range.start(), b_range.start()), value, b, b_range)
        }
        (None, Some(value)) => (TextRange::new(a_range.end(), b_range.end()), value, a, a_range),
        _ => return,
    };
    let Some(ty) = cx.ty(other) else { return };
    let db = cx.db();
    let Some(not) = cx.lang_trait(hir::LangItem::Not) else { return };
    let not_to_bool = ty.is_bool()
        || (ty.impls_trait(db, not, &[])
            && cx.assoc_ty(&ty, not, "Output").is_some_and(|it| it.is_bool()));
    if !not_to_bool || !ty.is_copy(db) {
        return;
    }
    let Some(path) = call.path() else { return };
    let Some(tt) = call.token_tree() else { return };
    let (Some(start), Some(end)) = (cx.range(path.syntax()), cx.range(tt.syntax())) else { return };
    let whole = TextRange::new(start.start(), end.end());
    let text = cx.snippet(whole);
    let bang = text.find('!').unwrap_or(text.len());
    let name_len = text[..bang].trim_end().len();
    let name_range = TextRange::at(whole.start(), TextSize::from(name_len as u32));
    let plain = &name[..name.len() - 3];

    let snippet = |node: &syntax::SyntaxNode| cx.source_of(node);
    let same_type = |prefix: &ast::PrefixExpr| {
        let inner = prefix.expr();
        let outer = cx.ty(&ast::Expr::PrefixExpr(prefix.clone()));
        inner.and_then(|it| cx.ty(&it)).zip(outer).is_some_and(|(a, b)| a == b)
    };
    let sugg = Sugg::of(other, &snippet, &same_type);
    let sugg = if value ^ eq {
        !sugg.maybe_paren()
    } else if ty.is_bool() {
        sugg
    } else {
        !!sugg.maybe_paren()
    };
    let parts = vec![
        (cx.span(name_range), plain.to_owned()),
        (cx.span(lit_range), String::new()),
        (cx.span(*other_range), sugg.to_string()),
    ];
    let span = cx.span(whole);
    cx.lint(LINT, call.syntax(), span, format!("used `{name}!` with a literal bool"))
        .multipart_suggestion(
            format!("replace it with `{plain}!(..)`"),
            parts,
            Applicability::MachineApplicable,
        );
}
