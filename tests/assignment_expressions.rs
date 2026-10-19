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
        ],
    );
}
