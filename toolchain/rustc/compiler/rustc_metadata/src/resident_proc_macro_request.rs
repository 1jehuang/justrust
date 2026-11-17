//! One fresh helper owner per compiler request. Merely declaring this API does
//! not admit proc-macro dependencies. Its caller must select the scheduler and
//! disable retained external decoder/query owners. Nested expansion is rejected
//! without replay rather than locking recursively across a bridge callback.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use rustc_data_structures::jobserver::resident::RequestClient;
use rustc_expand::proc_macro_client::{
    ProcMacroClient, REMOTE_DISPATCH, RemoteDispatch, RemoteMacro,
};
use rustc_proc_macro::bridge::PanicMessage;

use crate::resident_proc_macro_parent::MacroHelper;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
static OWNER: Mutex<Option<Owner>> = Mutex::new(None);

struct Owner {
    request: u64,
    executable: PathBuf,
    scheduler: RequestClient,
    helper: Option<MacroHelper>,
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
        *current = Some(Owner { request, executable, scheduler, helper: None });
        Ok(Self { request })
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
        owner.helper = Some(MacroHelper::spawn(&owner.executable, &owner.scheduler)?);
    }
    Ok(owner.helper.as_mut().unwrap())
}

pub fn load(path: &Path, symbol: &str) -> io::Result<Vec<ProcMacroClient>> {
    let mut current = selected()?;
    let owner = current.as_mut().ok_or_else(|| io::Error::other("no proc-macro request owner"))?;
    let (library, count) = helper(owner)?.load(path, symbol)?;
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
        helper(owner)?.expand(remote.library, remote.client, show_panics, &input, |request| {
            Ok(callback(request))
        })
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
