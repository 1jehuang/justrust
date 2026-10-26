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
