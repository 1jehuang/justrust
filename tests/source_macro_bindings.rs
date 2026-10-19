//! Source-scoped macro binding repair, with stock and native source intact.
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn user_macro_shadow_does_not_rebind_prelude_sources() {
    for source in [
        "macro_rules! panic {() => {23i32};} fn f()->i32 {panic!()} fn main(){assert_eq!(f(),23i32);}",
        "macro_rules! panic {() => {23i32};} use core::option::Option as Choice; fn main(){let x=Choice::Some(7i32);assert_eq!(x.unwrap(),7i32);assert_eq!(panic!(),23i32);}",
        "macro_rules! println {() => {31i32};} fn main(){assert_eq!(println!(),31i32);}",
        "macro_rules! assert {() => {37i32};} fn main(){assert_eq!(assert!(),37i32);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn textual_modules_shadow_restore_and_function_imports_keep_binding() {
    for source in [
        "macro_rules! value {()=>{11i32};} mod child {pub fn first()->i32{value!()} macro_rules! value {()=>{17i32};} pub fn second()->i32{value!()}} use child::{first,second}; fn main(){assert_eq!(first(),11i32);assert_eq!(second(),17i32);assert_eq!(value!(),11i32);}",
        "macro_rules! panic {()=>{23i32};} mod child {pub fn inherited()->i32{panic!()} macro_rules! panic {()=>{29i32};} pub fn own()->i32{panic!()}} use child::own as imported; fn main(){assert_eq!(child::inherited(),23i32);assert_eq!(imported(),29i32);assert_eq!(panic!(),23i32);}",
        "macro_rules! value {()=>{13i32};} fn f()->i32 {let a=value!();{macro_rules! value {()=>{19i32};} assert_eq!(value!(),19i32);} a+value!()} fn main(){assert_eq!(f(),26i32);}",
        "macro_rules! inner {()=>{41i32};} macro_rules! outer {()=>{inner!()};} mod child {macro_rules! inner {()=>{43i32};} pub fn f()->i32 {outer!()}} fn main(){assert_eq!(outer!(),41i32);assert_eq!(child::f(),43i32);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn file_modules_share_crate_textual_scope_but_mounted_crates_do_not() {
    oracle::Fixture::new(
        "macro_rules! value {()=>{47i32};} mod child; use child::f; fn main(){assert_eq!(f(),47i32);}",
        &[("child.rs", "pub fn f()->i32{value!()}")],
        None,
    ).assert_positive();
    oracle::Fixture::new(
        "macro_rules! value {()=>{53i32};} macro_rules! panic {()=>{23i32};} use dep::{f,g}; fn main(){assert_eq!(value!(),53i32);assert_eq!(panic!(),23i32);assert_eq!(f(),59i32);assert_eq!(g(),61i32);}",
        &[],
        Some("macro_rules! value {()=>{59i32};} pub fn f()->i32{value!()} mod child {macro_rules! value {()=>{61i32};} pub fn g()->i32{value!()}} pub use child::g; pub fn unused()->i32 {panic!(\"library panic\")}"),
    ).assert_positive();
}
