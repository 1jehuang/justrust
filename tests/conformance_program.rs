//! Invalid-program oracle checks, async-context boundaries, the pinned stock
//! version check and the bounded failure reducer smoke test.
use std::path::Path;
use std::process::Command;

use closure_oracle::pinned_stock as stock_rustc;

#[allow(dead_code)]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn async_context_unused_invalid_workers1_8_check_build() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "async-context-boundaries",
        &[
            Case {
                name: "outer_postfix_is_not_inside_its_operand",
                source: "fn unused(){let _value=async {7i32}.await;}fn main(){}",
                expect: Expect::InvalidAfterParse("E0728"),
            },
            Case {
                name: "ordinary_closure_resets_async_context",
                source: "fn unused(){let _future=async {let _f=|| {async {7i32}.await};};}fn main(){}",
                expect: Expect::InvalidAfterParse("E0728"),
            },
            Case {
                name: "wrong_context_after_valid_nested_async",
                source: "fn unused(){let _future=async {async {7i32}.await};let _bad=async {8i32}.await;}fn main(){}",
                expect: Expect::InvalidAfterParse("E0728"),
            },
            Case {
                name: "ordinary_closure_still_requires_async_context",
                source: "fn unused(){let _f=|| {let _value=async move {7i32}.await;};}fn main(){}",
                expect: Expect::InvalidAfterParse("E0728"),
            },
        ],
    );
    for source in [
        "fn unused(){let _value=async {7i32}.await;}fn main(){}",
        "fn unused(){let _future=async {let _f=|| {async {7i32}.await};};}fn main(){}",
        "fn unused(){let _future=async {async {7i32}.await};let _bad=async {8i32}.await;}fn main(){}",
        "fn unused(){let _f=|| {let _value=async move {7i32}.await;};}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure =
                    justrust_engine::compiler::compile_observed(source.into(), workers, check_only)
                        .err()
                        .expect("invalid await accepted");
                assert!(
                    failure.error.0.contains("E0728"),
                    "{workers}/{check_only}: {failure}"
                );
                closure_oracle::validate_semantic_rejection(&failure).unwrap();
                if check_only {
                    assert!(
                        failure
                            .report
                            .as_ref()
                            .unwrap()
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130 | 131 | 133)),
                        "CHECK demanded codegen: {failure}"
                    );
                }
            }
        }
    }
}

#[test]
fn async_context_stock_valid_boundaries_are_explicitly_unsupported_without_codegen() {
    use std::io::Write;
    use std::process::Stdio;
    let compiler = stock_rustc::stock_rustc();
    let metadata = std::env::temp_dir().join(format!(
        "jr-async-context-{}-{}.rmeta",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for source in [
        "fn unused(){let _future=async {7i32};}fn main(){}",
        "fn unused(){let _future=async move {async {7i32}.await};}fn main(){}",
        "fn unused(){let _future=async {let _f=|| {async {7i32}};};}fn main(){}",
        "fn unused(){let _future=async {let _f=|| {async {async {7i32}.await}};};}fn main(){}",
    ] {
        let mut child = Command::new(&compiler)
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "--crate-name=async_context_boundary",
                "--emit=metadata",
                "-Awarnings",
            ])
            .arg("-o")
            .arg(&metadata)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let stock = child.wait_with_output().unwrap();
        let _ = std::fs::remove_file(&metadata);
        assert!(
            stock.status.success(),
            "stock rejected {source}: {}",
            String::from_utf8_lossy(&stock.stderr)
        );
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure =
                    justrust_engine::compiler::compile_observed(source.into(), workers, check_only)
                        .err()
                        .expect("native future execution must remain Unsupported");
                assert!(
                    failure.error.0.contains("unsupported:"),
                    "{workers}/{check_only}: {failure}"
                );
                assert!(
                    !failure.error.0.contains("E0728"),
                    "well-contexted await rejected: {failure}"
                );
                assert!(failure.resource_usage.first_failure.is_none());
                assert!(!failure.resource_usage.poisoned);
                let report = failure
                    .report
                    .as_ref()
                    .expect("admitted source needs graph evidence");
                assert!(report.capture_complete && report.events_dropped == 0);
                for (kind, params) in [(121, vec![0]), (101, vec![])] {
                    assert!(
                        report.node_ids.iter().zip(&report.node_evaluations).any(
                            |(id, (key, _))| {
                                key.kind == kind
                                    && key.params == params
                                    && report.events.iter().any(|event| {
                                        event.kind == "publish" && event.node == Some(*id)
                                    })
                            }
                        ),
                        "source/declarations were not admitted: {failure}"
                    );
                }
                if check_only {
                    assert!(
                        report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130 | 131 | 133)),
                        "CHECK demanded code generation: {failure}"
                    );
                }
            }
        }
    }
}

fn observed_rejection(diagnostic: &str, with_report: bool) -> justrust_engine::compiler::Failure {
    justrust_engine::compiler::Failure {
        error: justrust_engine::engine::Error::new(diagnostic),
        report: with_report.then(|| {
            let mut report = justrust_engine::engine::Report::default();
            report.capture_complete = true;
            Box::new(report)
        }),
        analyses: Vec::new(),
        resource_usage: Box::default(),
    }
}

