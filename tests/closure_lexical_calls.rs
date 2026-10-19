//! Lexical callable identity controls for unchanged unqualified closure syntax.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn closure_calls_resolve_items_and_captured_bindings() {
    oracle::check(
        "closure_lexical_calls",
        &[
            Case {
                name: "module_function",
                source: "fn plus(x:i32)->i32{x+1}fn main(){let f=|x:i32|plus(x);assert_eq!(f(2),3);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_callable_shadows_item",
                source: "fn plus(x:i32)->i32{x+1}fn main(){let plus=|x:i32|x*3;let f=||plus(2);assert_eq!(f(),6);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "parameter_shadows_parent_and_item",
                source: "fn plus(x:i32)->i32{x+1}fn main(){let plus=|x:i32|x*3;let f=|plus:i32|plus+2;assert_eq!(f(4),6);assert_eq!(plus(2),6);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "source_module_import",
                source: "mod ops{pub fn plus(x:i32)->i32{x+1}}use ops::plus as next;fn main(){let f=|x:i32|next(x);assert_eq!(f(2),3);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_noncallable_shadows_item",
                source: "fn plus(x:i32)->i32{x+1}fn unused(){let plus=1i32;let f=||plus(2);}fn main(){}",
                expect: Expect::Invalid("E0618"),
            },
            Case {
                name: "unused_wrong_item_arity",
                source: "fn plus(x:i32)->i32{x+1}fn unused(){let f=||plus();}fn main(){}",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "unused_unknown_item",
                source: "fn unused(){let f=||missing(2);}fn main(){}",
                expect: Expect::Invalid("E0425"),
            },
        ],
    );
}
