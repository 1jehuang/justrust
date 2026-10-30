//! Associated factory result hints must preserve graph-realized receiver types.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn closure_output_closes_pending_associated_result() {
    oracle::stock_native(
        "trait Per<T>{type Output;const VALUE:Self::Output;}struct Day;impl Per<u64> for Day{type Output=u8;const VALUE:u8=7;}impl Day{fn mapped<T,F>(f:F)-><Self as Per<T>>::Output where Self:Per<T>,F:FnOnce()->T{let _=f;Self::VALUE}}fn main(){assert_eq!(Day::mapped(||9u64),7u8);println!(\"{}\",Day::mapped(||9u64));}",
    );
}

#[test]
fn associated_projection_output_does_not_infer_its_argument() {
    oracle::stock_and_native_reject(
        "trait Per<T>{type Output;}struct Day;struct Week;impl Per<Day> for Day{type Output=u8;}impl Per<Week> for Day{type Output=u8;}impl Day{fn make<T>()-><Self as Per<T>>::Output where Self:Per<T>{panic!()}}fn main(){let _:u8=Day::make();}",
    );
}

#[test]
fn associated_conversion_result_in_nested_assertion() {
    oracle::stock_native(
        "trait Per<T>{type Output;const VALUE:Self::Output;}#[derive(Clone,Copy)]struct Day;#[derive(Clone,Copy)]struct Week;impl Per<Week> for Day{type Output=u8;const VALUE:u8=7;}impl Day{const fn per<T:Copy>(_:T)-><Self as Per<T>>::Output where Self:Per<T>{Self::VALUE}}fn main(){assert_eq!(Day::per(Week),7u8);println!(\"{}\",Day::per(Week));}",
    );
}

#[test]
fn unused_associated_conversion_result_type_mismatch_is_rejected() {
    oracle::stock_and_native_reject(
        "trait Per<T>{type Output;const VALUE:Self::Output;}#[derive(Clone,Copy)]struct Day;#[derive(Clone,Copy)]struct Week;impl Per<Week> for Day{type Output=u8;const VALUE:u8=7;}impl Day{const fn per<T:Copy>(_:T)-><Self as Per<T>>::Output where Self:Per<T>{Self::VALUE}}fn unused(){let _:bool=Day::per(Week);}fn main(){}",
    );
}

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
