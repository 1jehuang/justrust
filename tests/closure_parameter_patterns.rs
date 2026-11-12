//! Retained source parameter patterns use ordinary checked entry destructuring.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn captured_iterator_results_with_parameter_patterns() {
    oracle::check(
        "captured-iterator-parameter-patterns",
        &[
            Case {
                name: "wildcard_captured_iterator",
                source: "fn checksum(bytes:&[u8])->u8{let f=|_:i32|bytes.iter().cloned();f(0).sum()}fn main(){println!(\"{}\",checksum(&[2u8,5]));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "destructured_captured_iterator",
                source: "fn checksum(bytes:&[u8])->u8{let f=|(_, _):(i32,i32)|bytes.iter().cloned();f((1,2)).sum()}fn main(){println!(\"{}\",checksum(&[7u8,9]));}",
                expect: Expect::Run("16\n"),
            },
            Case {
                name: "unused_pattern_capture_escape",
                source: "fn unused()->impl Iterator<Item=u8>{let bytes=[1u8,2];(0..2).flat_map(|_|bytes.iter().cloned())}fn main(){}",
                expect: Expect::Invalid("E0373"),
            },
        ],
    );
}

#[test]
fn stored_closure_constant_inputs() {
    oracle::check(
        "stored_closure_constant_inputs",
        &[
            Case {
                name: "generic_associated_zero_filter",
                source: "trait Zero{const ZERO:Self;}impl Zero for i32{const ZERO:i32=0;}fn count_nonzero<T:Zero+PartialEq>(xs:&[T])->usize{xs.iter().filter(|x|**x!=T::ZERO).count()}fn main(){println!(\"{}\",count_nonzero(&[0i32,2,0,4]));}",
                expect: Expect::Run("2\n"),
            },
            Case {
                name: "direct_constant_closure",
                source: "const LIMIT:i32=3;fn main(){let f=|x:i32|x+LIMIT;println!(\"{}\",f(4));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "nested_constant_closure",
                source: "const LIMIT:i32=3;fn main(){let outer=||{let inner=||LIMIT;inner()};println!(\"{}\",outer());}",
                expect: Expect::Run("3\n"),
            },
        ],
    );
}

#[test]
fn stored_closure_constant_inputs_reject_invalid_sources() {
    oracle::check(
        "stored_closure_constant_inputs_invalid",
        &[
            Case {
                name: "unused_missing_constant_bound",
                source: "fn unused<T:PartialEq>(xs:&[T])->usize{xs.iter().filter(|x|**x!=T::ZERO).count()}fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "unused_wrong_constant_type",
                source: "trait Zero{const ZERO:bool;}fn unused<T:Zero>(){let _f=||->i32{T::ZERO};}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_constant_does_not_authorize_escape",
                source: "const LIMIT:i32=3;fn unused<'a>()->&'a i32{let local=LIMIT;let f=||&local;f()}fn main(){}",
                expect: Expect::Invalid("E0373"),
            },
        ],
    );
}

