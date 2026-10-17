use justrust_engine::compiler;

#[test]
fn public_reexports_keep_original_native_callable_identity() {
    let declarations =
        "mod internal{pub fn value()->i32{7}}mod api{pub use crate::internal::value as exposed;}";
    let direct = format!("{declarations}fn main(){{let x=internal::value();}}");
    let exported = format!("{declarations}fn main(){{let x=api::exposed();}}");
    let expected = compiler::compile(direct, 1, false).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(exported.clone(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, expected.functions);
            assert_eq!(compiled.executable.is_none(), check);
            if !check {
                assert_eq!(compiled.executable, expected.executable);
            }
            for cfg in &compiled.control_flow {
                cfg.validate().unwrap();
            }
        }
    }
}

#[test]
fn unused_public_aliases_do_not_promote_private_declarations() {
    for source in [
        "fn secret(){}pub use secret as exposed;fn main(){}",
        "struct Secret{x:i32}pub use Secret as Exposed;fn main(){}",
        "mod hidden{}pub use hidden as visible;fn main(){}",
        "trait Secret{}pub use Secret as Exposed;fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
                assert!(
                    !failure
                        .error
                        .to_string()
                        .contains("pub use reexports are unsupported"),
                    "reexport must reach visibility checking: {}",
                    failure.error
                );
            }
        }
    }
}
