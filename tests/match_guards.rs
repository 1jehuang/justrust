//! Ordinary match-arm guards, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn match_guards_match_stock() {
    oracle::check(
        "match-guards",
        &[
            Case {
                name: "option_guards",
                source: r#"fn f(x: Option<i32>) -> i32 { match x { Some(n) if n > 5 => 1, Some(_) => 2, None => 3 } }
fn main() { println!("{} {} {}", f(Some(9)), f(Some(1)), f(None)); }"#,
                expect: Expect::Run("1 2 3\n"),
            },
            Case {
                name: "tuple_guards",
                source: r#"fn f(x: i32, y: i32) -> i32 { match (x, y) { (a, b) if a == b => 0, (a, _) if a < 0 => -1, _ => 1 } }
fn main() { println!("{} {} {}", f(2, 2), f(-1, 3), f(5, 1)); }"#,
                expect: Expect::Run("0 -1 1\n"),
            },
            Case {
                name: "reference_guards",
                source: r#"enum T { A(i32), B }
fn f(t: &T) -> i32 { match t { T::A(n) if *n > 3 => *n, T::A(_) => 0, T::B => -1 } }
fn main() { println!("{} {} {}", f(&T::A(9)), f(&T::A(1)), f(&T::B)); }"#,
                expect: Expect::Run("9 0 -1\n"),
            },
            Case {
                name: "guard_not_exhaustive",
                source: r#"fn f(x: Option<i32>) -> i32 { match x { Some(n) if n > 5 => 1, None => 3 } }
fn main() { println!("{}", f(Some(9))); }"#,
                expect: Expect::Invalid("E0004"),
            },
            Case {
                name: "guard_not_bool",
                source: r#"fn f(x: i32) -> i32 { match x { n if n => 1, _ => 0 } }
fn main() { println!("{}", f(1)); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
