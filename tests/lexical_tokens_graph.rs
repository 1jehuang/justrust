use justrust_engine::compiler;

#[test]
fn native_lexical_trivia_preserves_graph_identity_and_codegen_demand() {
    let plain = "fn value(x:i32)->i32{x+1}fn main(){let n=value(6);if n==7{println!(\"lexical graph ok\");}}";
    let decorated = "// Unicode comment: β\r\n/* outer /* nested */ comment */\r\nfn value ( x : i32 ) -> i32 { x + 1 }\r\nfn main ( ) { let n = value ( 6 ) ; if n == 7 { println ! ( \"lexical graph ok\" ) ; } }";
    let baseline = compiler::compile(plain.into(), 1, false).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile_observed(decorated.into(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.source_bytes, decorated.len());
            assert_eq!(compiled.source_files, 1);
            assert_eq!(compiled.functions, baseline.functions);
            assert_eq!(compiled.executable.is_none(), check);
            if check {
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| { ![103, 105].contains(&key.kind) })
                );
            } else {
                assert_eq!(compiled.executable, baseline.executable);
            }
        }
    }
}

#[test]
fn lexical_failures_complete_graph_without_publishing_objects() {
    for source in [
        "fn main(){let x=0b2;}",
        "fn main(){println!(\"bad\\q\");}",
        "fn main(){/* unterminated",
        "fn main(){`}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .expect("malformed lexical source must fail");
                let report = failure
                    .report
                    .expect("lexical rejection keeps graph report");
                assert!(report.capture_complete, "{source}");
                // BUILD may request an object task whose dependency then fails.
                // CHECK must never request code generation, even on rejection.
                if check {
                    assert!(
                        report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| { ![103, 105].contains(&key.kind) }),
                        "CHECK requested code generation after lexical failure: {source}"
                    );
                }
            }
        }
    }
}

#[test]
fn lexical_success_does_not_skip_unused_body_validation() {
    let source = "/* β */\r\nfn unused(){let x:i32=true;}fn main(){}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.into(), workers, check)
                .err()
                .expect("unused invalid body remains invalid after scanner cutover");
            assert!(
                failure
                    .report
                    .expect("body rejection keeps report")
                    .capture_complete
            );
        }
    }
}
