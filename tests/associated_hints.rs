//! Associated result hints must preserve owner actuals and infer only method slots.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn generic_static_result_hints_preserve_owner_and_method_arguments() {
    oracle::stock_native(
        "struct Pair<A,B>{a:A,b:B}impl<A:Copy,B:Copy> Pair<A,B>{fn choose<U>(a:A,b:B,value:U)->U{value}}fn main(){assert_eq!(Pair::<i32,bool>::choose(3,true,7u8),7u8);assert_eq!(Pair::<bool,i32>::choose(false,5,11i32),11);assert_eq!(Pair::<i32,bool>::choose::<u16>(3,true,13),13u16);}",
    );
}

#[test]
fn static_method_inference_does_not_change_owner_order_or_bounds() {
    for source in [
        "struct Pair<A,B>{a:A,b:B}impl<A:Copy,B:Copy> Pair<A,B>{fn choose<U>(a:A,b:B,value:U)->U{value}}fn bad(){Pair::<i32,bool>::choose(true,3,7u8);}fn main(){}",
        "struct Value<T>{value:T}impl<T:Copy> Value<T>{fn identity<U>(value:U)->U{value}}fn bad(){let x:i32=Value::<i32>::identity(true);}fn main(){}",
        "struct Value<T>{value:T}impl<T:Copy> Value<T>{fn identity<U>(value:U)->U{value}}fn bad(){Value::<i32>::identity::<u8>(true);}fn main(){}",
        "struct Value<T>{value:T}impl<T:Copy> Value<T>{fn identity<U>(value:U)->U{value}}fn bad(){Value::<String>::identity(7);}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
