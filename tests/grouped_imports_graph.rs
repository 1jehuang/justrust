use justrust_engine::compiler;

#[test]
fn grouped_aliases_preserve_canonical_calls_and_native_image() {
    let declarations = "mod m{pub fn first()->i32{3}pub mod n{pub fn second()->i32{4}}}";
    let direct = format!("{declarations}fn main(){{let n=m::first()+m::n::second();}}");
    let grouped = format!(
        "{declarations}use crate::m::{{first as a,n::{{second as b}},}};fn main(){{let n=a()+b();}}"
    );
    let expected = compiler::compile(direct, 1, false).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(grouped.clone(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, expected.functions);
            assert_eq!(compiled.executable.is_none(), check);
            if !check {
                assert_eq!(compiled.executable, expected.executable);
            }
            for cfg in &compiled.control_flow {
                cfg.validate().unwrap();
            }
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}

#[test]
fn grouped_imports_do_not_hide_unused_invalid_bodies_or_cycles() {
    for source in [
        "mod m{pub fn bad()->i32{true}}use crate::m::{bad as unused};fn main(){}",
        "mod m{}use crate::m::{missing};fn main(){}",
        "use crate::{a as b,b as a};fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
            }
        }
    }
}
