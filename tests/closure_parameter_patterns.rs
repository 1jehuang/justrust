//! Retained source parameter patterns use ordinary checked entry destructuring.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_shared_reference_parameter_copies_the_referent() {
    oracle::check(
        "closure_parameter_reference_pattern",
        &[Case {
            name: "double_reference_scalar",
            source: "fn main(){let n=7i32;let r=&n;let f=|&&x|x+1;assert_eq!(f(&r),8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn parameter_patterns_preserve_irrefutable_bindings() {
    oracle::check(
        "closure_parameter_bindings",
        &[
            Case {
                name: "tuple_shadow_and_wildcard",
                source: "fn main(){let x=90i32;let mut order=0i32;let f=|(x,_) :(i32,i32)|x+1;assert_eq!(f(({order=order*10+1;7},{order=order*10+2;8})),8);assert_eq!(order,12);assert_eq!(x,90);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_pattern_binding",
                source: "fn main(){let f=|mut x:i32|{x+=1;x};assert_eq!(f(7),8);}",
                expect: Expect::Run(""),
            },
        ],
    );
}

// Keep each invalid source independently named so an interrupted hosted run
// retains completed semantic verdicts rather than one buffered seven-case test.
fn invalid(name: &'static str, source: &'static str, fragment: &'static str) {
    oracle::check(
        "closure_parameter_invalid",
        &[Case {
            name,
            source,
            expect: Expect::Invalid(fragment),
        }],
    );
}

#[test]
fn unused_invalid_noncopy_shared_extraction() {
    invalid(
        "noncopy_shared_extraction",
        "fn unused(){let f=|&x:&String|x;}fn main(){}",
        "E0507",
    );
}
#[test]
fn unused_invalid_immutable_destructured_binding() {
    invalid(
        "immutable_destructured_binding",
        "fn unused(){let f=|&x:&i32|{x=2;};}fn main(){}",
        "E0384",
    );
}
#[test]
fn unused_invalid_wrong_reference_depth() {
    invalid(
        "wrong_reference_depth",
        "fn unused(){let f=|&&x:&i32|x;}fn main(){}",
        "E0308",
    );
}
#[test]
fn unused_invalid_mutable_pattern_on_shared_reference() {
    invalid(
        "mutable_pattern_on_shared_reference",
        "fn unused(){let f=|&mut x:&i32|x;}fn main(){}",
        "E0308",
    );
}
#[test]
fn unused_invalid_duplicate_parameter_binding() {
    invalid(
        "duplicate_parameter_binding",
        "fn unused(){let f=|(x,x):(i32,i32)|x;}fn main(){}",
        "E0415",
    );
}
#[test]
fn unused_invalid_refutable_parameter() {
    invalid(
        "refutable_parameter",
        "fn unused(){let f=|0:i32|1;}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_wrong_call_arity() {
    invalid(
        "wrong_call_arity",
        "fn unused(){let f=|&&x:&&i32|x;f();}fn main(){}",
        "E0057",
    );
}

#[test]
fn function_and_method_parameters_destructure_like_let() {
    oracle::check(
        "function_parameter_patterns",
        &[
            Case {
                name: "tuple_struct_reference_and_nested",
                source: "struct P{x:i32,y:i32}struct W(i32,String);fn tup((a,b):(i32,i32))->i32{a*10+b}fn st(P{x,y}:P)->i32{x-y}fn part(P{x:px,..}:&P)->i32{*px}fn ts(W(n,s):W)->String{format!(\"{s}{n}\")}fn rf(&(a,b):&(i32,i32))->i32{a+b}fn nested((a,(b,mut c)):(i32,(i32,i32)),_:u8,mut z:i32)->i32{c+=1;z+=1;a+b+c+z}fn cell(g:&[Vec<u8>],(r,c):(usize,usize))->u8{g[r][c]}fn bump(&mut (ref mut a,b):&mut (i32,i32)){*a+=b;}fn main(){let mut t=(1,2);bump(&mut t);println!(\"{} {} {} {} {} {} {} {:?}\",tup((3,4)),st(P{x:9,y:2}),part(&P{x:9,y:2}),ts(W(5,\"w\".to_string())),rf(&(5,6)),nested((1,(2,3)),0,4),cell(&[vec![1,2],vec![3,4]],(1,1)),t);}",
                expect: Expect::Run("34 7 9 w5 11 12 4 (3, 2)\n"),
            },
            Case {
                name: "methods_generics_and_trait_defaults",
                source: "struct P{x:i32,y:i32}struct G{c:Vec<char>,w:usize}impl G{fn at(&self,(r,c):(usize,usize))->&char{&self.c[r*self.w+c]}}impl P{fn of((x,y):(i32,i32))->Self{P{x,y}}fn add(&self,(dx,dy):(i32,i32))->P{P{x:self.x+dx,y:self.y+dy}}}trait T{fn d(&self,(a,b):(i32,i32))->i32{a+b}}impl T for P{}fn sw<A:Copy,B:Copy>((a,b):(A,B))->(B,A){(b,a)}fn main(){let g=G{c:vec!['a','b','c','d'],w:2};let p=P::of((1,2)).add((10,20));println!(\"{} {} {} {} {:?}\",g.at((1,0)),p.x,p.y,p.d((3,4)),sw((1u8,true)));}",
                expect: Expect::Run("c 11 22 7 (true, 1)\n"),
            },
            Case {
                name: "parameter_drop_order",
                source: "struct D(&'static str);impl Drop for D{fn drop(&mut self){println!(\"drop {}\",self.0);}}fn f((a,_):(D,D),_:D,(_,b):(D,D)){println!(\"body {} {}\",a.0,b.0);}fn g(x:D,(a,_):(D,D),mut y:D,n:i32,z:D){y=D(\"y2\");println!(\"body {} {} {} {} {}\",x.0,a.0,y.0,n,z.0);}fn main(){f((D(\"a\"),D(\"b\")),D(\"c\"),(D(\"d\"),D(\"e\")));println!(\"--\");g(D(\"x\"),(D(\"p\"),D(\"q\")),D(\"y\"),3,D(\"z\"));}",
                expect: Expect::Run(
                    "body a e\ndrop e\ndrop d\ndrop c\ndrop a\ndrop b\n--\ndrop y\nbody x p y2 3 z\ndrop z\ndrop y2\ndrop p\ndrop q\ndrop x\n",
                ),
            },
        ],
    );
}

#[test]
fn unused_invalid_refutable_function_parameter() {
    invalid(
        "refutable_function_parameter",
        "fn unused((1,x):(i32,i32))->i32{x}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_literal_function_parameter() {
    invalid(
        "literal_function_parameter",
        "fn unused(true:bool){}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_duplicate_function_parameter_binding() {
    invalid(
        "duplicate_function_parameter_binding",
        "fn unused(a:i32,(a,b):(i32,i32)){}fn main(){}",
        "E0415",
    );
}
#[test]
fn unused_invalid_function_parameter_pattern_type() {
    invalid(
        "function_parameter_pattern_type",
        "fn unused((a,b):(i32,i32,i32)){}fn main(){}",
        "E0308",
    );
}
#[test]
fn unused_invalid_bodiless_trait_method_pattern() {
    invalid(
        "bodiless_trait_method_pattern",
        "trait T{fn f(&self,(a,b):(i32,i32));}fn main(){}",
        "E0642",
    );
}
