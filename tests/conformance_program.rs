//! The compatibility evidence system is part of the ordinary test gate.
//! Python uses only the standard library. The bounded reduction utility has a
//! separate pinned-stock/current-native smoke test, not compatibility credit.
//! A green subset must never be presented as full compatibility.
use std::path::Path;
use std::process::Command;

use closure_oracle::pinned_stock as stock_rustc;

#[allow(dead_code)]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;

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

fn python(script: &str, args: &[&str]) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("python3")
        .arg(root.join(script))
        .args(args)
        .current_dir(root)
        .output()
        .expect("compatibility evidence checks require Python 3 standard library");
    assert!(
        output.status.success(),
        "{script}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn compatibility_manifest_maps_real_acceptance_suites_without_claiming_completeness() {
    let result = python("tools/conformance.py", &["validate"]);
    assert!(result.contains("\"manifest\": \"valid\""), "{result}");
    assert!(
        result.contains("\"full_rust_support_established\": false"),
        "{result}"
    );
    let full = python("tools/conformance.py", &["plan", "--profile", "full"]);
    assert!(full.contains("--all-targets"), "{full}");
    assert!(full.contains("--no-fail-fast"), "{full}");
    assert!(full.contains("\"known_gaps\""), "{full}");
    assert!(
        full.contains("\"semantic_inventory\": \"incomplete\""),
        "{full}"
    );
}

#[test]
fn compatibility_progress_comes_from_layered_exact_tests_not_manual_status() {
    let coverage = python("tools/conformance.py", &["coverage", "--json"]);
    for marker in [
        "\"passing_declared_gates_percent\": 0.0",
        "\"missing_test_gates\"",
        "\"unmapped_integration_suites\"",
        "\"interaction_matrix\"",
        "\"rule_proposals\"",
        "\"full_rust_support_established\": false",
    ] {
        assert!(coverage.contains(marker), "missing {marker}: {coverage}");
    }
    let plan = python(
        "tools/conformance.py",
        &["track", "--area", "interactions", "--dry-run"],
    );
    assert!(plan.contains("--exact"), "{plan}");
    assert!(plan.contains("conformance_generated"), "{plan}");
}

#[test]
fn compatibility_discovery_includes_new_suites_and_unsupported_boundaries() {
    let result = python("tools/conformance.py", &["discover"]);
    for marker in ["conformance_program", "full_programs", "triage_markers"] {
        assert!(result.contains(marker), "missing discovery marker {marker}");
    }
    assert!(result.contains("not count them as features or proven gaps"));
}

#[test]
fn evidence_rejects_missing_stale_tampered_filtered_and_empty_results() {
    python("tools/test_conformance.py", &["-v"]);
    python("tools/test_compatibility_scope.py", &["-v"]);
    python("tools/test_compatibility_inventory.py", &["-v"]);
    python("tools/test_extract_reference_rules.py", &["-v"]);
    python("tools/test_acquire_pinned_reference.py", &["-v"]);
    python("tools/test_pinned_acquisition.py", &["-v"]);
    python("tools/test_compact_library_surface_provenance.py", &["-v"]);
    python("tools/test_library_surface_backlog.py", &["-v"]);
    python("tools/test_classify_proc_macro_details.py", &["-v"]);
    python("tools/test_reconcile_token_rules.py", &["-v"]);
    python("tools/test_bounded_failure_reduce.py", &["-v"]);
    let inventory = python("tools/compatibility_inventory.py", &[]);
    for marker in [
        "\"rule_proposals\"",
        "\"documentation_backlog\"",
        "\"partial_witness_sources\"",
        "\"direct_witness_selectors\": 12",
        "\"execution_evidence_imported\": false",
        "\"source_file_byte_verification\": \"reverified_in_this_run\"",
        "\"tokens\"",
        "\"const-eval\"",
        "\"unsafe\"",
        "\"abi\"",
        "\"visibility\"",
        "\"global_rule_identities\": 2965",
        "\"proposed_rule_identities\": 228",
        "\"rule_identities_without_proposals\": 2737",
        "\"backlog\"",
        "\"total_pages\": 122",
        "\"missing_rule_sample_limit\": 100",
        "\"missing_rule_sample_truncated\": true",
        "\"reviewed_rule_rows\": 0",
        "\"completed_rule_rows\": 0",
        "\"source_block_byte_verification\": \"not_reverified_in_this_run\"",
    ] {
        assert!(inventory.contains(marker), "missing {marker}: {inventory}");
    }
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
