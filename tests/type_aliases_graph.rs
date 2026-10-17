use justrust_engine::compiler;

#[test]
fn aliases_preserve_native_nominal_and_callable_identity() {
    let declarations = "struct S<T>{x:T}type Alias<T>=S<T>;fn value()->i32{7}";
    let direct = format!("{declarations}fn main(){{let s=S::<i32>{{x:value()}};}}");
    let aliased = format!("{declarations}fn main(){{let s=Alias::<i32>{{x:value()}};}}");
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
fn unused_invalid_aliases_are_checked_without_codegen() {
    for source in [
        "type A=Missing;fn main(){}",
        "struct S<T>{x:T}type A=S;fn main(){}",
        "type A<T>=i32;fn main(){}",
        "type A=A;fn main(){}",
        "type A<T>=A<(T,T)>;fn main(){}",
        "mod a{}type A=a;fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failed = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failed.report.unwrap().capture_complete);
                assert!(
                    !failed.error.to_string().contains("unsupported"),
                    "invalid alias must reach semantic validation: {}",
                    failed.error
                );
            }
        }
    }
}
