//! `name @ pattern` bindings, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};
#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn at_patterns_match_stock() {
    oracle::check(
        "at-patterns",
        &[
            Case {
                name: "scalar_ranges",
                source: r#"fn f(x: i32) -> i32 { match x { n @ 1..=5 => n * 10, n => n } }
fn g(c: char) -> i32 { match c { d @ '0'..='9' => d as i32 - 48, _ => -1 } }
fn main() { println!("{} {} {} {}", f(3), f(9), g('7'), g('x')); }"#,
                expect: Expect::Run("30 9 7 -1\n"),
            },
            Case {
                name: "nested_and_whole",
                source: r#"enum T { A(i32), B }
fn f(t: &T) -> i32 { match t { whole @ T::A(n @ 1..=3) => { let _ = whole; *n } T::A(_) => 0, T::B => -1 } }
fn main() {
    println!("{} {} {}", f(&T::A(2)), f(&T::A(7)), f(&T::B));
    let p = (1, 2);
    match p { all @ (a, _) => println!("{} {}", all.1, a) }
    let v = Some(4);
    if let Some(x @ (1 | 4)) = v { println!("{}", x); }
    match v { s @ Some(_) => println!("{}", s.is_some()), None => {} }
}"#,
                expect: Expect::Run("2 0 -1\n2 1\n4\ntrue\n"),
            },
            Case {
                name: "ref_at",
                source: r#"struct S { v: i32 }
fn main() { let s = S { v: 3 }; match s { ref w @ S { v } => println!("{} {}", w.v, v) } }"#,
                expect: Expect::Run("3 3\n"),
            },
            Case {
                name: "duplicate_binding",
                source: r#"fn main() { match 1 { x @ x => println!("{}", x) } }"#,
                expect: Expect::Invalid("E0416"),
            },
        ],
    );
}

/// The @ name's type comes only from local inference here: no annotation, a
/// literal scrutinee, and the whole value is used. Strict oracle: CHECK and
/// BUILD at workers 1 and 8, and CHECK must not demand code generation.
#[test]
fn inferred_at_bindings_match_stock_at_workers_1_and_8() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "at-inferred",
        &[
            Case {
                name: "inferred_whole_binding",
                source: r#"fn main() {
    let whole @ (a, b) = (3, 4);
    println!("{} {}", whole.0 + whole.1, a + b);
    let x = Some(42);
    match x { w @ Some(_) => match w { Some(n) => println!("{}", n), None => {} }, None => {} }
    let pair = (1u8, 2u8);
    if let all @ (1, _) = pair { println!("{}", all.1); }
}"#,
                expect: Expect::Run("7 7\n42\n2\n"),
            },
            Case {
                name: "inferred_whole_binding_type_mismatch",
                source: r#"fn main() { let whole @ (a, _) = (3, true); let _: bool = whole.0; let _ = a; }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
