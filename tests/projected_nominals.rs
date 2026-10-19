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
            Case {
                name: "shared_ref_items",
                source: r#"trait Iter { type Item; fn next(&mut self) -> Option<Self::Item>; }
struct A<'a> { a: &'a [i32; 4], i: usize }
impl<'a> Iter for A<'a> {
    type Item = &'a i32;
    fn next(&mut self) -> Option<&'a i32> { if self.i < 4 { let r = &self.a[self.i]; self.i += 1; Some(r) } else { None } }
}
fn main() { let a = [1, 2, 3, 4]; let mut it = A { a: &a, i: 0 }; let mut s = 0; while let Some(x) = it.next() { s += *x; } println!("{}", s); }"#,
                expect: Expect::Run("10\n"),
            },
            Case {
                name: "write_while_item_live_rejected",
                source: r#"trait Iter { type Item; fn next(&mut self) -> Option<Self::Item>; }
struct W<'a> { r: &'a i32, done: bool }
impl<'a> Iter for W<'a> {
    type Item = &'a i32;
    fn next(&mut self) -> Option<&'a i32> { if self.done { None } else { self.done = true; Some(self.r) } }
}
fn main() { let mut a = 7; let mut it = W { r: &a, done: false }; let x = it.next(); a = 8; match x { Some(v) => println!("{}", v), None => {} } println!("{}", a); }"#,
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
