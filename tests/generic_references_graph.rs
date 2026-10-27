use justrust_engine::compiler;

const PROJECTED: &str = r#"trait Field{fn field(&self)->&i32;fn field_mut(&mut self)->&mut i32;}struct S{x:i32}impl Field for S{fn field(&self)->&i32{&self.x}fn field_mut(&mut self)->&mut i32{&mut self.x}}fn shared<T:Field>(x:&T)->&i32{x.field()}fn mutable<T:Field>(x:&mut T)->&mut i32{x.field_mut()}fn run()->i32{let mut s=S{x:3};*mutable::<S>(&mut s)=7;*shared::<S>(&s)}
fn main(){if run()==7{println!("generic references oracle ok");}}
"#;

const TWO_PHASE: &str = r#"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32)->&mut i32{self.x=x;&mut self.x}}fn run()->i32{let mut s=S{x:4};let mut i=0;while i<3{let r=s.set(s.get()+1);let _=*r;i+=1;}s.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"#;

#[test]
fn generic_projected_returns_are_proven_before_native_publication() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(PROJECTED.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.functions, 6);
            assert_eq!(compiled.control_flow.len(), 6);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for &kind in if check {
                &[][..]
            } else {
                &[102u32, 108, 109][..]
            } {
                let nodes: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(nodes.len(), 6);
                assert!(nodes.iter().all(|(_, count)| *count == 1));
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
fn two_phase_per_iteration_loans_have_worker_stable_native_graphs() {
    let one = compiler::compile(TWO_PHASE.to_owned(), 1, false).unwrap();
    let eight = compiler::compile(TWO_PHASE.to_owned(), 8, false).unwrap();
    assert_eq!(one.executable, eight.executable);
    for compiled in [&one, &eight] {
        assert!(compiled.report.capture_complete);
        assert_eq!(compiled.functions, 4);
        assert_eq!(compiled.control_flow.len(), 4);
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        assert!(
            compiled
                .report
                .node_evaluations
                .iter()
                .all(|(_, count)| *count == 1)
        );
    }
}

#[test]
fn unused_generic_dereference_move_is_rejected_universally() {
    let source = "fn bad<T>(x:&T)->T{*x}fn main(){}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(!report.published_kind(105));
        }
    }
}

#[test]
fn immediate_explicit_mutable_borrow_is_not_promoted_to_reservation() {
    let source = r#"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run(){let mut s=S{x:6};S::set(&mut s,s.get()+1);}fn main(){run();}"#;
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
            assert!(!report.published_kind(105));
        }
    }
}
