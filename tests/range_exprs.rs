//! Range expressions in every expression position, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn range_exprs_match_stock() {
    oracle::check(
        "range-exprs",
        &[
            Case {
                name: "inclusive_state",
                source: r#"fn main() {
    let a = 1..=3;
    let empty = a.is_empty();
    let pair = a.into_inner();
    println!("{} {} {}", empty, pair.0, pair.1);
    let b = 4..=2;
    let c = 255u8..=255u8;
    println!("{} {}", b.is_empty(), c.is_empty());
}"#,
                expect: Expect::Run("false 1 3\ntrue false\n"),
            },
            Case {
                name: "reverse_bounds",
                source: r#"fn bound(x: i32) -> i32 { print!("b{} ", x); x }
fn main() {
    for i in (bound(1)..bound(4)).rev() { if i == 2 { continue; } print!("{} ", i); }
    println!();
    for i in (0u8..=2u8).rev() { print!("{} ", i); }
    println!();
    for i in (0u8..0u8).rev() { println!("bad {}", i); }
    for i in (3..1).rev() { println!("bad {}", i); }
    for i in (-128i8..=-128i8).rev() { println!("{}", i); }
    for i in (0..3).rev().rev() { print!("{} ", i); }
    println!();
}"#,
                expect: Expect::Run("b1 b4 3 1 \n2 1 0 \n-128\n0 1 2 \n"),
            },
            Case {
                name: "all_forms",
                source: r#"fn take(r: std::ops::Range<i32>) -> i32 { r.end - r.start }
fn main() {
    let r = 2..5;
    let f = ..;
    let _ = f;
    let a = ..=4u8;
    let b = 1..;
    let c = ..7;
    let d = 1..=3;
    let ds = *d.start();
    let de = *d.end();
    println!("{} {} {} {} {} {}", r.end, a.end, b.start, c.end, ds, de);
    println!("{}", take(1 + 1..2 * 5));
    let receiver = (0..9).end;
    println!("{}", receiver);
    let n = 3;
    for i in 0..n - 1 { print!("{} ", i); }
    println!();
    let v = [1, 2, 3, 4];
    println!("{} {}", v[1..].len(), v[..=2].len());
}"#,
                expect: Expect::Run("5 4 1 7 1 3\n8\n9\n0 1 \n3 3\n"),
            },
            Case {
                name: "inclusive_needs_end",
                source: r#"fn main() { let r = 1..=; }"#,
                expect: Expect::Invalid("E0586"),
            },
        ],
    );
}
