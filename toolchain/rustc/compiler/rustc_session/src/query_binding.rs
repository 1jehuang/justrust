//! Typed compiler-entry binding facts, separate from query/process outcomes.
#[path = "../../../../shared/query_binding.rs"]
pub mod protocol;

/// Fail-open observation admission. No compiler diagnostics are emitted on failure.
#[cfg(target_os = "linux")]
/// # Safety
/// Native process main must exclusively own valid inherited protocol descriptors,
/// before starting threads or exposing them to embedded compiler callers.
pub unsafe fn admit_inherited(
    ticket_fd: i32,
) -> Option<crate::query_admission::QueryAdmissionObserver> {
    let (binding, directory) = unsafe { protocol::receive_inherited(ticket_fd) }?;
    let entry = entry_json(&binding)?;
    if !crate::query_admission::publish(&directory, &binding.entry_capture_id, entry.as_bytes()) {
        return None;
    }
    crate::query_admission::QueryAdmissionObserver::new_from_directory(
        directory,
        binding.capture_id,
    )
}
#[cfg(not(target_os = "linux"))]
pub unsafe fn admit_inherited(_: i32) -> Option<crate::query_admission::QueryAdmissionObserver> {
    None
}

/// The receipt must be published immediately after validated transport admission,
/// before argument expansion or compiler setup. It declares an expectation only.
pub fn entry_json(binding: &protocol::Binding) -> Option<String> {
    if !binding.valid() {
        return None;
    }
    Some(format!(
        "{{\"schema\":1,\"producer\":\"rustc-query-admission-entry\",\"scope\":\"compiler-entry-to-query-capture-binding\",\"capture_id\":\"{}\",\"run_id\":\"{}\",\"invocation_id\":\"{}\",\"parent_invocation_id\":\"{}\",\"parent_task_id\":\"{}\",\"expected_query_captures\":1,\"query_receipt_schema\":3,\"query_capture_ids\":[\"{}\"],\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null}}\n",
        binding.entry_capture_id,
        binding.run_id,
        binding.invocation_id,
        binding.parent_invocation_id,
        binding.parent_task_id,
        binding.capture_id
    ))
}

/// Parent binding proof, not whole pipeline or process completion.
pub fn binding_json(binding: &protocol::Binding) -> Option<String> {
    if !binding.valid() {
        return None;
    }
    Some(format!(
        "{{\"run_id\":\"{}\",\"invocation_id\":\"{}\",\"parent_invocation_id\":\"{}\",\"parent_task_id\":\"{}\",\"entry_capture_id\":\"{}\"}}",
        binding.run_id,
        binding.invocation_id,
        binding.parent_invocation_id,
        binding.parent_task_id,
        binding.entry_capture_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    fn bound_fixture() -> (std::path::PathBuf, protocol::Binding) {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static ID: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "bound-admission-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let binding = protocol::Binding {
            schema: 1,
            protocol: protocol::PROTOCOL.into(),
            receipt_schema: 3,
            run_id: "run".into(),
            invocation_id: "invocation".into(),
            parent_invocation_id: "parent".into(),
            parent_task_id: "task".into(),
            entry_capture_id: "entry".into(),
            capture_id: "query".into(),
        };
        (path, binding)
    }
    #[cfg(target_os = "linux")]
    fn transport_admit(
        path: &std::path::Path,
        binding: &protocol::Binding,
    ) -> Option<crate::query_admission::QueryAdmissionObserver> {
        let transport = protocol::prepare(path, binding).unwrap();
        let ticket = transport.ticket_fd();
        // Model exclusive process-main ownership transfer.
        std::mem::forget(transport);
        unsafe { admit_inherited(ticket) }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bound_entry_precedes_final_query_and_preserves_schema3_accounting() {
        let (path, binding) = bound_fixture();
        let observer = transport_admit(&path, &binding).unwrap();
        let entry = std::fs::read_to_string(path.join("entry.json")).unwrap();
        assert!(entry.contains("\"query_receipt_schema\":3"));
        assert!(!path.join("query.json").exists());
        let jobs = crate::query_admission::QueryJobAllocator::new(Some(&observer));
        use std::sync::atomic::Ordering;
        let id = jobs.fetch_add(1, Ordering::Relaxed);
        jobs.observer().unwrap().admit(id, None, std::time::Instant::now()).complete();
        drop(jobs);
        observer.finalize_interface_returned();
        drop(observer);
        let query = std::fs::read_to_string(path.join("query.json")).unwrap();
        assert!(query.contains("\"schema\":3"));
        assert!(query.contains("\"capture_id\":\"query\""));
        assert!(query.contains("\"capture_complete\":true"));
        assert!(query.contains("\"allocator_high_water\":2"));
        assert!(query.contains("\"expected_contexts\":1"));
        assert!(query.contains("\"full_pipeline_complete\":false"));
        assert!(!query.contains("directory_fd"));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn failed_entry_publication_disables_bound_capture_without_overwrite() {
        let (path, binding) = bound_fixture();
        std::fs::write(path.join("entry.json"), b"existing").unwrap();
        assert!(transport_admit(&path, &binding).is_none());
        assert_eq!(std::fs::read(path.join("entry.json")).unwrap(), b"existing");
        assert!(!path.join("query.json").exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bound_early_return_is_interrupted_not_successful_capture() {
        let (path, binding) = bound_fixture();
        drop(transport_admit(&path, &binding).unwrap());
        let query = std::fs::read_to_string(path.join("query.json")).unwrap();
        assert!(query.contains("\"interface_outcome\":\"interrupted\""));
        assert!(query.contains("\"capture_complete\":false"));
        assert!(path.join("entry.json").exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn entry_declares_one_exact_capture_without_outcome_claims() {
        let binding = protocol::Binding {
            schema: 1,
            protocol: protocol::PROTOCOL.into(),
            receipt_schema: 3,
            run_id: "run".into(),
            invocation_id: "invocation".into(),
            parent_invocation_id: "parent".into(),
            parent_task_id: "task".into(),
            entry_capture_id: "entry".into(),
            capture_id: "query".into(),
        };
        let entry = entry_json(&binding).unwrap();
        assert!(entry.contains("\"schema\":1"));
        assert!(entry.contains("\"capture_id\":\"entry\""));
        assert!(entry.contains("\"query_capture_ids\":[\"query\"]"));
        assert!(entry.contains("\"expected_query_captures\":1"));
        assert!(entry.contains("\"full_pipeline_complete\":false"));
        assert!(!entry.contains("directory_fd"));
        assert!(!entry.contains("interface_outcome"));
        let mut invalid = binding.clone();
        invalid.capture_id = "\"injection".into();
        assert!(entry_json(&invalid).is_none());
        assert!(binding_json(&invalid).is_none());
        let proof = binding_json(&binding).unwrap();
        assert!(proof.contains("\"entry_capture_id\":\"entry\""));
        assert!(!proof.contains("capture_complete"));
    }
}
