//! Expanded generic binders retain source occurrences and use stock textual lookup.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn definition_and_invocation_lifetimes_share_the_lexical_binder() {
    oracle::stock_native(
        "macro_rules! make {($lt:lifetime)=>{fn identity<'a>(x:&$lt i32)->&'a i32{x}};}make!('a);fn main(){let value=7;assert_eq!(*identity(&value),7);}",
    );
    oracle::stock_native(
        "macro_rules! make {($lt:lifetime)=>{fn identity<$lt>(x:&'a i32)->&$lt i32{x}};}make!('a);fn main(){let value=11;assert_eq!(*identity(&value),11);}",
    );
}

#[test]
fn definition_and_invocation_type_parameters_share_the_lexical_binder() {
    oracle::stock_native(
        "macro_rules! make {($ty:ty)=>{fn identity<T>(x:$ty)->T{x}};}make!(T);fn main(){assert_eq!(identity(13i32),13);}",
    );
    oracle::stock_native(
        "macro_rules! make {($name:ident)=>{fn identity<$name>(x:T)->$name{x}};}make!(T);fn main(){assert_eq!(identity(17i32),17);}",
    );
}

#[test]
fn same_spelling_parameters_in_unrelated_expansions_do_not_merge_declarations() {
    oracle::stock_native(
        "macro_rules! make {($name:ident)=>{fn $name<T>(x:T)->T{x}};}make!(first);make!(second);fn main(){assert_eq!(first(19i32),19);assert_eq!(second(true),true);}",
    );
}

#[test]
fn unused_invalid_expanded_binders_are_rejected() {
    for source in [
        "macro_rules! make {()=>{fn bad<'a>(x:&'b i32)->&'a i32{x}};}make!();fn main(){}",
        "macro_rules! make {()=>{fn bad<T>(x:U)->T{x}};}make!();fn main(){}",
        "macro_rules! make {()=>{fn bad<T,T>(x:T)->T{x}};}make!();fn main(){}",
        "macro_rules! make {()=>{fn bad<'a,'a>(x:&'a i32)->&'a i32{x}};}make!();fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn expanded_labels_remain_explicitly_unsupported() {
    let source = "macro_rules! make {()=>{'label:loop{break 'label;}};}fn main(){make!();}";
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match justrust_engine::compiler::compile(source.into(), workers, check) {
                Ok(_) => panic!("expanded label unexpectedly admitted"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains("expanded labels are unsupported"), "{error}");
        }
    }
}
