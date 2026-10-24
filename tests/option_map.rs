//! Stock differential coverage for `Option::map`, an ordinary prelude method.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn option_lazy_combinators_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "option-lazy",
        &[
            Case {
                name: "selected_branches_only",
                source: r#"fn main() {
                println!("{}", Some(2).and_then(|n| Some(n * 3)).unwrap());
                println!("{}", None::<i32>.and_then(|_| { println!("wrong and_then"); Some(9) }).is_none());
                println!("{}", Some(4).or_else(|| { println!("wrong or_else"); None }).unwrap());
                println!("{}", None::<i32>.or_else(|| Some(5)).unwrap());
                println!("{}", Some(6).unwrap_or_else(|| { println!("wrong fallback"); 0 }));
                println!("{}", None::<i32>.unwrap_or_else(|| { println!("fallback"); 7 }));
                println!("{}", Some(8).ok_or_else(|| { println!("wrong error"); 0u8 }).unwrap());
                println!("{}", None::<i32>.ok_or_else(|| 9u8).unwrap_err());
            }"#,
                expect: Expect::Run("6\ntrue\n4\n5\n6\nfallback\n7\n8\n9\n"),
            },
            Case {
                name: "once_capture_dropped_on_selected_and_skipped_branches",
                source: r#"struct Token { id: i32 }
            impl Drop for Token { fn drop(&mut self) { println!("drop {}", self.id); } }
            fn main() {
                { let spare = Token { id: 2 }; let value = Some(Token { id: 1 }).unwrap_or_else(move || spare); println!("value {}", value.id); }
                { let spare = Token { id: 3 }; let value = None::<Token>.unwrap_or_else(move || spare); println!("value {}", value.id); }
            }"#,
                expect: Expect::Run("drop 2\nvalue 1\ndrop 1\nvalue 3\ndrop 3\n"),
            },
            Case {
                name: "wrong_and_then_result",
                source: "fn unused(){let _=Some(1).and_then(|n|n+1);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_fallback_result",
                source: "fn unused(){let _:i32=None::<i32>.unwrap_or_else(||true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

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
fn option_predicates_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "option-predicates",
        &[
            Case {
                name: "predicates_are_lazy_and_consume_payload",
                source: r#"struct Value { number: i32 }
                impl Drop for Value { fn drop(&mut self) { println!("drop {}", self.number); } }
                fn main() {
                    println!("{}", Some(Value { number: 3 }).is_some_and(|v| v.number == 3));
                    println!("{}", None::<i32>.is_some_and(|_| { println!("wrong some"); true }));
                    println!("{}", Some(2).is_none_or(|v| v > 3));
                    println!("{}", None::<i32>.is_none_or(|_| { println!("wrong none"); false }));
                }"#,
                expect: Expect::Run("drop 3\ntrue\nfalse\nfalse\ntrue\n"),
            },
            Case {
                name: "skipped_predicate_drops_capture",
                source: r#"struct Token { number: i32 }
                impl Drop for Token { fn drop(&mut self) { println!("drop {}", self.number); } }
                fn main() {
                    let token = Token { number: 7 };
                    println!("{}", None::<i32>.is_some_and(move |_| { let owned = token; owned.number == 7 }));
                    let token = Token { number: 8 };
                    println!("{}", Some(8).is_none_or(move |v| { let owned = token; v == owned.number }));
                }"#,
                expect: Expect::Run("drop 7\nfalse\ndrop 8\ntrue\n"),
            },
            Case {
                name: "predicate_must_return_bool_even_when_unused",
                source: "fn unused(){let _=None::<i32>.is_some_and(|v|v);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "none_or_predicate_must_return_bool",
                source: "fn unused(){let _=None::<i32>.is_none_or(|v|v);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "predicate_consumes_noncopy_option",
                source: "fn main(){let value=Some(String::from(\"owned\")); let _=value.is_some_and(|s|s.len()>0); let _=value.unwrap();}",
                expect: Expect::Invalid("E0382"),
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
