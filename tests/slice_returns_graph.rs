use justrust_engine::compiler;

#[test]
fn slice_returns_preserve_typed_results_without_check_codegen() {
    let source = "fn shared(s:&[i32])->&[i32]{s}fn mutable(s:&mut[i32])->&mut[i32]{s}fn main(){let mut a=[1,2];let r=mutable(&mut a);r[0]=4;let q=shared(&a);let n=q.len();println!(\"{}\",n);}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true).unwrap();
        assert!(result.executable.is_none());
        assert!(result.report.capture_complete);
        assert!(result.analyses.iter().all(|analysis| analysis.passed()));
        assert!(!result.report.published_kind(103));
        assert_eq!(
            result
                .control_flow
                .iter()
                .filter(|function| function.result.slice_ref_element().is_some())
                .count(),
            2
        );
    }
}
