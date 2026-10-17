use justrust_engine::compiler;
use justrust_engine::compiler::source::SourceOptions;

fn options(enabled: bool) -> SourceOptions {
    let mut options = SourceOptions::default();
    if enabled {
        options.cfg.insert_user("selected").unwrap();
    }
    options
}

#[test]
fn configured_text_source_is_observed_and_check_stays_codegen_free() {
    let source = "#[cfg(selected)] fn value()->i32{7}\n#[cfg(not(selected))] fn value()->i32{9}\nfn main(){let x=value();if x==7{println!(\"configured graph ok\");}}";
    let baseline = compiler::compile(
        "fn value()->i32{7}fn main(){let x=value();if x==7{println!(\"configured graph ok\");}}"
            .into(),
        1,
        false,
    )
    .unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile_observed_with_options(
                source.into(),
                workers,
                check,
                options(true),
            )
            .unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.source_bytes, source.len());
            assert_eq!(compiled.source_files, 1);
            assert_eq!(compiled.functions, 2);
            let source_nodes: Vec<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == 107)
                .collect();
            assert_eq!(source_nodes.len(), 1);
            assert_eq!(source_nodes[0].1, 1);
            assert_eq!(compiled.executable.is_none(), check);
            if check {
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| ![103, 105].contains(&key.kind))
                );
            } else {
                assert_eq!(compiled.executable, baseline.executable);
            }
        }
    }
}

#[test]
fn configuration_changes_select_fresh_semantics_not_stale_source_facts() {
    let source = "#[cfg(selected)] fn value()->i32{7}\n#[cfg(not(selected))] fn value()->i32{9}\nfn main(){let x=value();if x==7{println!(\"selected\");}}";
    for workers in [1, 8] {
        let yes =
            compiler::compile_observed_with_options(source.into(), workers, false, options(true))
                .unwrap();
        let no =
            compiler::compile_observed_with_options(source.into(), workers, false, options(false))
                .unwrap();
        assert!(yes.report.capture_complete && no.report.capture_complete);
        assert_ne!(yes.executable, no.executable);
        let again =
            compiler::compile_observed_with_options(source.into(), workers, false, options(true))
                .unwrap();
        assert_eq!(yes.executable, again.executable);
    }
}

#[test]
fn disabled_bodies_skip_semantics_but_not_syntax_validation() {
    for workers in [1, 8] {
        for check in [true, false] {
            let valid = "#[cfg(any())] fn unused(){unknown_name();}fn main(){}";
            assert!(
                compiler::compile_observed_with_options(
                    valid.into(),
                    workers,
                    check,
                    options(false)
                )
                .is_ok()
            );
            for invalid in [
                "#[cfg(any())] fn unused(){let = ;}fn main(){}",
                "#[cfg(all())] fn unused(){unknown_name();}fn main(){}",
            ] {
                let failure = compiler::compile_observed_with_options(
                    invalid.into(),
                    workers,
                    check,
                    options(false),
                )
                .err()
                .expect("invalid configured source must be rejected");
                assert!(failure.report.unwrap().capture_complete);
            }
        }
    }
}

#[test]
fn public_configuration_cannot_spoof_native_target_or_profile_facts() {
    for workers in [1, 8] {
        for check in [true, false] {
            let mut options = SourceOptions::default();
            options.cfg.values.insert(
                "target_arch".into(),
                ["aarch64".into()].into_iter().collect(),
            );
            let failure = compiler::compile_observed_with_options(
                "fn main(){}".into(),
                workers,
                check,
                options,
            )
            .err()
            .expect("spoofed native target must be rejected");
            assert!(failure.error.to_string().contains("native cfg"));
            let report = failure
                .report
                .expect("rejection must retain graph accounting");
            assert!(report.capture_complete);
            if check {
                assert!(
                    report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| ![103, 105].contains(&key.kind))
                );
            }
        }
    }
}
