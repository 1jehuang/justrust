use justrust_engine::compiler;

#[test]
fn inferred_and_explicit_instances_share_facts_and_check_has_no_objects() {
    let source = "fn id<T>(x:T)->T{x}fn main(){let a=id(7u8);let b=id::<u8>(9u8);let c=id(18446744073709551615u64);if a==7u8&&b==9u8&&c==18446744073709551615u64{println!(\"ok\");}}";
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
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){let p=bad(7u64);}",
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
fn inference_failures_never_publish_objects() {
    for source in [
        "fn same<T>(a:T,b:T)->T{b}fn main(){let x=same(1u8,2u16);}",
        "fn empty<T>(){}fn main(){empty();}",
        "fn id<T>(x:T)->T{x}fn main(){let x=id(id(7));}",
        "fn same<T>(a:T,b:T)->T{b}fn main(){let x=same(1,{let y=2u8;y});}",
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
fn inferred_nominal_instances_are_deterministic_across_workers() {
    let source = "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn relay<T>(x:T,n:i32)->T{if n==0{x}else{relay(x,n-1)}}fn main(){let a=Wrap{value:7u8};let b:Wrap<u64>=Wrap{value:18446744073709551615u64};let c=relay(a,3);let d=id::<Wrap<u64>>(b);if c.value==7u8&&d.value==18446744073709551615u64{println!(\"ok\");}}";
    let one = compiler::compile(source.to_owned(), 1, false).unwrap();
    let eight = compiler::compile(source.to_owned(), 8, false).unwrap();
    assert_eq!(one.executable, eight.executable);
    assert_eq!(one.control_flow.len(), 3);
    for compiled in [&one, &eight] {
        assert!(compiled.report.capture_complete);
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        assert_eq!(
            compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == 103)
                .count(),
            3
        );
    }
}
