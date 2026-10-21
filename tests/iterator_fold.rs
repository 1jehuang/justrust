//! Stock differential coverage for `Iterator::fold`, a prelude default method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn iterator_fold_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-fold",
        &[
            Case {
                name: "sums_borrowed_items",
                source: r#"
fn main() {
    let v = vec![1i64, 2, 3, 4];
    let s = v.iter().fold(0i64, |a, x| a + *x);
    println!("{}", s);
}
"#,
                expect: Expect::Run("10\n"),
            },
            Case {
                name: "accumulator_type_differs_from_item_and_empty_returns_init",
                source: r#"
fn main() {
    let v = vec![3u8, 4, 5];
    let n = v.iter().fold(0u64, |acc, x| acc * 10 + (*x as u64));
    println!("{}", n);
    let e: Vec<u8> = Vec::new();
    println!("{}", e.iter().fold(7u32, |a, _| a + 1));
}
"#,
                expect: Expect::Run("345\n7\n"),
            },
            Case {
                name: "folds_left_to_right",
                source: r#"
fn main() {
    let v = vec![1i64, 2, 3];
    println!("{}", v.iter().fold(100i64, |a, x| a - *x * 10));
    println!("{}", v.iter().fold(0i64, |a, x| a * 2 + *x));
}
"#,
                expect: Expect::Run("40\n11\n"),
            },
            // A by-value `mut acc` closure parameter that calls `&mut self`
            // methods (`|mut acc, x| { acc.push(..); acc }`) is rejected by
            // the current checker ("canonical method receiver formal
            // mismatch"), independent of fold. Tracked in SEMANTIC-CORE.md.
            Case {
                name: "closure_must_return_accumulator_type",
                source: "fn main(){let v=vec![1i32];let _=v.iter().fold(0i32,|a,x| a > *x);}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
