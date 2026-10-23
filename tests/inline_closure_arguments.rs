//! Inline closure literals passed as the single argument of a generic call.
//! The closure lowers to a generated item path, which is not a local binding,
//! so the source inventory must not treat it as a local lookup.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_opaque_callable_composition_matches_stock() {
    oracle::check(
        "nested_opaque_composition",
        &[
            Case {
                name: "compose_owned_closures",
                source: r#"fn compose<F:Fn(i32)->i32,G:Fn(i32)->i32>(f:F,g:G)->impl Fn(i32)->i32{move|x|f(g(x))}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let c=compose(add(2),add(3));println!("{}",c(7));}"#,
                expect: Expect::Run("12\n"),
            },
            Case {
                name: "generic_argument_output_composition",
                source: r#"fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move|x|g(f(x))}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let c=compose(|x:i32|x*2,add(1));println!("{}",c(3));}"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "nested_opaque_owners",
                source: r#"fn compose<F:Fn(i32)->i32,G:Fn(i32)->i32>(f:F,g:G)->impl Fn(i32)->i32{move|x|f(g(x))}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let c=compose(compose(add(1),add(2)),add(3));println!("{}",c(7));}"#,
                expect: Expect::Run("13\n"),
            },
        ],
    );
}

#[test]
fn nested_opaque_public_contracts_reject_invalid_unused_calls() {
    oracle::check(
        "nested_opaque_invalid",
        &[
            Case {
                name: "nested_wrong_output",
                source: r#"fn compose<F:Fn(i32)->i32,G:Fn(i32)->i32>(f:F,g:G)->impl Fn(i32)->i32{move|x|f(g(x))}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn accept<F:Fn(i32)->bool>(f:F)->bool{f(0)}fn unused(){accept(compose(add(1),add(2)));}fn main(){}"#,
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "nested_hidden_fn_is_not_public",
                source: r#"fn compose<F:Fn(i32)->i32,G:Fn(i32)->i32>(f:F,g:G)->impl Fn(i32)->i32{move|x|f(g(x))}fn add(n:i32)->impl FnMut(i32)->i32{move|x|x+n}fn unused(){let _c=compose(add(1),add(2));}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn opaque_callable_bounds_drive_generic_calls() {
    oracle::check(
        "opaque_callable_generic_calls",
        &[
            Case {
                name: "owned_opaque",
                source: "fn ap<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){println!(\"{}\",ap(add(5),10));}",
                expect: Expect::Run("15\n"),
            },
            Case {
                name: "generic_output_inference",
                source: "fn ap<A,B,F:Fn(A)->B>(f:F,x:A)->B{f(x)}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){println!(\"{}\",ap(add(2),5));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "wrong_public_output",
                source: "fn ap<F:Fn(i32)->bool>(f:F)->bool{f(1)}fn add()->impl Fn(i32)->i32{|x|x+1}fn unused(){ap(add());}fn main(){}",
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "hidden_fn_not_public",
                source: "fn ap<F:Fn(i32)->i32>(f:F)->i32{f(1)}fn add()->impl FnMut(i32)->i32{|x|x+1}fn unused(){ap(add());}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn inline_closure_arguments_to_generic_functions() {
    oracle::check(
        "inline_closure_arguments",
        &[
            Case {
                name: "annotated",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){println!(\"{}\",ap(|x:u32|x*2));}",
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "inferred",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){println!(\"{}\",ap(|x|x*2));}",
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "capturing",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){let k=3u32;println!(\"{}\",ap(|x|x*k));}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "wrong_result",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn unused(){ap(|x|true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn closures_stored_in_generic_structs_through_let_bindings() {
    oracle::check(
        "closure_struct_bindings",
        &[
            Case {
                name: "field_call",
                source: "struct W<F>{f:F}fn wrap<F:Fn(u32)->u32>(f:F)->W<F>{W{f}}fn main(){let w=wrap(|x|x*2);println!(\"{}\",(w.f)(3));}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "method_call",
                source: "struct W<F>{f:F}impl<F:Fn(u32)->u32>W<F>{fn call(&self,x:u32)->u32{(self.f)(x)}}fn wrap<F:Fn(u32)->u32>(f:F)->W<F>{W{f}}fn main(){let w=wrap(|x:u32|x*2);println!(\"{}\",w.call(3));}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "local_literal_field_call",
                source: "struct W<F>{f:F}fn main(){let k=3;let w=W{f:|x:i32|x+k};println!(\"{}\",(w.f)(4));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "distinct_closures_distinct_applications",
                source: "struct W<F>{f:F}fn main(){let a=W{f:|x:u32|x+1};let b=W{f:|x:u32|x*10};println!(\"{} {}\",(a.f)(1),(b.f)(2));}",
                expect: Expect::Run("2 20\n"),
            },
            Case {
                name: "move_capture_with_scalar_field",
                source: "struct W<F>{f:F,n:i32}fn main(){let k=5;let w=W{f:move|x:i32|x+k,n:2};println!(\"{}\",(w.f)(w.n));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "distinct_closures_do_not_merge",
                source: "struct W<F>{f:F}fn main(){let mut a=W{f:|x:u32|x+1};a=W{f:|x:u32|x+2};}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "closure_application_is_not_scalar_application",
                source: "struct W<F>{f:F}fn main(){let a=W{f:|x:u32|x+1};let b:W<u32>=a;}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unconstrained_still_rejected",
                source: "struct W<T>{t:T}fn mk<T>()->W<T>{loop{}}fn main(){let w=mk();}",
                expect: Expect::Invalid("E0282"),
            },
        ],
    );
}
