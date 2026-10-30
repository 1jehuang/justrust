//! The lints, each a port of clippy's (its file says which), and what runs them on a node.

mod bool_assert_comparison;
mod byte_char_slices;
mod clone_on_copy;
mod collapsible_if;
mod crate_in_macro_def;
mod empty_enums;
mod format;
mod get_first;
mod io_other_error;
mod map_clone;
mod map_unwrap_or;
mod match_like_matches_macro;
mod needless_bool;
mod needless_borrow;
mod needless_borrows_for_generic_args;
mod needless_return;
mod new_without_default;
mod nonstandard_macro_braces;
mod redundant_static_lifetimes;
mod single_char_add_str;
mod unnecessary_map_or;
mod upper_case_acronyms;
mod useless_borrows_in_formatting;
mod useless_format;
mod useless_vec;
mod utils;

use rustc_hash::FxHashSet;
use syntax::{AstNode, SyntaxNode, ast};

use crate::{
    Level,
    check::Cx,
    level::{Levels, Lint},
    registry,
};

/// When rustc runs a lint's pass, which orders what it emits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Over the crate root's file before macros expand, and again after, as an early pass.
    PreExpansion,
    /// Over the expanded crate, before the late passes.
    Early,
    /// Over the crate's HIR.
    Late,
    /// After the late pass, from what it gathered (clippy's `check_crate_post`), in the order of
    /// where each is.
    AfterLate,
}

/// The lints implemented, and the pass clippy runs each in.
const IMPLEMENTED: &[(&str, Pass)] = &[
    ("bool_assert_comparison", Pass::Late),
    ("byte_char_slices", Pass::Late),
    ("clone_on_copy", Pass::Late),
    ("collapsible_if", Pass::Late),
    ("crate_in_macro_def", Pass::Early),
    ("empty_enums", Pass::Late),
    ("get_first", Pass::Late),
    ("io_other_error", Pass::Late),
    ("map_clone", Pass::Late),
    ("map_unwrap_or", Pass::Late),
    ("match_like_matches_macro", Pass::Late),
    ("needless_bool", Pass::Late),
    ("needless_borrow", Pass::Late),
    ("needless_borrows_for_generic_args", Pass::Late),
    ("needless_return", Pass::Late),
    ("new_without_default", Pass::Late),
    ("nonstandard_macro_braces", Pass::PreExpansion),
    ("redundant_static_lifetimes", Pass::Early),
    ("single_char_add_str", Pass::Late),
    ("unnecessary_map_or", Pass::Late),
    ("upper_case_acronyms", Pass::Late),
    ("useless_borrows_in_formatting", Pass::Late),
    ("useless_format", Pass::Late),
    ("useless_vec", Pass::AfterLate),
];

pub(crate) fn implemented(lint: &str) -> bool {
    IMPLEMENTED.iter().any(|(it, _)| *it == lint)
}

/// Which of rustc's passes over a crate emits `lint`, in their order, for code in the root
/// file or not.
pub(crate) fn phase(lint: &str, in_root_file: bool) -> u8 {
    match IMPLEMENTED.iter().find(|(it, _)| *it == lint).map(|(_, pass)| *pass) {
        Some(Pass::PreExpansion) if in_root_file => 0,
        Some(Pass::PreExpansion | Pass::Early) => 1,
        Some(Pass::AfterLate) => 3,
        _ => 2,
    }
}

/// Every lint implemented, by name.
pub fn names() -> impl Iterator<Item = &'static str> {
    IMPLEMENTED.iter().map(|(it, _)| *it)
}

/// The lints that may be at a level other than `allow` somewhere in a crate whose files have
/// `texts`: those the command line or their group does not allow, and those the crate names
/// in an attribute.
pub(crate) fn on(levels: &Levels, texts: &[&str]) -> FxHashSet<&'static str> {
    let mut on = FxHashSet::default();
    for (name, _) in IMPLEMENTED {
        let Some(info) = registry::lint(name) else { continue };
        let named = |text: &str| {
            text.contains(name) || text.contains(info.group.name()) || text.contains("clippy::all")
        };
        if levels.of(Lint::Clippy(info), &[]).level != Level::Allow
            || texts.iter().any(|it| named(it))
        {
            on.insert(*name);
        }
    }
    on
}

/// Whether the walk goes into what derives expand to: only lints that run there need it.
pub(crate) fn in_derives(cx: &Cx<'_, '_, '_>) -> bool {
    cx.on("empty_enums")
}

/// Whether the walk goes into what macro calls expand to, for the code handed to them: only
/// lints that run on code a macro can be handed need it.
pub(crate) fn in_calls(cx: &Cx<'_, '_, '_>) -> bool {
    IMPLEMENTED.iter().any(|(name, _)| *name != "empty_enums" && cx.on(name))
}

/// Runs the lints of `node`'s kind on it.
pub(crate) fn check_node(cx: &mut Cx<'_, '_, '_>, node: &SyntaxNode) {
    if cx.expansion.is_some() {
        if let Some(it) = ast::Enum::cast(node.clone()) {
            empty_enums::check(cx, &it);
        }
        return;
    }
    if let Some(it) = ast::Item::cast(node.clone()) {
        upper_case_acronyms::check(cx, &it);
    }
    if let Some(it) = ast::Enum::cast(node.clone()) {
        empty_enums::check(cx, &it);
    } else if let Some(it) = ast::Impl::cast(node.clone()) {
        new_without_default::check(cx, &it);
    } else if let Some(it) = ast::Fn::cast(node.clone()) {
        needless_return::check_fn(cx, &it);
    } else if let Some(it) = ast::Const::cast(node.clone()) {
        redundant_static_lifetimes::check_const(cx, &it);
    } else if let Some(it) = ast::Static::cast(node.clone()) {
        redundant_static_lifetimes::check_static(cx, &it);
    } else if let Some(it) = ast::MacroCall::cast(node.clone()) {
        nonstandard_macro_braces::check_call(cx, &it);
        bool_assert_comparison::check(cx, &it);
        useless_borrows_in_formatting::check(cx, &it);
        useless_format::check(cx, &it);
        useless_vec::check(cx, &it);
    } else if let Some(it) = ast::MacroRules::cast(node.clone()) {
        nonstandard_macro_braces::check_rules(cx, &it);
        crate_in_macro_def::check(cx, &it);
    } else if let Some(it) = ast::StmtList::cast(node.clone()) {
        needless_bool::check_block(cx, &it);
    } else if let Some(it) = ast::IdentPat::cast(node.clone()) {
        needless_borrow::check_ref_pat(cx, &it);
    }
    if let Some(it) = ast::Expr::cast(node.clone()) {
        if let ast::Expr::IfExpr(it) = &it {
            needless_bool::check_if(cx, it);
            collapsible_if::check(cx, it);
            match_like_matches_macro::check_if_let(cx, it);
        }
        if let ast::Expr::MatchExpr(it) = &it {
            match_like_matches_macro::check_match(cx, it);
        }
        io_other_error::check(cx, &it);
        map_clone::check(cx, &it);
        map_unwrap_or::check(cx, &it);
        needless_borrow::check(cx, &it);
        needless_borrows_for_generic_args::check(cx, &it);
        unnecessary_map_or::check(cx, &it);
        byte_char_slices::check(cx, &it);
        clone_on_copy::check(cx, &it);
        get_first::check(cx, &it);
        single_char_add_str::check(cx, &it);
        if let ast::Expr::ClosureExpr(it) = &it {
            needless_return::check_closure(cx, it);
        }
    }
}
