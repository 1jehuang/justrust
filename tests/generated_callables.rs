//! Ordinary source declarations and exact per-expansion callable completeness.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn generic_free_and_associated_macro_products_match_stock() {
    for source in [
        "macro_rules! make {()=>{fn identity<T>(x:T)->T{x}};}make!();fn main(){assert_eq!(identity(7i32),7);}",
        "struct Value<T>{value:T}macro_rules! make {()=>{impl<T:Copy> Value<T>{fn get(&self)->T{self.value}fn identity<U>(mut value:U)->U{value}}};}make!();fn main(){let x=Value{value:11i32};assert_eq!(x.get(),11);assert_eq!(Value::<i32>::identity(true),true);}",
        "macro_rules! make {()=>{trait Identity{fn identity<T>(value:T)->T{value}}impl Identity for i32{}};}make!();fn main(){assert_eq!(<i32 as Identity>::identity(13i32),13);}",
        "trait Value{fn get(&self)->i32;}struct Number{value:i32}macro_rules! make {()=>{impl Value for Number{fn get(&self)->i32{self.value}}};}make!();fn main(){let x=Number{value:17};assert_eq!(x.get(),17);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn static_mut_parameter_is_not_a_mut_self_receiver() {
    oracle::stock_native(
        "fn free(mut x:i32)->i32{x+=1;x}struct Value{value:i32}impl Value{fn associated(mut x:i32)->i32{x+=2;x}fn receiver(mut self)->i32{self.value+=3;self.value}}fn main(){assert_eq!(free(1),2);assert_eq!(Value::associated(1),3);assert_eq!(Value{value:1}.receiver(),4);}",
    );
}

#[test]
fn every_unused_generated_body_is_checked() {
    for source in [
        "macro_rules! make {()=>{fn bad<T>(x:T)->i32{x}};}make!();fn main(){}",
        "struct Value{value:i32}macro_rules! make {()=>{impl Value{fn bad<T>(x:T)->i32{x}}};}make!();fn main(){}",
        "macro_rules! make {()=>{trait Bad{fn bad<T>(x:T)->i32{x}}};}make!();fn main(){}",
        "trait Bad{fn bad()->i32;}macro_rules! make {()=>{impl Bad for i32{fn bad()->i32{true}}};}make!();fn main(){}",
        "struct Value{value:i32}impl Value{fn bad(mut self:i32)->i32{0}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
