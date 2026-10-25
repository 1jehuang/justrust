//! Stock differential coverage for ordinary Rust-source prelude methods.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn into_selects_lifetime_generic_from_by_expected_type() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "into-from",
        &[
            Case {
                name: "str_into_string_annotated_and_argument",
                source: "fn show(s:String){println!(\"{s}\");}fn main(){let s:String=\"x\".into();println!(\"{s}\");show(\"tri\".into());}",
                expect: Expect::Run("x\ntri\n"),
            },
            Case {
                name: "unused_bad_into_target_is_e0277",
                source: "struct N;fn unused(){let _n:N=\"x\".into();}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "i32_into_i64_annotated",
                source: "fn main(){let n:i64=5i32.into();println!(\"{n}\");}",
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "u8_into_u32_argument",
                source: "fn show(n:u32){println!(\"{n}\");}fn main(){let b=200u8;let w:u32=b.into();show(b.into());println!(\"{w}\");}",
                expect: Expect::Run("200\n200\n"),
            },
        ],
    );
    // Stock rejects an unannotated numeric `into()` with E0283. The native
    // compiler must reject it too (currently at its explicit inference
    // boundary) and never publish an executable.
    for workers in [1, 8] {
        for check_only in [true, false] {
            let compiled = justrust_engine::compiler::compile(
                "fn main(){let n=5i32.into();let _=n;}".into(),
                workers,
                check_only,
            );
            assert!(
                compiled.is_err(),
                "unannotated into accepted workers={workers} CHECK={check_only}"
            );
        }
    }
}

#[test]
fn vec_clone_is_elementwise_in_order() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "vec-clone",
        &[
            Case {
                name: "independent_copy",
                source: "fn main(){let v=vec![1i32,2,3];let mut w=v.clone();w.push(4);println!(\"{} {}\",v.len(),w.len());}",
                expect: Expect::Run("3 4\n"),
            },
            Case {
                name: "clones_each_element_once_front_to_back",
                source: "struct C(i32);impl Clone for C{fn clone(&self)->C{println!(\"c{}\",self.0);C(self.0)}}fn main(){let v=vec![C(1),C(2),C(3)];let w=v.clone();println!(\"{}\",w.len());}",
                expect: Expect::Run("c1\nc2\nc3\n3\n"),
            },
            // A bounded prelude method on an element without Clone reports
            // stock's unmet-bound E0599, not an inference boundary.
            Case {
                name: "non_clone_element_is_e0599",
                source: "struct N;fn main(){let v=vec![N];let _w=v.clone();}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "option_non_clone_payload_is_e0599",
                source: "struct N;fn main(){let _x=Some(N).clone();}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "unused_item_non_clone_is_e0599",
                source: "struct N;fn unused(){let v=vec![N];let _w=v.clone();}fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "user_trait_unmet_impl_bound_is_e0599",
                source: "struct N;struct W<T>(T);trait C{fn c(&self);}impl<T:Clone> C for W<T>{fn c(&self){}}fn main(){W(N).c();}",
                expect: Expect::Invalid("E0599"),
            },
        ],
    );
}

