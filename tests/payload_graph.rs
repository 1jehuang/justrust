use justrust_engine::{Key, compiler};

#[test]
fn payload_dispatch_and_proofs_are_cached_once_for_check_and_native_output() {
    let source = "struct S{x:i32}enum E{A(S,i32),B}fn make()->E{E::A(S{x:3},7)}fn main(){let e=make();let n:i32;match e{E::A(s,b)=>{n=s.x+b;},E::B=>{n=0;}}if n==10{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, 2);
            assert_eq!(compiled.structs, 1);
            assert_eq!(compiled.enums, 1);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|a| a.passed()));
            assert!(
                compiled
                    .analyses
                    .iter()
                    .map(|a| a.stats.variant_proof_checks)
                    .sum::<usize>()
                    >= 2
            );
            assert!(
                compiled
                    .analyses
                    .iter()
                    .map(|a| a.stats.variant_refinements)
                    .sum::<usize>()
                    >= 1
            );
            let actual_edges: usize = compiled
                .control_flow
                .iter()
                .flat_map(|f| &f.blocks)
                .map(|b| match b.terminator.value {
                    compiler::cfg::Terminator::Goto(_) | compiler::cfg::Terminator::Call { .. } => {
                        1
                    }
                    compiler::cfg::Terminator::Switch { .. }
                    | compiler::cfg::Terminator::SwitchVariant { .. } => 2,
                    _ => 0,
                })
                .sum();
            assert_eq!(compiled.cfg_edges, actual_edges);
            for index in 0..2 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let nodes: Vec<_> = compiled
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(k, _)| *k == key)
                        .collect();
                    assert_eq!(nodes.len(), 1);
                    assert_eq!(nodes[0].1, 1);
                }
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(k, _)| k.kind == 103)
                    .count(),
                if check { 0 } else { 2 }
            );
        }
    }
}

#[test]
fn moved_payload_failure_retains_the_exact_analysis() {
    let source = "struct S{x:i32}enum E{A(S,S),B}fn main(){let e=E::A(S{x:1},S{x:2});match e{E::A(s,_)=>{let x=s;},E::B=>{}}match e{E::A(_,_)=>{},E::B=>{}}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
            assert!(failure.report.unwrap().capture_complete);
        }
    }
}

#[test]
fn singleton_payload_patterns_do_not_add_tag_reads() {
    let source = "struct S{x:i32}enum E{A(S,S)}fn main(){let e=E::A(S{x:1},S{x:2});let E::A(a,_)=e;let E::A(_,b)=e;if a.x+b.x==3{println!(\"ok\");}}";
    let compiled = compiler::compile(source.to_owned(), 8, false).unwrap();
    assert!(compiled.analyses.iter().all(|a| a.passed()));
    assert!(
        compiled
            .control_flow
            .iter()
            .flat_map(|f| &f.blocks)
            .all(|b| !matches!(
                b.terminator.value,
                compiler::cfg::Terminator::SwitchVariant { .. }
            ))
    );
}
