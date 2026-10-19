//! A1 original associated-function docs only, not general attribute support.
//! Stock-first oracle: /usr/bin/rustc 1.98.1, commit
//! 48a229ceaefd4985c50990b14116b6d856af0985, edition2024,
//! x86_64-unknown-linux-gnu, lib metadata, default cfg/profile, 2026-10-19.
//! Warning-accepted doc=42/doc(foo), inner docs, associated const/type docs and
//! statement docs remain explicit native policy exclusions, not invalid Rust.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{
    self,
    source::{self, SourceErrorKind, SourceOptions},
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn original_associated_function_docs_check_build_execute_workers1_8() {
    let text = "trait T { /// required\n fn f(x:i32)->i32; /** default */ fn g(x:i32)->i32{x} } struct S{v:i32} impl T for S { #[doc(hidden)] fn f(x:i32)->i32{x} } impl S { #[doc=\"inherent\"] fn h()->i32{7} } fn main(){assert!(S::h()==7);}";
    for workers in [1, 8] {
        compiler::compile_observed(text.into(), workers, true)
            .unwrap_or_else(|e| panic!("CHECK workers{workers}: {e}"));
        let built = compiler::compile_observed(text.into(), workers, false)
            .unwrap_or_else(|e| panic!("BUILD workers{workers}: {e}"));
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!(
            "jr-associated-doc-{}-{workers}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let binary = dir.join("native");
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&binary).output().unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn docs_cannot_hide_unused_associated_type_name_or_move_errors() {
    let cases = [
        (
            "trait T { #[doc=\"unused\"] fn f()->u8{true} } fn main(){}",
            "bool",
        ),
        (
            "struct S{v:i32} impl S { /// unused\n fn f()->u8{true} } fn main(){}",
            "bool",
        ),
        (
            "struct S{v:i32} impl S { #[doc(hidden)] fn f()->i32{missing} } fn main(){}",
            "missing",
        ),
        (
            "struct V{v:i32} struct S{v:i32} impl S { #[doc=\"unused\"] fn f(){let a=V{v:1};let b=a;let c=a;} } fn main(){}",
            "move",
        ),
    ];
    for workers in [1, 8] {
        for (text, expected) in cases {
            let program = std::sync::Arc::new(compiler::frontend::parse(text).unwrap());
            let budget = compiler::ir::PlanBudget::default();
            let plan = compiler::frontend::PlanCtx::from_shared_program(program, &budget).unwrap();
            let bad_callable = (0..plan.callable_count())
                .find(|&index| {
                    let info = plan
                        .declaration(plan.callable_decl(index).unwrap())
                        .unwrap();
                    info.name.rsplit("::").next() == Some("f")
                })
                .expect("fixture has an exact named invalid source callable");
            // OBJECT keys index closed roots, not declaration ids or arbitrary
            // source function positions. Generic trait defaults have no root.
            let bad_root = plan
                .closed_roots()
                .iter()
                .position(|&index| index == bad_callable);
            for check_only in [true, false] {
                let failure = match compiler::compile_observed(text.into(), workers, check_only) {
                    Ok(_) => panic!("accepted unused invalid associated body {text}"),
                    Err(failure) => failure,
                };
                assert!(
                    failure.to_string().to_lowercase().contains(expected),
                    "{expected}: {failure}"
                );
                {
                    let report = failure
                        .report
                        .expect("semantic failure retains graph report");
                    assert!(report.capture_complete);
                    assert!(
                        !report.published_kind(105),
                        "failed BUILD published executable"
                    );
                    if check_only {
                        assert!(
                            !report
                                .node_evaluations
                                .iter()
                                .any(|(key, _)| key.kind == 103)
                        );
                    }
                    if let Some(index) = bad_root {
                        for (node, (key, _)) in report.node_ids.iter().zip(&report.node_evaluations)
                        {
                            if key.kind == 103 && key.params == [index as u64] {
                                assert!(
                                    !report.events.iter().any(|event| event.kind == "publish"
                                        && event.node == Some(*node)),
                                    "offending callable published object"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn stock_valid_forms_outside_docs_only_a1_remain_explicitly_unsupported() {
    for text in [
        "trait T { //! inner\n fn f(); }",
        "struct S{v:i32} impl S { #![doc=\"inner\"] fn f(){} }",
        "trait T { #[doc=42] fn f(); }",
        "trait T { #[doc(foo)] fn f(); }",
        "trait T { #[doc=\"const\"] const N:u8=1; }",
        "trait T { #[doc=\"type\"] type A; }",
        "struct S{v:i32} impl S { fn f(){ #[doc=\"statement\"] let x=1; } }",
        "struct S{v:i32} impl S { #[doc=\"prefix\"] #[inline] fn f(){} }",
        "struct S{v:i32} impl S { #[cfg(any())] #[doc=\"prefix\"] fn f(){} }",
        "trait T { #[cfg_attr(any(),doc=42)] fn f(); }",
        "trait T { #[doc=\"x\"] #[allow(dead_code)] fn f(); }",
    ] {
        let error = source::prepare_text("a1.rs", text, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{text}: {error}");
    }
}

#[test]
fn malformed_or_misplaced_docs_are_not_masked_into_valid_programs() {
    for text in [
        "trait T { #[doc=] fn f(); }fn main(){}",
        "trait T { #[doc=\"dangling\"] }fn main(){}",
        "trait T { fn #[doc=\"header\"] f(); }fn main(){}",
        "trait T { fn f(#[doc=\"parameter\"] x:i32); }fn main(){}",
        "struct S{#[doc=\"field\"]v:i32}fn main(){}",
        "trait T { #[doc=\"valid\"] fn f(x:i32 ->i32; }fn main(){}",
        "#[cfg(any())] trait T { #[doc=] fn f(); }fn main(){}",
        "#[cfg(any())] trait T { #[doc=\"\\q\"] fn f(); }fn main(){}",
        "#[cfg(any())] trait T { fn f(){let =;} }fn main(){}",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile_observed(text.into(), workers, true).is_err(),
                "masked invalid placement/syntax: {text}"
            );
        }
    }
    compiler::compile_observed(
        "#[cfg(any())] trait T { #[doc=42] fn f(); }fn main(){}".into(),
        1,
        true,
    )
    .unwrap();
}

#[test]
fn associated_docs_keep_source_span_offsets_and_literal_macro_fences() {
    let text = "trait T{#[doc=\"é\\n\"] fn f();}fn main(){}";
    let prepared = source::prepare_text("utf8.rs", text, &SourceOptions::default()).unwrap();
    assert_eq!(prepared.text.len(), text.len());
    assert_eq!(prepared.attributes.len(), 1);
    assert_eq!(
        prepared.attributes[0].span.lo as usize,
        text.find("doc=").unwrap()
    );
    let literal = "struct S{v:i32}impl S{fn f(){let s=\"#[doc=42] fn fake()\";}}fn main(){}";
    let prepared = source::prepare_text("literal.rs", literal, &SourceOptions::default()).unwrap();
    assert!(prepared.text.contains("#[doc=42] fn fake()"));
    for text in [
        "struct S{v:i32}impl S{m!{#[doc=\"generated\"] fn f(){}}}fn main(){}",
        "macro_rules! m {()=>{#[doc=\"generated\"] fn f(){}}}m!();fn main(){}",
    ] {
        assert!(
            compiler::compile_observed(text.into(), 1, true).is_err(),
            "generated docs admitted: {text}"
        );
    }
}
