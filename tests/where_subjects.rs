//! Where predicates retain general type subjects as authoritative obligations.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn general_where_subjects_match_stock() {
    oracle::check(
        "where-subjects",
        &[
            Case {
                name: "self_sized_method",
                source: r#"trait Value { fn value(self) -> i32 where Self: Sized; }
impl Value for i32 { fn value(self) -> i32 { self } }
fn main() { println!("{}", 7i32.value()); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "concrete_subject",
                source: r#"trait Mark {} impl Mark for i32 {}
fn value() -> i32 where i32: Mark { 9 }
fn main() { println!("{}", value()); }"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "qualified_projection_subject",
                source: r#"trait Has { type Item; } trait Mark {}
impl Mark for i32 {} struct S;
impl Has for S { type Item = i32; }
fn value<T: Has>() -> i32 where <T as Has>::Item: Mark { 11 }
fn main() { println!("{}", value::<S>()); }"#,
                expect: Expect::Run("11\n"),
            },
            Case {
                name: "self_projection_subject",
                source: r#"trait Mark {} impl Mark for i32 {}
trait Has { type Item; fn value(&self) -> i32 where Self::Item: Mark { 13 } }
struct S; impl Has for S { type Item = i32; }
fn main() { let x = S; println!("{}", x.value()); }"#,
                expect: Expect::Run("13\n"),
            },
            Case {
                name: "unsatisfied_projected_subject",
                source: r#"trait Has { type Item; } trait Mark {}
struct S; impl Has for S { type Item = i32; }
fn value<T: Has>() -> i32 where <T as Has>::Item: Mark { 11 }
fn main() { let _ = value::<S>(); }"#,
                expect: Expect::Invalid("trait"),
            },
            Case {
                name: "concrete_unsatisfied_unused",
                source: "trait Mark {} fn unused() where i32: Mark {} fn main() {}",
                expect: Expect::Invalid("trait"),
            },
            Case {
                name: "concrete_relaxed_sized",
                source: "fn unused() where i32: ?Sized {} fn main() {}",
                expect: Expect::Invalid("relaxed bound"),
            },
            Case {
                name: "self_relaxed_sized",
                source: "trait A where Self: ?Sized {} fn main() {}",
                expect: Expect::Invalid("relaxed bound"),
            },
            Case {
                name: "unknown_subject_unused",
                source: "trait Mark {} fn unused() where Missing: Mark {} fn main() {}",
                expect: Expect::Invalid("Missing"),
            },
        ],
    );
}
