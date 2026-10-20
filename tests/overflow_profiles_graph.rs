//! Profile-sensitive compiler results never leak across immutable graph snapshots.
use justrust_engine::compiler::{
    self,
    source::SourceOptions,
    ssa::{self, OptMode},
};

#[test]
fn check_never_demands_native_or_ssa_codegen_in_either_profile() {
    for overflow_checks in [false, true] {
        for workers in [1, 8] {
            let mut baseline = None;
            for opt in [OptMode::Off, OptMode::Adaptive, OptMode::Final] {
                eprintln!(
                    "overflow graph CHECK checked={overflow_checks} opt={opt:?} workers={workers}"
                );
                let checked = compiler::compile_observed_with_options(
                    "fn f(x:u8)->u8{x+1u8}fn main(){let x=f(1u8);}".into(),
                    workers,
                    true,
                    SourceOptions {
                        overflow_checks,
                        opt,
                        prelude: false,
                        ..SourceOptions::default()
                    },
                )
                .unwrap();
                assert!(checked.executable.is_none());
                let mut kinds = std::collections::BTreeMap::new();
                for (key, _) in &checked.report.node_evaluations {
                    *kinds.entry(key.kind).or_insert(0usize) += 1;
                    assert!(
                        !matches!(key.kind, 103 | 105 | 130..=133),
                        "CHECK demanded {key:?}"
                    );
                }
                if let Some(expected) = &baseline {
                    assert_eq!(&kinds, expected);
                } else {
                    baseline = Some(kinds);
                }
            }
        }
    }
}

#[test]
fn policy_changes_native_objects_but_parallel_results_are_deterministic() {
    let source = "fn f(x:u8)->u8{x+1u8}fn main(){println!(\"{}\",f(255u8));}";
    for opt in [OptMode::Off, OptMode::Adaptive, OptMode::Final] {
        let build = |overflow_checks, workers| {
            eprintln!(
                "overflow graph BUILD checked={overflow_checks} opt={opt:?} workers={workers}"
            );
            compiler::compile_observed_with_options(
                source.into(),
                workers,
                false,
                SourceOptions {
                    overflow_checks,
                    opt,
                    prelude: false,
                    ..SourceOptions::default()
                },
            )
            .unwrap()
        };
        let checked = build(true, 1);
        let unchecked = build(false, 1);
        assert_ne!(checked.executable, unchecked.executable);
        assert_eq!(checked.executable, build(true, 8).executable);
        assert_eq!(unchecked.executable, build(false, 8).executable);
        if opt != OptMode::Off {
            assert!(checked.optimizer.optimized > 0);
            assert!(unchecked.optimizer.optimized > 0);
        }
    }
}

#[test]
fn ssa_content_identity_includes_overflow_semantics() {
    fn function(overflow: ssa::Overflow) -> ssa::SsaFunction {
        let ty = ssa::SsaTy::Int(ssa::SsaIntTy::U8);
        let (mut builder, args) = ssa::Builder::new(vec![ty], ty);
        let one = builder.constant(ssa::SsaBlock(0), ty, 1);
        let sum = builder.inst(
            ssa::SsaBlock(0),
            ssa::InstKind::Binary {
                op: ssa::BinOp::Add,
                overflow,
                lhs: args[0],
                rhs: one,
            },
        );
        builder.terminate(ssa::SsaBlock(0), ssa::SsaTerminator::Return(sum));
        builder.finish()
    }
    assert_ne!(
        ssa::hash::content_hash(&function(ssa::Overflow::Trap)),
        ssa::hash::content_hash(&function(ssa::Overflow::Wrap))
    );
}
