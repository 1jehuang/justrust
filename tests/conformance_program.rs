//! The compatibility evidence system is part of the ordinary test gate.
//! Python uses only the standard library. It never compiles user Rust for the
//! native compiler and must not turn a green subset into full compatibility.
use std::path::Path;
use std::process::Command;

#[path = "support/stock_rustc.rs"]
mod stock_rustc;

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
    assert!(full.contains("borrowed-closure-result"), "{full}");
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
