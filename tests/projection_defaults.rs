//! Numeric defaults are used only after projected call constraints quiesce.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn unconstrained_integer_projection_receiver_defaults_to_i32() {
    oracle::stock_native(
        "trait Has{type Out;}impl Has for i32{type Out=i32;}fn apply<I:Has,F:Fn(I::Out)->i32>(f:F,value:I::Out,owner:I)->i32{f(value)}fn main(){let f=|x|x+1;println!(\"{}\",apply(f,2,0));}",
    );
}

#[test]
fn unconstrained_float_projection_receiver_defaults_to_f64() {
    oracle::stock_native(
        "trait Has{type Out;}impl Has for f64{type Out=f64;}fn apply<I:Has,F:Fn(I::Out)->f64>(f:F,value:I::Out,owner:I)->f64{f(value)}fn main(){let f=|x|x;println!(\"{}\",apply(f,2.5,0.0));}",
    );
}

#[test]
fn later_known_width_settles_before_projection_defaulting() {
    oracle::stock_native(
        "trait Has{type Out;}impl Has for i32{type Out=i32;}impl Has for u8{type Out=u8;}fn apply<I:Has,F:Fn(I::Out)->u8>(f:F,value:I::Out,owner:I,tag:I)->u8{f(value)}fn main(){let f=|x|x;println!(\"{}\",apply(f,7,0,1u8));}",
    );
}

#[test]
fn defaulting_cannot_erase_projected_argument_conflicts() {
    for source in [
        "trait Has{type Out;}impl Has for i32{type Out=i32;}fn apply<I:Has,F:Fn(I::Out)->i32>(f:F,value:I::Out,owner:I)->i32{f(value)}fn main(){let f=|x|x+1;apply(f,true,0);}",
        "trait Has{type Out;}impl Has for i32{type Out=i32;}impl Has for u8{type Out=u8;}fn apply<I:Has,F:Fn(I::Out)->u8>(f:F,value:I::Out,owner:I,tag:I)->u8{f(value)}fn main(){let f=|x|x;apply(f,256u16,0,1u8);}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
