//! Reference-bearing generic inputs and results, source-authored contexts,
//! recursive producer composition, and blanket method inference.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod differential;
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

fn check_generalized_cases(tag: &str, cases: &[Case]) {
    oracle::check(tag, cases);
    for case in cases {
        match case.expect {
            Expect::Run(_) => differential::stock_native(case.source),
            Expect::Invalid(_) => differential::stock_and_native_reject_observed(
                case.source,
                |workers, check_only, failure| {
                    let diagnostic = failure.to_string();
                    assert!(
                        !diagnostic.contains("unsupported")
                            && !diagnostic.contains("resource limit"),
                        "{} workers={workers} check_only={check_only}: {diagnostic}",
                        case.name
                    );
                },
            ),
        }
    }
}

fn generic_reference_return_cases() -> Vec<Case> {
    vec![
        Case {
            name: "first_of_distinct_generic_inputs",
            source: "fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let n=7i32;let b=true;let r=first(&n,&b);println!(\"{}\",*r);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "pair_of_generic_references",
            source: "fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn main(){let n=7i32;let b=true;let p=pair(&n,&b);println!(\"{} {}\",*p.0,*p.1);}",
            expect: Expect::Run("7 true\n"),
        },
        Case {
            name: "generic_option_reference",
            source: "fn wrap<T>(x:T)->Option<T>{Some(x)}fn main(){let n=7i32;let r=wrap(&n);match r{Some(value)=>println!(\"{}\",*value),None=>println!(\"none\")}}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "nested_reference_identity",
            source: "fn identity<T>(x:T)->T{x}fn main(){let n=7i32;let r=&n;let nested=identity(&r);println!(\"{}\",**nested);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "mutable_reference_result",
            source: "fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let mut n=7i32;let b=true;let r=first(&mut n,&b);*r+=1;println!(\"{}\",n);}",
            expect: Expect::Run("8\n"),
        },
        Case {
            name: "nested_reference_declared_lifetime",
            source: "fn choose<'a,T>(x:&'a T,y:&'a T,b:bool)->&'a T{if b{x}else{y}}fn main(){let a=7i32;let b=9i32;let ra=&a;let rb=&b;let result=choose(&ra,&rb,false);println!(\"{}\",**result);}",
            expect: Expect::Run("9\n"),
        },
        Case {
            name: "returned_borrow_cannot_outlive_local",
            source: "fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let r;{let n=7i32;let b=true;r=first(&n,&b);}println!(\"{}\",*r);}",
            expect: Expect::Invalid("E0597"),
        },
        Case {
            name: "returned_unique_borrow_blocks_owner_write",
            source: "fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let mut n=7i32;let b=true;let r=first(&mut n,&b);n=8;println!(\"{}\",*r);}",
            expect: Expect::Invalid("E0506"),
        },
        Case {
            name: "reference_actual_from_source_result_annotation",
            source: "fn empty<T>()->Option<T>{None}fn main(){let value:Option<&i32>=empty();match value{None=>println!(\"none\"),Some(_)=>println!(\"some\")}}",
            expect: Expect::Run("none\n"),
        },
    ]
}

fn check_generic_reference_return(index: usize) {
    let case = generic_reference_return_cases().remove(index);
    check_generalized_cases("generic-reference-results", &[case]);
}

#[test]
fn generic_first_reference_result_matches_stock() {
    check_generic_reference_return(0);
}
#[test]
fn generic_pair_reference_result_matches_stock() {
    check_generic_reference_return(1);
}
#[test]
fn generic_option_reference_result_matches_stock() {
    check_generic_reference_return(2);
}
#[test]
fn generic_nested_reference_result_matches_stock() {
    check_generic_reference_return(3);
}
#[test]
fn generic_mutable_reference_result_matches_stock() {
    check_generic_reference_return(4);
}
#[test]
fn generic_declared_nested_lifetime_result_matches_stock() {
    check_generic_reference_return(5);
}
#[test]
fn generic_reference_result_rejects_escape() {
    check_generic_reference_return(6);
}
#[test]
fn generic_reference_result_rejects_owner_mutation() {
    check_generic_reference_return(7);
}

