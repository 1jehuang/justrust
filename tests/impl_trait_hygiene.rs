//! Argument impl Trait parameters have compiler-owned, unspellable identities.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn anonymous_impl_parameters_do_not_alias_source_names() {
    oracle::check(
        "impl-trait-hygiene",
        &[
            Case {
                name: "ordinary_impl_0_is_distinct",
                source: r#"trait V { fn value(&self)->i32; }
impl V for u8 { fn value(&self)->i32 { *self as i32 } }
fn f<impl_0>(x:impl_0,y:impl V)->impl_0 { x }
fn main() { println!("{}",f::<i32>(7,2u8)); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "anonymous_parameter_is_not_source_addressable",
                source: r#"trait V {} fn f(x:impl V)->impl_0 { x } fn main() {}"#,
                expect: Expect::Invalid("E0425"),
            },
        ],
    );
}
