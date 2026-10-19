//! Generic trait method owner/method substitution and invalid unused items.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn method_and_owner_binders_stay_distinct() {
    oracle::stock_native(
        "trait Identity{fn identity<T>(&self,x:T)->T;}struct S;impl Identity for S{fn identity<T>(&self,x:T)->T{x}}fn main(){println!(\"{}\",S.identity::<i32>(42));}",
    );
    oracle::stock_native(
        "trait Identity{fn identity<T>(&self,x:T)->T{x}}struct S;impl Identity for S{}fn main(){println!(\"{}\",S.identity::<i32>(42));}",
    );
}

#[test]
fn invalid_signatures_bounds_arity_and_unused_bodies_reject() {
    for source in [
        "trait Tr{fn f<T>(&self,x:T)->T;}struct S;impl Tr for S{fn f<T>(&self,x:T)->i32{42}}fn main(){}",
        "trait Tr{fn f<T>(&self,x:T)->T;}struct S;impl Tr for S{fn f<T:Copy>(&self,x:T)->T{x}}fn main(){}",
        "trait Tr{fn f<T>(&self,x:T)->T;}struct S;impl Tr for S{fn f<T,U>(&self,x:T)->T{x}}fn main(){}",
        "trait Tr{fn f<T>(&self,x:T)->T{true}}fn main(){}",
        "trait Source{type Out:Copy;fn get(self)->Self::Out;}trait Local:Source{fn dup_local<U>(&self,y:U)->(U,U){(y,y)}}fn main(){}",
        "trait Tr{fn f<T>(&self,x:T)->T{x}}struct S;impl Tr for S{}fn main(){S.f::<i32,bool>(42);}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
