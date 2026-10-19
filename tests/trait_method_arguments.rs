//! Trait-owner arguments and a method's own arguments are separate ordered domains.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn explicit_and_inferred_ufcs_method_arguments_match_stock() {
    oracle::stock_native(
        "trait Convert<A,B>{fn choose<U:Copy,V>(a:A,b:B,u:U,v:V)->U{u}}impl Convert<i32,bool> for i32{}impl Convert<bool,i32> for i32{}fn main(){assert_eq!(<i32 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8,'x'),7u8);assert_eq!(<i32 as Convert<bool,i32>>::choose(false,5,11i32,'y'),11);}",
    );
}

#[test]
fn result_context_can_infer_a_zero_argument_trait_method() {
    oracle::stock_native(
        "trait Factory{fn empty<T>()->Option<T>{None}}impl Factory for i32{}fn main(){let value:Option<u8>=<i32 as Factory>::empty();assert!(value.is_none());}",
    );
}

#[test]
fn ufcs_method_inference_never_changes_trait_owner_arity_order_or_bounds() {
    let declarations = "trait Convert<A,B>{fn choose<U:Copy,V>(a:A,b:B,u:U,v:V)->U{u}}impl Convert<i32,bool> for i32{}";
    for bad in [
        "<u8 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8,'x');",
        "<i32 as Convert<bool,i32>>::choose::<u8,char>(false,3,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<char,u8>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8,char,bool>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<String,char>(3,true,String::new(),'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8);",
    ] {
        oracle::stock_and_native_reject(&format!("{declarations}fn bad(){{{bad}}}fn main(){{}}"));
    }
    oracle::stock_and_native_reject(
        "trait Bad{fn bad<U,V>(u:U,v:V)->U{v}}impl Bad for i32{}fn main(){}",
    );
}
