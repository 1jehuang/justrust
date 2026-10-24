//! Stock differential coverage for `Option::map`, an ordinary prelude method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn option_insert_returns_stored_mutable_value() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "option-insert",
        &[
            Case {
                name: "none_then_some_mutates_stored_value",
                source: r#"fn main() {
                    let mut value: Option<i32> = None;
                    *value.insert(4) += 3;
                    println!("{}", value.unwrap());
                    let mut value = Some(9);
                    let result = value.insert(12);
                    *result *= 2;
                    println!("{}", value.unwrap());
                }"#,
                expect: Expect::Run("7\n24\n"),
            },
            Case {
                name: "replacement_drops_previous_value_once",
                source: r#"struct Value { number: i32 }
                impl Drop for Value { fn drop(&mut self) { println!("drop {}", self.number); } }
                fn main() {
                    let mut value = Some(Value { number: 1 });
                    let inserted = value.insert(Value { number: 2 });
                    inserted.number = 3;
                    println!("stored {}", inserted.number);
                }"#,
                expect: Expect::Run("drop 1\nstored 3\ndrop 3\n"),
            },
            Case {
                name: "live_insert_result_excludes_second_mutation",
                source: r#"fn main() { let mut value = Some(1);
                    let result = value.insert(2); value = Some(3); println!("{}", *result); }"#,
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "insert_result_cannot_escape_local_option",
                source: r#"fn invalid<'a>() -> &'a mut i32 {
                    let mut value = None; value.insert(3)
                } fn main() {}"#,
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

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
