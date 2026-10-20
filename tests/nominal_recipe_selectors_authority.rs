//! Invalid indexing must not gain authority from a nominal recipe.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn valid_index_control_is_not_blanket_rejected() {
    oracle::stock_native("fn main(){let a=[7u8];println!(\"{}\",a[0]);}");
}

#[test]
fn nominal_without_index_implementation_rejects() {
    oracle::stock_and_native_reject(
        "struct Wrap{value:i32}fn main(){let w=Wrap{value:7};let n=w[0];}",
    );
}

#[test]
fn wrong_index_type_rejects() {
    oracle::stock_and_native_reject("fn main(){let v=vec![7i32];let n=v[false];}");
}

#[test]
fn immutable_index_assignment_rejects() {
    oracle::stock_and_native_reject("fn main(){let v=vec![7i32];v[0]=9;}");
}

#[test]
fn shared_index_borrow_cannot_authorize_mutation() {
    oracle::stock_and_native_reject("fn main(){let v=vec![7i32];let r=&v[0];*r=9;}");
}

#[test]
fn invalid_unused_function_still_rejects() {
    oracle::stock_and_native_reject(
        "struct Wrap{value:i32}fn unused(w:Wrap){let n=w[0];}fn main(){}",
    );
}
