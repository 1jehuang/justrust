use super::*;
use crate::compiler::expansion_budget::{ExpansionBudget, Storage};

fn runtime_error(kind: SourceErrorKind) -> SourceError {
    let mut message = String::with_capacity(128);
    message.push_str("diagnostic λ\nsecond line");
    SourceError {
        kind,
        primary: Some(SourceSpan {
            file: FileId(7),
            lo: 3,
            hi: 11,
        }),
        message,
        _diagnostic_lease: None,
        location: Some(SourceLocation {
            path: PathBuf::from("source/λ.rs"),
            line: 4,
            column: 9,
        }),
    }
}

#[test]
fn unleased_publication_moves_every_error_kind_without_reallocation() {
    for kind in [
        SourceErrorKind::Io,
        SourceErrorKind::Syntax,
        SourceErrorKind::Unsupported,
        SourceErrorKind::Limit,
        SourceErrorKind::Cycle,
        SourceErrorKind::Module,
        SourceErrorKind::Contract,
        SourceErrorKind::Infrastructure,
    ] {
        let error = runtime_error(kind);
        let rendered = error.to_string();
        let pointer = error.message.as_ptr();
        let capacity = error.message.capacity();
        let location_pointer = error
            .location
            .as_ref()
            .unwrap()
            .path
            .as_os_str()
            .as_encoded_bytes()
            .as_ptr();
        let published = SourceUnitError::try_from(error).unwrap();
        assert_eq!(published.message.as_ptr(), pointer);
        assert_eq!(published.message.capacity(), capacity);
        assert_eq!(
            published
                .location
                .as_ref()
                .unwrap()
                .path
                .as_os_str()
                .as_encoded_bytes()
                .as_ptr(),
            location_pointer
        );
        let restored = SourceError::from(published);
        assert_eq!(restored.kind, kind);
        assert_eq!(
            restored.primary,
            Some(SourceSpan {
                file: FileId(7),
                lo: 3,
                hi: 11
            })
        );
        assert_eq!(restored.to_string(), rendered);
        assert_eq!(restored.message.as_ptr(), pointer);
        assert!(restored._diagnostic_lease.is_none());
    }
}

#[test]
fn leased_publication_returns_original_error_without_refund_or_retention() {
    for storage in [Storage::Retained, Storage::Temporary] {
        let budget = ExpansionBudget::default();
        let mut error = runtime_error(SourceErrorKind::Syntax);
        let bytes = error.message.capacity() as u64;
        error._diagnostic_lease = Some(budget.reserve(storage, bytes).unwrap());
        let before = budget.snapshot();
        let pointer = error.message.as_ptr();
        let rendered = error.to_string();
        let error = SourceUnitError::try_from(error).unwrap_err();
        assert_eq!(budget.snapshot(), before);
        assert_eq!(error.message.as_ptr(), pointer);
        assert_eq!(error.to_string(), rendered);
        assert_eq!(error._diagnostic_lease.as_ref().unwrap().bytes(), bytes);
        drop(error);
        let after = budget.snapshot();
        assert_eq!(after.retained_bytes, 0);
        assert_eq!(after.temporary_bytes, 0);
        assert_eq!(after.reservations.current_reservations, 0);
        assert_eq!(
            after.reservations.cumulative_bytes,
            before.reservations.cumulative_bytes
        );
    }
}

#[test]
fn publication_rejects_leases_in_both_failure_and_terminal_without_consuming_lease() {
    for terminal in [false, true] {
        let budget = ExpansionBudget::default();
        let mut error = runtime_error(SourceErrorKind::Contract);
        let bytes = error.message.capacity() as u64;
        let pointer = error.message.as_ptr();
        error._diagnostic_lease = Some(budget.reserve(Storage::Retained, bytes).unwrap());
        let pending = if terminal {
            let mut unit =
                prepare_text_unit("input.rs", "fn main() {}", &SourceOptions::default()).unwrap();
            unit.terminal = Some(error);
            Ok(unit)
        } else {
            Err(PendingUnitFailure {
                error,
                anchored: true,
            })
        };
        let before = budget.snapshot();
        let error = publish_unit(pending).unwrap_err();
        assert_eq!(error.message.as_ptr(), pointer);
        assert_eq!(error._diagnostic_lease.as_ref().unwrap().bytes(), bytes);
        assert_eq!(budget.snapshot(), before);
        drop(error);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}

#[test]
fn published_failure_preserves_anchor_and_unique_and_shared_assembly() {
    for anchored in [false, true] {
        let pending = Err(PendingUnitFailure {
            error: runtime_error(SourceErrorKind::Io),
            anchored,
        });
        let published = publish_unit(pending).unwrap();
        assert_eq!(published.as_ref().unwrap_err().anchored, anchored);
        let unit = Arc::new(published);
        let mut shared = SourceAssembler::user_only(&SourceOptions::default());
        let shared_error = shared.provide_published(unit.clone()).err().unwrap();
        let mut unique = SourceAssembler::user_only(&SourceOptions::default());
        let unique_error = unique.provide_published(unit).err().unwrap();
        assert_eq!(shared_error.kind, unique_error.kind);
        assert_eq!(shared_error.primary, unique_error.primary);
        assert_eq!(shared_error.to_string(), unique_error.to_string());
    }
}

#[test]
fn published_terminal_preserves_unique_and_shared_location_and_payload_moves() {
    let options = SourceOptions {
        prelude: false,
        ..SourceOptions::default()
    };
    let pending = prepare_text_unit("λ.rs", "fn main() {}\n}", &options).unwrap();
    assert!(pending.terminal.is_some());
    let text_pointer = pending.text.as_ptr();
    let origins_pointer = pending.origins.as_ptr();
    let message_pointer = pending.terminal.as_ref().unwrap().message.as_ptr();
    let unit = publish_unit(Ok(pending)).unwrap();
    let payload = unit.as_ref().unwrap();
    assert_eq!(payload.text.as_ptr(), text_pointer);
    assert_eq!(payload.origins.as_ptr(), origins_pointer);
    assert_eq!(
        payload.terminal.as_ref().unwrap().message.as_ptr(),
        message_pointer
    );
    let unit = Arc::new(unit);
    let mut shared = SourceAssembler::user_only(&options);
    let shared_error = shared.provide_published(unit.clone()).err().unwrap();
    let mut unique = SourceAssembler::user_only(&options);
    let unique_error = unique.provide_published(unit).err().unwrap();
    let direct = prepare_text("λ.rs", "fn main() {}\n}", &options).unwrap_err();
    assert_eq!(shared_error.to_string(), direct.to_string());
    assert_eq!(unique_error.to_string(), direct.to_string());
    assert_eq!(unique_error.primary, direct.primary);
}

#[test]
fn published_fact_types_are_send_sync_without_runtime_lease_carriers() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<SourceUnitError>();
    send_sync::<FileUnit>();
    send_sync::<UnitResult>();
}
