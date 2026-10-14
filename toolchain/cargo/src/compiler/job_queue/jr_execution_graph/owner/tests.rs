use super::Owner;
use crate::compiler::job_queue::jr_execution_graph::{Capture, publish_json};
use serde_json::{Value, json};
use std::path::Path;

fn snapshot(dir: &Path, suffix: &str) -> Value {
    let paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(suffix))
        .collect();
    assert_eq!(paths.len(), 1, "suffix {suffix}");
    serde_json::from_slice(&std::fs::read(&paths[0]).unwrap()).unwrap()
}

#[test]
fn declaration_precedes_child_and_binds_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Owner::new(Some(dir.path().into()));
    let declaration = snapshot(dir.path(), ".declaration.json");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(declaration["owners"][0]["interrupted"], false);
    assert_eq!(declaration["owners"][0]["outcome"], "pending");
    let ticket = &declaration["owners"][0]["tickets"][0];
    assert_eq!(ticket["policy"], "required");
    assert_eq!(ticket["lifecycle"], "declared");
    assert_eq!(ticket["execute_entered"], false);
    let mut capture = Capture::<u32>::for_owner(&owner);
    capture.seal();
    owner.execute_enter();
    capture.finalize();
    owner.execute_result(true);
    owner.finish(true);
    drop(owner);
    let manifest = snapshot(dir.path(), ".manifest.json");
    let envelope = snapshot(dir.path(), ".envelope.json");
    let receipt_path = dir
        .path()
        .join(format!("{}.json", ticket["capture_id"].as_str().unwrap()));
    let receipt: Value = serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(manifest["owners"][0]["sealed"], true);
    assert_eq!(manifest["owners"][0]["interrupted"], false);
    assert_eq!(manifest["owners"][0]["lost_events"], 0);
    assert_eq!(manifest["owners"][0]["outcome"], "success");
    assert_eq!(manifest["owners"][0]["tickets"][0]["execute_entered"], true);
    assert_eq!(envelope["invocation_id"], ticket["invocation_id"]);
    assert_eq!(envelope["parent_task_id"], ticket["parent_task_id"]);
    assert_eq!(
        envelope["parent_invocation_id"],
        declaration["authority_root"]
    );
    assert_eq!(envelope["run_id"], declaration["run_id"]);
    assert_eq!(envelope["capture_id"], receipt["capture_id"]);
    assert_eq!(envelope["schema"], 1);
    assert_eq!(envelope["receipt_schema"], receipt["schema"]);
    assert_eq!(receipt["structural_complete"], true);
    assert_eq!(receipt["full_pipeline_complete"], false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            assert_eq!(
                entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn preparation_failure_requires_incomplete_bound_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Owner::new(Some(dir.path().into()));
    drop(Capture::<u32>::for_owner(&owner));
    owner.finish(false);
    let manifest = snapshot(dir.path(), ".manifest.json");
    let record = &manifest["owners"][0];
    assert_eq!(record["sealed"], true);
    assert_eq!(record["outcome"], "failed");
    assert_eq!(record["tickets"][0]["lifecycle"], "not_started");
    assert_eq!(record["tickets"][0]["execute_entered"], false);
    let envelope = snapshot(dir.path(), ".envelope.json");
    let path = dir
        .path()
        .join(format!("{}.json", envelope["capture_id"].as_str().unwrap()));
    let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt["structural_complete"], false);
}

#[test]
fn execute_failure_and_post_execute_failure_remain_distinct() {
    for execute_success in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut owner = Owner::new(Some(dir.path().into()));
        owner.execute_enter();
        owner.execute_result(execute_success);
        owner.finish(false);
        let manifest = snapshot(dir.path(), ".manifest.json");
        assert_eq!(manifest["owners"][0]["outcome"], "failed");
        assert_eq!(
            manifest["owners"][0]["tickets"][0]["lifecycle"],
            if execute_success {
                "succeeded"
            } else {
                "failed"
            }
        );
        assert_eq!(manifest["owners"][0]["tickets"][0]["execute_entered"], true);
        let entered = snapshot(dir.path(), ".execute-entered.json");
        assert_eq!(entered["owners"][0]["sealed"], false);
        assert_eq!(
            entered["owners"][0]["tickets"][0]["lifecycle"],
            "execute_entered"
        );
    }
}

