use justrust_engine::compiler;

#[test]
fn wide_scalar_graphs_preserve_both_limbs_and_alias_identity() {
    let declarations = "type Wide=u128;fn id<T>(x:T)->T{x}";
    let direct = format!(
        "{declarations}fn main(){{let x:u128=170141183460469231731687303715884105735;let y=id::<u128>(x);if y==x{{println!(\"wide graph ok\");}}}}"
    );
    let aliased = direct.replace("let x:u128", "let x:Wide");
    let baseline = compiler::compile(direct, 1, false).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let actual = compiler::compile(aliased.clone(), workers, check).unwrap();
            assert!(actual.report.capture_complete);
            assert_eq!(actual.functions, baseline.functions);
            assert_eq!(actual.executable.is_none(), check);
            if !check {
                assert_eq!(actual.executable, baseline.executable);
            }
            for cfg in &actual.control_flow {
                cfg.validate().unwrap();
            }
        }
    }
}

#[test]
fn wide_constants_with_equal_low_limbs_do_not_share_native_identity() {
    let low = "fn main(){let x:u128=7;let y=x; if y==7{println!(\"wide graph ok\");}}";
    let high = low.replace("let x:u128=7", "let x:u128=18446744073709551623");
    for workers in [1, 8] {
        let low = compiler::compile(low.into(), workers, false).unwrap();
        let high = compiler::compile(high.clone(), workers, false).unwrap();
        assert!(low.report.capture_complete && high.report.capture_complete);
        assert_ne!(low.executable, high.executable);
    }
}

#[test]
fn unused_invalid_wide_values_are_validated_before_codegen() {
    for (source, expected) in [
        (
            "fn unused(){let x:u128=340282366920938463463374607431768211456;}fn main(){}",
            "exceeds u128",
        ),
        (
            "fn unused(){let x:i128=170141183460469231731687303715884105728;}fn main(){}",
            "out of range",
        ),
        ("fn unused(){let x=-1u128;}fn main(){}", "unsigned"),
        (
            "fn unused(){let x=1u128+1i128;}fn main(){}",
            "does not match",
        ),
        ("fn unused(){let x=1u128<<128u128;}fn main(){}", "shift"),
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
                let diagnostic = failure.error.to_string();
                assert!(
                    diagnostic.contains(expected),
                    "invalid wide value must reach semantic validation: {diagnostic}"
                );
            }
        }
    }
}
