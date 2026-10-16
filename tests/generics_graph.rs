use justrust_engine::compiler;

#[test]
fn generic_instances_reuse_facts_and_check_has_no_objects() {
    let source = "fn id<T>(x:T)->T{x}fn main(){let a=id::<u8>(7u8);let b=id::<u8>(9u8);let c=id::<u64>(18446744073709551615u64);if a==7u8&&b==9u8&&c==18446744073709551615u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for kind in [102, 108, 109] {
                let facts: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(facts.len(), 3);
                assert!(facts.iter().all(|(_, count)| *count == 1));
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

#[test]
fn unused_generic_moves_are_checked_universally_before_native_publication() {
    for source in [
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){println!(\"must not publish\");}",
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){let p=bad::<u64>(7u64);}",
    ] {
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
fn unused_generic_types_and_nominal_applications_cannot_use_concrete_witnesses() {
    for source in [
        "fn bad<T>(x:T)->T{x+1}fn main(){}",
        "struct Wrap<T>{value:T}fn bad<T>(x:Wrap<T>)->Wrap<u64>{x}fn main(){}",
        "fn bad<T>(x:T)->u8{x as u8}fn main(){}",
        "fn bad<T>(x:T)->T{x}fn main(){let x=7u8;let y=bad::<&u8>(&x);}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
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
fn exact_generic_recursion_deduplicates_but_growing_instances_fail_boundedly() {
    let exact = "fn relay<T>(x:T,n:i32)->T{if n==0{x}else{relay::<T>(x,n-1)}}fn main(){let x=relay::<u64>(18446744073709551615u64,3);if x==18446744073709551615u64{println!(\"ok\");}}";
    let growing = "fn grow<T>(x:T,n:i32)->i32{if n==0{7}else{grow::<(T,)>((x,),n-1)}}fn main(){let x=grow::<u8>(1u8,1);}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(exact.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.control_flow.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            let failure = compiler::compile_observed(growing.to_owned(), workers, check)
                .err()
                .unwrap();
            let message = failure.error.to_string().to_lowercase();
            assert!(
                message.contains("generic") && message.contains("limit"),
                "{message}"
            );
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(
                !report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 103)
            );
        }
    }
}
