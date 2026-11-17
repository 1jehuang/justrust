//! One fresh helper owner per compiler request. Merely declaring this API does
//! not admit proc-macro dependencies. Its caller must select the scheduler and
//! disable retained external decoder/query owners. Nested expansion is rejected
//! without replay rather than locking recursively across a bridge callback.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use rustc_data_structures::jobserver::resident::RequestClient;
use rustc_expand::proc_macro_client::{
    ProcMacroClient, REMOTE_DISPATCH, RemoteDispatch, RemoteMacro,
};
use rustc_proc_macro::bridge::PanicMessage;

use crate::resident_proc_macro_parent::MacroHelper;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
static OWNER: Mutex<Option<Owner>> = Mutex::new(None);

/// Native request-local operation totals. Expansion time includes callbacks and
/// lazy helper startup; load time includes lazy helper startup. Counts include
/// failed attempts. These measurements never participate in cache identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestMacroStats {
    pub helper_spawn_count: u64,
    pub helper_spawn_ns: u64,
    pub load_count: u64,
    pub load_ns: u64,
    pub expand_count: u64,
    pub expand_ns: u64,
    pub callback_count: u64,
    pub callback_input_bytes: u64,
    pub callback_output_bytes: u64,
    pub callback_ns: u64,
}

fn add(counter: &mut u64, value: u128) {
    *counter = counter.saturating_add(u64::try_from(value).unwrap_or(u64::MAX));
}

impl RequestMacroStats {
    fn callback(
        &mut self,
        input: Vec<u8>,
        callback: &mut dyn FnMut(Vec<u8>) -> Vec<u8>,
    ) -> Vec<u8> {
        add(&mut self.callback_count, 1);
        add(&mut self.callback_input_bytes, input.len() as u128);
        let started = Instant::now();
        let output = callback(input);
        add(&mut self.callback_ns, started.elapsed().as_nanos());
        add(&mut self.callback_output_bytes, output.len() as u128);
        output
    }
}

struct Owner {
    request: u64,
    executable: PathBuf,
    scheduler: RequestClient,
    helper: Option<MacroHelper>,
    stats: RequestMacroStats,
}

fn selected() -> io::Result<MutexGuard<'static, Option<Owner>>> {
    OWNER.try_lock().map_err(|_| {
        io::Error::other(
            "proc-macro helper owner unavailable, poisoned, or nested expansion unsupported",
        )
    })
}

/// Must be dropped before restoring request stdio and finishing its scheduler.
/// No macro, metadata decoder or descriptor may escape this request scope.
#[must_use]
pub struct RequestMacros {
    request: u64,
}

impl RequestMacros {
    pub fn begin(executable: PathBuf, scheduler: RequestClient) -> io::Result<Self> {
        let mut current = selected()?;
        if current.is_some() {
            return Err(io::Error::other("overlapping proc-macro helper requests"));
        }
        let request = NEXT_REQUEST
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
            .map_err(|_| io::Error::other("proc-macro request identity exhausted"))?;
        REMOTE_DISPATCH.swap(&(dispatch as RemoteDispatch));
        *current = Some(Owner {
            request,
            executable,
            scheduler,
            helper: None,
            stats: RequestMacroStats::default(),
        });
        Ok(Self { request })
    }

    /// Snapshot only this request's totals. Retired identities and unavailable
    /// owners fail closed rather than returning another request's measurements.
    pub fn stats(&self) -> io::Result<RequestMacroStats> {
        let current = selected()?;
        current
            .as_ref()
            .filter(|owner| owner.request == self.request)
            .map(|owner| owner.stats)
            .ok_or_else(|| io::Error::other("retired proc-macro request statistics"))
    }
}

impl Drop for RequestMacros {
    fn drop(&mut self) {
        // Recover the poisoned lock only to destroy the child and scheduler
        // handles. The lock remains poisoned and future admission fails closed.
        let mut current = OWNER.lock().unwrap_or_else(|error| error.into_inner());
        if current.as_ref().is_some_and(|owner| owner.request == self.request) {
            drop(current.take());
        }
    }
}

fn helper(owner: &mut Owner) -> io::Result<&mut MacroHelper> {
    if owner.helper.is_none() {
        add(&mut owner.stats.helper_spawn_count, 1);
        let started = Instant::now();
        let spawned = MacroHelper::spawn(&owner.executable, &owner.scheduler);
        add(&mut owner.stats.helper_spawn_ns, started.elapsed().as_nanos());
        owner.helper = Some(spawned?);
    }
    Ok(owner.helper.as_mut().unwrap())
}

