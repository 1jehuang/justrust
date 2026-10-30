//! `nonstandard_macro_braces`: a macro called with other brackets than the ones it is usually
//! called with, as `format! { .. }` or `vec!(..)`; `standard-macro-braces` in `clippy.toml`
//! names more macros and their brackets.
//!
//! Follows clippy's `clippy_lints/src/nonstandard_macro_braces.rs` at rust-lang/rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use rustc_hash::FxHashMap;
use syntax::{AstNode, NodeOrToken, SyntaxKind, SyntaxNode, TextRange, ast};

use crate::{check::Cx, diag::Applicability};

const LINT: &str = "nonstandard_macro_braces";

/// The macros clippy knows the brackets of, and the ones `standard-macro-braces` adds.
fn braces(cx: &Cx<'_, '_, '_>) -> FxHashMap<String, (char, char)> {
    let mut braces: FxHashMap<String, (char, char)> = [
        ("assert_matches", ('(', ')')),
        ("cfg_select", ('{', '}')),
        ("debug_assert_matches", ('(', ')')),
        ("format", ('(', ')')),
        ("format_args", ('(', ')')),
        ("eprint", ('(', ')')),
        ("eprintln", ('(', ')')),
        ("print", ('(', ')')),
        ("println", ('(', ')')),
        ("write", ('(', ')')),
        ("writeln", ('(', ')')),
        ("vec", ('[', ']')),
        ("matches", ('(', ')')),
    ]
    .into_iter()
    .map(|(name, braces)| (name.to_owned(), braces))
    .collect();
    let configured = cx.conf().value("standard-macro-braces").and_then(|it| it.as_array());
    for matcher in configured.into_iter().flatten() {
        let name = matcher.get("name").and_then(|it| it.as_str());
        let brace = matcher.get("brace").and_then(|it| it.as_str());
        let pair = match brace {
            Some("(" | ")" | "()") => ('(', ')'),
            Some("[" | "]" | "[]") => ('[', ']'),
            Some("{" | "}" | "{}") => ('{', '}'),
            _ => continue,
        };
        if let Some(name) = name {
            braces.insert(name.to_owned(), pair);
        }
    }
    braces
}

pub(super) fn check_call(cx: &mut Cx<'_, '_, '_>, call: &ast::MacroCall) {
    // clippy's `unnested_or_local` is meant to let a call inside a local macro through, but
    // stock clippy lints no call inside another's arguments, local or not (`vec!(..)` in
    // `assert!(..)`, `println!(..)` or a `macro_rules!` of the crate); what an attribute
    // macro such as `#[test]` hands back is linted.
    if !cx.on(LINT) || cx.in_bang_call() {
        return;
    }
    let (Some(path), Some(tt)) = (call.path(), call.token_tree()) else { return };
    let Some(segment) = path.segment() else { return };
    let name = segment.syntax().text().to_string();
    let Some(&braces) = braces(cx).get(&name) else { return };
    let Some(open) = tt.l_paren_token().or(tt.l_brack_token()).or(tt.l_curly_token()) else {
        return;
    };
    let open = open.text().chars().next().unwrap_or_default();
    if open == braces.0 {
        return;
    }
    let (Some(start), Some(end)) = (cx.range(path.syntax()), cx.range(tt.syntax())) else { return };
    let span = TextRange::new(start.start(), end.end());
    // A statement written with braces and no semicolon needs one once it has other brackets.
    let statement = call.syntax().parent().and_then(ast::MacroExpr::cast).and_then(|it| {
        let parent = it.syntax().parent()?;
        let bare = ast::ExprStmt::cast(parent.clone())
            .is_some_and(|it| it.semicolon_token().is_none())
            || ast::StmtList::cast(parent).is_some();
        Some(bare)
    });
    let add_semi = open == '{' && statement == Some(true);
    emit(cx, call.syntax(), span, braces, add_semi);
}

pub(super) fn check_rules(cx: &mut Cx<'_, '_, '_>, rules: &ast::MacroRules) {
    if !cx.on(LINT) {
        return;
    }
    let Some(body) = rules.token_tree() else { return };
    let braces = braces(cx);
    let mut found = Vec::new();
    in_tokens(body.syntax(), &braces, &mut found);
    for (span, pair) in found {
        emit(cx, rules.syntax(), span, pair, false);
    }
}

/// The calls `ident ! (..)` in a macro's body whose brackets are not their macro's.
fn in_tokens(
    tree: &SyntaxNode,
    braces: &FxHashMap<String, (char, char)>,
    found: &mut Vec<(TextRange, (char, char))>,
) {
    let children: Vec<_> =
        tree.children_with_tokens().filter(|it| !it.kind().is_trivia()).collect();
    let inner = children.get(1..children.len().saturating_sub(1)).unwrap_or_default();
    for (at, child) in inner.iter().enumerate() {
        match child {
            NodeOrToken::Node(node) if node.kind() == SyntaxKind::TOKEN_TREE => {
                in_tokens(node, braces, found);
            }
            NodeOrToken::Token(ident) if ident.kind() == SyntaxKind::IDENT => {
                let bang = inner.get(at + 1).is_some_and(|it| it.kind() == SyntaxKind::BANG);
                let Some(NodeOrToken::Node(args)) = inner.get(at + 2) else { continue };
                if !bang || args.kind() != SyntaxKind::TOKEN_TREE {
                    continue;
                }
                let Some(&pair) = braces.get(ident.text()) else { continue };
                let open = args.first_token().and_then(|it| it.text().chars().next());
                if matches!(open, Some('(' | '[' | '{')) && open != Some(pair.0) {
                    let span = TextRange::new(ident.text_range().start(), args.text_range().end());
                    found.push((span, pair));
                }
            }
            _ => {}
        }
    }
}

fn emit(
    cx: &mut Cx<'_, '_, '_>,
    at: &SyntaxNode,
    span: TextRange,
    (open, close): (char, char),
    add_semi: bool,
) {
    let snippet = cx.snippet(span);
    let Some((name, args)) = snippet.split_once('!') else { return };
    let mut args = args.trim().to_owned();
    args.pop();
    if !args.is_empty() {
        args.remove(0);
    }
    let semi = if add_semi { ";" } else { "" };
    let sugg = format!("{name}!{open}{args}{close}{semi}");
    let span = cx.span(span);
    let message = format!("use of irregular braces for `{name}!` macro");
    cx.lint(LINT, at, span.clone(), message).suggestion(
        span,
        "consider writing",
        sugg,
        Applicability::MachineApplicable,
    );
}
