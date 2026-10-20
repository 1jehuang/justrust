//! Source-scoped macro binding repair, with stock and native source intact.
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn supported_fragment_and_parameter_projection_preserve_local_hygiene() {
    for source in [
        "macro_rules! unused {()=>{$missing};} macro_rules! value {($x:expr)=>{$x};()=>{0};} fn main(){assert_eq!(value!(3+4),7);assert_eq!(value!(),0);}",
        "macro_rules! typed {($value:expr)=>{{let copy:T=$value;copy}};} fn identity<T>(value:T)->T{typed!(value)} fn main(){assert_eq!(identity(7u64),7u64);}",
        "fn main(){let value=7;macro_rules! read {()=>{value};} {let value=11;assert_eq!(read!(),7);assert_eq!(value,11);}}",
        "macro_rules! item {()=>{struct Data{value:i32} impl Data{fn read(&self)->i32{self.value}}};} item!();fn main(){let data=Data{value:7};assert_eq!(data.read(),7);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn invalid_macros_and_hygienic_misses_are_source_errors_even_when_unused() {
    for source in [
        "macro_rules! unused {($x:unknown_fragment)=>{};} fn main(){}",
        "macro_rules! missing {()=>{$missing};} fn unused(){let value=missing!();} fn main(){}",
        "fn unused(){macro_rules! read {()=>{value};} let value=7;let result=read!();} fn main(){}",
        "macro_rules! item {($name:ident,$parameter:ident)=>{fn $name($parameter:i32)->i32{value}};} item!(unused,value);fn main(){}",
        // Stock rejects the unbound lifetime. Native currently rejects the
        // named local annotation as unsupported, not lifetime-resolution parity.
        "macro_rules! refer {($value:expr)=>{{let copy:&'a i32=$value;copy}};} fn unused(value:&i32)->&i32{refer!(value)} fn main(){}",
    ] {
        let fixture = oracle::Fixture::new(source, &[], None);
        let stock = fixture.stock(true);
        assert!(
            !stock.status.success(),
            "stock accepted invalid hygiene control: {source}"
        );
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = fixture.native(workers, check).err().unwrap_or_else(|| {
                    panic!("native accepted workers={workers} check={check}: {source}")
                });
                assert!(failure.report.as_ref().is_some_and(|r| r.capture_complete));
                let diagnostic = failure.to_string().to_lowercase();
                for unexpected in [
                    "invalid metadata",
                    "invalidmetadata",
                    "budget",
                    "limit exceeded",
                ] {
                    assert!(!diagnostic.contains(unexpected), "{failure}: {source}");
                }
            }
        }
    }
}

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
