//! Declaration-owned projections in struct and enum field schemas.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn nested_struct_projection_executes() {
    oracle::stock_native(
        "trait It{type Item;}struct C;impl It for C{type Item=i32;}struct Peek<I:It>{item:Option<Option<I::Item>>}fn main(){let p:Peek<C>=Peek{item:Some(Some(7))};if let Some(Some(n))=p.item{println!(\"{}\",n);}}",
    );
}

#[test]
fn enum_projection_executes() {
    oracle::stock_native(
        "trait It{type Item;}struct C;impl It for C{type Item=i32;}enum Value<I:It>{Item(I::Item),Empty}fn main(){let v:Value<C>=Value::Item(9);if let Value::Item(n)=v{println!(\"{}\",n);}}",
    );
}

#[test]
fn unused_nominal_projection_without_membership_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It{type Item;}struct Bad<I>{item:<I as It>::Item}fn main(){}",
    );
}

#[test]
fn unused_nominal_projection_with_wrong_member_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It{type Item;}struct Bad<I:It>{item:<I as It>::Missing}fn main(){}",
    );
}

#[test]
fn unused_nominal_projection_with_wrong_ordered_membership_is_rejected() {
    oracle::stock_and_native_reject(
        "trait It<A,B>{type Item;}struct Bad<I:It<i32,bool>>{item:<I as It<bool,i32>>::Item}fn main(){}",
    );
}
