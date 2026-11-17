//! Request ownership and explicit scoped selection. This never changes `super::client()` and
//! does not itself enable managed compiler routing.
//!
//! A future dispatcher must migrate raw clients, helper threads, proxies, and returned tokens to
//! tracked request ownership before using this API to run a compiler request. Merely installing a
//! lease around a compiler entry point is NOT sufficient: every scheduler consumer must use the
//! coordinated tracked facade, with helpers retired and delivered tokens returned before finish.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use jobserver_crate::{Acquired, Client, HelperThread};

static ADMISSION: Admission = Admission::new();
static CURRENT: Mutex<Option<RequestClient>> = Mutex::new(None);

#[cfg(test)]
pub(super) static SELECTION_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Returns a tracked clone for compiler workers only while a request explicitly selects it.
pub(super) fn current_client() -> Option<RequestClient> {
    CURRENT.lock().unwrap().clone()
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectionError {
    OverlappingSelection,
}

struct SelectionGuard;

impl Drop for SelectionGuard {
    fn drop(&mut self) {
        let mut current = CURRENT.lock().unwrap();
        drop(current.take());
    }
}

struct Admission(Mutex<State>);

#[derive(Clone, Copy)]
enum State {
    Idle,
    Active,
    Closed,
}

impl Admission {
    const fn new() -> Self {
        Self(Mutex::new(State::Idle))
    }

    fn begin(&'static self, client: Client) -> Result<RequestLease, BeginError> {
        self.begin_with(client, Client::acquire)
    }

    fn begin_with(
        &'static self,
        client: Client,
        acquire: impl FnOnce(&Client) -> io::Result<Acquired>,
    ) -> Result<RequestLease, BeginError> {
        let mut state = self.0.lock().unwrap();
        match *state {
            State::Active => return Err(BeginError::OverlappingRequest),
            State::Closed => return Err(BeginError::OutstandingHandles),
            State::Idle => {}
        }
        *state = State::Active;
        drop(state);
        let mut reservation = Reservation { admission: self, committed: false };
        // Reserve admission before waiting, but never hold its mutex while acquiring a token.
        // Anonymous-pipe clients do not support try_acquire. Do not change their descriptor flags
        // or infer an implicit token from process ancestry. No environment is read.
        let implicit = acquire(&client).map_err(BeginError::Acquire)?;
        let inner =
            Arc::new(RequestInner { active: AtomicBool::new(true), _implicit: implicit, client });
        reservation.committed = true;
        Ok(RequestLease { admission: self, inner: Some(inner) })
    }
}

struct Reservation {
    admission: &'static Admission,
    committed: bool,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.committed {
            *self.admission.0.lock().unwrap() = State::Idle;
        }
    }
}

/// Failure to establish exclusive resident request ownership.
#[derive(Debug)]
pub enum BeginError {
    OverlappingRequest,
    /// A prior lease ended while tracked clients or tokens remained. Admission stays closed.
    OutstandingHandles,
    Acquire(io::Error),
}

struct RequestInner {
    active: AtomicBool,
    // Keep this real token until the last tracked handle is destroyed, even after failed cleanup.
    _implicit: Acquired,
    client: Client,
}

/// Exclusive ownership of one supplied jobserver for one serial resident request.
///
/// This API never imports descriptor numbers from the environment and never alters the ordinary
/// process jobserver. It reserves a real token, not Cargo's process-lifetime implicit token.
/// Supplying a clone does not grant ownership of other raw clones retained by the caller: a future
/// dispatcher must control those separately. Do not mix this with untracked compiler clients.
#[must_use]
pub struct RequestLease {
    admission: &'static Admission,
    inner: Option<Arc<RequestInner>>,
}

impl RequestLease {
    /// Selects this request for compiler workers on all threads, only for the closure's duration.
    /// Nested or concurrent selections are rejected. Unwinding clears the selection. Escaped
    /// tracked clients still prevent clean lease completion after this scope returns.
    pub fn with_current<R>(&self, f: impl FnOnce() -> R) -> Result<R, SelectionError> {
        let mut current = CURRENT.lock().unwrap();
        if current.is_some() {
            return Err(SelectionError::OverlappingSelection);
        }
        *current = Some(self.client());
        drop(current);
        let _selection = SelectionGuard;
        Ok(f())
    }

    /// Acquires a real token from the supplied client. This can block until that client has a
    /// token available. The dispatcher must arrange explicit capacity, independently of Cargo's
    /// process-lifetime implicit token. Other begin calls are rejected while acquisition waits.
    pub fn begin(client: Client) -> Result<Self, BeginError> {
        ADMISSION.begin(client)
    }

    /// Represents one explicitly delegated scheduler slot with no spare capacity.
    ///
    /// The dispatcher must keep its external Cargo job alive until this lease is
    /// finished, forbid frontend/backend/linker parallelism, and kill the whole
    /// worker process group if that job disconnects. This is not an independent
    /// grant of CPU capacity and must never be used to replace an inherited
    /// parallel jobserver. No raw client escapes this constructor.
    pub fn begin_serial() -> Result<Self, BeginError> {
        let client = Client::new(1).map_err(BeginError::Acquire)?;
        ADMISSION.begin(client)
    }

    pub fn client(&self) -> RequestClient {
        RequestClient { inner: Arc::clone(self.inner.as_ref().unwrap()) }
    }

    /// Ends the request and reports whether every tracked client/token was returned.
    /// Failure closes admission permanently, including when cleanup occurs during unwinding.
    pub fn finish(mut self) -> Result<(), BeginError> {
        self.end()
    }

