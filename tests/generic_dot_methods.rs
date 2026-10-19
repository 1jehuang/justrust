//! Generic dot-call inference preserves canonical receiver search and binder domains.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn generic_trait_dot_arguments_match_stock() {
    oracle::stock_native(
        "trait Convert<A,B>{fn choose<U:Copy,V>(&self,a:A,b:B,u:U,v:V)->U{u}}struct S;impl Convert<i32,bool> for S{}fn main(){let s=S;assert_eq!(s.choose(3i32,true,7u8,'x'),7u8);assert_eq!(s.choose::<i32,char>(4i32,false,11i32,'y'),11i32);}",
    );
}

#[test]
fn inferred_dot_keeps_inherent_precedence_and_expected_results() {
    oracle::stock_native(
        "struct S;trait Pick{fn pick<T>(&self,value:T)->i32{2}}impl Pick for S{}impl S{fn pick<T>(&self,value:T)->i32{1}}trait Factory{fn empty<T>(&self)->Option<T>{None}}impl Factory for S{}fn main(){let s=S;assert_eq!(s.pick(true),1);let value:Option<u8>=s.empty();assert!(value.is_none());}",
    );
}

#[test]
fn generic_dot_receiver_modes_match_stock() {
    oracle::stock_native(
        "struct S{value:i32}trait Shared{fn shared<T>(&self,value:T)->T{value}}impl Shared for S{}trait Mutable{fn update<T>(&mut self,value:T)->T;}impl Mutable for S{fn update<T>(&mut self,value:T)->T{self.value+=1;value}}trait Owned{fn owned<T>(self,value:T)->T;}impl Owned for S{fn owned<T>(self,value:T)->T{value}}fn main(){let mut s=S{value:3};let r=&s;assert_eq!(r.shared(7u8),7u8);assert_eq!(s.update(true),true);assert_eq!(s.value,4);assert_eq!(s.owned('x'),'x');}",
    );
}

#[test]
fn generic_dot_rejects_invalid_unused_calls_and_ambiguous_candidates() {
    for source in [
        "trait Pick{fn pick<T:Copy>(&self,value:T)->T{value}}struct S;impl Pick for S{}fn bad(){S.pick(String::new());}fn main(){}",
        "trait Pick{fn pick<T,U>(&self,a:T,b:U)->T{a}}struct S;impl Pick for S{}fn bad(){S.pick::<u8,char>(true,'x');}fn main(){}",
        "trait Pick{fn pick<T,U>(&self,a:T,b:U)->T{a}}struct S;impl Pick for S{}fn bad(){S.pick::<u8>(1u8,'x');}fn main(){}",
        "trait A{fn choose<T>(&self,x:T)->T{x}}trait B{fn choose<T>(&self,x:T)->T{x}}struct S;impl A for S{}impl B for S{}fn bad(){S.choose(1i32);}fn main(){}",
        "struct S;impl S{fn choose<T:Copy>(&self,x:T)->T{x}}trait A{fn choose<T>(&self,x:T)->T{x}}impl A for S{}fn bad(){S.choose(String::new());}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
