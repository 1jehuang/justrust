//! Opt-in anonymous diagnostics for admitted QueryJobs only, not a compiler graph.
//! Never call publication methods while holding query shard or latch locks.
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(test))]
use rustc_data_structures::sync::AtomicU64 as JobAtomicU64;
#[cfg(test)]
use std::sync::atomic::AtomicU64 as JobAtomicU64;

const MAX_EVENTS: usize = 100_000;
const MAX_CONTEXTS: usize = 1024;
static FILE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct QueryAdmissionObserver(Arc<Inner>);
struct Inner {
    // FD-relative publication stays private if the original path is replaced.
    _directory_handle: File,
    epoch: Instant,
    next_context: AtomicU64,
    returned: AtomicBool,
    data: Mutex<Data>,
}
#[derive(Default)]
struct Data {
    contexts: Vec<(u64, Option<u64>)>,
    events: Vec<Event>,
    lost: u64,
}
struct Event {
    kind: &'static str,
    context: u64,
    job: u64,
    parent: Option<u64>,
    at: u64,
}
#[derive(Clone)]
pub struct QueryAdmissionContext {
    observer: QueryAdmissionObserver,
    id: u64,
}
pub struct QueryAdmissionJob {
    context: QueryAdmissionContext,
    id: u64,
    terminal: bool,
}

/// Non-generic allocator owner. Its Drop cannot tighten dropck for tcx arena references.
/// With observation disabled this is only an inline atomic and None, with no allocation.
pub struct QueryJobAllocator {
    counter: JobAtomicU64,
    observer: Option<QueryAdmissionContext>,
}

impl QueryJobAllocator {
    pub fn new(observer: Option<&QueryAdmissionObserver>) -> Self {
        let counter = JobAtomicU64::new(1);
        debug_assert_eq!(counter.load(Ordering::Relaxed), 1);
        Self { counter, observer: observer.map(QueryAdmissionObserver::context) }
    }

    pub fn observer(&self) -> Option<&QueryAdmissionContext> {
        self.observer.as_ref()
    }
}

impl std::ops::Deref for QueryJobAllocator {
    type Target = JobAtomicU64;

    fn deref(&self) -> &Self::Target {
        &self.counter
    }
}

impl Drop for QueryJobAllocator {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            observer.snapshot(self.counter.load(Ordering::Relaxed));
        }
    }
}

impl Inner {
    fn data(&self) -> MutexGuard<'_, Data> {
        match self.data.lock() {
            Ok(data) => data,
            Err(error) => {
                let mut data = error.into_inner();
                data.lost = data.lost.saturating_add(1);
                data
            }
        }
    }
}

