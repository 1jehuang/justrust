use justrust_engine::{Key, compiler};

#[test]
fn heterogeneous_shifts_reuse_checked_facts_and_check_has_no_objects() {
    let source = "fn left(x:i8,n:u64)->i8{x<<n}fn right(x:u64,n:i16)->u64{x>>n}fn main(){if left(1i8,7u64)==-128i8&&right(9223372036854775808u64,63i16)==1u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let c = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(c.report.capture_complete);
            assert_eq!(c.executable.is_none(), check);
            assert_eq!(c.control_flow.len(), 3);
            assert!(c.analyses.iter().all(|a| a.passed()));
            for index in 0..3 {
                for &kind in if check {
                    &[][..]
                } else {
                    &[102u32, 108, 109][..]
                } {
                    let key = Key::new(kind, vec![index]);
                    let evaluations: Vec<_> = c
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(k, _)| *k == key)
                        .collect();
                    assert_eq!(evaluations.len(), 1);
                    assert_eq!(evaluations[0].1, 1);
                }
            }
            assert_eq!(
                c.report
                    .node_evaluations
                    .iter()
                    .filter(|(k, _)| k.kind == 103)
                    .count(),
                if check { 0 } else { 3 }
            );
        }
    }
}

#[test]
fn unused_shift_bodies_require_integer_endpoints_and_valid_known_counts() {
    for source in [
        "fn bad()->u8{1u8<<8u8}fn main(){}",
        "fn bad()->u16{1u16>>-1i8}fn main(){}",
        "fn bad()->u8{1u8<<(4u8+4u8)}fn main(){}",
        "fn bad()->u8{1u8<<true}fn main(){}",
        "fn bad()->bool{true>>1u8}fn main(){}",
        "fn bad(p:&u8){let x=p<<1u8;}fn main(){}",
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
fn shift_operand_loan_violation_blocks_allbody_publication() {
    let source = "fn bad(p:&mut u8,n:u64){let r=&*p;*p=2u8;let x=*r<<n;}fn main(){println!(\"must not publish\");}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            if check {
                assert!(!report.node_evaluations.iter().any(|(k, _)| k.kind == 103));
            }
        }
    }
}

#[test]
fn typed_noncompleting_shift_operand_retains_completion_without_constant_value() {
    let source = "fn f()->u8{1u8<<{return 7u8;0u64}}fn main(){if f()==7u8{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let c = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(c.report.capture_complete);
            assert_eq!(c.executable.is_none(), check);
            assert!(c.analyses.iter().all(|a| a.passed()));
        }
    }
}
