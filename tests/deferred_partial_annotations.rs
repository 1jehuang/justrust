//! Deferred local annotation holes use the same structural inference facts.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn deferred_partial_annotations_match_stock() {
    oracle::check(
        "deferred-partial-annotations",
        &[
            Case {
                name: "nominal_and_tuple_deferred",
                source: r#"struct P<T> { value: T }
fn main() {
    let p: P<_>;
    p = P { value: 7u16 };
    let t: (_, u8);
    t = (9i64, 3u8);
    println!("{} {} {}", p.value, t.0, t.1);
}"#,
                expect: Expect::Run("7 9 3\n"),
            },
            Case {
                name: "deferred_concrete_constraint",
                source: r#"fn main() { let p: (_, bool); p = (1u8, 2u8); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn partial_pattern_annotations_match_stock() {
    oracle::check(
        "partial-pattern-annotations",
        &[
            Case {
                name: "partition_into_partial_tuple_pattern",
                source: r#"fn num(s: &str) -> Result<u8, String> { s.parse().map_err(|e: std::num::ParseIntError| e.to_string()) }
fn main() {
    let (big, small): (Vec<_>, Vec<_>) = vec![1u8, 5, 2, 9].into_iter().partition(|x| *x > 3);
    println!("{big:?} {small:?}");
    for e in small.iter().map(|x| *x + 1) { println!("{e}"); }
    let (ok, bad): (Vec<_>, Vec<_>) = "1 x 3".split(' ').map(num).partition(Result::is_ok);
    let ok: Vec<u8> = ok.into_iter().map(Result::unwrap).collect();
    for e in bad.into_iter().map(|r| r.unwrap_err()) { println!("error: {e}"); }
    println!("{ok:?}");
}"#,
                expect: Expect::Run(
                    "[5, 9] [1, 2]\n2\n3\nerror: invalid digit found in string\n[1, 3]\n",
                ),
            },
            Case {
                name: "partial_pattern_element_mismatch",
                source: r#"fn main() { let (a, b): (Vec<_>, Vec<_>) = vec![1u8, 2].into_iter().partition(|x| *x > 1); let c: Vec<u16> = a; println!("{c:?}{b:?}"); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "partial_pattern_shape_mismatch",
                source: r#"fn main() { let (a, b): (Vec<_>, String) = vec![1u8, 2].into_iter().partition(|x| *x > 1); println!("{a:?}{b}"); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn partial_annotations_with_omitted_defaults_match_stock() {
    oracle::check(
        "partial-omitted-defaults",
        &[
            Case {
                name: "hashmap_partial_holes",
                source: r#"use std::collections::HashMap;
fn main() {
    let mut m: HashMap<_, Vec<i32>> = HashMap::new();
    m.insert("k", vec![1, 2]);
    let mut n: HashMap<_, _> = HashMap::new();
    n.insert(3u8, String::from("x"));
    println!("{:?} {:?} {}", m["k"], n[&3], m.len() + n.len());
}"#,
                expect: Expect::Run("[1, 2] \"x\" 2\n"),
            },
            Case {
                name: "hashmap_partial_value_mismatch",
                source: r#"use std::collections::HashMap; fn main() { let mut m: HashMap<_, _> = HashMap::new(); m.insert(1u8, 2u8); let _: HashMap<u8, u8, u8> = m; }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "hashmap_too_few_arguments",
                source: r#"use std::collections::HashMap; fn unused() { let m: HashMap<_> = HashMap::new(); let _ = m; } fn main() {}"#,
                expect: Expect::Invalid("E0107"),
            },
        ],
    );
}
