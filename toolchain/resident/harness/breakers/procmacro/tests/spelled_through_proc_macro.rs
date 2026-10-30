//! Found in blaze's self-test: salsa's `#[salsa::tracked]` hands the type of an impl on to
//! `stringify!` to name its queries, and hir-def's incremental tests compare the names of the
//! queries that ran. rustc gives the tokens that come out of a proc macro the spacing the macro
//! said, `Alone` for all but a joint punctuation, and spells `AstId < ast :: Macro >`; ra-jit
//! looked them up in the source they came from and spelled `AstId<ast::Macro>`.

#[test]
fn spelled() {
    assert_eq!(
        procmacro::spelled(),
        [
            ["AstId < ast :: Macro >", "AstId < ast :: Macro >"],
            ["Modules < 'db >", "Modules < 'db >"],
            ["Vec < (u8, [u16; 2]) >", "Vec < (u8, [u16; 2]) >"],
        ]
    );
}