/// A method name with no inherent, prelude-trait or in-scope trait candidate
/// on a final receiver type is stock's E0599, even in unused functions.
#[test]
fn absent_method_on_final_receiver_is_e0599() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "absent-method",
        &[
            Case {
                name: "slice_receiver",
                source: "fn f(v:&[i32])->i32{v.nonexistent()}fn main(){}",
                expect: Expect::Invalid("no method named `nonexistent` found"),
            },
            Case {
                name: "struct_receiver",
                source: "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn f(s:&S)->i32{s.nonexistent()}fn main(){println!(\"{}\",S{x:1}.get());}",
                expect: Expect::Invalid("no method named `nonexistent` found"),
            },
            Case {
                name: "generic_param_receiver",
                source: "trait A{fn a(&self);}fn f<T:A>(t:T){t.nonexistent()}fn main(){}",
                expect: Expect::Invalid("no method named `nonexistent` found"),
            },
            Case {
                name: "inferred_vec_receiver",
                source: "fn f(){let v=Vec::new();v.nonexistent();let _:Vec<i32>=v;}fn main(){}",
                expect: Expect::Invalid("no method named `nonexistent` found"),
            },
            Case {
                name: "existing_method_still_runs",
                source: "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn f(v:&[i32])->usize{v.len()}fn main(){println!(\"{} {}\",S{x:4}.get(),f(&[1,2]));}",
                expect: Expect::Run("4 2\n"),
            },
        ],
    );
    for (source, receiver) in [
        (
            "fn f(v:&[i32])->i32{v.nonexistent()}fn main(){}",
            "`&[i32]`",
        ),
        ("struct S;fn f(s:&S){s.nonexistent()}fn main(){}", "`&S`"),
        ("fn f<T>(t:T){t.nonexistent()}fn main(){}", "`T`"),
    ] {
        for workers in [1, 8] {
            let error = compiler::compile(source.into(), workers, true)
                .err()
                .expect("absent method accepted")
                .to_string();
            assert!(
                error.contains("E0599: no method named `nonexistent` found for ")
                    && error.contains(receiver),
                "workers={workers}: {error}"
            );
        }
    }
    // A receiver that is still an inference variable is not final: stock
    // reports E0282 there and the native boundary must stay explicit.
    let error = compiler::compile(
        "fn f(){let x=None;x.nonexistent();}fn main(){}".into(),
        1,
        true,
    )
    .err()
    .expect("unknown receiver accepted")
    .to_string();
    assert!(!error.contains("E0599"), "{error}");
}

