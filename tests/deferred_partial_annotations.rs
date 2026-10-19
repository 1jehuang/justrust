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
