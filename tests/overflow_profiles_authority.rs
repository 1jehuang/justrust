//! Source policy is immutable provenance, not a caller-forgeable cfg predicate.
use justrust_engine::compiler::{
    self, frontend,
    source::{self, SourceOptions},
    ssa::OptMode,
};

#[test]
fn source_policy_snapshot_is_independent_of_later_options() {
    for checked in [false, true] {
        let mut options = SourceOptions {
            overflow_checks: checked,
            ..SourceOptions::default()
        };
        options.cfg.set_debug_assertions(!checked);
        let prepared =
            source::prepare_text_with_prelude("policy.rs", "fn main(){}", &options).unwrap();
        options.overflow_checks = !checked;
        assert_eq!(prepared.overflow_checks(), checked);
        assert_eq!(prepared.debug_assertions(), !checked);
        assert_eq!(prepared.clone().overflow_checks(), checked);
        frontend::parse_prepared(&prepared).unwrap();
    }
    assert!(SourceOptions::default().overflow_checks);
}

#[test]
fn forged_prepared_stream_does_not_reuse_embedded_authority() {
    let options = SourceOptions {
        overflow_checks: false,
        ..SourceOptions::default()
    };
    let mut prepared =
        source::prepare_text_with_prelude("policy.rs", "fn main(){}", &options).unwrap();
    prepared.text.push_str(" fn forged(){}");
    assert!(frontend::parse_prepared(&prepared).is_err());
}

#[test]
fn unchecked_policy_does_not_accept_invalid_operand_types() {
    for opt in [OptMode::Off, OptMode::Adaptive, OptMode::Final] {
        for workers in [1, 8] {
            for source in [
                "fn bad(x:u8,y:bool)->u8{x+y}fn main(){}",
                "fn bad(x:u8,y:bool)->u8{x<<y}fn main(){}",
                "fn bad(x:u8)->u8{-x}fn main(){}",
            ] {
                eprintln!("overflow invalid CHECK opt={opt:?} workers={workers}: {source}");
                let options = SourceOptions {
                    overflow_checks: false,
                    opt,
                    ..SourceOptions::default()
                };
                assert!(
                    compiler::compile_observed_with_options(source.into(), workers, true, options)
                        .is_err(),
                    "{source}"
                );
            }
        }
    }
}
