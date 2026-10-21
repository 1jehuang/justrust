//! Stock differential coverage for `Option::map`, an ordinary prelude method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn option_map_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "option-map",
        &[
            Case {
                name: "some_and_none_and_type_change",
                source: r#"
fn half(n: u32) -> Option<u32> { if n % 2 == 0 { Some(n / 2) } else { None } }
fn main() {
    let a = half(10).map(|x| x + 1);
    let b = half(3).map(|x| x * 100);
    let c: Option<bool> = Some(7u8).map(|v| v > 3);
    println!("{} {} {}", a.unwrap_or(0), b.unwrap_or(9), c.unwrap());
}
"#,
                expect: Expect::Run("6 9 true\n"),
            },
            Case {
                name: "closure_result_type_mismatch_rejected",
                source: r#"
fn main() {
    let a: Option<u8> = Some(1u8).map(|x| x > 0);
    println!("{}", a.unwrap());
}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
