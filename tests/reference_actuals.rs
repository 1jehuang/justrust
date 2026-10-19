//! Reference-containing aggregates as generic actuals (`T = Option<&i32>`),
//! e.g. `{:?}` of `Option<&T>` through the blanket DebugArg impl.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn reference_actuals_match_stock() {
    oracle::check(
        "reference-actuals",
        &[
            Case {
                name: "debug_option_ref",
                source: r#"fn main(){ let x = 5; let o: Option<&i32> = Some(&x); println!("{:?}", o); }"#,
                expect: Expect::Run("Some(5)\n"),
            },
            Case {
                name: "debug_struct_with_ref",
                source: r#"#[derive(Debug)]
struct W<'a> { r: &'a i32 }
fn main() { let x = 5; let w = W { r: &x }; let o = Some(&x); println!("{:?} {:?}", w, o); }"#,
                expect: Expect::Run("W { r: 5 } Some(5)\n"),
            },
            Case {
                name: "blanket_method_on_option_ref",
                source: r#"trait Id { fn id(self) -> Self; }
impl<T> Id for T { fn id(self) -> Self { self } }
fn main() { let x = 5; let r = Some(&x).id(); match r { Some(v) => println!("{}", v), None => {} } }"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "write_while_borrowed_through_actual_rejected",
                source: r#"trait Id { fn id(self) -> Self; }
impl<T> Id for T { fn id(self) -> Self { self } }
fn main() { let mut x = 5; let r = Some(&x).id(); x = 6; match r { Some(v) => println!("{}", v), None => {} } println!("{}", x); }"#,
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
