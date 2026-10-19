//! Associated equality outputs constrain impl parameters, without inference
//! backwards through a non-injective associated projection.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn associated_output_determines_unused_impl_type_parameter() {
    oracle::stock_native(
        "trait Has{type Out;} struct S; impl Has for S{type Out=i32;} struct Wrap<I>(I); trait Value{fn value(&self)->i32;} impl<T,I:Has<Out=T>> Value for Wrap<I>{fn value(&self)->i32{42}} fn main(){let x=Wrap(S);println!(\"{}\",x.value());}",
    );
}

#[test]
fn unused_unconstrained_impl_parameters_still_reject() {
    for source in [
        "trait Tr{} struct S; impl<T> Tr for S{} fn main(){}",
        "trait Has{type Out;} trait Tr{} struct S; impl<T:Has<Out=U>,U:Has<Out=T>> Tr for S{} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("unused invalid impl must reject");
                assert!(error.to_string().contains("unconstrained"), "{error}");
            }
        }
    }
}