impl QueryAdmissionObserver {
    /// Requires a private directory. Unsupported platforms fail open by disabling capture.
    pub fn new(directory: &Path) -> Option<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = directory;
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            if let Err(error) = builder.create(directory) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return None;
                }
            }
            if !fs::symlink_metadata(directory).ok()?.is_dir() {
                return None;
            }
            // Linux O_NOFOLLOW | O_DIRECTORY closes the leaf symlink/replacement race.
            let handle = fs::OpenOptions::new()
                .read(true)
                .custom_flags(131072 | 65536)
                .open(directory)
                .ok()?;
            let metadata = handle.metadata().ok()?;
            if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
                return None;
            }
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            // SAFETY: geteuid has no arguments or memory preconditions on Linux.
            let owner = unsafe { geteuid() };
            if metadata.uid() != owner {
                return None;
            }
            Some(Self(Arc::new(Inner {
                _directory_handle: handle,
                epoch: Instant::now(),
                next_context: AtomicU64::new(1),
                returned: AtomicBool::new(false),
                data: Mutex::new(Data::default()),
            })))
        }
    }
    pub fn context(&self) -> QueryAdmissionContext {
        let id = self.0.next_context.fetch_add(1, Ordering::Relaxed);
        let mut data = self.0.data();
        if data.contexts.len() < MAX_CONTEXTS && data.contexts.try_reserve(1).is_ok() {
            data.contexts.push((id, None));
        } else {
            data.lost = data.lost.saturating_add(1);
        }
        QueryAdmissionContext { observer: self.clone(), id }
    }
    /// This is an interface callback outcome, never a process/driver outcome.
    pub fn finalize_interface_returned(&self) {
        self.0.returned.store(true, Ordering::Relaxed);
    }
}
impl QueryAdmissionContext {
    /// Lock-free identity check. Numeric context IDs alone are not unique across roots.
    pub fn same_context(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.observer.0, &other.observer.0)
    }

    /// Publish only outside query shard/latch locks, including namespace mismatch loss.
    pub fn mark_loss(&self) {
        let mut data = self.observer.0.data();
        data.lost = data.lost.saturating_add(1);
    }

    pub fn admit(&self, id: u64, parent: Option<u64>, at: Instant) -> QueryAdmissionJob {
        self.event("admitted", id, parent, at);
        QueryAdmissionJob { context: self.clone(), id, terminal: false }
    }
    pub fn snapshot(&self, high_water: u64) {
        let mut data = self.observer.0.data();
        if let Some(context) = data.contexts.iter_mut().find(|c| c.0 == self.id) {
            context.1 = Some(high_water);
        }
    }
    fn event(&self, kind: &'static str, job: u64, parent: Option<u64>, at: Instant) {
        let elapsed = at.saturating_duration_since(self.observer.0.epoch).as_nanos();
        let at = u64::try_from(elapsed).unwrap_or(u64::MAX);
        let mut data = self.observer.0.data();
        if data.events.len() < MAX_EVENTS && data.events.try_reserve(1).is_ok() {
            data.events.push(Event { kind, context: self.id, job, parent, at });
        } else {
            data.lost = data.lost.saturating_add(1);
        }
    }
}
impl QueryAdmissionJob {
    pub fn complete(mut self) {
        self.terminal = true;
        self.context.event("completed", self.id, None, Instant::now());
    }
}
impl Drop for QueryAdmissionJob {
    fn drop(&mut self) {
        if !self.terminal {
            self.terminal = true;
            self.context.event("poisoned", self.id, None, Instant::now());
        }
    }
}
impl Data {
    fn context_inventory_matches(&self, expected_contexts: u64) -> bool {
        // This count comes from the root allocator, never from the captured context list.
        // Removing an entire context and all of its events must still fail reconciliation.
        if self.contexts.len() as u64 != expected_contexts {
            return false;
        }
        let mut contexts = Vec::new();
        if contexts.try_reserve(self.contexts.len()).is_err() {
            return false;
        }
        contexts.extend(self.contexts.iter().map(|context| context.0));
        contexts.sort_unstable();
        contexts.iter().enumerate().all(|(index, &id)| id == index as u64 + 1)
    }

