use justrust_engine::{Key, compiler};

#[test]
fn mixed_capabilities_retain_actual_input_kinds_and_result_authority() {
    let source = "struct Pair{left:i32,right:usize}fn read<'a,'b>(p:&'a mut Pair,flag:&'b bool)->&'a usize{&p.right}fn write<'a,'b>(p:&'a mut Pair,flag:&'b bool)->&'a mut usize{&mut p.right}fn main(){let mut p=Pair{left:1,right:4294967296usize};let flag=true;let r=write(&mut p,&flag);*r=18446744073709551615usize;let s=read(&mut p,&flag);if *s==18446744073709551615usize{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|a| a.passed()));
            for (index, function) in compiled.control_flow[..2].iter().enumerate() {
                let kind = if index == 0 {
                    compiler::ir::BorrowKind::Shared
                } else {
                    compiler::ir::BorrowKind::Mut
                };
                let paths = match &function.ref_contract.returns {
                    compiler::ir::ReturnContract::SharedInputPaths(paths) if index == 0 => paths,
                    compiler::ir::ReturnContract::MutInputPaths(paths) if index == 1 => paths,
                    _ => panic!("expected result-capability contract"),
                };
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0].param, 0);
                assert_eq!(
                    paths[0].projection.as_ref(),
                    [compiler::ir::Projection::Field(1)]
                );
                assert_eq!(function.loan_sites.len(), 1);
                assert_eq!(function.loan_sites[0].kind, kind);
                assert_eq!(function.loan_sites[0].place.local, function.params[0]);
                assert_eq!(
                    function.loan_sites[0].place.projection.as_slice(),
                    [
                        compiler::ir::Projection::Deref,
                        compiler::ir::Projection::Field(1)
                    ]
                );
                assert!(matches!(
                    function.locals[function.params[0].0].ty,
                    compiler::ir::Ty::Ref {
                        kind: compiler::ir::BorrowKind::Mut,
                        ..
                    }
                ));
                assert!(matches!(
                    function.locals[function.params[1].0].ty,
                    compiler::ir::Ty::Ref {
                        kind: compiler::ir::BorrowKind::Shared,
                        ..
                    }
                ));
            }
            for index in 0..3 {
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
                if check { 0 } else { 3 }
            );
        }
    }
}

fn fails_in_analysis(source: &str) {
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
fn unrelated_shared_input_with_same_region_pins_mutable_result() {
    fails_in_analysis(
        "fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x}fn main(){let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r=7;}",
    );
}

#[test]
fn unused_shared_to_mutable_upgrade_blocks_executable_publication() {
    fails_in_analysis(
        "fn bad<'a>(x:&'a i32)->&'a mut i32{&mut *x}fn main(){println!(\"must not publish\");}",
    );
}

#[test]
fn explicit_mutable_borrow_to_shared_coercion_remains_unsupported() {
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(
                "fn get(x:&mut i32)->&i32{&mut *x}fn main(){}".to_owned(),
                workers,
                check,
            )
            .err()
            .unwrap();
            assert!(failure.to_string().contains("unsupported"));
            assert!(failure.report.unwrap().capture_complete);
        }
    }
}
