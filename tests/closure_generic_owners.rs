//! Canonical generic owner domains and unused-body rejection, compared with
//! stock Rust through explicit CHECK/BUILD/run at workers 1 and 8.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn current_parent_generic_owner_consumer() {
    oracle::check(
        "closure_generic_owner_current_parent",
        &[Case {
            name: "ordered_move_owner_instantiations",
            source: "fn outer<A,B>(a:A,b:B)->i32{let f=move||{let first=a;let second=b;9};f()}fn main(){assert_eq!(outer(String::from(\"owned\"),8u8),9);assert_eq!(outer(7u8,String::from(\"second\")),9);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn generic_parent_closures_preserve_owner_domains() {
    oracle::check(
        "closure_generic_owners",
        &[
            Case {
                name: "noncapturing_generic_parent",
                source: "fn outer<T>(value:T)->i32{let f=|x:i32|x+1;f(2)}fn main(){assert_eq!(outer(7u8),3);assert_eq!(outer(8i64),3);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "generic_move_capture",
                source: "fn outer<T>(value:T)->i32{let f=move||{let taken=value;7};f()}fn main(){assert_eq!(outer(String::from(\"owned\")),7);assert_eq!(outer(8u8),7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "ordered_distinct_owner_arguments",
                source: "fn outer<A,B>(a:A,b:B)->i32{let f=move||{let first=a;let second=b;9};f()}fn main(){assert_eq!(outer(7u8,8i64),9);assert_eq!(outer(8i64,7u8),9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_generic_move_reused",
                source: "fn outer<T>(value:T)->i32{let f=move||{let taken=value;7};let first=f();f()}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "unused_generic_bound_not_assumed",
                source: "fn requires_copy<T:Copy>(value:T){}fn outer<T>(value:T)->i32{let f=move||{requires_copy(value);7};f()}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
