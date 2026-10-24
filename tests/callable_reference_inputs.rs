//! Callable argument tuples retain exact checked reference input carriers.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn unchanged_generic_container_matches_stock() {
    oracle::stock_native(include_str!(
        "fixtures/full_programs/14_generic_container.rs"
    ));
}

#[test]
fn closed_literal_generic_inputs_match_stock() {
    oracle::stock_native(
        r#"use std::fmt::Display;
struct Pair<T> { a:T, b:T }
trait Summary { fn summarize(&self)->String; }
impl<T:Display> Summary for Pair<T> {
    fn summarize(&self)->String { format!("({}, {})",self.a,self.b) }
}
fn print_summary(item:&impl Summary) { println!("summary: {}",item.summarize()); }
fn consume<T>(item:T)->bool { let _=item; true }
fn main() {
    print_summary(&Pair { a:"x", b:"y" });
    println!("{} {}",consume(Pair { a:"left", b:"right" }),consume(&Pair { a:'a', b:'b' }));
}"#,
    );
    for source in [
        "struct Pair<T>{a:T,b:T}trait Missing{}fn consume<T:Missing>(x:T){}fn unused(){consume(&Pair{a:\"x\",b:\"y\"});}fn main(){}",
        "struct Pair<T>{a:T,b:T}fn escape<'a,T>(x:&T)->&'a T{x}fn main(){let x=escape(&Pair{a:\"x\",b:\"y\"});}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn ordinary_generic_scalar_reference_inputs_match_stock() {
    oracle::stock_native(
        r#"fn consume<T>(value: T) -> i32 { let _ = value; 7 }
fn main() {
    let number = 4i32;
    let flag = true;
    let character = 'x';
    let mut mutable = 9i32;
    println!("{} {} {} {}", consume(&number), consume(&flag), consume(&character), consume(&mut mutable));
    let reference: &i32 = &number;
    println!("{} {}", consume(reference), mutable);
}"#,
    );
    oracle::stock_native(
        r#"fn consume<T>(value:T)->i32 { let _=value;7 }
        fn main() {
            let mut number=4i32;
            let moved:&mut i32=&mut number;
            println!("{}",consume(moved));
            let reborrowed:&mut i32=&mut number;
            println!("{}",consume::<&mut i32>(reborrowed));
            *reborrowed+=1;
            println!("{}",number);
        }"#,
    );
}

#[test]
fn ordinary_generic_scalar_reference_inputs_preserve_borrow_rules() {
    for source in [
        "fn consume<T>(x:T){let _=x;}fn main(){let x=1i32;consume(&mut x);}",
        "fn consume<T>(x:T){let _=x;}fn main(){let mut x=1i32;let r=&x;consume(&mut x);println!(\"{}\",r);}",
        "fn consume<T:Copy>(x:T){let _=x;}fn main(){let mut x=1i32;consume(&mut x);}",
        "fn consume<T>(x:T){let _=x;}fn main(){let mut x=1i32;let r:&mut i32=&mut x;consume(r);*r=2;}",
        "fn bad<'a,T>(x:T)->&'a i32{let _=x;let y=1;&y}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

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
