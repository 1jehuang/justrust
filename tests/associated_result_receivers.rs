//! Associated factory result hints must preserve graph-realized receiver types.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn explicit_factory_result_receivers_execute() {
    oracle::stock_native(
        "struct Holder<T>{value:T}impl<T> Holder<T>{fn wrap(value:T)->Option<T>{Some(value)}fn make(value:T)->Self{Self{value}}fn marker(&self)->u32{7}}fn main(){println!(\"{} {}\",Holder::<u32>::wrap(3u32).is_some(),Holder::<u32>::make(4u32).marker());}",
    );
}

#[test]
fn unused_factory_chain_with_wrong_argument_is_rejected() {
    oracle::stock_and_native_reject(
        "struct Holder<T>{value:T}impl<T> Holder<T>{fn wrap(value:T)->Option<T>{Some(value)}}fn unused(){let _=Holder::<u32>::wrap(false).is_some();}fn main(){}",
    );
}

#[test]
fn unused_factory_chain_with_missing_member_is_rejected() {
    oracle::stock_and_native_reject(
        "struct Holder<T>{value:T}impl<T> Holder<T>{fn make(value:T)->Self{Self{value}}}fn unused(){let _=Holder::<u32>::make(4u32).missing();}fn main(){}",
    );
}
