//! Shorthand projection qualifiers reuse a bound application, not its equality list.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn shorthand_keeps_ordered_trait_arguments_and_original_equalities() {
    oracle::stock_native(
        "trait Has<A>{type Out;}struct S;impl Has<bool> for S{type Out=u8;}fn id<I:Has<bool,Out=u8>>(x:I::Out)->u8{x}fn main(){assert_eq!(id::<S>(7u8),7u8);}",
    );
}

#[test]
fn projected_bounds_do_not_authorize_invalid_qualifiers_or_wrong_owners() {
    for source in [
        "trait Has{type Out;}fn bad<I:Has<Out=u8>>(x:<I as Has<Out=u8>>::Out){}fn main(){}",
        "trait Has{type Out;}fn bad<I:Has<Out=u8>>(x:I::Out)->bool{x}fn main(){}",
        "trait Has<A>{type Out;}struct S;impl Has<bool> for S{type Out=u8;}fn id<I:Has<i32,Out=u8>>(x:I::Out)->u8{x}fn bad(){id::<S>(7u8);}fn main(){}",
        "trait A{type Out;}trait B{type Out;}fn bad<I:A+B>(x:I::Out){}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
