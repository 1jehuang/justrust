//! Bounds checks stay per-function graph work and do not demand codegen at CHECK.
use justrust_engine::compiler;

#[test]
fn bounds_check_reuses_typed_control_flow_without_codegen() {
    let source = "struct S{x:i32}fn read(a:&[S],i:usize){let _r=&a[i];}fn write(a:&mut[i32],i:usize){a[i]=9;}fn main(){let a=[S{x:1}];read(&a,0);let mut b=[1];write(&mut b,0);}";
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(result.report.capture_complete);
            assert_eq!(result.executable.is_none(), check);
            assert!(result.analyses.iter().all(|analysis| analysis.passed()));
            assert!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(_, evaluations)| *evaluations == 1)
            );
            if check {
                assert!(
                    !result
                        .report
                        .node_evaluations
                        .iter()
                        .any(|(key, _)| key.kind == 103)
                );
            }
        }
    }
}
