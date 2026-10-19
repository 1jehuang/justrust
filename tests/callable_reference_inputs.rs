//! Callable argument tuples retain exact checked reference input carriers.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn shared_reference_callable_input_matches_stock() {
    oracle::stock_native(
        "fn apply<F: Fn(&i32) -> i32>(f: F, x: &i32) -> i32 { f(x) } fn main() { let f = |x: &i32| *x + 1; let n = 3; println!(\"{}\", apply(f, &n)); }",
    );
}

#[test]
fn mutable_reference_callable_input_matches_stock() {
    oracle::stock_native(
        "fn apply<F: FnMut(&mut i32) -> i32>(mut f: F, x: &mut i32) -> i32 { f(x) } fn main() { let f = |x: &mut i32| { *x += 1; *x }; let mut n = 3; println!(\"{}\", apply(f, &mut n)); println!(\"{}\", n); }",
    );
}

#[test]
fn unused_reference_callable_body_rejects_wrong_argument() {
    oracle::stock_and_native_reject(
        "fn bad<F: Fn(&i32) -> i32>(f: F) -> i32 { f(true) } fn main() {}",
    );
}

#[test]
fn reference_callable_inputs_reject_strengthening_aliases_and_escape() {
    for source in [
        "fn apply<F:FnMut(&mut i32)->i32>(mut f:F,x:&mut i32)->i32{f(x)}fn main(){let f=|x:&i32|*x;let mut n=1;apply(f,&mut n);}",
        "fn apply<F:FnMut(&mut i32,&mut i32)->i32>(mut f:F,x:&mut i32,y:&mut i32)->i32{f(x,y)}fn main(){let f=|x:&mut i32,y:&mut i32|{*x+=1;*y};let mut n=1;apply(f,&mut n,&mut n);}",
        "fn apply<F:FnMut(&i32)>(mut f:F,x:&i32){f(x)}fn main(){let mut out:Option<&i32>=None;let f=|x:&i32|{out=Some(x);};let n=3;apply(f,&n);}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
