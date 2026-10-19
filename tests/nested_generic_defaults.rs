//! Numeric defaults remain local to inferred expressions, not universal binders.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_generic_calls_default_only_concrete_numeric_variables() {
    oracle::check(
        "nested-generic-defaults",
        &[
            Case {
                name: "block_argument_default",
                source: "fn id<T>(x:T)->T{x} fn main(){let x=id({let a=1;2});println!(\"{}\",x);}",
                expect: Expect::Run("2\n"),
            },
            Case {
                name: "nested_call_default",
                source: "fn id<T>(x:T)->T{x} fn main(){let x=id(id(1));println!(\"{}\",x);}",
                expect: Expect::Run("1\n"),
            },
            Case {
                name: "concrete_types_stay_distinct",
                source: "fn same<T>(x:T,y:T)->T{y} fn main(){let x=same(1u8,2u16);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "universal_move_stays_invalid",
                source: "fn bad<T>(x:T)->(T,T){(x,x)} fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}
