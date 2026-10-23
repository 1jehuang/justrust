//! Canonical overloaded operators do not equate receiver, RHS and Output.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn nominal_comparisons_allow_quiescent_numeric_defaulting() {
    for source in [
        "#[derive(Clone,PartialEq)] struct Pair<T>{a:T,b:T} impl<T:Copy> Pair<T>{fn swap(self)->Self{Self{a:self.b,b:self.a}}} fn main(){let p=Pair{a:3,b:9};let q=p.clone().swap();println!(\"{}\",p==q);}",
        "#[derive(Clone,PartialEq)] struct Pair<T>{a:T,b:T} impl<T:Copy> Pair<T>{fn swap(self)->Self{Self{a:self.b,b:self.a}}} fn main(){let p=Pair{a:3.0,b:9.0};let q=p.clone().swap();println!(\"{}\",p==q);}",
        "#[derive(Clone,PartialEq)] struct Pair<T>{a:T,b:T} fn narrow(_: &Pair<u8>){} fn main(){let p=Pair{a:3,b:9};let q=p.clone();let equal=p==q;narrow(&p);println!(\"{}\",equal);}",
        "#[derive(Clone,PartialEq)] struct Pair<T>{a:T,b:T} fn narrow(_: &Pair<f32>){} fn main(){let p=Pair{a:3.0,b:9.0};let q=p.clone();let equal=p==q;narrow(&p);println!(\"{}\",equal);}",
    ] {
        oracle::stock_native(source);
    }
    for source in [
        "#[derive(PartialEq)] struct Pair<T>{a:T} fn main(){let p=Pair{a:3u8};let q=Pair{a:3u16};let _equal=p==q;}",
        "struct Pair<T>{a:T} fn unused(){let p=Pair{a:3};let q=Pair{a:3};let _equal=p==q;} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn independent_rhs_and_output_flow_through_nested_operators() {
    oracle::stock_native(
        "use std::ops::{Add,Mul,Neg};struct A{v:i32}struct B{v:i32}impl Add<u8> for A{type Output=B;fn add(self,n:u8)->B{B{v:self.v+n as i32}}}impl Mul<i32> for B{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}impl Neg for A{type Output=i32;fn neg(self)->i32{-self.v}}fn main(){let x=(A{v:4}+3u8)*2;let y=-A{v:9};println!(\"{} {}\",x,y);}",
    );
}

#[test]
fn compound_rhs_is_not_the_receiver_type() {
    oracle::stock_native(
        "use std::ops::AddAssign;struct A{v:i32}impl AddAssign<u8> for A{fn add_assign(&mut self,n:u8){self.v+=n as i32;}}fn main(){let mut x=A{v:3};let n=4;x+=n;let width:u8=n;println!(\"{} {}\",x.v,width);}",
    );
}

#[test]
fn primitive_shift_widths_stay_independent() {
    oracle::stock_native(
        "fn main(){let mut x=16i128;let shift=2u128;let y=x>>shift;x>>=shift;println!(\"{} {}\",x,y);}",
    );
}

