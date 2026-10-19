//! Real stored closures with canonical projected callable bounds.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn stored_closure_projected_argument_executes_with_closed_proof() {
    oracle::stock_native(
        r#"
trait Has { type Out; }
struct S;
impl Has for S { type Out = i32; }
fn apply<I: Has, F: Fn(<I as Has>::Out) -> i32>(f: F, x: <I as Has>::Out) -> i32 { f(x) }
fn main() { let f = |x| x + 1; println!("{}", apply::<S, _>(f, 2)); }
"#,
    );
}

#[test]
fn stored_closure_projected_u8_argument_executes_with_closed_proof() {
    oracle::stock_native(
        "trait Has{type Out;}struct S;impl Has for S{type Out=u8;}fn apply<I:Has,F:Fn(<I as Has>::Out)->u8>(f:F,x:<I as Has>::Out)->u8{f(x)}fn main(){let f=|x|x;let n:u8=apply::<S,_>(f,7);println!(\"{}\",n);}",
    );
}

#[test]
fn later_owner_argument_resolves_projected_closure_input() {
    oracle::stock_native(
        "trait Has { type Out; }\nstruct S;\nimpl Has for S { type Out = u8; }\nfn apply<I: Has, F: Fn(I::Out) -> u8>(f: F, value: I::Out, owner: I) -> u8 { f(value) }\nfn main() { let f = |x| x; let n: u8 = apply(f, 7, S); println!(\"{}\", n); }\n",
    );
}

#[test]
fn independent_callable_equality_preserves_unassigned_family() {
    oracle::stock_native(
        r#"
trait Has { type Out; }
struct S;
impl Has for S { type Out = i32; }
fn independent<I: Has, F: Fn(i32) -> i32>(f: F, x: I::Out) -> I::Out { let n = f(1); x }
fn main() { let f = |x| x + 1; println!("{}", independent::<S, _>(f, 42)); }
"#,
    );
}

#[test]
fn unused_projected_callable_bounds_reject_invalid_memberships_and_cycles() {
    for source in [
        "trait A:B{}trait B:A{} fn unused<T:A>(){} fn main(){}",
        "trait Grow<T>:Grow<(T,)>{} fn unused<I:Grow<i32>>(){} fn main(){}",
        "trait Has{type Out;} trait Apply<A>{} fn bad<I,F:Apply<I::Out>>(){} fn main(){}",
        "trait Pair<A,B>{type Out;} trait Has{type Out;fn bad()-><Self as Pair<Self::Out,i32>>::Out;} fn main(){}",
        "trait Has{type Out;} struct S;impl Has for S{type Out=i32;}fn bad<I:Has,F:Fn(I::Out)->i32>(f:F,x:I::Out)->i32{f(true)}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
