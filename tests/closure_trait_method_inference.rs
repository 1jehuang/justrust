//! Generic method schemas constrain closure terms before checked metadata exists.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn opaque_callable_bound_does_not_wait_for_hidden_owner_closures() {
    let cases = [
        Case {
            name: "compose_opaque_with_inferred_owner_closure",
            source: "fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move|x|g(f(x))}fn main(){let f=compose(|x:i32|x*2,add(1));println!(\"{}\",f(3));}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "opaque_callable_wrong_input_remains_invalid",
            source: "fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move|x|g(f(x))}fn main(){let f=compose(|x:i32|x*2,add(1));let _=f(true);}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "opaque_callable_wrong_output_remains_invalid",
            source: "fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move|x|g(f(x))}fn main(){let f=compose(|x:i32|x*2,add(1));let _:bool=f(3);}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "opaque_ordinary_call_member_wrong_input_remains_invalid",
            source: "trait Fake{fn call(&self,x:i32)->i32;}struct S;impl Fake for S{fn call(&self,x:i32)->i32{x}}fn make<F>(f:F)->impl Fake{S}fn main(){let f=make(|x:i32|x);let _=f.call(true);}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "same_spelling_call_bound_is_not_callable_syntax",
            source: "trait Fake{fn call(&self,x:i32)->i32;}struct S;impl Fake for S{fn call(&self,x:i32)->i32{x}}fn make<F>(f:F)->impl Fake{S}fn main(){let f=make(|x:i32|x);let _=f(3);}",
            expect: Expect::Invalid("E0618"),
        },
    ];
    oracle::check("opaque_callable_deferred_owner", &cases);
    oracle::check(
        "opaque_ordinary_call_member",
        &[Case {
            name: "same_spelling_call_bound_allows_explicit_method",
            source: "trait Fake{fn call(&self,x:i32)->i32;}struct S;impl Fake for S{fn call(&self,x:i32)->i32{x}}fn make<F>(f:F)->impl Fake{S}fn main(){let f=make(|x:i32|x);println!(\"{}\",f.call(3));}",
            expect: Expect::Run("3\n"),
        }],
    );
    for case in &cases[1..] {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure = match justrust_engine::compiler::compile_observed(
                    case.source.into(),
                    workers,
                    check_only,
                ) {
                    Ok(_) => panic!("{}: invalid opaque callable accepted", case.name),
                    Err(failure) => failure,
                };
                assert!(
                    failure.resource_usage.first_failure.is_none()
                        && !failure.error.to_string().contains("unsupported"),
                    "{} workers={workers} check={check_only}: {}",
                    case.name,
                    failure.error,
                );
            }
        }
    }
}

#[test]
fn canonical_trait_method_inputs_constrain_closures() {
    oracle::check(
        "trait_method_closure_inputs",
        &[Case {
            name: "source_u8_input",
            source: "struct Seed;trait Transform{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Transform for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn main(){assert_eq!(Seed.apply(|x|x+1),8u8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn associated_trait_method_input_constrains_closure() {
    oracle::check(
        "trait_method_associated_closure_input",
        &[Case {
            name: "associated_input",
            source: "struct Seed;trait Transform{type Item;fn apply<F:FnOnce(Self::Item)->i32>(self,f:F)->i32;}impl Transform for Seed{type Item=i32;fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32{f(7)}}fn main(){assert_eq!(Seed.apply(|x|x+1),8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn trait_method_constraints_do_not_choose_ambiguous_or_invalid_calls() {
    oracle::check(
        "trait_method_closure_invalid",
        &[
            Case {
                name: "ambiguous_traits",
                source: "struct Seed;trait Left{fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32;}trait Right{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Left for Seed{fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32{f(7)}}impl Right for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn unused(){Seed.apply(|x|x);}fn main(){}",
                expect: Expect::Invalid("E0034"),
            },
            Case {
                name: "wrong_result",
                source: "struct Seed;trait Transform{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Transform for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn unused(){Seed.apply(|x|true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn inferred_owner_uses_impl_closure_bounds() {
    oracle::check(
        "inferred_owner_closure_bounds",
        &[
            Case {
                name: "memo_cache",
                source: r#"use std::collections::HashMap;
use std::hash::Hash;
struct Cache<K, V, F: Fn(&K) -> V> { f: F, map: HashMap<K, V> }
impl<K: Eq + Hash + Clone, V: Clone, F: Fn(&K) -> V> Cache<K, V, F> {
    fn new(f: F) -> Self { Cache { f, map: HashMap::new() } }
    fn get(&mut self, k: K) -> V { if let Some(v) = self.map.get(&k) { return v.clone(); } let v = (self.f)(&k); self.map.insert(k, v.clone()); v }
}
struct Apply<A, B, F: FnMut(A) -> B> { f: F, last: Option<B>, _a: std::marker::PhantomData<A> }
impl<A, B: Copy, F: FnMut(A) -> B> Apply<A, B, F> {
    fn new(f: F) -> Self { Apply { f, last: None, _a: std::marker::PhantomData } }
    fn run(&mut self, a: A) -> B { let b = (self.f)(a); self.last = Some(b); b }
}
fn main() {
    let mut c = Cache::new(|x: &u64| (1..=*x).product::<u64>());
    for k in [5, 10, 5, 3] { print!("{} ", c.get(k)); }
    println!("{}", c.map.len());
    let mut s = Cache::new(|s: &String| s.len());
    println!("{} {}", s.get(String::from("abc")), s.get(String::from("abc")));
    let mut n = 0;
    let mut a = Apply::new(|x: i32| { n += 1; x * 2 });
    println!("{} {}", a.run(3), a.run(4));
    println!("{:?}", a.last);
}
"#,
                expect: Expect::Run("120 3628800 120 6 3\n3 3\n6 8\nSome(8)\n"),
            },
            Case {
                name: "closure_bound_not_satisfied",
                source: "struct C<K, V, F: Fn(&K) -> V> { f: F, k: Option<K>, v: Option<V> } impl<K, V, F: Fn(&K) -> V> C<K, V, F> { fn new(f: F) -> Self { C { f, k: None, v: None } } } fn unused() { let _c = C::new(|x: u64| x); } fn main() {}",
                expect: Expect::Invalid("E0631"),
            },
            Case {
                name: "closure_result_mismatch",
                source: "struct C<K, V: Copy, F: Fn(&K) -> V> { f: F, k: Option<K>, v: Option<V> } impl<K, V: Copy, F: Fn(&K) -> V> C<K, V, F> { fn new(f: F) -> Self { C { f, k: None, v: None } } } fn unused() { let _c = C::new(|x: &u64| String::new()); } fn main() {}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
