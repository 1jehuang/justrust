//! Local scalar inference with real stock acceptance/rejection witnesses.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod differential;
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn loop_break_results_retain_later_type_constraints() {
    let cases = [
        Case {
            name: "nested_break_value",
            source: "fn main(){let x=loop{break 7+loop{break 2;};};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("9\n"),
        },
        Case {
            name: "tuple_break_projection",
            source: "fn main(){let pair=loop{break (7,true);};let wide:u64=pair.0;println!(\"{} {}\",wide,pair.1);}",
            expect: Expect::Run("7 true\n"),
        },
        Case {
            name: "outer_label_not_inner_loop",
            source: "fn main(){let x='outer:loop{let y=loop{if false{break 'outer 9;}break true;};if y{break 7;}};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "labeled_block_break_value",
            source: "fn main(){let x='done:{if true{break 'done 11;}13};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("11\n"),
        },
        Case {
            name: "conflicting_break_types",
            source: "fn unused(){let _x=loop{if true{break 1u8;}break 2u16;};}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "shadowed_labels",
            source: "fn main(){let x='same:loop{let y='same:loop{break 'same 3;};let narrow:u8=y;break 'same 1000+narrow as u64;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("1003\n"),
        },
        Case {
            name: "closure_loop_does_not_constrain_outer",
            source: "fn main(){let x=loop{let f=||->u8{loop{break 3;}};let y=f();break 1000+y as u64;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("1003\n"),
        },
        Case {
            name: "outer_break_through_while",
            source: "fn main(){let x='outer:loop{while true{break 'outer 7;}break 9;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "outer_break_through_for",
            source: "fn main(){let x='outer:loop{for n in 0..3{if n==1{break 'outer 7;}}break 9;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "outer_break_through_while_let",
            source: "fn main(){let x='outer:loop{while let true=true{break 'outer 7;}break 9;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "nested_operand_breaks_outer",
            source: "fn main(){let x='outer:loop{break loop{break 'outer 7;};};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "labeled_block_unit_break",
            source: "fn main(){let x='block:{break 'block;};let unit:()=x;println!(\"{}\",7);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "continue_does_not_supply_result",
            source: "fn main(){let mut first=true;let x='outer:loop{if first{first=false;continue 'outer;}break 7;};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "no_break_diverges",
            source: "fn unused()->u64{let x=loop{};x}fn main(){let x=if true{7}else{loop{continue;}};let wide:u64=x;println!(\"{}\",wide);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "diverging_break_operand",
            source: "fn unused()->u64{loop{break loop{};}}fn nested()->u64{loop{break ('inner:{loop{}});}}fn main(){println!(\"{}\",7);}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "closure_cannot_break_outer",
            source: "fn unused(){'outer:loop{let f=||{break 'outer 1;};f();}}fn main(){}",
            expect: Expect::Invalid("E0767"),
        },
        Case {
            name: "closure_cannot_continue_outer",
            source: "fn unused(){'outer:loop{let f=||{continue 'outer;};f();}}fn main(){}",
            expect: Expect::Invalid("E0767"),
        },
        Case {
            name: "while_cannot_break_value",
            source: "fn unused(){while true{break 1;}}fn main(){}",
            expect: Expect::Invalid("E0571"),
        },
        Case {
            name: "for_cannot_break_value",
            source: "fn unused(){for n in 0..3{break 1;}}fn main(){}",
            expect: Expect::Invalid("E0571"),
        },
        Case {
            name: "block_cannot_continue",
            source: "fn unused(){'block:{continue 'block;}}fn main(){}",
            expect: Expect::Invalid("E0696"),
        },
        Case {
            name: "block_tail_conflicts_with_break",
            source: "fn unused(){let x='block:{if true{break 'block 1u8;}2u16};}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "unlabeled_break_does_not_skip_block",
            source: "fn unused(){loop{'block:{break;}}}fn main(){}",
            expect: Expect::Invalid("E0695"),
        },
    ];
    for case in cases {
        eprintln!("loop inference case: {}", case.name);
        match case.expect {
            Expect::Run(_) => differential::stock_native(case.source),
            Expect::Invalid(_) => {
                differential::stock_and_native_reject_observed(
                    case.source,
                    |workers, check, failure| {
                        assert!(
                            failure
                                .report
                                .as_ref()
                                .is_some_and(|report| report.capture_complete),
                            "{} workers={workers} check={check}: incomplete failure capture: {failure}",
                            case.name,
                        );
                        let diagnostic = failure.error.to_string().to_lowercase();
                        for infrastructure in [
                            "invalidmetadata",
                            "invalid metadata",
                            "resource limit",
                            "work limit",
                            "work-limit",
                            "limit exceeded",
                            "limit-exceeded",
                            "exceeded limit",
                            "budget",
                        ] {
                            assert!(
                                !diagnostic.contains(infrastructure),
                                "{} workers={workers} check={check}: not a source rejection: {failure}",
                                case.name,
                            );
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn unused_scalar_conflicts_are_user_errors() {
    let cases = [
        Case {
            name: "unused_fixed_width_conflict",
            source: "fn unused(){let x=1u128+1i128;}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "unused_annotation_conflict",
            source: "fn unused(){let x:u8=1u16;}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "unused_float_integer_conflict",
            source: "fn unused(){let x=1.0f32+1i32;}fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "unused_assignment_conflict",
            source: "fn unused(){let mut x=1u8;x=2u16;}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "unused_return_conflict",
            source: "fn unused()->u32{let x=1u64;x}fn main(){}",
            expect: Expect::Invalid("E0308"),
        },
    ];
    oracle::check("unused-scalar-conflicts", &cases);
    let domains = [
        ("Fixed(Int(U128))", "Fixed(Int(I128))"),
        ("Fixed(Int(U16))", "Fixed(Int(U8))"),
        ("Fixed(Float(F32))", "Fixed(Int(I32))"),
        ("Fixed(Int(U8))", "Fixed(Int(U16))"),
        ("Fixed(Int(U64))", "Fixed(Int(U32))"),
    ];
    for (case, (left, right)) in cases.iter().zip(domains) {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure =
                    justrust_engine::compiler::compile_observed(case.source.into(), workers, check)
                        .err()
                        .unwrap();
                assert!(failure.report.unwrap().capture_complete);
                let diagnostic = failure.error.to_string();
                // Stock may classify an unsupported operator as E0277;
                // native inference reports the conflicting domains as E0308.
                assert!(diagnostic.starts_with("E0308:"), "{diagnostic}");
                assert!(diagnostic.contains("mismatched types"), "{diagnostic}");
                assert!(!diagnostic.contains("InvalidMetadata"), "{diagnostic}");
                assert!(diagnostic.contains(left), "{diagnostic}");
                assert!(diagnostic.contains(right), "{diagnostic}");
            }
        }
    }
}

#[test]
fn later_numeric_constraints_match_stock() {
    oracle::check(
        "local-literals",
        &[
            Case {
                name: "frozen_collatz",
                source: include_str!("fixtures/full_programs/03_collatz.rs"),
                expect: Expect::Run(include_str!("fixtures/full_programs/03_collatz.stdout")),
            },
            Case {
                name: "field_peer",
                source: "struct S{n:usize}fn main(){let s=S{n:4};let x=1+s.n;println!(\"{}\",x);}",
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "explicit_generic_peer",
                source: "fn id<T>(x:T)->T{x}fn main(){let x=1+id::<u8>(2);println!(\"{}\",x);}",
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "omitted_generic_peer",
                source: "fn id<T>(x:T)->T{x}fn main(){let x=1+id(2u8);let y=id(3)+4u8;println!(\"{} {}\",x,y);}",
                expect: Expect::Run("3 7\n"),
            },
            Case {
                name: "tail",
                source: "fn f()->u32{let mut steps=0;steps+=1;steps}fn main(){println!(\"{}\",f());}",
                expect: Expect::Run("1\n"),
            },
            Case {
                name: "return",
                source: "fn f()->u64{let x=7;return x;}fn main(){println!(\"{}\",f());}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "assignment",
                source: "fn main(){let mut x=0;x=9u64;println!(\"{}\",x);}",
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "aliases",
                source: "fn main(){let x=3;let y=x;let z=y;let fixed:u64=z;println!(\"{} {}\",x,fixed);}",
                expect: Expect::Run("3 3\n"),
            },
            Case {
                name: "binary_initializer",
                source: "fn main(){let x=3+4;let fixed:u64=x;println!(\"{}\",fixed);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "argument",
                source: "fn f(x:u64)->u64{x}fn main(){let x=7;println!(\"{}\",f(x));}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "comparison",
                source: "fn main(){let x=3;let fixed=4u64;if x<fixed{println!(\"ok\");}}",
                expect: Expect::Run("ok\n"),
            },
            Case {
                name: "nested_assignment",
                source: "fn f()->u32{8}fn main(){let mut x=0;if true{let y=f();x=y;}println!(\"{}\",x);}",
                expect: Expect::Run("8\n"),
            },
            Case {
                name: "loop_range",
                source: "fn main(){let mut x=0;for n in 1..4u64{x=n;}println!(\"{}\",x);}",
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "shadow",
                source: "fn main(){let x=1;{let x=2;let y:u64=x;println!(\"{}\",y);}let y:i32=x;println!(\"{}\",y);}",
                expect: Expect::Run("2\n1\n"),
            },
            Case {
                name: "old_frontier",
                source: "fn main(){let x=1;let x=x+2;let y:u64=x;println!(\"{}\",y);}",
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "float_assignment",
                source: "fn main(){let mut x=1.0;x=7.0f32;println!(\"{}\",x as u32);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "float_tail",
                source: "fn f()->f32{let x=3.5;x}fn main(){println!(\"{}\",f() as u32);}",
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "integer_default",
                source: "fn main(){let x=1;println!(\"{}\",x-2);}",
                expect: Expect::Run("-1\n"),
            },
            Case {
                name: "float_default",
                source: "fn main(){let x=16777217.0;println!(\"{}\",x as u32);}",
                expect: Expect::Run("16777217\n"),
            },
            Case {
                name: "shift_independent",
                source: "fn main(){let x=1;let y:u64=x;println!(\"{}\",y<<2u8);}",
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "explicit_fixed",
                source: "fn main(){let mut x:i32=0;x=1u64;}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "suffix_fixed",
                source: "fn main(){let mut x=0i32;x=1u64;}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "conflicting_uses",
                source: "fn a(x:u32){}fn b(x:u64){}fn main(){let x=0;a(x);b(x);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "conflicting_aliases",
                source: "fn main(){let x=0;let y=x;let a:u32=x;let b:u64=y;}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "out_of_range",
                source: "fn main(){let x=256;let y:u8=x;}",
                expect: Expect::Invalid("overflowing_literals"),
            },
            Case {
                name: "negative_unsigned",
                source: "fn main(){let x=-1;let y:u32=x;}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "float_fixed",
                source: "fn main(){let mut x=0.0f64;x=1.0f32;}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "mixed_classes",
                source: "fn main(){let mut x=0;x=1.0;}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
