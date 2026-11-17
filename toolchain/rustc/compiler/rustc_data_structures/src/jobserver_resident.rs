//! Ownership prerequisite only: this does not route `super::client()` or compiler workers.
//!
//! A future dispatcher must migrate raw clients, helper threads, proxies, and returned tokens to
//! tracked request ownership before using this API to run a compiler request. In particular, merely
//! installing a lease around today's compiler entry point is NOT sufficient.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use jobserver_crate::{Acquired, Client};

static ADMISSION: Admission = Admission::new();

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
    /// Acquires a real token from the supplied client. This can block until that client has a
    /// token available. The dispatcher must arrange explicit capacity, independently of Cargo's
    /// process-lifetime implicit token. Other begin calls are rejected while acquisition waits.
    pub fn begin(client: Client) -> Result<Self, BeginError> {
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

/// A tracked client with deliberately no raw-client, raw-token, or helper-thread escape hatch.
#[derive(Clone)]
pub struct RequestClient {
    inner: Arc<RequestInner>,
}

impl RequestClient {
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
