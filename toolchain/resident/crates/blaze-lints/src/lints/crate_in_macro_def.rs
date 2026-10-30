//! `crate_in_macro_def`: `crate::` in an exported `macro_rules!`, which names the crate the
//! macro is called from rather than its own.
//!
//! Follows clippy's `clippy_lints/src/crate_in_macro_def.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use syntax::{AstNode, NodeOrToken, SyntaxKind, SyntaxNode, TextRange, ast, ast::HasAttrs as _};

use crate::{check::Cx, diag::Applicability};

const LINT: &str = "crate_in_macro_def";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, rules: &ast::MacroRules) {
    if !cx.on(LINT) || !cx.in_file() {
        return;
    }
    let exported = rules.attrs().any(|attr| {
        attr.path().is_some_and(|path| {
            path.qualifier().is_none() && path.syntax().text() == "macro_export"
        })
    });
    let Some(body) = rules.token_tree() else { return };
    if !exported {
        return;
    }
    let Some(range) = unhygienic_crate(body.syntax()) else { return };
    let span = cx.span(range);
    cx.lint(LINT, rules.syntax(), span.clone(), "`crate` references the macro call's crate")
        .suggestion(
            span,
            "to reference the macro definition's crate, use",
            "$crate".to_owned(),
            Applicability::MachineApplicable,
        );
}

/// clippy's `contains_unhygienic_crate_reference`: the first `crate` followed by `::` and not
/// by `$`, in the trees of `tt` in order.
fn unhygienic_crate(tt: &SyntaxNode) -> Option<TextRange> {
    let trees: Vec<_> = tt.children_with_tokens().filter(|it| !it.kind().is_trivia()).collect();
    let inner = trees.get(1..trees.len().saturating_sub(1)).unwrap_or_default();
    let mut prev_is_dollar = false;
    for (at, tree) in inner.iter().enumerate() {
        if !prev_is_dollar && tree.kind() == SyntaxKind::CRATE_KW && path_sep_at(inner, at + 1) {
            return Some(tree.text_range());
        }
        if let NodeOrToken::Node(node) = tree
            && let Some(found) = unhygienic_crate(node)
        {
            return Some(found);
        }
        prev_is_dollar = tree.kind() == SyntaxKind::DOLLAR;
    }
    None
}

/// Whether `::` starts at `trees[at]`, as one token or as two joined colons.
fn path_sep_at(trees: &[NodeOrToken<SyntaxNode, syntax::SyntaxToken>], at: usize) -> bool {
    match (trees.get(at), trees.get(at + 1)) {
        (Some(first), _) if first.kind() == SyntaxKind::COLON2 => true,
        (Some(NodeOrToken::Token(first)), Some(NodeOrToken::Token(second))) => {
            first.kind() == SyntaxKind::COLON
                && second.kind() == SyntaxKind::COLON
                && first.text_range().end() == second.text_range().start()
        }
        _ => false,
    }
}
