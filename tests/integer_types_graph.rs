use justrust_engine::{Key, compiler};

#[test]
fn integer_types_share_exact_checked_facts_without_check_objects() {
    let source = "fn small(x:i8)->i8{x+1i8}fn wide(x:u64)->u64{x}fn get(p:&mut(i16,u64))->&mut u64{&mut p.1}fn main(){let mut pair=(-32768i16,18446744073709551615u64);let r=get(&mut pair);*r=wide(9223372036854775808u64);if small(-128i8)==-127i8&&pair.1==9223372036854775808u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 4);
            assert_eq!(compiled.analyses.len(), 4);
            assert!(compiled.analyses.iter().all(|a| a.passed()));
            assert_eq!(
                compiled.control_flow[0].result,
                compiler::ir::Ty::Int(compiler::ir::IntTy::I8)
            );
            assert_eq!(
                compiled.control_flow[1].result,
                compiler::ir::Ty::Int(compiler::ir::IntTy::U64)
            );
            for index in 0..4 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let evaluated: Vec<_> = compiled
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(actual, _)| *actual == key)
                        .collect();
                    assert_eq!(evaluated.len(), 1);
                    assert_eq!(evaluated[0].1, 1);
                }
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 4 }
            );
        }
    }
}

#[test]
fn unused_integer_reference_violation_blocks_publication() {
    let source = "fn bad(p:&i16){*p=3i16;}fn main(){println!(\"must not publish\");}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            if check {
                assert!(
                    !report
                        .node_evaluations
                        .iter()
                        .any(|(key, _)| key.kind == 103)
                );
            }
        }
    }
}

#[test]
fn equal_width_primitives_do_not_gain_implicit_identity() {
    for source in [
        "fn f(x:u64)->usize{x}fn main(){}",
        "fn f(x:i64)->isize{x}fn main(){}",
        "fn main(){let a=[1u8];let i=0u64;let x=a[i];}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
            }
        }
    }
}

#[test]
fn positive_out_of_range_literal_cannot_hide_inside_group_or_unused_function() {
    for source in [
        "fn bad()->i8{128i8}fn main(){}",
        "fn bad()->i8{-{128i8}}fn main(){}",
        "fn bad()->u64{18446744073709551616u64}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(compiler::compile_observed(source.to_owned(), workers, check).is_err());
            }
        }
    }
}
