use justrust_engine::{Key, compiler};

#[test]
fn integer_casts_share_exact_checked_facts_and_check_emits_no_objects() {
    let source = "fn narrow(x:i64)->i8{x as i8}fn wide(x:i8)->u64{x as u64}fn read(p:&mut(u8,u64))->u64{p.0 as u64}fn main(){let mut p=(255u8,18446744073709551615u64);if narrow(255i64)==-1i8&&wide(-1i8)==18446744073709551615u64&&read(&mut p)==255u64{println!(\"ok\");}}";
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
                for &kind in if check {
                    &[][..]
                } else {
                    &[102u32, 108, 109][..]
                } {
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
fn unused_cast_operand_loan_violation_blocks_publication() {
    let source = "fn bad(p:&mut i8){let s=&*p;*p=2i8;let n=*s as u64;}fn main(){println!(\"must not publish\");}";
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
                assert!(!report.published_kind(105));
            }
        }
    }
}

#[test]
fn unused_casts_cannot_bypass_source_ranges_or_reference_escape_boundaries() {
    for source in [
        "fn bad()->u8{300 as u8}fn main(){}",
        "fn bad()->u16{256u8 as u16}fn main(){}",
        "fn bad()->u8{(-1) as u8}fn main(){}",
        "fn bad(p:&i8)->usize{p as usize}fn main(){}",
        // This bool conversion is valid Rust, but explicitly unsupported here.
        "fn bad()->u8{true as u8}fn main(){}",
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
fn integer_typed_noncompletion_retains_type_without_a_physical_cast_value() {
    let source =
        "fn f()->u64{({return 7u64;0i8}) as u64}fn main(){if f()==7u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled.control_flow[0].result,
                compiler::ir::Ty::Int(compiler::ir::IntTy::U64)
            );
        }
    }
}