/// `Option<&T>::unwrap` and friends return a reference carried by the by-value
/// generic `self`. The result must keep the input loan alive (E0597 when the
/// referent dies first) without rejecting ordinary in-scope uses.
#[test]
fn by_value_generic_self_reference_results_keep_input_loans() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "generic-self-ref-results",
        &[
            Case {
                name: "in_scope_unwrap_expect_unwrap_or",
                source: r#"
fn main() {
    let v = 7;
    let x = 8;
    let r = Some(&v).unwrap();
    let e = Some(&x).expect("x");
    let none: Option<&i32> = None;
    let u = none.unwrap_or(&v);
    let q: Result<&i32, ()> = Ok(&x);
    let k = q.unwrap();
    let s = *r + *e * 10 + *u * 100 + *k * 1000;
    println!("{}", s);
}
"#,
                expect: Expect::Run("8787\n"),
            },
            Case {
                name: "unwrap_or_variants_in_scope",
                source: r#"
fn main() {
    let v = 7;
    let w = 8;
    let n: Option<&i32> = None;
    let a = n.unwrap_or(&w);
    let b = Some(&v).unwrap_or(&w);
    let r: Result<&i32, ()> = Err(());
    let c = r.unwrap_or(&v);
    let r2: Result<&i32, ()> = Ok(&w);
    let d = r2.unwrap_or(&v);
    let s = *a + *b * 10 + *c * 100 + *d * 1000;
    println!("{}", s);
}
"#,
                expect: Expect::Run("8778\n"),
            },
            Case {
                name: "unwrap_or_fallback_escapes_scope",
                source: r#"
fn main() { let r: &i32; { let v = 7; let n: Option<&i32> = None; r = n.unwrap_or(&v); } let k = *r; println!("{}", k); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "result_unwrap_or_fallback_escapes_scope",
                source: r#"
fn main() { let r: &i32; { let v = 7; let o: Result<&i32, ()> = Err(()); r = o.unwrap_or(&v); } let k = *r; println!("{}", k); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unwrap_escapes_scope",
                source: r#"
fn main() { let r: &i32; { let v = 7; let o = Some(&v); r = o.unwrap(); } println!("{}", *r); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "expect_escapes_scope",
                source: r#"
fn main() { let r: &i32; { let v = 7; r = Some(&v).expect("x"); } let n = *r; println!("{}", n); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unwrap_or_escapes_scope",
                source: r#"
fn main() { let w = 1; let r: &i32; { let v = 7; r = Some(&v).unwrap_or(&w); } let n = *r; println!("{}", n); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "result_unwrap_escapes_scope",
                source: r#"
fn main() { let r: &i32; { let v = 7; let o: Result<&i32, ()> = Ok(&v); r = o.unwrap(); } let n = *r; println!("{}", n); }
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "unused_item_unwrap_escapes_scope",
                source: r#"
fn unused() -> i32 { let r: &i32; { let v = 7; r = Some(&v).unwrap(); } *r }
fn main() { println!("ok"); }
"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn result_or_changes_error_type_without_changing_value_or_evaluation() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "result-or",
        &[
            Case {
                name: "different_error_types_and_eager_fallback",
                source: r#"
fn fallback() -> Result<i32, bool> { println!("fallback"); Err(true) }
fn main() {
    let ok: Result<i32, i32> = Ok(7);
    let out: Result<i32, bool> = ok.or(fallback());
    println!("{}", out.unwrap());
    let err: Result<i32, i32> = Err(9);
    let out: Result<i32, bool> = err.or(fallback());
    println!("{}", out.unwrap_err());
}
"#,
                expect: Expect::Run("fallback\n7\nfallback\ntrue\n"),
            },
            Case {
                name: "moves_noncopy_value_and_replacement_error",
                source: r#"
struct Value { n: i32 }
struct Old { n: i32 }
struct New { n: i32 }
fn main() {
    let ok: Result<Value, Old> = Ok(Value { n: 11 });
    let out: Result<Value, New> = ok.or(Err(New { n: 99 }));
    match out { Ok(v) => println!("{}", v.n), Err(e) => println!("bad {}", e.n) }
    let err: Result<Value, Old> = Err(Old { n: 3 });
    let out: Result<Value, New> = err.or(Err(New { n: 13 }));
    match out { Ok(v) => println!("bad {}", v.n), Err(e) => println!("{}", e.n) }
}
"#,
                expect: Expect::Run("11\n13\n"),
            },
            Case {
                name: "value_type_cannot_change",
                source: "fn main(){let r:Result<i32,i32>=Ok(7);let other:Result<bool,bool>=Ok(true);let _=r.or(other);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "consumes_original_noncopy_result",
                source: "struct V{n:i32}fn main(){let r:Result<V,i32>=Ok(V{n:7});let _:Result<V,bool>=r.or(Err(true));let _=r.is_ok();}",
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}

static NEXT: AtomicU64 = AtomicU64::new(0);

fn differential(source: &str) -> Output {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "jr-prelude-methods-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("input.rs");
    let stock = dir.join("stock");
    let native = dir.join("native");
    fs::write(&input, source).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=prelude_methods",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .expect("stock rustc oracle must be installed");
    assert!(
        output.status.success(),
        "stock rejected source: {}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = Command::new(&stock).output().unwrap();
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("native rejected source: {error}\n{source}"));
        fs::write(&native, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&native, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(&native).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code(), "{source}");
        assert_eq!(actual.stdout, expected.stdout, "{source}");
        if expected.status.success() {
            assert_eq!(actual.stderr, expected.stderr, "{source}");
        }
    }
    fs::remove_dir_all(&dir).unwrap();
    expected
}

#[test]
fn panicking_unwrap_matches_stock_exit() {
    for source in [
        "fn main() { let value: Option<i32> = None; value.unwrap(); }",
        "fn main() { let value: Result<i32, bool> = Err(true); value.unwrap(); }",
    ] {
        let expected = differential(source);
        assert!(!expected.status.success());
    }
}

#[test]
fn option_and_result_methods_match_stock() {
    let output = differential(
        r#"
fn main() {
    let some: Option<i32> = Some(7);
    if some.is_some() && !some.is_none() { println!("some"); }
    if some.unwrap_or(0) == 7 { println!("value"); }
    let none: Option<i32> = None;
    if none.is_none() && !none.is_some() { println!("none"); }
    if none.unwrap_or(4) == 4 { println!("default"); }
    let ok: Result<i32, bool> = Ok(9);
    if ok.is_ok() && !ok.is_err() { println!("ok"); }
    if ok.unwrap_or(0) == 9 { println!("value"); }
    let err: Result<i32, bool> = Err(true);
    if err.is_err() && !err.is_ok() { println!("err"); }
    if err.unwrap_or(5) == 5 { println!("default"); }
}
"#,
    );
    assert!(output.status.success());
}

#[test]
fn result_constructor_alias_paths_and_user_shadowing_match_stock() {
    differential(
        r#"
fn main() {
    let ok: std::result::Result<i32, bool> = core::result::Result::Ok(3);
    match ok { Ok(value) => if value == 3 { println!("path"); }, Err(_) => {} }
}
"#,
    );
    differential(
        r#"
enum Result { Mine }
mod mine { pub fn Ok(x: i32) -> i32 { x + 1 } }
use mine::Ok;
fn main() {
    let x = Result::Mine;
    match x { Result::Mine => println!("shadow") }
    if Ok(2) == 3 { println!("import"); }
}
"#,
    );
}

#[test]
fn predicates_borrow_noncopy_values_and_defaults_are_eager() {
    differential(
        r#"
struct Value { n: i32 }
fn default() -> Value { println!("default evaluated"); Value { n: 5 } }
fn main() {
    let value: Option<Value> = Some(Value { n: 8 });
    if value.is_some() && !value.is_none() { println!("borrow"); }
    let taken = value.unwrap_or(default());
    if taken.n == 8 { println!("moved"); }
    let result: Result<Value, Value> = Err(Value { n: 4 });
    if result.is_err() && !result.is_ok() { println!("borrow err"); }
    let fallback = result.unwrap_or(default());
    if fallback.n == 5 { println!("fallback"); }
}
"#,
    );
}

#[test]
fn iterator_max_is_last_and_min_is_first_among_ties() {
    differential(
        r#"
#[derive(Debug)]
struct K(u8, u8);
impl PartialEq for K { fn eq(&self, o: &K) -> bool { self.0 == o.0 } }
impl Eq for K {}
impl PartialOrd for K { fn partial_cmp(&self, o: &K) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) } }
impl Ord for K { fn cmp(&self, o: &K) -> std::cmp::Ordering { self.0.cmp(&o.0) } }
fn main() {
    let v = vec![3i64, -1, 7, 7, -1];
    println!("{:?} {:?}", v.iter().copied().max(), v.iter().copied().min());
    println!("{:?} {}", vec![K(1, 0), K(2, 1), K(0, 2), K(2, 3), K(0, 4)].into_iter().max(), 0);
    println!("{:?} {}", vec![K(1, 0), K(2, 1), K(0, 2), K(2, 3), K(0, 4)].into_iter().min(), 0);
    let e: Vec<u8> = Vec::new();
    println!("{:?} {:?}", e.clone().into_iter().max(), e.into_iter().min());
    println!("{:?}", (1u32..5).min());
}
"#,
    );
}

#[test]
fn unsuffixed_literal_trait_method_receivers_use_literal_fallback() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "literal-receiver-fallback",
        &[Case {
            name: "to_string_and_ord_on_unsuffixed_literals",
            source: "fn main(){let t=5.to_string();println!(\"{}\",t);let b=2.5;println!(\"{}\",b.to_string());println!(\"{}\",7.max(3));}",
            expect: Expect::Run("5\n2.5\n7\n"),
        }],
    );
}
