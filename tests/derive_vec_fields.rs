//! Derived impls over `Vec<nongeneric derived>` fields: one canonical nominal
//! realization per generic callee instance. Stock-differential at workers 1/8.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn derived_vec_field_impls_match_stock() {
    oracle::check(
        "derive_vec_fields_run",
        &[
            Case {
                name: "clone_vec_of_derived",
                source: "#[derive(Clone)]struct I{n:i32}#[derive(Clone)]struct Inv{items:Vec<I>}fn main(){let a=Inv{items:vec![I{n:1},I{n:2}]};let b=a.clone();println!(\"{} {}\",a.items.len(),b.items.len());}",
                expect: Expect::Run("2 2\n"),
            },
            Case {
                name: "pretty_debug_vec_of_derived",
                source: "#[derive(Debug)]struct I{n:i32}#[derive(Debug)]struct Inv{items:Vec<I>}fn main(){println!(\"{:#?}\",Inv{items:vec![I{n:1},I{n:2}]});}",
                expect: Expect::Run(
                    "Inv {\n    items: [\n        I {\n            n: 1,\n        },\n        I {\n            n: 2,\n        },\n    ],\n}\n",
                ),
            },
            Case {
                name: "debug_vec_of_derived",
                source: "#[derive(Debug)]struct I{a:i32}#[derive(Debug)]struct O{v:Vec<I>}fn main(){println!(\"{:?}\",O{v:vec![I{a:1}]});}",
                expect: Expect::Run("O { v: [I { a: 1 }] }\n"),
            },
        ],
    );
}

#[test]
fn vec_field_without_element_impl_rejects_like_stock() {
    oracle::check(
        "derive_vec_fields_reject",
        &[Case {
            name: "clone_vec_of_non_clone",
            source: "struct I{n:i32}#[derive(Clone)]struct Inv{items:Vec<I>}fn main(){}",
            expect: Expect::Invalid("E0277"),
        }],
    );
}
