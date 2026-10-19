//! Universal associated projections remain exact until closed instantiation.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn provided_iterator_default_checks_universally_and_runs_closed() {
    oracle::stock_native(
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn last(mut self)->Option<Self::Item> where Self:Sized{let mut out=None;while let Some(x)=self.next(){out=Some(x);}out}}struct Counter{n:i32}impl It for Counter{type Item=i32;fn next(&mut self)->Option<i32>{if self.n<3{self.n+=1;Some(self.n)}else{None}}}fn main(){println!(\"{}\",Counter{n:0}.last().unwrap());}",
    );
}

#[test]
fn rigid_generic_projection_identity_is_not_an_eager_concrete_guess() {
    oracle::stock_native(
        "trait Has{type Out;}struct S;impl Has for S{type Out=i32;}fn identity<I:Has>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){println!(\"{}\",identity::<S>(42));}",
    );
}

#[test]
fn unused_universal_body_and_unproved_projection_reject() {
    for source in [
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out)->bool{x}fn main(){}",
        "trait Has{type Out;}fn bad<I>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){}",
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out){let first=x;let second=x;}fn main(){}",
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn bad(self)->Self::Item where Self:Sized{true}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
