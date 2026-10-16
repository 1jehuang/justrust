use justrust_engine::{Key, compiler};

#[test]
fn matches_retain_one_shared_checked_cfg_and_analysis() {
    let source = "enum State{Ready,Busy,Done} fn next(s:State)->State{match s{State::Ready=>State::Busy,State::Busy=>State::Done,State::Done=>State::Ready}} fn main(){let s=next(State::Ready);match s{State::Busy=>println!(\"ok\"),_=>{}};match s{State::Ready=>{},State::Busy=>{},State::Done=>{}};}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compilation = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert_eq!(compilation.enums, 1);
            assert_eq!(compilation.functions, 2);
            assert_eq!(compilation.structs, 0);
            assert_eq!(compilation.executable.is_none(), check);
            assert!(compilation.report.capture_complete);
            assert_eq!(compilation.control_flow.len(), 2);
            assert_eq!(compilation.analyses.len(), 2);
            assert!(compilation.analyses.iter().all(|a| a.passed()));
            assert_eq!(
                compilation.cfg_blocks,
                compilation
                    .control_flow
                    .iter()
                    .map(|f| f.blocks.len())
                    .sum()
            );
            for index in 0..2 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let evaluations: Vec<_> = compilation
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
                compilation
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
fn testing_a_moved_enum_retains_failed_analysis() {
    let source = "enum E{A,B} fn main(){let e=E::A;let moved=e;match e{E::A=>{},E::B=>{}};}";
    for check in [true, false] {
        let failure = compiler::compile_observed(source.to_owned(), 8, check)
            .err()
            .unwrap();
        assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
        assert!(failure.report.unwrap().capture_complete);
    }
}

#[test]
fn wildcard_only_match_of_moved_place_has_no_discriminant_read() {
    let source = "enum E{A,B} fn main(){let e=E::A;let moved=e;match e{_=>{}};}";
    let compilation = compiler::compile(source.to_owned(), 8, false).unwrap();
    assert!(compilation.analyses.iter().all(|a| a.passed()));
    assert!(
        compilation
            .control_flow
            .iter()
            .flat_map(|f| &f.blocks)
            .flat_map(|b| &b.statements)
            .all(|s| !matches!(
                &s.value,
                compiler::cfg::Statement::Assign {
                    value: compiler::cfg::Rvalue::Discriminant(_),
                    ..
                }
            ))
    );
}
