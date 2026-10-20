use justrust_engine::compiler;

#[test]
fn private_stride_check_remains_codegen_free_without_new_effect_root() {
    let source =
        "fn main(){let value=Box::new(7u64);let got=*value.as_ref();assert_eq!(got,7u64);}";
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(!checked.report.published_kind(103));
        assert!(!checked.report.published_kind(137));
    }
}
