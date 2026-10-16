use justrust_engine::{Key, compiler};

#[test]
fn eager_bitwise_reuses_checked_facts_and_check_publishes_no_objects() {
    let source = "fn bits(x:i8,y:i8)->i8{x^y}fn eager(x:bool,y:bool)->bool{x&y}fn read(p:&mut(u8,u64))->u64{(p.0 as u64)|p.1}fn main(){let mut p=(255u8,9223372036854775808u64);if bits(-1i8,127i8)==-128i8&&eager(true,true)&&read(&mut p)==9223372036854776063u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 4);
            assert_eq!(compiled.analyses.len(), 4);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for index in 0..4 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let facts: Vec<_> = compiled
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(actual, _)| *actual == key)
                        .collect();
                    assert_eq!(facts.len(), 1);
                    assert_eq!(facts[0].1, 1);
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
fn unused_bitwise_loan_violation_blocks_publication() {
    let source = "fn bad(p:&mut i8){let s=&*p;*p=2i8;let n=*s&1i8;}fn main(){println!(\"must not publish\");}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(
                failure
                    .analyses
                    .iter()
                    .any(|(_, analysis)| !analysis.passed())
            );
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
fn unused_bitwise_bodies_still_require_exact_primitive_operands() {
    for source in [
        "fn bad()->u8{1u8&1u16}fn main(){}",
        "fn bad()->bool{true|1u8}fn main(){}",
        "fn bad(p:&u8,q:&u8){let x=p^q;}fn main(){}",
        "fn bad(){let x=()&();}fn main(){}",
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
fn eager_boolean_rhs_noncompletion_is_not_a_short_circuit_result() {
    let source = "fn f()->bool{false&{return true;false}}fn main(){if f(){println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(compiled.control_flow[0].result, compiler::ir::Ty::Bool);
        }
    }
}
