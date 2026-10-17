use justrust_engine::compiler;

#[test]
fn restricted_aliases_preserve_native_canonical_identity() {
    let declarations = "mod hidden{pub(crate) fn value()->i32{7}pub(crate) struct S{pub(crate) x:i32}}mod api{pub(crate) use crate::hidden::{value as exposed,S};}";
    let direct = format!("{declarations}fn main(){{let s=hidden::S{{x:hidden::value()}};}}");
    let aliased = format!("{declarations}fn main(){{let s=api::S{{x:api::exposed()}};}}");
    let expected = compiler::compile(direct, 1, false).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let actual = compiler::compile(aliased.clone(), workers, check).unwrap();
            assert!(actual.report.capture_complete);
            assert_eq!(actual.functions, expected.functions);
            assert_eq!(actual.executable.is_none(), check);
            if !check {
                assert_eq!(actual.executable, expected.executable);
            }
            for cfg in &actual.control_flow {
                cfg.validate().unwrap();
            }
        }
    }
}

#[test]
fn nested_trait_impls_keep_lexical_owner_visibility() {
    let source = "mod outer{pub(crate) struct S{pub(crate) x:i32}pub(crate) trait T{fn value(&self)->i32;}impl T for S{fn value(&self)->i32{self.x}}pub(crate) fn run()->i32{let s=S{x:7};s.value()}}fn main(){let x=outer::run();}";
    for workers in [1, 8] {
        for check in [true, false] {
            let actual = compiler::compile(source.into(), workers, check).unwrap();
            assert!(actual.report.capture_complete);
            assert_eq!(actual.executable.is_none(), check);
            assert!(actual.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}

#[test]
fn unused_restricted_scopes_are_validated_before_codegen() {
    for source in [
        "pub(super) fn unused(){}fn main(){}",
        "mod a{pub(in crate::a::missing) fn unused(){}}fn main(){}",
        "mod a{}mod b{pub(in crate::a) struct Unused{x:i32}}fn main(){}",
        "mod a{pub(in crate::missing) struct Unused{x:i32}}fn main(){}",
        "mod a{pub(crate) fn value(){}pub use self::value as widened;}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failed = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failed.report.unwrap().capture_complete);
                assert!(
                    !failed.error.to_string().contains("unsupported"),
                    "{}",
                    failed.error
                );
            }
        }
    }
}
