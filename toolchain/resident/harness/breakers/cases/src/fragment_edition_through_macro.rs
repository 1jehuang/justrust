//! tests/ui/macros/metavar_cross_edition_recursive_macros.rs, which rustc lists as its own bug
//! (rust-lang/rust#130484): a macro of a 2021 crate writes a macro here, and the kind of
//! fragment is named here. rustc reads `expr` by the edition of the macro that put the name in
//! place, 2021, where `const { 0 }` is not one; ra-jit reads it by the edition it was written
//! in, 2024, which is what the test says rustc will do once the bug is fixed.

breakers2021::make_matcher!(is_expr, expr, $);
breakers2021::make_matcher!(is_pat, pat, $);

pub fn run() -> [bool; 2] {
    [is_expr!(const { 0 }), is_pat!(A | B)]
}