#[test]
fn generic_reference_result_annotation_matches_stock() {
    check_generic_reference_return(8);
}

#[test]
fn generic_aggregate_expression_results_match_stock() {
    check_generalized_cases(
        "generic-aggregate-results",
        &[Case {
            name: "aggregate_expression_input_and_result",
            source: "fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn main(){let n=7i32;let b=true;let value=pair((&n,&b),Some(&n));println!(\"{} {}\",*value.0.0,*value.0.1);match value.1{Some(r)=>println!(\"{}\",*r),None=>println!(\"none\")}}",
            expect: Expect::Run("7 true\n7\n"),
        }],
    );
}

#[test]
fn generic_reference_results_preserve_rigid_caller_types() {
    check_generalized_cases(
        "generic-rigid-results",
        &[Case {
            name: "rigid_caller_reference_forwarding",
            source: "fn first<T,U>(x:T,y:U)->T{let _=y;x}fn forward<'a,T,U>(x:&'a T,y:&U)->&'a T{first(x,y)}fn main(){let n=7i32;let b=true;let r=forward(&n,&b);println!(\"{}\",*r);}",
            expect: Expect::Run("7\n"),
        }],
    );
}

#[test]
fn generic_reference_results_compose_across_calls() {
    check_generalized_cases(
        "generic-composed-results",
        &[Case {
            name: "returned_reference_as_next_generic_input",
            source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let n=7i32;let b=true;let previous=identity(&n);let result=first(previous,&b);println!(\"{}\",*result);}",
            expect: Expect::Run("7\n"),
        }],
    );
}

#[test]
fn generic_reference_result_conflicts_are_rejected() {
    check_generalized_cases(
        "generic-result-conflict",
        &[Case {
            name: "source_result_annotation_conflicts_with_input",
            source: "fn wrap<T>(x:T)->Option<T>{Some(x)}fn main(){let n=7i32;let value:Option<&bool>=wrap(&n);let _=value;}",
            expect: Expect::Invalid("E0308"),
        }],
    );
}

#[test]
fn composed_reference_results_retain_borrow_restrictions() {
    check_generalized_cases(
        "generic-composed-borrow",
        &[Case {
            name: "composed_unique_result_blocks_owner_write",
            source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let mut n=7i32;let b=true;let previous=identity(&mut n);let result=first(previous,&b);n=9;println!(\"{}\",*result);}",
            expect: Expect::Invalid("E0506"),
        }],
    );
}

#[test]
fn generic_reference_result_uses_declared_return_context() {
    check_generalized_cases(
        "generic-declared-result",
        &[Case {
            name: "declared_function_result_context",
            source: "fn empty<T>()->Option<T>{None}fn make<'a>()->Option<&'a i32>{empty()}fn main(){let value=make();match value{None=>println!(\"none\"),Some(_)=>println!(\"some\")}}",
            expect: Expect::Run("none\n"),
        }],
    );
}

#[test]
fn generic_results_compose_with_scoped_bindings() {
    check_generalized_cases(
        "generic-scoped-results",
        &[Case {
            name: "nested_shadowed_source_bindings",
            source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let n=1i32;{let n=7i32;let r=identity(&n);let flag=true;let result=first(r,&flag);println!(\"{}\",*result);}println!(\"{}\",n);}",
            expect: Expect::Run("7\n1\n"),
        }],
    );
}

#[test]
fn generic_results_compose_with_explicit_producers() {
    check_generalized_cases(
        "generic-explicit-results",
        &[
            Case {
                name: "explicit_producer_type_argument",
                source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let n=7i32;let flag=true;let r=identity::<&i32>(&n);let result=first(r,&flag);println!(\"{}\",*result);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "nested_explicit_producer_type_argument",
                source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let n=7i32;let flag=true;let result=first(identity::<&i32>(&n),&flag);println!(\"{}\",*result);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "explicit_output_only_producer_type_argument",
                source: "fn empty<T>()->Option<T>{None}fn identity<T>(x:T)->T{x}fn main(){let value=empty::<&i32>();let result=identity(value);match result{None=>println!(\"none\"),Some(_)=>println!(\"some\")}}",
                expect: Expect::Run("none\n"),
            },
        ],
    );
}

#[test]
fn scoped_generic_results_reject_escaping_borrows() {
    check_generalized_cases(
        "generic-scoped-escape",
        &[Case {
            name: "nested_producer_borrow_escape",
            source: "fn identity<T>(x:T)->T{x}fn first<T,U>(x:T,y:U)->T{let _=y;x}fn main(){let result;{let n=7i32;let flag=true;let r=identity(&n);result=first(r,&flag);}println!(\"{}\",*result);}",
            expect: Expect::Invalid("E0597"),
        }],
    );
}

#[test]
fn generic_scoped_result_annotation_constrains_inference() {
    check_generalized_cases(
        "generic-scoped-context",
        &[Case {
            name: "nested_result_annotation",
            source: "fn empty<T>()->Option<T>{None}fn main(){{let value:Option<&i32>=empty();match value{None=>println!(\"none\"),Some(_)=>println!(\"some\")}}}",
            expect: Expect::Run("none\n"),
        }],
    );
}

#[test]
fn generic_results_compose_inside_control_flow() {
    check_generalized_cases(
        "generic-branch-results",
        &[Case {
            name: "distinct_branch_local_origins",
            source: "fn identity<T>(x:T)->T{x}fn show(flag:bool){if flag{let n=7i32;let r=identity(&n);println!(\"{}\",*r);}else{let n=9i32;let r=identity(&n);println!(\"{}\",*r);}}fn main(){show(true);show(false);}",
            expect: Expect::Run("7\n9\n"),
        }],
    );
}

#[test]
fn generic_reference_inputs_preserve_array_literal_constraints() {
    check_generalized_cases(
        "generic-array-constraints",
        &[
            Case {
                name: "array_suffix_constraints_are_order_independent",
                source: "fn length<T>(s:&[T])->usize{s.len()}fn main(){let a=[3u8,4];let b=[3,4u8];println!(\"{} {}\",length(&a),length(&b));}",
                expect: Expect::Run("2 2\n"),
            },
            Case {
                name: "conflicting_array_suffix_constraints",
                source: "fn length<T>(s:&[T])->usize{s.len()}fn main(){let a=[3u8,4u16];println!(\"{}\",length(&a));}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn reference_actuals_match_stock() {
    check_generalized_cases(
        "reference-actuals",
        &[
            Case {
                name: "debug_option_ref",
                source: r#"fn main(){ let x = 5; let o: Option<&i32> = Some(&x); println!("{:?}", o); }"#,
                expect: Expect::Run("Some(5)\n"),
            },
            Case {
                name: "debug_struct_with_ref",
                source: r#"#[derive(Debug)]
struct W<'a> { r: &'a i32 }
fn main() { let x = 5; let w = W { r: &x }; let o = Some(&x); println!("{:?} {:?}", w, o); }"#,
                expect: Expect::Run("W { r: 5 } Some(5)\n"),
            },
            Case {
                name: "blanket_method_on_option_ref",
                source: r#"trait Id { fn id(self) -> Self; }
impl<T> Id for T { fn id(self) -> Self { self } }
fn main() { let x = 5; let r = Some(&x).id(); match r { Some(v) => println!("{}", v), None => {} } }"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "write_while_borrowed_through_actual_rejected",
                source: r#"trait Id { fn id(self) -> Self; }
impl<T> Id for T { fn id(self) -> Self { self } }
fn main() { let mut x = 5; let r = Some(&x).id(); x = 6; match r { Some(v) => println!("{}", v), None => {} } println!("{}", x); }"#,
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