    fn complete(&self, expected_contexts: u64) -> bool {
        if self.lost != 0 || !self.context_inventory_matches(expected_contexts) {
            return false;
        }
        if self
            .events
            .iter()
            .any(|event| !matches!(event.kind, "admitted" | "completed" | "poisoned"))
        {
            return false;
        }
        self.contexts.iter().all(|&(context, high)| {
            let Some(high) = high else {
                return false;
            };
            if high < 1 {
                return false;
            }
            let admissions =
                self.events.iter().filter(|e| e.context == context && e.kind == "admitted").count()
                    as u64;
            if admissions != high - 1 {
                return false;
            }
            // Sorted IDs check uniqueness, contiguous allocator coverage, and one terminal each.
            let mut admitted = Vec::new();
            let mut terminal = Vec::new();
            if admitted.try_reserve(self.events.len()).is_err()
                || terminal.try_reserve(self.events.len()).is_err()
            {
                return false;
            }
            admitted.extend(
                self.events
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.context == context && e.kind == "admitted")
                    .map(|(index, e)| (e.job, e.at, e.parent, index)),
            );
            terminal.extend(
                self.events
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.context == context && e.kind != "admitted")
                    .map(|(index, e)| (e.job, e.at, index)),
            );
            admitted.sort_unstable();
            terminal.sort_unstable();
            admitted.len() == terminal.len()
                && admitted.iter().zip(&terminal).enumerate().all(|(i, (admission, terminal))| {
                    admission.0 == i as u64 + 1
                        && admission.0 == terminal.0
                        && terminal.1 >= admission.1
                        && terminal.2 > admission.3
                        && admission.2.is_none_or(|parent| parent >= 1 && parent < admission.0)
                        && admission.2.is_none_or(|parent| {
                            admitted
                                .get((parent - 1) as usize)
                                .is_some_and(|parent| parent.1 <= admission.1)
                        })
                })
        }) && self.events.iter().all(|e| self.contexts.iter().any(|c| c.0 == e.context))
    }
    fn json(&self, returned: bool, capture_id: &str, expected_contexts: u64) -> String {
        let mut out = format!(
            "{{\"schema\":1,\"capture_id\":\"{capture_id}\",\"producer\":\"rustc-query-admission\",\"scope\":\"query-job-admission-lifecycle\",\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"interface_outcome\":\"{}\",\"capture_complete\":{},\"lost_events\":{},\"expected_contexts\":{expected_contexts},\"opaque_boundaries\":[\"fast-cache-hits\",\"race-cache-hits\",\"green-ensure-no-job\",\"feeds\",\"direct-paths\",\"nonquery\",\"resource-internals\",\"driver-and-process-outcome\",\"cargo-parent-binding\",\"cache-read-dependencies\",\"wait-and-resource-dependencies\"],\"contexts\":[",
            if returned { "returned" } else { "interrupted" },
            returned && self.complete(expected_contexts),
            self.lost
        );
        for (i, &(id, high)) in self.contexts.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let high_text = high.map_or_else(|| "null".into(), |h| h.to_string());
            let expected = high
                .and_then(|h| h.checked_sub(1))
                .map_or_else(|| "null".into(), |h| h.to_string());
            let _ = write!(
                out,
                "{{\"context_id\":{id},\"allocator_baseline\":1,\"allocator_high_water\":{high_text},\"expected_jobs\":{expected}}}"
            );
        }
        out.push_str("],\"events\":[");
        for (i, event) in self.events.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"kind\":\"{}\",\"context_id\":{},\"job_id\":{},\"at_ns\":{}",
                event.kind, event.context, event.job, event.at
            );
            if event.kind == "admitted" {
                let parent = event.parent.map_or_else(|| "null".into(), |p| p.to_string());
                let _ = write!(out, ",\"parent_job_id\":{parent}");
            }
            out.push('}');
        }
        out.push_str("]}\n");
        out
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let data = match self.data.get_mut() {
            Ok(data) => data,
            Err(error) => {
                let data = error.into_inner();
                data.lost = data.lost.saturating_add(1);
                data
            }
        };
        let id = FILE_ID.fetch_add(1, Ordering::Relaxed);
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let name = format!("{epoch}-{}-{id}", std::process::id());
        // All context/job handles own this root Arc. Its final Drop observes a stable
        // independent allocator highwater, including contexts whose receipts were lost.
        let expected_contexts = self.next_context.load(Ordering::Relaxed).saturating_sub(1);
        if !data.context_inventory_matches(expected_contexts) {
            data.lost = data.lost.saturating_add(1);
        }
        let json = data.json(self.returned.load(Ordering::Relaxed), &name, expected_contexts);
        #[cfg(target_os = "linux")]
        publish(&self._directory_handle, &name, json.as_bytes());
        #[cfg(not(target_os = "linux"))]
        let _ = json;
    }
}

