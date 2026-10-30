//! Stock differential coverage for Result mapping and fallible Vec collection.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn unchanged_error_handling_program_matches_stock() {
    closure_oracle::check(
        "full-error-handling",
        &[closure_oracle::Case {
            name: "unchanged_error_handling",
            source: include_str!("fixtures/full_programs/15_error_handling.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "fixtures/full_programs/15_error_handling.stdout"
            )),
        }],
    );
}

#[test]
fn result_lazy_combinators_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "result-lazy",
        &[
            Case {
                name: "selected_branches_only",
                source: r#"fn main() {
                println!("{}", Ok::<i32,u8>(2).and_then(|n| Ok(n * 3)).unwrap());
                println!("{}", Err::<i32,u8>(4).and_then(|_| { println!("wrong and_then"); Ok(9i32) }).unwrap_err());
                println!("{}", Ok::<i32,u8>(5).or_else(|_| { println!("wrong or_else"); Err::<i32,u16>(0u16) }).unwrap());
                println!("{}", Err::<i32,u8>(6).or_else(|e| Ok::<i32,u16>(e as i32 + 1)).unwrap());
                println!("{}", Ok::<i32,u8>(8).unwrap_or_else(|_| { println!("wrong fallback"); 0 }));
                println!("{}", Err::<i32,u8>(9).unwrap_or_else(|e| { println!("fallback {}", e); 10 }));
            }"#,
                expect: Expect::Run("6\n4\n5\n7\n8\nfallback 9\n10\n"),
            },
            Case {
                name: "once_capture_dropped_on_selected_and_skipped_branches",
                source: r#"struct Token { id: i32 }
            impl Drop for Token { fn drop(&mut self) { println!("drop {}", self.id); } }
            fn main() {
                { let spare = Token { id: 2 }; let value = Ok::<Token,u8>(Token { id: 1 }).unwrap_or_else(move |_| spare); println!("value {}", value.id); }
                { let spare = Token { id: 3 }; let value = Err::<Token,u8>(9).unwrap_or_else(move |_| spare); println!("value {}", value.id); }
            }"#,
                expect: Expect::Run("drop 2\nvalue 1\ndrop 1\nvalue 3\ndrop 3\n"),
            },
            Case {
                name: "wrong_and_then_result",
                source: "fn unused(){let _=Ok::<i32,u8>(1).and_then(|n|n+1);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_error_type",
                source: "fn unused(){let _=Ok::<i32,u8>(1).and_then(|n|Err::<i32,bool>(true));}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn structural_enum_partial_moves_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "enum-partial-drop",
        &[
            Case {
                name: "enum_partial_complete_moves_and_reassignment",
                source: r#"
struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.id); } }
enum E { Pair(D, D), Single(D), Empty }
fn partial(b: bool) {
    let mut e = E::Pair(D { id: 1 }, D { id: 2 });
    if b {
        match e { E::Pair(a, _) => println!("moved {}", a.id), _ => () }
        e = E::Single(D { id: 3 });
    }
    println!("end");
}
fn main() {
    partial(true);
    partial(false);
    let e = E::Pair(D { id: 4 }, D { id: 5 });
    match e { E::Pair(a, b) => println!("both {} {}", a.id, b.id), _ => () }
    let e = E::Single(D { id: 6 });
    match e { E::Single(a) => println!("single {}", a.id), _ => () }
    let e = E::Empty;
    match e { E::Empty => println!("empty"), _ => () }
}
"#,
                expect: Expect::Run(
                    "moved 1\ndrop 1\ndrop 2\nend\ndrop 3\nend\ndrop 1\ndrop 2\nboth 4 5\ndrop 5\ndrop 4\nsingle 6\ndrop 6\nempty\n",
                ),
            },
            Case {
                name: "nested_enum_payload_partial_move",
                source: r#"
struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.id); } }
enum Inner { Pair(D, D), Empty }
enum Outer { Wrap(Inner, D), Empty }
fn main() {
    let e = Outer::Wrap(Inner::Pair(D { id: 1 }, D { id: 2 }), D { id: 3 });
    match e {
        Outer::Wrap(Inner::Pair(a, _), _) => println!("moved {}", a.id),
        _ => (),
    }
    println!("end");
}
"#,
                expect: Expect::Run("moved 1\ndrop 1\nend\ndrop 2\ndrop 3\n"),
            },
        ],
    );
}

