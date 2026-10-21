//! General or-patterns (bindings, nominal rows, nested, guarded), stock-checked.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn or_patterns_match_stock() {
    oracle::check(
        "or-patterns",
        &[
            Case {
                name: "binding_rows",
                source: r#"enum T { A(i32), B(i32), C }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(x) => x, T::C => 0 } }
fn main() { println!("{} {} {}", f(T::A(3)), f(T::B(4)), f(T::C)); }"#,
                expect: Expect::Run("3 4 0\n"),
            },
            Case {
                name: "enum_by_reference",
                source: r#"enum T { A, B, C }
fn f(t: &T) -> i32 { match t { T::A | T::B => 1, T::C => 2 } }
fn main() { println!("{} {}", f(&T::B), f(&T::C)); }"#,
                expect: Expect::Run("1 2\n"),
            },
            Case {
                name: "chars_and_ranges",
                source: r#"fn f(c: char) -> i32 { match c { '(' | '[' | '{' => 1, '0'..='9' | '.' => 2, _ => 0 } }
fn main() { println!("{} {} {} {}", f('['), f('.'), f('7'), f('x')); }"#,
                expect: Expect::Run("1 2 2 0\n"),
            },
            Case {
                name: "nested",
                source: r#"enum T { A(i32), B(i32), C }
fn g(o: Option<i32>) -> i32 { match o { Some(1 | 2) => 1, Some(_) => 2, None => 3 } }
fn f(t: (T, bool)) -> i32 { match t { (T::A(x) | T::B(x), true) => x, _ => 0 } }
fn main() {
    println!("{} {} {}", g(Some(2)), g(Some(5)), g(None));
    println!("{} {}", f((T::B(4), true)), f((T::C, true)));
    if let Some(1 | 3) = Some(3) { println!("y"); }
}"#,
                expect: Expect::Run("1 2 3\n4 0\ny\n"),
            },
            Case {
                name: "guarded",
                source: r#"fn f(x: i32) -> i32 { match x { 1 | 2 if x > 1 => 1, _ => 0 } }
fn main() { println!("{} {} {}", f(1), f(2), f(3)); }"#,
                expect: Expect::Run("0 1 0\n"),
            },
            Case {
                name: "unbound_in_one_row",
                source: r#"enum T { A(i32), B(i32) }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(y) => x } }
fn main() { let _ = f(T::A(1)); }"#,
                expect: Expect::Invalid("E0408"),
            },
            Case {
                name: "fixture_bracket_pairs",
                source: r#"fn ok(open: char, c: char) -> bool { matches!((open, c), ('(', ')') | ('[', ']') | ('{', '}')) }
fn kind(c: char) -> i32 { match c { '(' | '[' | '{' => 1, ')' | ']' | '}' => 2, _ => 0 } }
fn main() { println!("{} {} {} {}", ok('[', ']'), ok('(', ']'), kind('{'), kind('}')); }"#,
                expect: Expect::Run("true false 1 2\n"),
            },
            Case {
                name: "nested_option_tuple",
                source: r#"fn f(p: (Option<i32>, i32)) -> i32 { match p { (Some(1 | 2), y) | (None, y) => y, (Some(_), _) => -1 } }
fn main() { println!("{} {} {}", f((Some(2), 5)), f((None, 6)), f((Some(9), 7))); }"#,
                expect: Expect::Run("5 6 -1\n"),
            },
            Case {
                name: "binding_type_mismatch",
                source: r#"enum T { A(i32), B(bool) }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(x) => 1 } }
fn main() { let _ = f(T::A(1)); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unbound_in_unused_item",
                source: r#"enum T { A(i32), B(i32) }
fn unused(t: T) -> i32 { match t { T::A(x) | T::B(y) => y } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0408"),
            },
            Case {
                name: "nonexhaustive_unused",
                source: r#"fn unused(c: char) -> i32 { match c { 'a' | 'b' => 1 } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0004"),
            },
        ],
    );
}
