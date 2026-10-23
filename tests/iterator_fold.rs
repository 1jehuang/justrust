//! Stock differential coverage for `Iterator::fold`, a prelude default method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn iterator_collect_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect",
        &[
            Case {
                name: "character_vector",
                source: r#"fn main() { let chars: Vec<char> = "aé🦀".chars().collect(); println!("{} {} {} {}", chars.len(), chars[0], chars[1], chars[2]); }"#,
                expect: Expect::Run("3 a é 🦀\n"),
            },
            Case {
                name: "string_characters",
                source: r#"fn main() { let text: String = "aé🦀".chars().collect(); println!("{}", text); }"#,
                expect: Expect::Run("aé🦀\n"),
            },
            Case {
                name: "string_borrowed_characters",
                source: r#"fn main() { let chars = ['a', 'é', '🦀']; let text: String = chars.iter().collect(); println!("{}", text); }"#,
                expect: Expect::Run("aé🦀\n"),
            },
            Case {
                name: "empty_collection",
                source: r#"fn main() { let values: Vec<char> = "".chars().collect(); let text: String = "".chars().collect(); println!("{} {}", values.len(), text.len()); }"#,
                expect: Expect::Run("0 0\n"),
            },
            Case {
                name: "user_defined_iterator",
                source: r#"struct Count { n: i32 } impl Iterator for Count { type Item = i32; fn next(&mut self) -> Option<i32> { if self.n == 3 { None } else { let n = self.n; self.n += 1; Some(n) } } } fn main() { let values: Vec<i32> = Count { n: 0 }.collect(); println!("{} {}", values.len(), values[2]); }"#,
                expect: Expect::Run("3 2\n"),
            },
            Case {
                name: "wrong_element_type",
                source: r#"fn main() { let values: Vec<bool> = (0i32..3i32).collect(); }"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn iterator_collect_mapped_range_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect-map",
        &[Case {
            name: "range_map",
            source: r#"fn main() { let values: Vec<i32> = (0i32..4i32).map(|x| x * x).collect(); println!("{} {} {}", values.len(), values[1], values[3]); }"#,
            expect: Expect::Run("4 1 9\n"),
        }],
    );
}

#[test]
fn iterator_collect_mapped_strings_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-collect-mapped-strings",
        &[Case {
            name: "borrowed_integer_strings",
            source: r#"fn main() { let row = [1i64, 2i64]; let parts: Vec<String> = row.iter().map(|x| x.to_string()).collect(); println!("[{}]", parts.join(", ")); }"#,
            expect: Expect::Run("[1, 2]\n"),
        }],
    );
}

#[test]
fn generic_method_owner_projections_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "method-owner-projections",
        &[
            Case {
                name: "projected_parameter_and_result",
                source: r#"
trait Source {
    type Item;
    fn choose<U>(&self, value: Self::Item, tag: U) -> Option<Self::Item>;
}
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, value: i32, tag: U) -> Option<i32> { Some(value) }
}
fn main() {
    let source = Numbers;
    let chosen: Option<i32> = source.choose(7i32, true);
    println!("{:?}", chosen);
}
"#,
                expect: Expect::Run("Some(7)\n"),
            },
            Case {
                name: "generic_owner_and_method_binders",
                source: r#"
trait Source { type Item; fn choose<U>(&self, value: Self::Item, tag: U) -> Self::Item; }
struct Holder<T> { value: T }
impl<T> Source for Holder<T> {
    type Item = T;
    fn choose<U>(&self, value: T, tag: U) -> T { value }
}
fn main() {
    let source = Holder { value: 1i32 };
    println!("{}", source.choose(9i32, false));
}
"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "wrong_projected_implementation_unused",
                source: r#"
trait Source { type Item; fn choose<U>(&self, value: Self::Item, tag: U) -> Self::Item; }
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, value: u8, tag: U) -> i32 { 7 }
}
fn main() {}
"#,
                expect: Expect::Invalid("E0053"),
            },
            Case {
                name: "projected_result_mismatch_unused",
                source: r#"
trait Source {
    type Item;
    fn choose<U>(&self, tag: U) -> Option<Self::Item>;
}
struct Numbers;
impl Source for Numbers {
    type Item = i32;
    fn choose<U>(&self, tag: U) -> Option<i32> { Some(7) }
}
fn bad() { let chosen: Option<bool> = Numbers.choose(true); }
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

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

/// `Iterator::filter` source adapter: `Filter<I, P>` with the
/// `P: FnMut(&I::Item) -> bool` projection, consumed by `next()`. The parent
/// may re-check and re-mint an equal closure shape seal.
#[test]
fn iterator_filter_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "iterator-filter",
        &[
            Case {
                name: "filter_owned_items_next",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2, 3, 4];
    let mut m = v.iter().copied().filter(|x: &i32| *x % 2 == 0);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Run("Some(2)\n"),
            },
            Case {
                name: "map_then_next",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![5, 6];
    let mut m = v.iter().map(|x: &i32| *x * 2);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Run("Some(10)\n"),
            },
            Case {
                name: "filter_predicate_not_bool",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2];
    let mut m = v.iter().copied().filter(|x: &i32| *x);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "filter_predicate_takes_item_by_value",
                source: r#"
fn main() {
    let v: Vec<i32> = vec![1, 2];
    let mut m = v.iter().copied().filter(|x: i32| x > 1);
    println!("{:?}", m.next());
}
"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
