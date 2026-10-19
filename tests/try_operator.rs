//! Postfix `?` on Option/Result, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn try_operator_matches_stock() {
    oracle::check(
        "try-operator",
        &[
            Case {
                name: "result_and_option",
                source: r#"enum E { Bad(i32) }
fn half(x: i32) -> Result<i32, E> { if x % 2 == 0 { Ok(x / 2) } else { Err(E::Bad(x)) } }
fn quarter(x: i32) -> Result<i32, E> { let h = half(x)?; Ok(half(h)? + 100) }
fn first(v: Option<i32>) -> Option<i32> { Some(v? + 1) }
fn chain(a: Option<i32>, b: Option<i32>) -> Option<i32> { let x = a?; let y = b?; Some(x * y) }
fn main() {
    match quarter(8) { Ok(v) => println!("{}", v), Err(E::Bad(b)) => println!("err {}", b) }
    match quarter(6) { Ok(v) => println!("{}", v), Err(E::Bad(b)) => println!("err {}", b) }
    match first(Some(3)) { Some(v) => println!("{}", v), None => println!("none") }
    match first(None) { Some(v) => println!("{}", v), None => println!("none") }
    match chain(Some(6), Some(7)) { Some(v) => println!("{}", v), None => println!("none") }
    match chain(Some(6), None) { Some(v) => println!("{}", v), None => println!("none") }
}"#,
                expect: Expect::Run("102\nerr 3\n4\nnone\n42\nnone\n"),
            },
            Case {
                name: "loop_early_exit",
                source: r#"fn sum(v: [i32; 4]) -> Result<i32, i32> {
    let mut total = 0;
    let mut i: usize = 0;
    while i < 4 { total += check(v[i])?; i += 1; }
    Ok(total)
}
fn check(x: i32) -> Result<i32, i32> { if x < 0 { Err(x) } else { Ok(x) } }
fn main() {
    match sum([1, 2, 3, 4]) { Ok(v) => println!("ok {}", v), Err(e) => println!("bad {}", e) }
    match sum([1, -2, 3, -4]) { Ok(v) => println!("ok {}", v), Err(e) => println!("bad {}", e) }
}"#,
                expect: Expect::Run("ok 10\nbad -2\n"),
            },
            Case {
                name: "try_in_unit_fn",
                source: r#"fn f(x: Option<i32>) -> i32 { x? }
fn main() { println!("{}", f(Some(1))); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "try_on_non_carrier",
                source: r#"fn f(x: i32) -> Option<i32> { Some(x?) }
fn main() { let _ = f(1); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "option_in_result_fn",
                source: r#"fn f(x: Option<i32>) -> Result<i32, i32> { Ok(x?) }
fn main() { let _ = f(Some(1)); }"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