#[test]
fn closure_argument_call_aggregate_results() {
    oracle::check(
        "closure_argument_call_aggregate_results",
        &[
            Case {
                name: "split_once_filter_map",
                source: "fn main(){let v=\"a=1;b=2;;c=3\".split(';').filter(|p|!p.is_empty()).filter_map(|p|p.split_once('=')).collect::<Vec<_>>();println!(\"{:?}\",v);}",
                expect: Expect::Run("[(\"a\", \"1\"), (\"b\", \"2\"), (\"c\", \"3\")]\n"),
            },
            Case {
                name: "tuple_call_argument_sources",
                source: "fn pair(s:&str)->(&str,&str){(s,s)}fn main(){let v=\"a b\".split(' ').map(|s|pair(s)).collect::<Vec<_>>();println!(\"{:?}\",v);}",
                expect: Expect::Run("[(\"a\", \"a\"), (\"b\", \"b\")]\n"),
            },
            Case {
                name: "unused_owned_argument_escape",
                source: "fn unused(){let _f=|s:String|s.split_once('=');}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_argument_result_type",
                source: "fn unused(){let _f=|s:&str|->Option<(bool,bool)>{s.split_once('=')};}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn generic_iterator_closure_projection() {
    oracle::check(
        "generic_iterator_closure_projection",
        &[Case {
            name: "generic_map_sum",
            source: "fn total<I:Iterator<Item=i32>>(it:I)->i32{it.map(|x|x+1).sum()}fn main(){println!(\"{}\",total((0..3).into_iter()));}",
            expect: Expect::Run("6\n"),
        }],
    );
}

#[test]
fn generic_iterator_fold_source_projection() {
    oracle::check(
        "generic_iterator_closure_projection",
        &[
            Case {
                name: "unchanged_m2_generic_traits",
                source: include_str!("milestones/m2/02_generic_traits.rs"),
                expect: Expect::Run(include_str!("milestones/m2/02_generic_traits.stdout")),
            },
            Case {
                name: "generic_fold_parameter_and_reference_pattern",
                source: "use std::ops::Add;fn sum_all<T:Add<Output=T>+Default+Copy>(xs:&[T])->T{xs.iter().fold(T::default(),|a,&b|a+b)}fn main(){println!(\"{} {}\",sum_all(&[1u32,2,3]),sum_all(&[0.25f64,0.5]));}",
                expect: Expect::Run("6 0.75\n"),
            },
        ],
    );
}

#[test]
fn generic_iterator_closure_projection_rejects_invalid_sources() {
    oracle::check(
        "generic_iterator_closure_projection_invalid",
        &[
            Case {
                name: "unused_wrong_closure_argument",
                source: "fn unused<I:Iterator<Item=i32>>(it:I)->i32{it.map(|x:bool|if x{1}else{0}).sum()}fn main(){}",
                expect: Expect::Invalid("E0631"),
            },
            Case {
                name: "unused_noncopy_reference_pattern",
                source: "use std::ops::Add;fn unused<T:Add<Output=T>+Default>(xs:&[T])->T{xs.iter().fold(T::default(),|a,&b|a+b)}fn main(){}",
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "unused_generic_result_local_escape",
                source: "use std::ops::Add;fn unused<'a,T:Add<Output=T>+Copy>(x:T)->&'a T{let f=|a:T,b:T|a+b;let value=f(x,x);&value}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

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
fn struct_patterns_move_fields_with_destructors() {
    oracle::check(
        "drop_field_struct_patterns",
        &[Case {
            name: "let_and_parameter_struct_patterns",
            source: "struct D(&'static str);impl Drop for D{fn drop(&mut self){println!(\"drop {}\",self.0);}}struct S{a:D,b:D}struct W(D,i32);fn f(S{a,..}:S,W(d,n):W){println!(\"{} {} {}\",a.0,d.0,n);}fn main(){let s=S{a:D(\"a\"),b:D(\"b\")};let S{b,..}=s;println!(\"{}\",b.0);let p=(S{a:D(\"c\"),b:D(\"d\")},D(\"e\"));let(S{a,..},c)=p;println!(\"{} {}\",c.0,a.0);f(S{a:D(\"f\"),b:D(\"g\")},W(D(\"w\"),3));println!(\"end\");}",
            expect: Expect::Run(
                "b\ne c\nf w 3\ndrop w\ndrop f\ndrop g\nend\ndrop e\ndrop c\ndrop d\ndrop b\ndrop a\n",
            ),
        }],
    );
}

#[test]
fn static_str_fields_accept_static_parameters() {
    oracle::check(
        "static_str_field_parameters",
        &[Case {
            name: "static_parameter_into_static_field",
            source: "struct D(&'static str);struct N{s:&'static str}fn mk(s:&'static str)->N{N{s}}fn main(){let d=D(\"a\");let n=mk(\"b\");let f=D;let e=f(\"c\");let v:Vec<D>=[\"x\",\"y\"].into_iter().map(D).collect();println!(\"{} {} {} {}\",d.0,n.s,e.0,v[1].0);}",
            expect: Expect::Run("a b c y\n"),
        }],
    );
}
#[test]
fn unused_invalid_elided_parameter_into_static_field() {
    invalid(
        "elided_parameter_into_static_field",
        "struct N{s:&'static str}fn unused(s:&str)->N{N{s}}fn main(){}",
        "lifetime may not live long enough",
    );
}
#[test]
fn unused_invalid_local_borrow_into_static_tuple_field() {
    invalid(
        "local_borrow_into_static_tuple_field",
        "struct D(&'static str);fn unused(){let s=String::new();let _d=D(&s);}fn main(){}",
        "E0597",
    );
}

#[test]
fn tuple_pattern_binders_feed_inferred_borrows() {
    oracle::check(
        "tuple_pattern_borrows",
        &[Case {
            name: "identity_and_vec_of_pattern_binders",
            source: "fn id<T>(x:T)->T{x}fn main(){let (n,m)=(3usize,7usize);let r=id(&n);let v=vec![&n,&m];let (p,(q,b)):(i64,(u8,bool))=(5,(9,true));let s=id(&q);let t=id(&b);let (mut k,_)=(1i32,2i32);let u=id(&mut k);*u+=1;let (a,c)=(String::from(\"x\"),2u8);let w=vec![&a];println!(\"{} {} {} {} {} {} {} {} {}\",r,v[0],v[1],s,t,p,k,w[0],c);}",
            expect: Expect::Run("3 3 7 9 true 5 2 x 2\n"),
        }],
    );
}
#[test]
fn unused_invalid_move_of_borrowed_pattern_binder() {
    invalid(
        "move_borrowed_pattern_binder",
        "fn unused(){let (a,b)=(String::from(\"x\"),2u8);let v=vec![&a];drop(a);println!(\"{} {}\",v[0],b);}fn main(){}",
        "E0505",
    );
}
#[test]
fn unused_invalid_assign_of_borrowed_pattern_binder() {
    invalid(
        "assign_borrowed_pattern_binder",
        "fn id<T>(x:T)->T{x}fn unused(){let (mut n,m)=(3i32,7i32);let r=id(&n);n=5;println!(\"{} {} {}\",r,n,m);}fn main(){}",
        "E0506",
    );
}

#[test]
fn unused_invalid_refutable_enum_function_parameter() {
    invalid(
        "refutable_enum_function_parameter",
        "fn unused(Some(x):Option<i32>)->i32{x}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_refutable_nested_enum_parameter() {
    invalid(
        "refutable_nested_enum_parameter",
        "enum E{A(i32),B}fn unused((E::A(x),_):(E,u8))->i32{x}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_refutable_enum_let() {
    invalid(
        "refutable_enum_let",
        "enum E{A(i32),B}fn unused(e:E)->i32{let E::A(x)=e;x}fn main(){}",
        "E0005",
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

#[test]
fn default_binding_reborrow_result_outlives_closure() {
    oracle::check(
        "default-binding-reborrow-result",
        &[
            Case {
                name: "tuple_field_as_str",
                source: "fn main(){let v:Vec<(String,String)>=vec![(\"the\".to_string(),\"cat\".to_string()),(\"a\".to_string(),\"dog\".to_string())];let mut s:Vec<&str>=v.iter().filter(|(a,_)|a==\"the\").map(|(_,b)|b.as_str()).collect();s.sort();println!(\"{s:?} {}\",v.iter().map(|(a,_)|a.as_str()).collect::<Vec<_>>().join(\",\"));}",
                expect: Expect::Run("[\"cat\"] the,a\n"),
            },
            Case {
                name: "by_value_capture_reference_escapes",
                source: "fn main(){let v:Vec<(String,String)>=vec![(\"a\".to_string(),\"b\".to_string())];let s=String::from(\"x\");let f=move|(_,_):&(String,String)|->&str{s.as_str()};println!(\"{:?}\",v.iter().map(f).count());}",
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "reborrow_result_outlives_vec",
                source: "fn main(){let s:Vec<&str>;{let v:Vec<(String,String)>=vec![(\"a\".to_string(),\"b\".to_string())];s=v.iter().map(|(_,b)|b.as_str()).collect();}println!(\"{s:?}\");}",
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}
