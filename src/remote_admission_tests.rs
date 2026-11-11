use super::*;
use crate::remote_hosted::{HostedUpdate, RunAdmission};
use std::cell::Cell;

fn update() -> HostedUpdate {
    serde_json::from_value(serde_json::json!({
        "id": "maintenance-test", "reason": "test update", "state": "announced",
        "started_at": 1, "completed_at": null, "interrupted_work_stopped": false
    }))
    .unwrap()
}

fn charged(maintenance_checked: bool) -> RunAdmission {
    RunAdmission::Charged {
        builds: None,
        maintenance_checked,
    }
}

#[test]
fn acknowledged_admission_needs_no_extra_status_round_trip() {
    let result = await_hosted_admission(
        || Ok(charged(true)),
        || panic!("acknowledged admission must not issue a status GET"),
        |_| panic!("no maintenance was announced"),
    )
    .unwrap();
    assert!(result.is_ok());
}

#[test]
fn legacy_admission_keeps_status_check_before_execution() {
    let checked = Cell::new(false);
    let waited = Cell::new(false);
    let result = await_hosted_admission(
        || Ok(charged(false)),
        || {
            checked.set(true);
            Ok(Some(update()))
        },
        |_| {
            assert!(checked.get());
            waited.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert!(result.is_ok());
    assert!(checked.get() && waited.get());
}

#[test]
fn maintenance_retries_same_admission_only_after_wait_completes() {
    let attempts = Cell::new(0);
    let waited = Cell::new(false);
    let result = await_hosted_admission(
        || {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Ok(RunAdmission::Updating(update()))
            } else {
                assert!(waited.get());
                Ok(charged(true))
            }
        },
        || panic!("new server does not require legacy status"),
        |_| {
            waited.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert!(result.is_ok());
    assert_eq!(attempts.get(), 2);
}

#[test]
fn update_failure_is_not_a_fallback_safe_admission_failure() {
    for legacy in [false, true] {
        let result = await_hosted_admission(
            || {
                if legacy {
                    Ok(charged(false))
                } else {
                    Ok(RunAdmission::Updating(update()))
                }
            },
            || Ok(Some(update())),
            |_| anyhow::bail!("maintenance failed"),
        );
        assert!(
            result.is_err(),
            "maintenance failure must be an outer error"
        );
    }
}

#[test]
fn billing_error_does_not_wait_or_query_status() {
    let result = await_hosted_admission(
        || Err("subscription required".into()),
        || panic!("rejected admission must not query status"),
        |_| panic!("rejected admission must not wait"),
    )
    .unwrap();
    assert_eq!(result.unwrap_err(), "subscription required");
}

#[test]
fn repeated_uncleared_maintenance_has_a_bounded_retry() {
    let waits = Cell::new(0);
    let result = await_hosted_admission(
        || Ok(RunAdmission::Updating(update())),
        || panic!("new server does not require legacy status"),
        |_| {
            waits.set(waits.get() + 1);
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(waits.get(), 1);
}

#[test]
fn observational_and_locality_controls_are_forwarded_without_paths() {
    let controls = [
        "JUSTRUST_PASSES",
        "JUSTRUST_RECORD_PHASE_TRACE",
        "JUSTRUST_SLOT_WARM_REUSE",
    ];
    let values = forwarded_env(|name| controls.contains(&name).then(|| "0".into()));
    assert_eq!(values.len(), controls.len());
    assert!(controls.iter().all(|name| {
        values
            .iter()
            .any(|(key, value)| key == name && value == "0")
    }));
    assert!(
        forwarded_env(
            |name| (name == "JUSTRUST_RUSTC_PHASES_FILE").then(|| "/private/host/path".into())
        )
        .is_empty()
    );
}