    fn end(&mut self) -> Result<(), BeginError> {
        let mut state = self.admission.0.lock().unwrap();
        let Some(inner) = self.inner.take() else { return Ok(()) };
        inner.active.store(false, Ordering::Release);
        let clean = Arc::strong_count(&inner) == 1;
        *state = if clean { State::Idle } else { State::Closed };
        // Finish releasing this request's ownership before allowing another begin operation.
        drop(inner);
        if clean { Ok(()) } else { Err(BeginError::OutstandingHandles) }
    }
}

impl Drop for RequestLease {
    fn drop(&mut self) {
        let _ = self.end();
    }
}

/// A tracked client with deliberately no raw-client, raw-token, or raw-helper escape hatch.
#[derive(Clone)]
pub struct RequestClient {
    inner: Arc<RequestInner>,
}

impl RequestClient {
    /// Creates a tracked worker. Tokens sent to other threads remain request-owned.
    ///
    /// Dropping the helper requests shutdown, but does not prove worker retirement. Use the
    /// receipt returned by `RequestHelper::retire` to observe callback destruction before ending
    /// the lease. The callback must not form an ownership cycle with its helper.
    pub fn into_helper_thread<F>(self, callback: F) -> io::Result<RequestHelper>
    where
        F: FnMut(io::Result<RequestToken>) + Send + 'static,
    {
        self.check_active()?;
        let retired = Arc::new(RetirementState::default());
        let mut callback = TrackedCallback {
            callback,
            owner: Arc::clone(&self.inner),
            _completion: RetirementNotifier(Arc::clone(&retired)),
        };
        let helper = self.inner.client.clone().into_helper_thread(move |token| {
            // Capture the whole object, not disjoint fields: user callback destruction must
            // precede ownership release and publication of retirement.
            callback.deliver(token);
        })?;
        Ok(RequestHelper { helper: Some(helper), owner: self.inner, retired })
    }

    /// May block. Ending the request with this handle outstanding permanently closes admission.
    pub fn acquire(&self) -> io::Result<RequestToken> {
        self.check_active()?;
        let token = self.inner.client.acquire()?;
        self.check_active()?;
        Ok(RequestToken { _token: token, _owner: Arc::clone(&self.inner) })
    }

    fn check_active(&self) -> io::Result<()> {
        if self.inner.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(io::Error::other("resident jobserver request has ended"))
        }
    }

    /// Returns Unsupported when the supplied client cannot acquire nonblockingly.
    pub fn try_acquire(&self) -> io::Result<Option<RequestToken>> {
        self.check_active()?;
        let token = self.inner.client.try_acquire()?;
        self.check_active()?;
        Ok(token.map(|token| RequestToken { _token: token, _owner: Arc::clone(&self.inner) }))
    }
}

struct TrackedCallback<F> {
    // Field order is significant even if the callback panics or the worker detaches.
    callback: F,
    owner: Arc<RequestInner>,
    _completion: RetirementNotifier,
}

impl<F: FnMut(io::Result<RequestToken>)> TrackedCallback<F> {
    fn deliver(&mut self, token: io::Result<Acquired>) {
        if !self.owner.active.load(Ordering::Acquire) {
            // Release late tokens without entering user code after observing request end.
            return;
        }
        (self.callback)(
            token.map(|token| RequestToken { _token: token, _owner: Arc::clone(&self.owner) }),
        );
    }
}

#[derive(Default)]
struct RetirementState {
    done: Mutex<bool>,
    wake: Condvar,
}

impl RetirementState {
    fn is_retired(&self) -> bool {
        *self.done.lock().unwrap()
    }
}

struct RetirementNotifier(Arc<RetirementState>);

impl Drop for RetirementNotifier {
    fn drop(&mut self) {
        *self.0.done.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}

/// A tracked helper producer. Its worker independently owns the request until callback destruction.
/// Field order stops the raw helper before releasing producer ownership. Raw helper Drop may
/// detach, so worker ownership and queued tokens are deliberately independent of this wrapper.
#[must_use]
pub struct RequestHelper {
    helper: Option<HelperThread>,
    owner: Arc<RequestInner>,
    retired: Arc<RetirementState>,
}

impl RequestHelper {
    pub fn request_token(&self) -> io::Result<()> {
        if !self.owner.active.load(Ordering::Acquire) {
            return Err(io::Error::other("resident jobserver request has ended"));
        }
        self.helper.as_ref().unwrap().request_token();
        Ok(())
    }

    /// Requests shutdown and returns a non-owning retirement observation, not a join guarantee.
    /// May block according to the underlying platform's shutdown behavior. If it detaches,
    /// `is_retired()` remains false until the callback and worker ownership are destroyed.
    pub fn retire(mut self) -> HelperRetirement {
        drop(self.helper.take());
        HelperRetirement { retired: Arc::clone(&self.retired) }
    }
}

/// Does not keep the request alive. Retirement does not imply queued tokens have been returned.
#[must_use]
pub struct HelperRetirement {
    retired: Arc<RetirementState>,
}

impl HelperRetirement {
    pub fn is_retired(&self) -> bool {
        self.retired.is_retired()
    }

    /// Waits without spinning for callback destruction and worker ownership release. A true result
    /// says nothing about tokens already delivered to queues, which retain separate ownership.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let done = self.retired.done.lock().unwrap();
        let (done, _) = self.retired.wake.wait_timeout_while(done, timeout, |done| !*done).unwrap();
        *done
    }
}

/// Drops the acquired token before releasing its tracked request ownership.
/// There is intentionally no `drop_without_releasing` or conversion to a raw token.
#[must_use]
pub struct RequestToken {
    _token: Acquired,
    _owner: Arc<RequestInner>,
}

#[cfg(test)]
#[path = "jobserver_resident_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "jobserver_resident_helper_tests.rs"]
mod helper_tests;
