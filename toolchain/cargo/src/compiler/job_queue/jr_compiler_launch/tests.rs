use super::*;
use serde_json::Value;
use std::os::unix::process::ExitStatusExt;

fn registry(dir: &std::path::Path) -> Registry<u32> {
    let mut state = State {
        directory: dir.into(),
        ids: HashMap::default(),
        finished: false,
        root: Root {
            schema: 1,
            profile: PROFILE,
            run_id: "run".into(),
            authority_root: "compiler-test".into(),
            parent_authority_root: "buildrunner-test".into(),
            scheduler_capture_id: "cargo-test".into(),
            expected_units: 0,
            expected_launches: 0,
            expected_journals: 0,
            sealed: false,
            interrupted: false,
            lost_events: 0,
            outcome: "pending",
            units: vec![],
            launches: vec![],
        },
    };
    state.publish_root("declaration");
    Registry(Some(Arc::new(Mutex::new(state))))
}
fn root(dir: &std::path::Path, suffix: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(dir.join(format!("compiler-owner-compiler-test.{suffix}.json"))).unwrap(),
    )
    .unwrap()
}
#[test]
fn fresh_and_cutoff_expect_no_compiler_ticket() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    for (key, disposition) in [(1, "fresh"), (2, "cutoff")] {
        r.declare(&key, Some(key as u64 + 40), "rustc");
        r.resolve(&key, disposition);
        r.terminal(&key, true);
    }
    r.finish(true);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["expected_units"], 2);
    assert_eq!(v["expected_launches"], 0);
    assert_eq!(v["units"][0]["unit_id"], 41);
    assert_eq!(v["expected_journals"], 6);
    assert_eq!(v["units"][1]["disposition"], "cutoff");
}
#[test]
fn actual_wait_is_independent_of_unit_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(91), "rustc");
    r.resolve(&1, "required");
    let launch = r.launch(&1, true, 3).unwrap();
    assert_eq!(launch.binding.parent_task_id, "cargo-unit-91");
    assert_eq!(launch.binding.parent_invocation_id, "compiler-test");
    launch.observer.spawn_attempt();
    launch.observer.spawn_result(true, false);
    r.terminal(&1, true);
    r.update(|s| assert!(!s.root.launches[0].terminal));
    launch.observer.wait(ExitStatus::from_raw(9));
    r.finish(false);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["launches"][0]["wait_status"], 9);
    assert_eq!(v["launches"][0]["outcome"], "failure");
    assert_eq!(v["lost_events"], 0);
}
#[test]
fn unsupported_and_missing_child_stay_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(1), "rustc");
    r.resolve(&1, "required");
    assert!(r.launch(&1, false, 3).is_none());
    r.terminal(&1, true);
    r.declare(&2, Some(2), "rustc");
    r.resolve(&2, "required");
    let launch = r.launch(&2, true, 3).unwrap();
    launch.observer.spawn_attempt();
    launch.observer.spawn_result(true, false);
    r.terminal(&2, true);
    r.finish(true);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["units"][0]["disposition"], "unsupported");
    assert_eq!(v["launches"][0]["terminal"], false);
    assert_eq!(v["launches"][0]["wait_status"], Value::Null);
}
#[test]
fn retry_spawn_failure_is_not_a_wait() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(1), "rustc");
    r.resolve(&1, "required");
    let l = r.launch(&1, true, 3).unwrap();
    l.observer.spawn_attempt();
    l.observer.spawn_result(false, true);
    l.observer.spawn_attempt();
    l.observer.spawn_result(false, false);
    r.terminal(&1, false);
    r.finish(false);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["launches"][0]["spawn_attempts"], 2);
    assert_eq!(v["launches"][0]["outcome"], "spawn_failed");
    assert_eq!(v["launches"][0]["wait_status"], Value::Null);
}
#[test]
fn counts_remain_independent_and_lost_journals_are_detectable() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, None, "rustc");
    r.finish(false);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["expected_units"], 1);
    assert_eq!(v["units"].as_array().unwrap().len(), 0);
    assert_eq!(v["lost_events"], 1);
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(1), "rustc");
    r.resolve(&1, "fresh");
    r.terminal(&1, true);
    r.finish(true);
    std::fs::remove_file(dir.path().join("compiler-owner-compiler-test.event-2.json")).unwrap();
    assert_eq!(root(dir.path(), "manifest")["expected_journals"], 3);
    assert!(
        !dir.path()
            .join("compiler-owner-compiler-test.event-2.json")
            .exists()
    );
}
#[test]
fn drop_never_seals_or_invents_completed_child() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(1), "rustc");
    drop(r);
    let v = root(dir.path(), "interrupted.manifest");
    assert_eq!(v["sealed"], false);
    assert_eq!(v["interrupted"], true);
    assert_eq!(v["units"][0]["terminal"], false);
}
#[test]
fn unstarted_opportunity_has_no_launch() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry(dir.path());
    r.declare(&1, Some(1), "rustc");
    r.finish(false);
    let v = root(dir.path(), "manifest");
    assert_eq!(v["units"][0]["outcome"], "not_started");
    assert_eq!(v["expected_launches"], 0);
}
#[test]
fn publication_failure_is_only_observation_loss() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-directory");
    std::fs::write(&file, b"x").unwrap();
    let r = registry(&file);
    r.declare(&1, Some(1), "rustc");
    r.finish(true);
    r.update(|s| {
        assert!(s.root.lost_events >= 3);
        assert_eq!(s.root.outcome, "success");
    });
}

#[test]
fn new_sink_is_private_and_existing_permissions_are_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let new_sink = parent.path().join("new").join("sink");
    let r = registry(&new_sink);
    r.finish(true);
    assert_eq!(
        std::fs::metadata(&new_sink).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let existing = parent.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
    let r = registry(&existing);
    r.finish(true);
    assert_eq!(
        std::fs::metadata(existing).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
