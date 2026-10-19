//! Sealed named-constant scalar types constrain body inference before fallback.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn named_and_qualified_usize_constants_constrain_range_items() {
    for source in [
        "const N:usize=3;fn index(_:usize){}fn main(){let mut total=0usize;for i in 0..N{index(i);total+=i;}assert_eq!(total,3usize);}",
        "mod dimensions{pub const N:usize=3;}fn index(_:usize){}fn main(){let mut total=0usize;for i in 0..dimensions::N{index(i);total+=i;}assert_eq!(total,3usize);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn const_constrained_items_reject_incompatible_stores_even_in_unused_bodies() {
    for source in [
        "const N:usize=3;fn main(){let mut a=[0u8;3];for i in 0..N{a[i]=i;}}",
        "mod dimensions{pub const N:usize=3;}fn unused(){for i in 0..dimensions::N{let _:u8=i;}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
