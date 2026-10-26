//! Callable argument tuples retain exact checked reference input carriers.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn generic_mutable_reference_inputs_match_stock() {
    oracle::stock_native(
        r#"fn consume<A,B>(a:A,b:B)->i32 { let _=a; let _=b; 31 }
fn main() {
    let mut n=4i32;
    let flag=true;
    println!("{}",consume(&mut n,&flag));
    n+=1;
    let reference:&mut i32=&mut n;
    println!("{}",consume(reference,&flag));
    n+=1;
    println!("{}",n);
}"#,
    );
}

#[test]
fn generic_mutable_input_preserves_trait_dispatch() {
    oracle::stock_native(
        r#"trait Increment { fn increment(self); }
impl Increment for &mut i32 { fn increment(self) { *self+=1; } }
fn update<A:Increment,B>(a:A,b:B) { a.increment();let _=b; }
fn main() {
    let mut n=4i32;
    let flag=true;
    update(&mut n,&flag);
    update(&mut n,&flag);
    println!("{}",n);
}"#,
    );
}

#[test]
fn generic_reference_schema_reborrows_match_stock() {
    oracle::stock_native(
        r#"fn use_refs<T,U>(a:&mut T,b:&U)->i32 { let _=a;let _=b;43 }
fn consume<A,B>(a:A,b:B)->i32 { let _=a;let _=b;47 }
fn main() {
    let mut n=4i32;
    let flag=true;
    let r:&mut i32=&mut n;
    println!("{} {}",use_refs(r,&flag),use_refs(r,&flag));
    println!("{}",consume::<&mut i32,&bool>(r,&flag));
    *r+=1;
    println!("{}",n);
}"#,
    );
}

#[test]
fn generic_nested_reference_inputs_match_stock() {
    oracle::stock_native(
        r#"fn consume<A,B>(a:A,b:B)->bool { let _=a; let _=b; true }
fn main() {
    let mut n=4i32;
    let flag=true;
    let reference:&mut i32=&mut n;
    let shared:&&mut i32=&reference;
    println!("{}",consume(shared,&flag));
    *reference+=1;
    println!("{}",n);
}"#,
    );
}

#[test]
fn generic_tuple_and_array_reference_inputs_match_stock() {
    oracle::stock_native(
        r#"fn consume<A,B>(a:A,b:B)->i32 { let _=a; let _=b; 37 }
fn main() {
    let a=4i32;
    let b=9i32;
    let flag=true;
    let pair:(&i32,&bool)=(&a,&flag);
    let values:[&i32;2]=[&a,&b];
    println!("{} {}",consume(pair,&b),consume(values,&flag));
    println!("{} {}",*pair.0,*values[1]);
}"#,
    );
}

#[test]
fn generic_nominal_reference_inputs_match_stock() {
    oracle::stock_native(
        r#"struct Pair<'a> { a:&'a i32, b:&'a i32 }
fn consume<A,B>(a:A,b:B)->i32 { let _=a; let _=b; 41 }
fn main() {
    let a=4i32;
    let b=9i32;
    let pair:Pair<'_>=Pair { a:&a,b:&b };
    let optional:Option<&i32>=Some(&a);
    println!("{} {}",consume(pair,&b),consume(optional,&b));
}"#,
    );
}

#[test]
fn deferred_reference_product_matches_stock() {
    oracle::stock_native(
        "fn main(){let n=7i32;let pair;pair=(&n,true);println!(\"{} {}\",*pair.0,pair.1);}",
    );
}

#[test]
fn generalized_reference_inputs_reject_moves_aliases_and_escapes() {
    for source in [
        "fn consume<A,B>(a:A,b:B){let _=a;let _=b;}fn main(){let mut n=1i32;let b=true;let r=&mut n;consume(r,&b);*r=2;}",
        "fn consume<A,B>(a:A,b:B){let _=a;let _=b;}fn main(){let mut n=1i32;consume(&mut n,&n);}",
        "fn consume<A,B>(a:A,b:B){let _=a;let _=b;}fn main(){let mut n=1i32;let a=[&mut n];consume(a,&true);*a[0]=2;}",
        "fn consume<A,B>(a:A,b:B){let _=a;let _=b;}fn main(){let escaped;{let n=1i32;escaped=(&n,);}consume(escaped,&true);}",
    ] {
        oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            let message = failure.to_string();
            assert!(
                !message.contains("unsupported") && !message.contains("resource limit"),
                "ownership control hit an implementation boundary (workers={workers}, check={check}): {message}"
            );
        });
    }
}

#[test]
fn independent_reference_type_parameters_match_stock() {
    oracle::stock_native(
        r#"fn inspect<A: Copy, B: Copy>(a: A, b: B) -> i32 { let _ = a; let _ = b; 17 }
fn main() {
    let number = 4i32;
    let flag = true;
    let character = 'q';
    let reference = &number;
    println!("{} {} {}", inspect(&number, &flag), inspect(&character, &number), inspect(&reference, &flag));
}"#,
    );
}

#[test]
fn shaped_shared_reference_parameters_match_stock() {
    oracle::stock_native(
        r#"fn inspect<T, U>(a: &T, b: &&U, c: &T) -> i32 { let _ = a; let _ = b; let _ = c; 23 }
fn main() {
    let number = 4i32;
    let other = 9i32;
    let flag = true;
    let reference = &flag;
    println!("{}", inspect(&number, &reference, &other));
}"#,
    );
}

#[test]
fn mixed_reference_parameter_schemas_match_stock() {
    oracle::stock_native(
        r#"fn inspect<A, B>(a: A, b: &B, c: A) -> i32 { let _ = a; let _ = b; let _ = c; 29 }
fn main() {
    let number = 4i32;
    let other = 9i32;
    let flag = true;
    let reference = &flag;
    println!("{} {}", inspect(&number, &flag, &other), inspect(&number, &reference, &other));
}"#,
    );
}

#[test]
fn multi_slot_reference_constraints_preserve_values() {
    oracle::stock_native(
        r#"fn equal_pairs<A: PartialEq, B: PartialEq>(a:A,b:B,c:A,d:B)->bool { a==c && b==d }
fn main() {
    let first=4i32;
    let equal=4i32;
    let different=9i32;
    let yes=true;
    let no=false;
    println!("{} {} {}", equal_pairs(&first,&yes,&equal,&yes), equal_pairs(&first,&yes,&different,&yes), equal_pairs(&first,&yes,&equal,&no));
}"#,
    );
}

#[test]
fn generalized_reference_inputs_preserve_constraints() {
    for source in [
        "fn same<T>(a:&T,b:&T)->i32{let _=a;let _=b;1}fn unused(){let n=1i32;let b=true;same(&n,&b);}fn main(){}",
        "fn same<A,B>(a:A,b:&B,c:A)->i32{let _=a;let _=b;let _=c;1}fn unused(){let n=1i32;let b=true;same(&n,&b,&b);}fn main(){}",
        "fn missing<A,B>(a:A)->i32{let _=a;1}fn unused(){let n=1i32;missing(&n);}fn main(){}",
        "fn bounded<A:Copy,B:Copy>(a:A,b:B)->i32{let _=a;let _=b;1}fn unused(){let mut n=1i32;let b=true;bounded(&mut n,&b);}fn main(){}",
        "fn consume<A,B>(a:A,b:B)->i32{let _=a;let _=b;1}fn main(){let mut n=1i32;let r=&n;let b=true;consume(&mut n,&b);println!(\"{}\",r);}",
        "fn escape<'a,T,U>(a:&T,b:&U)->&'a T{let _=b;a}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

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