#[test]
fn frozen_owned_enum_patterns_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "frozen-owned-enum",
        &[
            Case {
                name: "owned_match",
                source: include_str!("fixtures/drop_normal/unsupported/owned_match.rs"),
                expect: Expect::Run("matched owned value\ndrop matched owner\nafter match\n"),
            },
            Case {
                name: "owned_if_let",
                source: include_str!("fixtures/drop_normal/unsupported/owned_if_let.rs"),
                expect: Expect::Run("iflet owned value\ndrop iflet owner\nafter iflet\n"),
            },
        ],
    );
}

#[test]
fn enum_own_drop_move_is_invalid() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "enum-own-drop-move",
        &[Case {
            name: "enum_own_drop_cannot_move_payload",
            source: "struct D; impl Drop for D{fn drop(&mut self){}} enum E{One(D)} impl Drop for E{fn drop(&mut self){}} fn main(){let e=E::One(D);match e { E::One(d) => () }}",
            expect: Expect::Invalid("E0509"),
        }],
    );
}

fn result_collection_cases() -> [closure_oracle::Case; 7] {
    use closure_oracle::{Case, Expect};
    [
        Case {
            name: "all_ok_and_empty",
            source: r#"
fn main() {
    let values: Result<Vec<i32>, u8> = (0i32..3i32).map(|x| Ok::<i32, u8>(x * 2)).collect();
    let values = values.unwrap();
    println!("{} {} {} {}", values.len(), values[0], values[1], values[2]);
    let empty: Result<Vec<i32>, u8> = (0i32..0i32).map(|x| Ok::<i32, u8>(x)).collect();
    println!("{}", empty.unwrap().len());
}
"#,
            expect: Expect::Run("3 0 2 4\n0\n"),
        },
        Case {
            name: "first_and_middle_error_stop_polling",
            source: r#"
struct Probe { n: i32, fail: i32 }
impl Iterator for Probe {
    type Item = Result<i32, i32>;
    fn next(&mut self) -> Option<Result<i32, i32>> {
        println!("poll {}", self.n);
        let n = self.n;
        self.n += 1;
        if n == self.fail { Some(Err(n)) } else { Some(Ok(n)) }
    }
}
fn main() {
    let first: Result<Vec<i32>, i32> = Probe { n: 0, fail: 0 }.collect();
    println!("error {}", first.unwrap_err());
    let middle: Result<Vec<i32>, i32> = Probe { n: 0, fail: 2 }.collect();
    println!("error {}", middle.unwrap_err());
}
"#,
            expect: Expect::Run("poll 0\nerror 0\npoll 0\npoll 1\npoll 2\nerror 2\n"),
        },
        Case {
            name: "non_copy_payload_error_and_drop_order",
            source: r#"
#[derive(Debug)]
struct Token { id: i32 }
impl Drop for Token { fn drop(&mut self) { println!("drop {}", self.id); } }
struct Probe { n: i32, fail: bool }
impl Iterator for Probe {
    type Item = Result<Token, Token>;
    fn next(&mut self) -> Option<Result<Token, Token>> {
        let n = self.n;
        self.n += 1;
        println!("poll {}", n);
        if n == 2 {
            if self.fail { Some(Err(Token { id: 9 })) } else { None }
        } else { Some(Ok(Token { id: n })) }
    }
}
impl Drop for Probe { fn drop(&mut self) { println!("drop iterator"); } }
fn main() {
    let result: Result<Vec<Token>, Token> = Probe { n: 0, fail: true }.collect();
    match result { Ok(_) => println!("unexpected"), Err(error) => println!("error {}", error.id) }
    let result: Result<Vec<Token>, Token> = Probe { n: 0, fail: false }.collect();
    match result { Ok(values) => println!("len {}", values.len()), Err(_) => println!("unexpected") }
}
"#,
            expect: Expect::Run(
                "poll 0\npoll 1\npoll 2\ndrop iterator\ndrop 0\ndrop 1\nerror 9\ndrop 9\npoll 0\npoll 1\npoll 2\ndrop iterator\nlen 2\ndrop 0\ndrop 1\n",
            ),
        },
        Case {
            name: "wrong_item",
            source: "fn main(){let _:Result<Vec<i32>,u8>=(0i32..2i32).collect();}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "wrong_payload",
            source: "fn main(){let _:Result<Vec<bool>,u8>=(0i32..2i32).map(|x|Ok::<i32,u8>(x)).collect();}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "wrong_error",
            source: "fn main(){let _:Result<Vec<i32>,bool>=(0i32..2i32).map(|x|Ok::<i32,u8>(x)).collect();}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "invalid_unused",
            source: "fn bad(){let _:Result<Vec<i32>,u8>=(0i32..2i32).collect();}fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
    ]
}

#[test]
fn result_vec_all_ok_and_empty() {
    closure_oracle::check("result-collect-ok", &result_collection_cases()[0..1]);
}

#[test]
fn result_vec_short_circuit() {
    closure_oracle::check("result-collect-err", &result_collection_cases()[1..2]);
}

#[test]
fn result_vec_non_copy_drops() {
    closure_oracle::check("result-collect-drop", &result_collection_cases()[2..3]);
}

#[test]
fn result_vec_invalid_types() {
    closure_oracle::check("result-collect-invalid", &result_collection_cases()[3..]);
}

#[test]
fn result_map_and_map_err_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "result-map",
        &[
            Case {
                name: "map_transforms_ok_and_passes_err",
                source: r#"
fn main() {
    let a: Result<i32, bool> = Ok(4);
    let b: Result<i32, bool> = Err(true);
    let a2: Result<u64, bool> = a.map(|x| (x * 3) as u64);
    let b2: Result<u64, bool> = b.map(|x| (x * 3) as u64);
    println!("{} {}", a2.unwrap(), b2.unwrap_err());
}
"#,
                expect: Expect::Run("12 true\n"),
            },
            // A user struct produced by a generic closure result currently
            // gets two nominal identities in the existing checker (graph
            // realization vs source slot), the same failure 19's `fold`
            // hits. Scalar error types keep this case about map_err.
            Case {
                name: "map_err_changes_error_type_and_runs_only_on_err",
                source: r#"
fn main() {
    let a: Result<i32, u8> = Err(7);
    let b: Result<i32, u8> = Ok(5);
    let a2: Result<i32, i64> = a.map_err(|e| e as i64 * 100);
    let b2: Result<i32, i64> = b.map_err(|e| { println!("not called"); e as i64 });
    println!("{}", a2.unwrap_err());
    println!("{}", b2.unwrap());
}
"#,
                expect: Expect::Run("700\n5\n"),
            },
            Case {
                name: "map_err_composes_with_try",
                source: r#"
fn parse(ok: bool) -> Result<u32, u8> { if ok { Ok(9) } else { Err(2) } }
fn run(ok: bool) -> Result<u32, i64> {
    // `parse(ok).map_err(..)?` without this annotation is rejected by the
    // current checker ("unresolved method generic arguments"): it cannot
    // infer the closure output from the enclosing `?` conversion. The
    // semantic core resolves it (SEMANTIC-CORE.md); program 15 needs it.
    let mapped: Result<u32, i64> = parse(ok).map_err(|e| e as i64 - 10);
    let v = mapped?;
    Ok(v + 1)
}
fn main() {
    println!("{}", run(true).unwrap());
    println!("{}", run(false).unwrap_err());
}
"#,
                expect: Expect::Run("10\n-8\n"),
            },
            Case {
                name: "map_err_closure_cannot_take_ok_type",
                source: "fn main(){let r:Result<i32,u8>=Ok(1);let _=r.map_err(|e:i32| e);}",
                expect: Expect::Invalid("E0631"),
            },
        ],
    );
}
