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
            // A graph-realized nominal accumulator: the closure body shares the
            // parent's nominal identity authority (corpus 19 reduction).
            Case {
                name: "nominal_accumulator_with_derived_debug",
                source: r#"
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
fn main() {
    let v = vec![V { x: 1 }, V { x: 2 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| V { x: acc.x + p.x });
    println!("{:?}", s);
}
"#,
                expect: Expect::Run("V { x: 3 }\n"),
            },
            // A user `Add` impl called inside the closure body: the realized
            // nominal call is typed in the closure's own arena (corpus 19).
            Case {
                name: "nominal_operator_trait_call_in_closure_body",
                source: r#"
use std::ops::Add;
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
impl Add for V { type Output = V; fn add(self, o: V) -> V { V { x: self.x + o.x } } }
fn main() {
    let v = vec![V { x: 1 }, V { x: 2 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| acc + p);
    println!("{:?}", s);
}
"#,
                expect: Expect::Run("V { x: 3 }\n"),
            },
            Case {
                name: "nominal_operator_without_impl_in_closure_body_rejects",
                source: r#"
#[derive(Clone, Copy, Debug)]
struct V { x: i32 }
fn main() {
    let v = vec![V { x: 1 }];
    let s = v.iter().fold(V { x: 0 }, |acc, &p| acc + p);
    println!("{:?}", s);
}
"#,
                expect: Expect::Invalid(""),
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

/// `Iterator::map` source adapter: a closure-bearing `Map<I, F>` realized by
/// the parent body, consumed by `fold` and by direct `next()`.
#[test]
fn iterator_map_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-map",
        &[
            Case {
                name: "map_then_fold",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3];
    let s = v.iter().map(|x: &i32| *x * 2).fold(0i32, |a: i32, b: i32| a + b);
    println!("{}", s);
}
"#,
                expect: Expect::Run("12\n"),
            },
            Case {
                name: "map_changes_item_type_with_capture",
                source: r#"
fn main() {
    let v: Vec<u8> = vec![3, 4, 5];
    let k = 10u64;
    let m = v.iter().map(|x: &u8| *x as u64 * k);
    println!("{}", m.fold(0u64, |a: u64, b: u64| a + b));
}
"#,
                expect: Expect::Run("120\n"),
            },
            Case {
                name: "map_result_mismatch_rejected",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3];
    let s: bool = v.iter().map(|x: &i32| *x * 2).fold(0i32, |a: i32, b: i32| a + b);
}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
