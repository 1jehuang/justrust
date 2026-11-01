//! Built-in `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` on nongeneric and
//! generic structs and enums. Derives expand per item into ordinary impls
//! (source_derive.rs) that are resolved and checked like hand-written code.
//! Stock-differential at workers 1 and 8; missing field impls are rejected
//! even when the deriving type is unused.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn derived_reference_fields_keep_original_lifetime_authority() {
    oracle::check(
        "derive-reference-field-authority",
        &[
            Case {
                name: "reference_clone_survives_carrier",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let value=7i32;let escaped;{let item=Item{value:&value};escaped=item.clone();}println!(\"{}\",escaped.value);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "reference_self_not_referent_must_implement_clone",
                source: "struct NonClone;#[derive(Clone)]struct Item<'a>{value:&'a NonClone,tag:i32}fn main(){let value=NonClone;let item=Item{value:&value,tag:9i32};let cloned=item.clone();let _=cloned.value;println!(\"{}\",cloned.tag);}",
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "reference_clone_cannot_escape_original_referent",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let escaped;{let value=7i32;let item=Item{value:&value};escaped=item.clone();}println!(\"{}\",escaped.value);}",
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "reference_clone_keeps_original_referent_loan",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let mut value=7i32;let item=Item{value:&value};let cloned=item.clone();value=8i32;println!(\"{}\",cloned.value);}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "unused_mutable_reference_field_is_not_clone",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a mut i32}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn derived_impls_match_stock() {
    oracle::check(
        "derive_traits_run",
        &[
            Case {
                name: "struct_enum_generic_debug_clone_eq",
                source: "#[derive(Clone, Copy, PartialEq, Eq, Debug)]struct P{x:i32,y:i32}#[derive(Clone, Copy, PartialEq, Eq, Debug)]struct T(i32,bool);#[derive(Clone, Copy, PartialEq, Eq, Debug)]struct U;#[derive(Clone, PartialEq, Debug)]enum E{A,B(i32),C{v:i32,w:bool}}#[derive(Clone, Copy, PartialEq, Debug)]struct W<T>{inner:T}fn main(){let p=P{x:1,y:-2};let q=p;println!(\"{:?} {:?} {}\",p,q,p==q);println!(\"{:?} {:?}\",T(3,true),U);let e=E::C{v:5,w:false};let f=e.clone();println!(\"{:?} {:?} {:?} {} {}\",E::A,E::B(7),f,e==f,E::A==E::B(1));let w=W{inner:p};println!(\"{:?} {}\",w,w==w.clone());println!(\"{:#?}\",p);println!(\"{:#?}\",E::B(1));println!(\"{:#?}\",w);}",
                expect: Expect::Run(
                    "P { x: 1, y: -2 } P { x: 1, y: -2 } true\nT(3, true) U\nA B(7) C { v: 5, w: false } true false\nW { inner: P { x: 1, y: -2 } } true\nP {\n    x: 1,\n    y: -2,\n}\nB(\n    1,\n)\nW {\n    inner: P {\n        x: 1,\n        y: -2,\n    },\n}\n",
                ),
            },
            Case {
                name: "tuple_fields_and_tuple_generic_args",
                source: "#[derive(Debug,Clone,PartialEq,Eq)]struct I{t:(u8,bool)}fn f<T:std::fmt::Debug>(t:&T){println!(\"{:?}\",t);}fn main(){let i=I{t:(6,true)};println!(\"{:?} {}\",i.clone(),i==i);println!(\"{:#?}\",i);f(&(1,));f(&(1,2,(3,4)));}",
                expect: Expect::Run(
                    "I { t: (6, true) } true\nI {\n    t: (\n        6,\n        true,\n    ),\n}\n(1,)\n(1, 2, (3, 4))\n",
                ),
            },
            Case {
                name: "generic_eq_constructor_operand",
                source: "#[derive(PartialEq,PartialOrd,Debug)]struct W<T>{a:T}#[derive(PartialEq)]enum G<T>{N,S(T)}fn main(){let x:W<u8>=W{a:1};let g:G<u16>=G::S(3);println!(\"{} {} {} {} {}\",x==W{a:1},x!=W{a:2},x<W{a:2},g==G::S(3),g==G::N);}",
                expect: Expect::Run("true true true true false\n"),
            },
            Case {
                name: "generic_eq_field_assertion",
                source: "#[derive(PartialEq,Eq,Debug)]struct W<T>{a:T,b:i32}#[derive(PartialEq,Eq,Debug)]enum G<T>{N,S(T)}#[derive(Debug)]struct I{a:i32}#[derive(Debug)]struct O{v:I,g:G<i32>}fn main(){let x:W<u8>=W{a:1,b:2};let y:W<u8>=W{a:1,b:3};let p:G<i32>=G::S(4);let q:G<i32>=G::N;println!(\"{} {} {:?} {} {:?} {:?}\",x==y,x==x,x,p==q,p,q);println!(\"{:#?}\",O{v:I{a:1},g:G::S(2)});}",
                expect: Expect::Run(
                    "false true W { a: 1, b: 2 } false S(4) N\nO {\n    v: I {\n        a: 1,\n    },\n    g: S(\n        2,\n    ),\n}\n",
                ),
            },
        ],
    );
}

#[test]
fn comparison_constructor_reverse_generic_operands() {
    oracle::check(
        "comparison-constructor-authority",
        &[Case {
            name: "reverse_generic_constructor_operands",
            source: "#[derive(PartialEq,PartialOrd)]struct W<T>{a:T}#[derive(PartialEq)]enum G<T>{N,S(T)}fn main(){let x:W<u8>=W{a:1};let g:G<u16>=G::S(3);println!(\"{} {} {} {}\",W{a:1}==x,W{a:2}>x,G::S(3)==g,G::N==g);}",
            expect: Expect::Run("true true true false\n"),
        }],
    );
}

#[test]
fn comparison_constructor_unique_cross_nominal_rhs() {
    oracle::check(
        "comparison-constructor-cross-nominal",
        &[Case {
            name: "unique_cross_nominal_rhs_equation",
            source: "struct L<T>{a:T}struct R<T>{a:T}impl<T:PartialEq> PartialEq<R<T>> for L<T>{fn eq(&self,r:&R<T>)->bool{self.a==r.a}}fn main(){let x:L<u8>=L{a:7};println!(\"{}\",x==R{a:7});}",
            expect: Expect::Run("true\n"),
        }],
    );
}

#[test]
fn comparison_constructor_preserves_explicit_rhs_type() {
    oracle::check(
        "comparison-constructor-explicit-rhs",
        &[Case {
            name: "derived_equation_does_not_erase_explicit_rhs_type",
            source: "#[derive(PartialEq)]struct W<T>{a:T}fn main(){let x:W<u8>=W{a:1};let y:W<u16>=W{a:1};let same=x==y;}",
            expect: Expect::Invalid("E0308"),
        }],
    );
}

#[test]
fn comparison_constructor_preserves_field_bound() {
    oracle::check(
        "comparison-constructor-field-bound",
        &[Case {
            name: "derived_equation_does_not_prove_field_bound",
            source: "struct N;#[derive(PartialEq)]struct W<T>{a:T}fn main(){let x:W<N>=W{a:N};let same=x==W{a:N};}",
            expect: Expect::Invalid("E0369"),
        }],
    );
}

#[test]
fn comparison_constructor_rejects_noncanonical_eq() {
    oracle::check(
        "comparison-constructor-noncanonical",
        &[Case {
            name: "lookalike_eq_method_is_not_canonical_comparison",
            source: "struct W<T>{a:T}trait FakeEq<R>{fn eq(&self,r:&R)->bool;}impl<T> FakeEq<W<T>> for W<T>{fn eq(&self,r:&W<T>)->bool{true}}fn main(){let x:W<u8>=W{a:1};let same=x==W{a:1u8};}",
            expect: Expect::Invalid("E0369"),
        }],
    );
}

#[test]
fn missing_field_impls_reject_like_stock() {
    let cases = [
        Case {
            name: "partial_eq_field",
            source: "struct N;#[derive(PartialEq)]struct S{n:N}fn main(){}",
            expect: Expect::Invalid("E0369"),
        },
        Case {
            name: "partial_eq_enum_field",
            source: "struct N;#[derive(PartialEq)]enum E{A(N)}fn main(){}",
            expect: Expect::Invalid("E0369"),
        },
        Case {
            name: "clone_field",
            source: "struct N;#[derive(Clone)]struct S{n:N}fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "debug_field",
            source: "struct N;#[derive(Debug)]struct S(N);fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "eq_field",
            source: "#[derive(PartialEq)]struct N;#[derive(PartialEq,Eq)]struct S{n:N}fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "copy_without_clone",
            source: "#[derive(Copy)]struct S;fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "eq_without_partial_eq",
            source: "#[derive(Eq)]struct S;fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "copy_non_copy_field",
            source: "#[derive(Clone)]struct N;#[derive(Clone,Copy)]struct S{n:N}fn main(){}",
            expect: Expect::Invalid("E0204"),
        },
        Case {
            name: "generic_debug_unsatisfied_use",
            source: "struct N;#[derive(Debug)]struct W<T>(T);fn main(){println!(\"{:?}\",W(N));}",
            expect: Expect::Invalid("E0277"),
        },
    ];
    oracle::check("derive_traits_invalid", &cases);
    // The oracle only checks stock's code. Native must report the same code.
    for case in &cases {
        let Expect::Invalid(code) = case.expect else {
            unreachable!()
        };
        for workers in [1, 8] {
            for check_only in [true, false] {
                let error =
                    justrust_engine::compiler::compile(case.source.into(), workers, check_only)
                        .err()
                        .unwrap_or_else(|| panic!("{} accepted", case.name))
                        .to_string();
                assert!(
                    error.contains(code),
                    "{} workers={workers} CHECK={check_only}: expected {code}, got: {error}",
                    case.name
                );
            }
        }
    }
}