#[test]
fn dropped_owner_never_seals_or_rewrites_started_child_as_not_started() {
    for entered in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut owner = Owner::new(Some(dir.path().into()));
            if entered {
                owner.execute_enter();
            }
        }
        let manifest = snapshot(dir.path(), ".interrupted.manifest.json");
        let record = &manifest["owners"][0];
        assert_eq!(record["sealed"], false);
        assert_eq!(record["interrupted"], true);
        assert_eq!(
            record["tickets"][0]["lifecycle"],
            if entered {
                "execute_entered"
            } else {
                "declared"
            }
        );
        assert_eq!(record["tickets"][0]["execute_entered"], entered);
    }
}

#[test]
fn child_writer_failure_does_not_erase_owner_expectation() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Owner::new(Some(dir.path().into()));
    let declaration = snapshot(dir.path(), ".declaration.json");
    assert_eq!(declaration["owners"][0]["interrupted"], false);
    assert_eq!(declaration["owners"][0]["outcome"], "pending");
    let ticket = &declaration["owners"][0]["tickets"][0];
    let receipt_path = dir
        .path()
        .join(format!("{}.json", ticket["capture_id"].as_str().unwrap()));
    std::fs::write(&receipt_path, b"existing").unwrap();
    let mut capture = Capture::<u32>::for_owner(&owner);
    capture.seal();
    owner.execute_enter();
    capture.finalize();
    owner.execute_result(true);
    owner.finish(true);
    assert_eq!(std::fs::read(receipt_path).unwrap(), b"existing");
    let manifest = snapshot(dir.path(), ".manifest.json");
    assert_eq!(
        manifest["owners"][0]["tickets"][0]["invocation_id"],
        ticket["invocation_id"]
    );
    assert_eq!(
        manifest["owners"][0]["tickets"][0]["capture_id"],
        ticket["capture_id"]
    );
}

#[test]
fn lost_declaration_is_recorded_and_sink_failure_is_fail_open() {
    let dir = tempfile::tempdir().unwrap();
    let sink = dir.path().join("sink");
    std::fs::write(&sink, b"blocked").unwrap();
    let mut owner = Owner::new(Some(sink.clone()));
    assert_eq!(std::fs::read(&sink).unwrap(), b"blocked");
    std::fs::remove_file(&sink).unwrap();
    owner.execute_enter();
    owner.execute_result(true);
    owner.finish(true);
    let manifest = snapshot(&sink, ".manifest.json");
    assert_eq!(manifest["owners"][0]["lost_events"], 1);
}

#[test]
fn owner_ids_are_unique_and_disabled_owner_is_inert() {
    let dir = tempfile::tempdir().unwrap();
    let a = Owner::new(Some(dir.path().into()));
    let b = Owner::new(Some(dir.path().into()));
    assert_ne!(
        a.binding().unwrap().1.capture_id,
        b.binding().unwrap().1.capture_id
    );
    let mut disabled = Owner::new(None);
    assert!(disabled.binding().is_none());
    disabled.execute_enter();
    disabled.execute_result(false);
    disabled.finish(false);
}

#[test]
fn journal_publication_is_bounded_private_and_no_replace() {
    let dir = tempfile::tempdir().unwrap();
    let data = json!({"bounded": "payload"});
    assert!(publish_json(dir.path(), "small", &data, 1).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    publish_json(dir.path(), "journal", &data, 1024).unwrap();
    assert!(publish_json(dir.path(), "journal", &json!("replacement"), 1024).is_err());
    assert_eq!(snapshot(dir.path(), "journal.json"), data);
}
