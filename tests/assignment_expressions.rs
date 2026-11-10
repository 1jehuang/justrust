//! Assignment expression syntax, including expression-only match arm bodies.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn assignment_expressions_match_stock() {
    oracle::check(
        "assignment-expressions",
        &[
            Case {
                name: "match_arm_and_unit",
                source: r#"
fn main() {
    let mut n = 0;
    for v in 0..4 { match v { 0 => n = 3, 1 => n += 5, 2 => n *= 2, _ => n -= 1, } }
    let u: () = (n += 4);
    let mut a = [1, 2];
    let _ = match n { 19 => a[1] += n, _ => a[0] = 0 };
    let mut p = (1, 2);
    match 1 { 1 => p.0 = 7, _ => p.1 += 3 }
    println!("{} {} {} {} {} {:?}", n, a[0], a[1], p.0, p.1, u);
}"#,
                expect: Expect::Run("19 1 21 7 2 ()\n"),
            },
            Case {
                name: "indexed_rhs_first",
                source: r#"fn index(c: &mut i32) -> usize { *c = *c * 10 + 1; 0 }
fn value(c: &mut i32) -> i32 { *c = *c * 10 + 2; 4 }
fn main() { let mut a = [1]; let mut c = 0;
    match 0 { _ => a[index(&mut c)] += value(&mut c) }
    println!("{} {}", a[0], c);
}"#,
                expect: Expect::Run("5 21\n"),
            },
            Case {
                name: "immutable_assignment",
                source: r#"fn main() { let n = 1; match 0 { _ => n += 2 } }"#,
                expect: Expect::Invalid("E0384"),
            },
            Case {
                name: "assignment_is_unit",
                source: r#"fn main() { let mut n = 1; let _: i32 = (n = 2); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "invalid_place",
                source: r#"fn main() { match 0 { _ => (1 + 2) = 3 } }"#,
                expect: Expect::Invalid("E0070"),
            },
            Case {
                name: "blocklike_arm_bodies_and_assignment_tails",
                source: r#"fn f(x: i32, v: &[i32]) -> i32 {
    let mut n = 0;
    match x {
        0 => for e in v { n += e }
        1 => while n < 10 { n += 3 },
        2 => 'a: while n < 100 { n += 50; if n > 60 { break 'a } }
        3 => loop { n -= 1; if n < -2 { break } },
        4 => if n == 0 { n = 4 } else { n = 5 },
        5 => if let [a, ..] = v { n = *a }
        6 => { n = 6 }
        7 => unsafe { n = 7 },
        _ => n = 99,
    }
    if x > 100 { n = -1 }
    let g = |m: &mut i32| { *m += 1 };
    g(&mut n);
    n
}
fn main() { for x in 0..9 { print!("{} ", f(x, &[1, 2, 3])); } println!(); }"#,
                expect: Expect::Run("7 13 101 -2 5 2 7 8 100 \n"),
            },
            Case {
                name: "assignment_block_tail_is_unit",
                source: r#"fn main() { let mut n = 0; let x: i32 = { n = 1 }; println!("{x} {n}"); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "immutable_assignment_block_tail",
                source: r#"fn main() { let n = 0; match 1 { 1 => if true { n = 2 }, _ => {} } println!("{n}"); }"#,
                expect: Expect::Invalid("E0384"),
            },
            Case {
                name: "loop_arm_body_is_unit",
                source: r#"fn main() { let mut n = 0; let y = match 1 { 1 => while n < 3 { n += 1 }, _ => 5 }; println!("{n}"); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