pub fn active() -> bool {
    selected().is_ok_and(|owner| owner.is_some())
}

pub fn load(path: &Path, symbol: &str) -> io::Result<Vec<ProcMacroClient>> {
    let mut current = selected()?;
    let owner = current.as_mut().ok_or_else(|| io::Error::other("no proc-macro request owner"))?;
    add(&mut owner.stats.load_count, 1);
    let started = Instant::now();
    let loaded = helper(owner).and_then(|helper| helper.load(path, symbol));
    add(&mut owner.stats.load_ns, started.elapsed().as_nanos());
    let (library, count) = loaded?;
    Ok((0..count)
        .map(|client| {
            ProcMacroClient::Remote(RemoteMacro { request: owner.request, library, client })
        })
        .collect())
}

fn dispatch(
    remote: RemoteMacro,
    show_panics: bool,
    input: Vec<u8>,
    callback: &mut dyn FnMut(Vec<u8>) -> Vec<u8>,
) -> Result<Vec<u8>, PanicMessage> {
    let result = (|| {
        let mut current = selected()?;
        let owner =
            current.as_mut().ok_or_else(|| io::Error::other("retired proc-macro request"))?;
        if owner.request != remote.request {
            return Err(io::Error::other("proc-macro descriptor belongs to a different request"));
        }
        // Keep the owner selected across callbacks. Reentry uses try_lock and
        // fails explicitly, never waits for itself and never replays a macro.
        add(&mut owner.stats.expand_count, 1);
        let started = Instant::now();
        let expanded = (|| {
            helper(owner)?;
            let Owner { helper, stats, .. } = owner;
            helper.as_mut().unwrap().expand(
                remote.library,
                remote.client,
                show_panics,
                &input,
                |request| Ok(stats.callback(request, callback)),
            )
        })();
        add(&mut owner.stats.expand_ns, started.elapsed().as_nanos());
        expanded
    })();
    result.map_err(|error: io::Error| {
        PanicMessage::String(format!("request-owned proc-macro transport failed: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resident_proc_macro_parent::REQUEST_MACRO_TEST_LOCK;
    use rustc_data_structures::jobserver::resident::RequestLease;

    #[test]
    fn counters_saturate_and_callbacks_preserve_payloads() {
        let mut counter = u64::MAX - 1;
        add(&mut counter, 2);
        assert_eq!(counter, u64::MAX);
        counter = 0;
        add(&mut counter, u128::MAX);
        assert_eq!(counter, u64::MAX);

        let mut stats = RequestMacroStats::default();
        let output = stats.callback(vec![1, 2, 3], &mut |input| {
            assert_eq!(input, [1, 2, 3]);
            vec![4, 5]
        });
        assert_eq!(output, [4, 5]);
        assert_eq!(stats.callback_count, 1);
        assert_eq!(stats.callback_input_bytes, 3);
        assert_eq!(stats.callback_output_bytes, 2);
        assert_eq!(stats.expand_count, 0);
        stats.callback_count = u64::MAX;
        stats.callback_input_bytes = u64::MAX;
        stats.callback_output_bytes = u64::MAX;
        stats.callback_ns = u64::MAX;
        stats.callback(vec![1], &mut |input| input);
        assert_eq!(stats.callback_count, u64::MAX);
        assert_eq!(stats.callback_input_bytes, u64::MAX);
        assert_eq!(stats.callback_output_bytes, u64::MAX);
        assert_eq!(stats.callback_ns, u64::MAX);
    }

    #[test]
    fn statistics_reset_and_stale_access_fails_closed() {
        let _serial = REQUEST_MACRO_TEST_LOCK.lock().unwrap();
        let mut retired = None;
        for _ in 0..2 {
            let lease = RequestLease::begin_serial().unwrap();
            let owner = RequestMacros::begin(
                PathBuf::from("/nonexistent-justrust-test-helper/statistics"),
                lease.client(),
            )
            .unwrap();
            assert_eq!(owner.stats().unwrap(), RequestMacroStats::default());
            if let Some(request) = retired {
                assert!(RequestMacros { request }.stats().is_err());
            }
            assert!(load(Path::new("/must-not-be-opened.so"), "unused").is_err());
            let loaded = owner.stats().unwrap();
            assert_eq!(loaded.load_count, 1);
            assert_eq!(loaded.helper_spawn_count, 1);
            assert_eq!(loaded.expand_count, 0);
            assert!(loaded.load_ns >= loaded.helper_spawn_ns);
            assert!(
                dispatch(
                    RemoteMacro { request: owner.request, library: 0, client: 0 },
                    false,
                    vec![],
                    &mut |_| panic!("failed spawn reached callback"),
                )
                .is_err()
            );
            let expanded = owner.stats().unwrap();
            assert_eq!(expanded.expand_count, 1);
            assert_eq!(expanded.helper_spawn_count, 2);
            assert_eq!(expanded.callback_count, 0);
            assert_eq!(expanded.load_count, 1);
            assert_eq!(expanded.load_ns, loaded.load_ns);
            assert!(expanded.helper_spawn_ns >= loaded.helper_spawn_ns);
            assert!(expanded.expand_ns >= expanded.helper_spawn_ns - loaded.helper_spawn_ns);
            let request = owner.request;
            drop(owner);
            assert!(RequestMacros { request }.stats().is_err());
            retired = Some(request);
            lease.finish().unwrap();
        }
    }

    #[test]
    fn generations_and_overlap_fail_before_process_access() {
        let _serial = REQUEST_MACRO_TEST_LOCK.lock().unwrap();
        let mut retired: Option<RemoteMacro> = None;
        for _ in 0..2 {
            let lease = RequestLease::begin_serial().unwrap();
            let owner =
                RequestMacros::begin(PathBuf::from("/must-not-be-executed"), lease.client())
                    .unwrap();
            assert!(RequestMacros::begin(PathBuf::new(), lease.client()).is_err());
            if let Some(remote) = retired {
                assert!(
                    dispatch(remote, false, vec![], &mut |_| {
                        panic!("stale generation reached bridge callback")
                    })
                    .is_err()
                );
                assert!(selected().unwrap().as_ref().unwrap().helper.is_none());
                assert_ne!(remote.request, owner.request);
            }
            retired = Some(RemoteMacro { request: owner.request, library: 0, client: 0 });
            drop(owner);
            assert!(selected().unwrap().is_none());
            lease.finish().unwrap();
        }
    }

    #[test]
    #[ignore = "requires explicitly supplied matching native helper and macro library"]
    fn native_request_generation_and_helper_teardown() {
        let _serial = REQUEST_MACRO_TEST_LOCK.lock().unwrap();
        let executable = PathBuf::from(std::env::var_os("JUSTRUST_TEST_RESIDENT_HELPER").unwrap());
        let library =
            PathBuf::from(std::env::var_os("JUSTRUST_TEST_RESIDENT_MACRO_LIBRARY").unwrap());
        let symbol = std::env::var("JUSTRUST_TEST_RESIDENT_MACRO_SYMBOL").unwrap();
        let mut retired: Option<RemoteMacro> = None;
        for _ in 0..2 {
            let lease = RequestLease::begin_serial().unwrap();
            let owner = RequestMacros::begin(executable.clone(), lease.client()).unwrap();
            if let Some(remote) = retired {
                assert!(
                    dispatch(remote, false, vec![], &mut |_| panic!("stale callback")).is_err()
                );
                assert!(selected().unwrap().as_ref().unwrap().helper.is_none());
            }
            let clients = load(&library, &symbol).unwrap();
            assert!(!clients.is_empty());
            let ProcMacroClient::Remote(remote) = clients[0] else { panic!("local descriptor") };
            let again = load(&library, &symbol).unwrap();
            let ProcMacroClient::Remote(repeated) = again[0] else { panic!("local descriptor") };
            assert_eq!(remote, repeated);
            retired = Some(remote);
            drop(clients);
            drop(again);
            // The request owner closes RPC, kills/reaps its helper, and releases
            // every tracked scheduler clone before the lease may finish.
            drop(owner);
            assert!(selected().unwrap().is_none());
            lease.finish().unwrap();
            assert!(dispatch(remote, false, vec![], &mut |_| panic!("retired callback")).is_err());
        }
    }

    #[test]
    fn recursive_owner_selection_never_blocks() {
        let _serial = REQUEST_MACRO_TEST_LOCK.lock().unwrap();
        let guard = OWNER.lock().unwrap();
        assert!(selected().is_err());
        drop(guard);
    }

    #[test]
    fn unowned_descriptor_never_executes_or_calls_back() {
        let _serial = REQUEST_MACRO_TEST_LOCK.lock().unwrap();
        let error = dispatch(
            RemoteMacro { request: u64::MAX, library: 0, client: 0 },
            false,
            vec![],
            &mut |_| panic!("retired descriptor issued a callback"),
        );
        assert!(error.is_err());
        assert!(load(Path::new("/must-not-be-opened.so"), "unused").is_err());
    }
}
