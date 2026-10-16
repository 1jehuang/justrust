use justrust_engine::compiler;

const CONDITIONAL: &str = r#"struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn consume<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{consume::<W<W<S>>>(W::<W<S>>{x:W::<S>{x:S{x:7}}})}
fn main(){if run()==7{println!("traits oracle ok");}}
"#;

#[test]
fn required_signatures_have_no_native_slots_and_conditional_calls_close_before_objects() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(CONDITIONAL.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            // Source callable declarations exclude required trait items. The
            // generic W::value body contributes two distinct native instances.
            assert_eq!(compiled.functions, 5);
            assert_eq!(compiled.control_flow.len(), 6);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for kind in [102, 108, 109] {
                let facts: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(facts.len(), 6);
                assert!(facts.iter().all(|(_, count)| *count == 1));
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| {
                        key.kind == 112
                            && key
                                .params
                                .first()
                                .is_some_and(|index| *index < compiled.functions as u64)
                    })
                    .count(),
                2
            );
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 6 }
            );
        }
    }
}

#[test]
fn unused_bounded_body_still_needs_universal_move_proof() {
    let source = r#"trait Value{fn value(self)->i32;}fn bad<T:Value>(x:T)->(T,T){(x,x)}fn run()->i32{7}
fn main(){run();}
"#;
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
                report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 112)
            );
            assert!(
                !report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 103)
            );
        }
    }
}

#[test]
fn unused_conformance_overlap_and_missing_bound_fail_before_any_native_publication() {
    for source in [
        r#"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct S{x:i32}trait Value{fn value(self,x:i32)->i32;}impl Value for S{fn value(self,x:u8)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{3}}impl Value for W<i32>{fn value(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct U{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn bounded<T:Value>(x:T)->i32{x.value()}fn missing<T>(x:T)->i32{bounded::<T>(x)}fn dead(){if false{bounded::<U>(U{x:7});}}fn run()->i32{W::<U>{x:U{x:7}}.value()}
fn main(){run();}
"#,
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
fn bound_forwarding_and_conditional_instances_are_worker_deterministic() {
    let forwarding = r#"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}fn bounded<T:Value>(x:T)->i32{x.value()}fn forward<U:Value>(x:U)->i32{bounded::<U>(x)}fn run()->i32{forward::<S>(S{x:7})}
fn main(){if run()==7{println!("traits oracle ok");}}
"#;
    for (source, instances) in [(CONDITIONAL, 6), (forwarding, 5)] {
        let one = compiler::compile(source.to_owned(), 1, false).unwrap();
        let eight = compiler::compile(source.to_owned(), 8, false).unwrap();
        assert_eq!(one.executable, eight.executable);
        for compiled in [&one, &eight] {
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.control_flow.len(), instances);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                instances
            );
        }
    }
}