#[test]
fn genuine_numeric_literals_do_not_select_a_nominal_self_rhs() {
    for source in [
        "use std::ops::Mul;struct A{v:i32}impl Mul<A> for A{type Output=bool;fn mul(self,n:A)->bool{self.v==n.v}}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}fn main(){let x=A{v:7}*2;println!(\"{}\",x);}",
        "use std::ops::Mul;struct A{v:f64}impl Mul<A> for A{type Output=bool;fn mul(self,n:A)->bool{self.v==n.v}}impl Mul<f64> for A{type Output=f64;fn mul(self,n:f64)->f64{self.v*n}}fn main(){let x=A{v:7.0}*2.0;println!(\"{}\",x);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn generic_operator_output_uses_the_callers_canonical_bound() {
    oracle::stock_native(
        "use std::ops::Mul;struct A{v:i32}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}fn apply<T:Mul<i32,Output=i32>>(x:T)->i32{x*2}fn main(){println!(\"{}\",apply(A{v:7}));}",
    );
}

#[test]
fn exact_rhs_application_selects_its_own_output() {
    oracle::stock_native(
        "use std::ops::Mul;struct A{v:i32}impl Mul<u8> for A{type Output=i32;fn mul(self,n:u8)->i32{self.v+n as i32}}impl Mul<u16> for A{type Output=bool;fn mul(self,n:u16)->bool{self.v==n as i32}}fn main(){let x=A{v:4}*2u8;let y=A{v:4}*4u16;println!(\"{} {}\",x,y);}",
    );
}

#[test]
fn unsuffixed_rhs_uses_unique_proved_numeric_application_before_defaulting() {
    for source in [
        "use std::ops::Mul;struct A;impl Mul<u8> for A{type Output=u8;fn mul(self,r:u8)->u8{r+1}}fn main(){println!(\"{}\",A*2);}",
        "use std::ops::Mul;struct A;impl Mul<f32> for A{type Output=f32;fn mul(self,r:f32)->f32{r+1.0}}fn main(){println!(\"{}\",A*2.0);}",
        "use std::ops::Mul;struct A;impl Mul<u8> for A{type Output=u8;fn mul(self,r:u8)->u8{r+1}}fn apply<T:Mul<u8,Output=u8>>(x:T)->u8{x*2}fn main(){println!(\"{}\",apply(A));}",
        "use std::ops::Mul;struct A;impl Mul<u8> for A{type Output=u8;fn mul(self,r:u8)->u8{r+1}}impl Mul<i32> for A{type Output=i32;fn mul(self,r:i32)->i32{r+10}}fn main(){println!(\"{}\",A*2);}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn multiple_narrow_rhs_widths_do_not_select_the_first_impl() {
    oracle::stock_and_native_reject(
        "use std::ops::Mul;struct A;impl Mul<u8> for A{type Output=u8;fn mul(self,r:u8)->u8{r}}impl Mul<u16> for A{type Output=u16;fn mul(self,r:u16)->u16{r}}fn main(){let x=A*2;}",
    );
}

#[test]
fn invalid_operators_remain_rejected_even_in_unused_bodies() {
    for source in [
        "trait Mul<R>{type Output;fn mul(self,r:R)->Self::Output;}struct A{v:i32}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}fn main(){let x=A{v:1}*2;}",
        "use std::ops::Mul;struct A{v:i32}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->bool{true}}fn main(){}",
        "struct A{v:i32}fn unused(){let x=A{v:1}*2;}fn main(){}",
        "use std::ops::Mul;struct A{v:i32}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}fn main(){let x=A{v:1}*true;}",
        "use std::ops::Mul;struct A{v:i32}impl Mul<i32> for A{type Output=i32;fn mul(self,n:i32)->i32{self.v*n}}fn main(){let x:bool=A{v:1}*2;}",
        "use std::ops::AddAssign;struct A{v:i32}impl AddAssign<u8> for A{fn add_assign(&mut self,n:u8){self.v+=n as i32;}}fn main(){let mut a=A{v:1};a+=3u16;}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn primitive_reference_operands_use_forwarding_impls() {
    // Stock forwards `&i32 op i32`, `i32 op &i32` and `&i32 op &i32` to the
    // scalar impl; comparisons need equal reference depth on both sides.
    oracle::stock_native(
        "fn sq(x:&i32)->i32{x*x} fn f(a:&f64,b:f64)->f64{a+b} fn main(){let a=3;let r=&a;let b=7u8;let rb=&b;let c=2.5;println!(\"{} {} {} {} {} {} {} {}\",sq(&a),r+1,1-r,rb<<1u8,rb&3,f(&c,1.0),r==&3,r<&4);}",
    );
    for source in [
        // Mutable references have no forwarding operator impls.
        "fn main(){let mut a=3i32;let r=&mut a;let x=r*2;println!(\"{}\",x);}",
        // Comparison operands must have equal reference depth.
        "fn main(){let a=3i32;let b=(&a)==3;println!(\"{}\",b);}",
        // Mismatched scalar referents still reject.
        "fn main(){let a=3i32;let b=1u8;let x=&a+&b;println!(\"{}\",x);}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn string_comparisons_do_not_construct_unsized_scalar_dereferences() {
    oracle::stock_native(
        r#"fn equal(a:&str,b:&str)->bool{a==b}
fn main(){
    assert!(equal("same","same"));
    assert!(!equal("same","other"));
    assert!("a"!="b");
    assert!("a"<"aa");
    assert!("aa"<="aa");
    assert!("é">"z");
    assert!("🙂">="🙂");
    assert!(""<"x");
    assert!("a\0b"<"a\0c");
    let a=String::from("owned");
    assert!(equal(&a,"owned"));
}"#,
    );
}

#[test]
fn owned_string_ordering_matches_stock() {
    oracle::stock_native(
        r#"fn compare(a:&str,b:&str){
    let left=String::from(a);let right=String::from(b);
    println!("{} {} {} {} {} {} {}",left==right,left<right,left<=right,left>right,left>=right,left.cmp(&right).is_lt(),left.partial_cmp(&right).unwrap().is_eq());
}
fn main(){compare("","");compare("","a");compare("a","");compare("abc","abc");compare("ab","abc");compare("abc","ab");compare("a\0b","a\0c");compare("é","z");compare("日","é");compare("🙂","🦀");compare("e\u{301}","é");}"#,
    );
}
