use super::Capture;
use serde_json::Value;
use std::path::Path;

fn receipt(dir: &Path) -> Value {
    let files: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    serde_json::from_slice(&std::fs::read(files[0].as_ref().unwrap().path()).unwrap()).unwrap()
}

#[test]
fn scheduler_success_fresh_rmeta_and_privacy() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "/private/project/src/secret.rs";
    let mut c = Capture::new(Some(dir.path().into()));
    c.declare(&"dependent", [(&secret, "rmeta")].into_iter());
    c.declare(&secret, std::iter::empty());
    c.seal();
    c.start(&secret, false);
    c.finish(&secret, "rmeta", true);
    c.start(&"dependent", true);
    c.finish(&"dependent", "all", true);
    c.finish(&secret, "all", true);
    c.finalize();
    drop(c);
    let r = receipt(dir.path());
    assert_eq!(r["schema"], 1);
    assert_eq!(r["producer"], "cargo-scheduler");
    assert_eq!(r["scope"], "cargo-unit-scheduler");
    assert_eq!(r["structural_complete"], true);
    assert_eq!(r["full_pipeline_complete"], false);
    assert_eq!(r["declared_nodes"], 2);
    assert_eq!(r["declared_edges"], 1);
    assert_eq!(r["edges"][0]["artifact"], "rmeta");
    assert_eq!(r["nodes"][0]["outcome"], "fresh");
    assert_eq!(r["nodes"][1]["outcome"], "success");
    assert_eq!(r["events"].as_array().unwrap().len(), 5);
    assert!(r["events"][1]["outcome"].is_null());
    assert!(!r["opaque_boundaries"].as_array().unwrap().is_empty());
    assert!(!r.to_string().contains(secret));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn scheduler_failure_closes_not_started_units() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::new(Some(dir.path().into()));
    c.declare(&1, std::iter::empty());
    c.declare(&2, [(&1, "all")].into_iter());
    c.seal();
    c.start(&1, false);
    c.finish(&1, "all", false);
    c.finalize();
    let r = receipt(dir.path());
    assert_eq!(r["structural_complete"], true);
    assert_eq!(r["nodes"][0]["outcome"], "failure");
    assert_eq!(r["nodes"][1]["outcome"], "not_started");
    assert!(r["nodes"][1]["start_ns"].is_null());
    assert!(r["nodes"][1]["end_ns"].is_null());
}

#[test]
fn scheduler_drop_finalizes_incomplete_and_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut c = Capture::new(Some(dir.path().into()));
        c.declare(&1, std::iter::empty());
        c.start(&1, false);
    }
    let r = receipt(dir.path());
    assert_eq!(r["structural_complete"], false);
    assert_eq!(r["nodes"][0]["outcome"], "cancelled");
    assert!(
        r["issues"]
            .as_array()
            .unwrap()
            .contains(&Value::from("interrupted"))
    );
    assert!(
        r["issues"]
            .as_array()
            .unwrap()
            .contains(&Value::from("unsealed-declarations"))
    );
}

#[test]
fn scheduler_invalid_lifecycle_and_missing_declaration_are_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::new(Some(dir.path().into()));
    c.declare(&1, [(&2, "all")].into_iter());
    c.seal();
    c.finish(&1, "all", true);
    c.start(&1, false);
    c.start(&1, false);
    c.finish(&1, "all", true);
    c.finish(&1, "all", true);
    c.finalize();
    let r = receipt(dir.path());
    assert_eq!(r["structural_complete"], false);
    for code in [
        "undeclared-prerequisite",
        "invalid-finish",
        "duplicate-start",
    ] {
        assert!(r["issues"].as_array().unwrap().contains(&Value::from(code)));
    }
}

#[test]
fn scheduler_failed_metadata_is_not_available_and_duplicate_terminal_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::new(Some(dir.path().into()));
    c.declare(&1, std::iter::empty());
    c.seal();
    c.start(&1, false);
    c.finish(&1, "rmeta", false);
    c.finish(&1, "all", false);
    c.finalize();
    let r = receipt(dir.path());
    assert_eq!(r["nodes"][0]["outcome"], "failure");
    assert_eq!(r["structural_complete"], false);
    assert!(
        r["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["event"] != "artifact-ready")
    );
}

#[test]
fn scheduler_cycles_and_duplicate_declarations_are_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::new(Some(dir.path().into()));
    c.declare(&1, [(&2, "all")].into_iter());
    c.declare(&2, [(&1, "rmeta")].into_iter());
    c.declare(&1, std::iter::empty());
    c.seal();
    c.finalize();
    let r = receipt(dir.path());
    assert_eq!(r["structural_complete"], false);
    for code in ["dependency-cycle", "duplicate-declaration"] {
        assert!(r["issues"].as_array().unwrap().contains(&Value::from(code)));
    }
}

#[test]
fn scheduler_receipts_do_not_replace_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::<u32>::new(Some(dir.path().into()));
    let path = dir.path().join(format!(
        "{}.json",
        c.enabled.as_ref().unwrap().receipt.capture_id
    ));
    std::fs::write(&path, b"existing-private-data").unwrap();
    c.seal();
    c.finalize();
    assert_eq!(std::fs::read(path).unwrap(), b"existing-private-data");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn scheduler_streaming_cap_cleans_partial_and_accepts_exact_budget() {
    for fits in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Capture::<u32>::new(Some(dir.path().into()));
        let enabled = c.enabled.as_mut().unwrap();
        // Exercise the production writer directly without Drop retrying with
        // the normal budget after this deliberately undersized test attempt.
        enabled.finalized = true;
        let len = serde_json::to_vec(&enabled.receipt).unwrap().len();
        let result = enabled.write_with_budget(if fits { len } else { len - 1 });
        if fits {
            result.unwrap();
            let r = receipt(dir.path());
            assert_eq!(r["capture_id"], enabled.receipt.capture_id);
        } else {
            assert!(result.is_err());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn scheduler_partial_collision_preserves_existing_private_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Capture::<u32>::new(Some(dir.path().into()));
    let enabled = c.enabled.as_mut().unwrap();
    enabled.finalized = true;
    let partial = dir
        .path()
        .join(format!("{}.partial", enabled.receipt.capture_id));
    std::fs::write(&partial, b"existing-partial").unwrap();
    assert!(enabled.write_with_budget(super::MAX_RECEIPT_BYTES).is_err());
    assert_eq!(std::fs::read(partial).unwrap(), b"existing-partial");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn scheduler_streaming_writer_never_exceeds_byte_budget() {
    use std::io::Write;
    let mut writer = super::CappedWriter {
        inner: Vec::new(),
        remaining: 3,
    };
    writer.write_all(b"abc").unwrap();
    assert!(writer.write_all(b"d").is_err());
    assert_eq!(writer.inner, b"abc");
    assert_eq!(writer.remaining, 0);
}

#[test]
fn scheduler_disabled_and_write_failures_do_not_fail() {
    let mut disabled = Capture::<u32>::new(None);
    disabled.declare(&1, std::iter::empty());
    disabled.seal();
    disabled.start(&1, false);
    disabled.finish(&1, "all", true);
    disabled.finalize();
    assert!(disabled.enabled.is_none());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, b"keep").unwrap();
    let mut c = Capture::<u32>::new(Some(file.clone()));
    c.declare(&1, std::iter::empty());
    c.seal();
    c.finalize();
    drop(c);
    assert_eq!(std::fs::read(file).unwrap(), b"keep");
}