#[cfg(target_os = "linux")]
fn publish(directory: &File, name: &str, bytes: &[u8]) {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::PermissionsExt;
    // Refuse publication if the directory was made public while capture ran.
    if !directory.metadata().is_ok_and(|m| m.permissions().mode() & 0o077 == 0) {
        return;
    }
    let Ok(temp) = CString::new(format!(".{name}.tmp")) else {
        return;
    };
    let Ok(receipt) = CString::new(format!("{name}.json")) else {
        return;
    };
    unsafe extern "C" {
        fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, mode: u32) -> i32;
        fn linkat(
            oldfd: i32,
            old: *const std::ffi::c_char,
            newfd: i32,
            new: *const std::ffi::c_char,
            flags: i32,
        ) -> i32;
        fn unlinkat(fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
    }
    let fd = directory.as_raw_fd();
    // SAFETY: valid held directory FD and NUL-terminated anonymous filename. Linux flags
    // O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC refuse existing files and symlinks.
    let output = unsafe { openat(fd, temp.as_ptr(), 1 | 64 | 128 | 524288, 0o600) };
    if output < 0 {
        return;
    }
    // SAFETY: openat returned a newly owned FD, transferred exactly once to File.
    let mut file = unsafe { File::from_raw_fd(output) };
    if file.write_all(bytes).is_ok() && file.sync_all().is_ok() {
        // SAFETY: both names remain valid for the call, FD is held. linkat is atomic
        // and cannot replace an existing destination, unlike rename.
        let _ = unsafe { linkat(fd, temp.as_ptr(), fd, receipt.as_ptr(), 0) };
        let _ = directory.sync_all();
    }
    // SAFETY: only our successfully created temporary anonymous file is unlinked.
    let _ = unsafe { unlinkat(fd, temp.as_ptr(), 0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    fn private_directory() -> std::path::PathBuf {
        let id = FILE_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("query-admission-test-{}-{id}", std::process::id()))
    }
    #[cfg(target_os = "linux")]
    fn receipt(directory: &Path) -> String {
        let files: Vec<_> = fs::read_dir(directory).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].extension().unwrap(), "json");
        let text = fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains(&format!(
            "\"capture_id\":\"{}\"",
            files[0].file_stem().unwrap().to_str().unwrap()
        )));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
        text
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn lifetime_terminal_once_and_root_outlives_session() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let root = observer.clone();
        let first = observer.context();
        let second = observer.context();
        first.admit(1, None, Instant::now()).complete();
        let parent = first.admit(2, Some(1), Instant::now());
        drop(parent);
        second.admit(1, None, Instant::now()).complete();
        first.snapshot(3);
        second.snapshot(2);
        drop(first);
        drop(second);
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        root.finalize_interface_returned();
        drop(root);
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":true"));
        assert_eq!(text.matches("\"kind\":\"completed\"").count(), 2);
        assert_eq!(text.matches("\"kind\":\"poisoned\"").count(), 1);
        assert!(text.contains("\"context_id\":2"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn allocator_off_is_inline_and_retains_native_baseline() {
        let jobs = QueryJobAllocator::new(None);
        assert!(jobs.observer().is_none());
        assert_eq!(jobs.load(Ordering::Relaxed), 1);
        assert_eq!(jobs.fetch_add(1, Ordering::Relaxed), 1);
        assert_eq!(jobs.fetch_add(1, Ordering::Relaxed), 2);
        assert_eq!(jobs.load(Ordering::Relaxed), 3);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn allocator_drop_snapshots_highwater_without_generic_owner_drop() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let jobs = QueryJobAllocator::new(Some(&observer));
        let id = jobs.fetch_add(1, Ordering::Relaxed);
        jobs.observer().unwrap().admit(id, None, Instant::now()).complete();
        assert!(observer.0.data().contexts[0].1.is_none());
        drop(jobs);
        assert_eq!(observer.0.data().contexts[0].1, Some(2));
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"allocator_high_water\":2"));
        assert!(text.contains("\"expected_jobs\":1"));
        assert!(text.contains("\"capture_complete\":true"));
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn context_identity_uses_context_id_and_root_identity() {
        let path = private_directory();
        let other_path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let other_observer = QueryAdmissionObserver::new(&other_path).unwrap();
        let context = observer.context();
        let same = context.clone();
        let foreign_context = observer.context();
        let foreign_root = other_observer.context();
        assert!(context.same_context(&context));
        assert!(context.same_context(&same));
        assert!(!context.same_context(&foreign_context));
        assert_eq!(context.id, foreign_root.id);
        assert!(!context.same_context(&foreign_root));
        context.admit(1, None, Instant::now()).complete();
        foreign_context.admit(1, None, Instant::now()).complete();
        foreign_root.admit(1, None, Instant::now()).complete();
        context.snapshot(2);
        foreign_context.snapshot(2);
        foreign_root.snapshot(2);
        drop(context);
        drop(same);
        drop(foreign_context);
        drop(foreign_root);
        observer.finalize_interface_returned();
        other_observer.finalize_interface_returned();
        drop(observer);
        drop(other_observer);
        assert!(receipt(&path).contains("\"capture_complete\":true"));
        assert!(receipt(&other_path).contains("\"capture_complete\":true"));
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(other_path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn foreign_parent_loss_avoids_fabricated_local_numeric_parent() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let context = observer.context();
        let foreign = observer.context();
        context.admit(1, None, Instant::now()).complete();
        foreign.admit(1, None, Instant::now()).complete();
        // A foreign job1 must not silently name the unrelated local job1 for local job2.
        let parent = if context.same_context(&foreign) {
            Some(1)
        } else {
            context.mark_loss();
            None
        };
        context.admit(2, parent, Instant::now()).complete();
        context.snapshot(3);
        foreign.snapshot(2);
        drop(context);
        drop(foreign);
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":false"));
        assert!(text.contains("\"lost_events\":1"));
        assert_eq!(text.matches("\"parent_job_id\":null").count(), 3);
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn unwind_is_interrupted_even_with_complete_jobs() {
        let path = private_directory();
        let result = std::panic::catch_unwind(|| {
            let observer = QueryAdmissionObserver::new(&path).unwrap();
            let context = observer.context();
            context.snapshot(2);
            let _job = context.admit(1, None, Instant::now());
            panic!("simulated query panic");
        });
        assert!(result.is_err());
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":false"));
        assert!(text.contains("\"interface_outcome\":\"interrupted\""));
        assert_eq!(text.matches("\"kind\":\"poisoned\"").count(), 1);
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn publication_no_replace_and_path_replacement_stays_anchored() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        publish(&observer.0._directory_handle, "1-2-3", b"first");
        publish(&observer.0._directory_handle, "1-2-3", b"second");
        assert_eq!(fs::read(path.join("1-2-3.json")).unwrap(), b"first");
        let moved = path.with_extension("moved");
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        observer.finalize_interface_returned();
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 2);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(moved).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_public_and_symlink_directories_and_fails_open() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = private_directory();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(QueryAdmissionObserver::new(&path).is_none());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let link = path.with_extension("link");
        symlink(&path, &link).unwrap();
        assert!(QueryAdmissionObserver::new(&link).is_none());
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        observer.finalize_interface_returned();
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        fs::remove_file(link).unwrap();
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bounds_mark_capture_incomplete() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let context = observer.context();
        for id in 1..=(MAX_EVENTS as u64 / 2 + 1) {
            context.admit(id, None, Instant::now()).complete();
        }
        context.snapshot(MAX_EVENTS as u64 / 2 + 2);
        let data = observer.0.data.lock().unwrap();
        assert_eq!(data.events.len(), MAX_EVENTS);
        assert_eq!(data.lost, 2);
        assert!(!data.complete(1));
        drop(data);
        for _ in 1..=MAX_CONTEXTS {
            drop(observer.context());
        }
        assert_eq!(observer.0.data.lock().unwrap().contexts.len(), MAX_CONTEXTS);
        drop(context);
        drop(observer);
        assert!(receipt(&path).contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn independent_allocator_detects_loss_and_duplicates() {
        let mut data = Data { contexts: vec![(1, Some(2))], ..Data::default() };
        assert!(!data.complete(1));
        data.events.push(Event { kind: "admitted", context: 1, job: 1, parent: None, at: 0 });
        data.events.push(Event { kind: "completed", context: 1, job: 1, parent: None, at: 1 });
        assert!(data.complete(1));
        data.events.push(Event { kind: "poisoned", context: 1, job: 1, parent: None, at: 2 });
        assert!(!data.complete(1));
    }
    #[test]
    fn independent_root_allocator_detects_deleted_and_invalid_contexts() {
        let mut data = Data { contexts: vec![(1, Some(1)), (2, Some(1))], ..Data::default() };
        assert!(data.complete(2));
        data.contexts.pop();
        assert!(!data.complete(2));
        data.contexts.push((1, Some(1)));
        assert!(!data.complete(2));
        data.contexts[1].0 = 3;
        assert!(!data.complete(2));
        data.contexts[1].0 = 0;
        assert!(!data.complete(2));
        data.contexts[1].0 = 2;
        data.contexts.swap(0, 1);
        assert!(data.complete(2));
        assert!(Data::default().complete(0));
        assert!(!Data::default().complete(1));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn deleting_whole_context_and_jobs_marks_root_loss() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let first = observer.context();
        let second = observer.context();
        first.admit(1, None, Instant::now()).complete();
        first.snapshot(2);
        second.admit(1, None, Instant::now()).complete();
        second.snapshot(2);
        {
            let mut data = observer.0.data();
            data.contexts.retain(|context| context.0 != second.id);
            data.events.retain(|event| event.context != second.id);
            assert!(!data.complete(2));
        }
        drop(first);
        drop(second);
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"expected_contexts\":2"));
        assert!(text.contains("\"lost_events\":1"));
        assert!(text.contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn parent_admission_cannot_follow_child_admission() {
        let mut data = Data {
            contexts: vec![(1, Some(3))],
            events: vec![
                Event { kind: "admitted", context: 1, job: 1, parent: None, at: 20 },
                Event { kind: "admitted", context: 1, job: 2, parent: Some(1), at: 10 },
                Event { kind: "completed", context: 1, job: 2, parent: None, at: 30 },
                Event { kind: "completed", context: 1, job: 1, parent: None, at: 40 },
            ],
            lost: 0,
        };
        assert!(!data.complete(1));
        data.events[1].at = 20;
        assert!(data.complete(1));
        data.events[1].at = 25;
        assert!(data.complete(1));
    }
    #[test]
    fn invalid_parent_time_kind_and_order_are_incomplete() {
        let mut data = Data {
            contexts: vec![(1, Some(2))],
            events: vec![
                Event { kind: "admitted", context: 1, job: 1, parent: None, at: 10 },
                Event { kind: "completed", context: 1, job: 1, parent: None, at: 11 },
            ],
            lost: 0,
        };
        assert!(data.complete(1));
        data.events[0].parent = Some(1);
        assert!(!data.complete(1));
        data.events[0].parent = Some(0);
        assert!(!data.complete(1));
        data.events[0].parent = None;
        data.events[1].at = 9;
        assert!(!data.complete(1));
        data.events[1].at = 11;
        data.events[1].kind = "invalid";
        assert!(!data.complete(1));
        data.events[1].kind = "completed";
        data.events.swap(0, 1);
        assert!(!data.complete(1));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn poisoned_observer_mutex_does_not_poison_compilation() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let result = std::panic::catch_unwind(|| {
            let _data = observer.0.data.lock().unwrap();
            panic!("simulated recorder failure");
        });
        assert!(result.is_err());
        let context = observer.context();
        context.admit(1, None, Instant::now()).complete();
        context.snapshot(2);
        drop(context);
        observer.finalize_interface_returned();
        drop(observer);
        assert!(receipt(&path).contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn missing_snapshot_and_empty_baseline() {
        let mut data = Data { contexts: vec![(1, None)], ..Data::default() };
        assert!(!data.complete(1));
        data.contexts[0].1 = Some(1);
        assert!(data.complete(1));
        assert!(data.json(false, "1-1", 1).contains("\"interface_outcome\":\"interrupted\""));
        data.lost = 1;
        assert!(!data.complete(1));
    }
}