#[test]
fn invalid_oracle_allows_semantic_and_pregraph_syntax_rejections() {
    for diagnostic in [
        "1:12: expected expression",
        "InvalidMetadata: unresolved function `budget`",
        "cannot find value `resource` in this scope",
        "cannot find value `unsupported` in this scope",
        "cannot find value `infrastructure` in this scope",
        "cannot find value `panicked` in this scope",
        "resource.rs:3:7: cannot find value `budget` in this scope",
        "unsupported.rs:3:7: expected expression",
        "cannot find value `infrastructure: failed` in this scope",
    ] {
        for with_report in [false, true] {
            let failure = observed_rejection(diagnostic, with_report);
            assert_eq!(
                closure_oracle::validate_invalid_failure(&failure),
                Ok(()),
                "{diagnostic}"
            );
        }
    }
}

#[test]
fn invalid_oracle_rejects_resource_failure_even_with_semantic_diagnostic() {
    use justrust_engine::compiler::ir::{ResourceBudgetFailure, ResourceBudgetKind};
    for with_report in [false, true] {
        let mut failure = observed_rejection("expected expression", with_report);
        failure.resource_usage.first_failure = Some(ResourceBudgetFailure {
            kind: ResourceBudgetKind::Work,
            requested: 2,
            used: 1,
            limit: 1,
            arithmetic_overflow: false,
            file: file!(),
            line: line!(),
            column: column!(),
            graph_id: None,
            task_id: None,
        });
        assert_eq!(
            closure_oracle::validate_invalid_failure(&failure),
            Err("resource exhaustion is not semantic rejection")
        );
    }
}

#[test]
fn invalid_oracle_rejects_poisoned_ledger_without_a_first_failure() {
    for with_report in [false, true] {
        let mut failure = observed_rejection("expected expression", with_report);
        failure.resource_usage.poisoned = true;
        assert_eq!(
            closure_oracle::validate_invalid_failure(&failure),
            Err("poisoned resource ledger is not semantic rejection")
        );
    }
}

#[test]
fn invalid_oracle_rejects_incomplete_and_dropped_capture_independently() {
    let mut failure = observed_rejection("expected expression", true);
    failure.report.as_mut().unwrap().capture_complete = false;
    assert_eq!(
        closure_oracle::validate_invalid_failure(&failure),
        Err("incomplete graph capture is not semantic rejection")
    );
    failure.report.as_mut().unwrap().capture_complete = true;
    failure.report.as_mut().unwrap().events_dropped = 1;
    assert_eq!(
        closure_oracle::validate_invalid_failure(&failure),
        Err("dropped graph events are not semantic rejection")
    );
}

#[test]
fn invalid_oracle_rejects_reserved_nonsemantic_categories_without_a_report() {
    for diagnostic in [
        "infrastructure: missing native toolchain crt1.o",
        "Infrastructure: allocation failed",
        "ResourceLimit: work exhausted",
        "source planning ResourceLimit (diagnostic transport exhausted)",
        "contract: stale graph evidence",
        "unsupported: closure boundary",
        "Unsupported: narrow source boundary",
        "unsupported nominal realization key version",
        "main.rs:2:3: infrastructure: source unavailable",
        "main.rs:2:3: unsupported: source syntax boundary",
        "2:3: Unsupported: source syntax boundary",
        "cannot spawn worker: OS error",
        "engine worker panicked: fault",
        "provider outputs panicked: fault",
        "provider start panicked: fault",
        "provider future poll panicked: fault",
        "provider future drop panicked: fault",
        "engine execution aborted",
        "task future missing during poll",
    ] {
        for with_report in [false, true] {
            let failure = observed_rejection(diagnostic, with_report);
            assert!(
                closure_oracle::validate_invalid_failure(&failure).is_err(),
                "{diagnostic}"
            );
        }
    }
}

#[test]
fn invalid_oracle_rejects_nonsemantic_node_failure_hidden_by_root_diagnostic() {
    let mut failure = observed_rejection("expected expression", true);
    failure.report.as_mut().unwrap().node_errors.push((
        7,
        justrust_engine::engine::Error::new("infrastructure: source unavailable"),
    ));
    assert_eq!(
        closure_oracle::validate_invalid_failure(&failure),
        Err("nonsemantic graph failure is not invalid Rust evidence")
    );
}

#[test]
fn bounded_reducer_checks_pinned_stock_and_current_native() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("python3")
        .arg(root.join("tools/test_bounded_failure_reduce.py"))
        .args([
            "ActualReductionTests.test_pinned_stock_current_native_authored_reduction",
            "-v",
        ])
        .env("JR_REDUCER_STOCK", stock_rustc::stock_rustc())
        .env("JR_REDUCER_NATIVE", env!("CARGO_BIN_EXE_jr-rust"))
        .env_remove("RUSTC_BOOTSTRAP")
        .current_dir(root)
        .output()
        .expect("bounded reducer acceptance requires Python 3");
    assert!(
        output.status.success(),
        "pinned reduction utility acceptance: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("pinned-reducer-acceptance:"),
        "actual reducer acceptance must execute, not skip: {stdout}"
    );
    println!("{stdout}");
}

#[test]
fn oracle_rejects_changed_version_commit_host_llvm_and_failed_probe() {
    let version = "release: 1.98.1\ncommit-hash: 48a229ceaefd4985c50990b14116b6d856af0985\nhost: x86_64-unknown-linux-gnu\nLLVM version: 22.1.8\n";
    stock_rustc::verify_version_output(true, version).unwrap();
    assert!(stock_rustc::verify_version_output(false, version).is_err());
    for changed in [
        version.replace("1.98.1", "1.98.2"),
        version.replace("48a229cea", "000000000"),
        version.replace("x86_64", "aarch64"),
        version.replace("22.1.8", "22.1.9"),
        version.replace("release:", "not-release:"),
    ] {
        assert!(stock_rustc::verify_version_output(true, &changed).is_err());
    }
}
