//! Associated projections inside nominal actuals (`Option<Self::Item>`):
//! user Iterator-style traits with next() compile and run.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn projected_nominals_match_stock() {
    oracle::check(
        "projected-nominals",
        &[
            Case {
                name: "countdown_next",
                source: r#"trait Iter { type Item; fn next(&mut self) -> Option<Self::Item>; }
struct C { n: i32 }
impl Iter for C { type Item = i32; fn next(&mut self) -> Option<i32> { if self.n > 0 { self.n -= 1; Some(self.n) } else { None } } }
fn main() { let mut c = C { n: 3 }; while let Some(x) = c.next() { println!("{}", x); } }"#,
                expect: Expect::Run("2\n1\n0\n"),
            },
            Case {
                name: "projection_in_impl_signature",
                source: r#"trait Iter { type Item; fn next(&mut self) -> Option<Self::Item>; }
struct C { n: i32 }
impl Iter for C { type Item = i32; fn next(&mut self) -> Option<Self::Item> { if self.n > 0 { self.n -= 1; Some(self.n * 10) } else { None } } }
fn main() { let mut c = C { n: 2 }; let x = <C as Iter>::next(&mut c); match x { Some(v) => println!("{}", v), None => {} } }"#,
                expect: Expect::Run("10\n"),
            },
        ],
    );
}
