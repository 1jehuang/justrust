#[path = "jobserver_ticketed.rs"]
pub mod ticketed;
pub use ticketed::TicketedProxy;
#[path = "jobserver_resource.rs"]
pub mod resource;
pub use resource::{
    AcquireCompletion, BindFailure, CaptureCutReason, ControllerId, FixedCheckpoint,
    PreallocatedResourceExportBuffer, PrefixFacts, ResourceAttachmentDescriptor,
    ResourceCaptureConfig, ResourceCaptureControl, ResourceEndorsementToken,
};
#[path = "frontend_admission.rs"]
mod frontend_admission;
pub use frontend_admission::FrontendAdmission;

#[path = "jobserver_resident.rs"]
pub mod resident;

use std::sync::{Arc, OnceLock};

pub use jobserver_crate::Acquired;
use jobserver_crate::{Client, FromEnv, FromEnvErrorKind, HelperThread};
use parking_lot::{Condvar, Mutex};

// We stick the jobserver client into a global and initialize it once, because there could be
// multiple compiler instances in this process, and the jobserver is per-process.
static CLIENT: OnceLock<Client> = OnceLock::new();

/// Initializes a jobserver client for the current rustc process.
/// If inheriting jobserver from the environment fails for some reason, an new jobserver owned by
/// the current rustc process will be created. If the inheritance failure reason is non-benign,
/// the passed callback will be used to report the error.
pub fn initialize(limit: usize, report: impl FnOnce(String)) {
    CLIENT.get_or_init(|| {
        // Safety: the checked client construction ensures that the jobserver file descriptors
        // (if any) are open and valid. We also try to initialize the jobserver as early as possible
        // to avoid unrelated file descriptors with matching values becoming open and valid between
        // the process start and the jobserver initialization.
        let FromEnv { client, var } = unsafe { Client::from_env_ext(true) };

        let error = match client {
            Ok(client) => return client,
            Err(error) => error,
        };

        if !matches!(
            error.kind(),
            FromEnvErrorKind::NoEnvVar
                | FromEnvErrorKind::NoJobserver
                | FromEnvErrorKind::NegativeFd
                | FromEnvErrorKind::Unsupported
        ) {
            // Environment specifies jobserver, but it looks incorrect.
            // Can unwrap because `var` is `None` only when the error kind is `NoEnvVar`.
            let (name, value) = var.unwrap();
            let msg = "failed to connect to jobserver from environment variable";
            report(format!("{msg} `{name}={value:?}`: {error}"));
        }

        // Create a new jobserver if there's no inherited one.
        let client = Client::new(limit).expect("failed to create jobserver");
        // Acquire the single token that is always held by the rustc process.
        // This is an equivalent of the single token held by a higher level build tool while
        // running this instance of rustc. This token is never released - if we are here, then
        // rustc owns the jobserver, it is teared down when rustc exits, and there's no one to
        // return the token to.
        client.acquire_raw().ok();
        client
    });
}

/// Returns the jobserver client previously initialized by `initialize_checked`.
///
/// # Assumptions about holding jobserver tokens
///
/// Rustc process must always hold a single token to avoid being permanently starved and blocked.
/// - If the jobserver is inherited from a higher level build tool, the assumption is that the tool
///   will hold the token and not release it until the rustc process exits.
/// - If the jobserver is owned by the current rustc, the token is acquired by `default_client`.
///
/// To avoid releasing the last token, users of the client returned by this function must ensure
/// that they never release more tokens than was previously explicitly acquired.
/// Example of a sequence that can accidentally release the last token:
/// `release_raw` -> `wait` -> `acquire_raw`.
/// To avoid situations like this use the `jobserver::Proxy` wrapper instead,
/// it will ensure that the last token is never released.
pub fn client() -> Client {
    CLIENT.get().expect("uninitialized jobserver client").clone()
}

/// Codegen and linker access to the current request's scheduler. Ordinary
/// compilation retains the process-scoped client and its existing behavior.
/// This does not install a resident request or authorize managed routing.
pub fn compiler_client() -> CompilerClient {
    match resident::current_client() {
        Some(client) => CompilerClient::Resident(client),
        None => CompilerClient::Ordinary(client()),
    }
}

pub enum CompilerClient {
    Ordinary(Client),
    Resident(resident::RequestClient),
}

impl CompilerClient {
    pub fn is_resident(&self) -> bool {
        matches!(self, Self::Resident(_))
    }

    pub fn try_acquire(&self) -> std::io::Result<Option<CompilerAcquired>> {
        match self {
            Self::Ordinary(client) => client.try_acquire().map(|token| {
                token.map(|token| CompilerAcquired {
                    _token: CompilerToken::Ordinary { _token: token },
                })
            }),
            Self::Resident(client) => client.try_acquire().map(|token| {
                token.map(|token| CompilerAcquired {
                    _token: CompilerToken::Resident { _token: token },
                })
            }),
        }
    }

    pub fn into_helper_thread<F>(self, mut callback: F) -> std::io::Result<CompilerHelper>
    where
        F: FnMut(std::io::Result<CompilerAcquired>) + Send + 'static,
    {
        let helper = match self {
            Self::Ordinary(client) => {
                CompilerHelperKind::Ordinary(client.into_helper_thread(move |token| {
                    callback(token.map(|token| CompilerAcquired {
                        _token: CompilerToken::Ordinary { _token: token },
                    }));
                })?)
            }
            Self::Resident(client) => {
                CompilerHelperKind::Resident(client.into_helper_thread(move |token| {
                    callback(token.map(|token| CompilerAcquired {
                        _token: CompilerToken::Resident { _token: token },
                    }));
                })?)
            }
        };
        Ok(CompilerHelper(Some(helper)))
    }
}

/// An acquired codegen/linker token. No raw-token conversion is available:
/// resident tokens retain their request owner until normal destruction.
pub struct CompilerAcquired {
    _token: CompilerToken,
}

enum CompilerToken {
    Ordinary { _token: Acquired },
    Resident { _token: resident::RequestToken },
}

pub struct CompilerHelper(Option<CompilerHelperKind>);

enum CompilerHelperKind {
    Ordinary(HelperThread),
    Resident(resident::RequestHelper),
}

impl CompilerHelper {
    pub fn request_token(&self) {
        match self.0.as_ref().expect("retired compiler helper") {
            CompilerHelperKind::Ordinary(helper) => helper.request_token(),
            CompilerHelperKind::Resident(helper) => helper
                .request_token()
                .expect("codegen requested a token after its resident request ended"),
        }
    }
}

impl Drop for CompilerHelper {
    fn drop(&mut self) {
        if let Some(CompilerHelperKind::Resident(helper)) = self.0.take() {
            // Returning from codegen must not race a detached helper's owner
            // destruction. A timeout leaves the real owner tracked, so request
            // completion fails closed rather than reusing an active worker.
            let _ = helper.retire().wait_timeout(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
#[path = "jobserver_compiler_tests.rs"]
mod compiler_tests;

struct ProxyData {
    /// The number of tokens assigned to actively working threads,
    /// possibly including the single permanently held token.
    /// If this number is 0, the single token is still held by the process,
    /// but is not currently used for active CPU work.
    /// This can happen, for example, if the main thread is waiting for something,
    /// in that case some other thread can start using this token to do work.
    used: u16,
    /// The number of threads currently requesting a token and waiting.
    /// If the proxy releases a token it can immediately give it to one of these threads
    /// without going through the real jobserver.
    pending: u16,
}

/// A wrapper around jobserver client used for two purposes:
/// - Ensuring that the single token that must be permanently held by the rustc process
///   cannot be accidentally released.
/// - "Token buffering", immediately acquiring freshly released tokens if necessary,
///   without going through the real jobserver.
pub struct Proxy {
    /// The wrapped jobserver client.
    client: Client,
    /// Helper thread associated with the wrapped client.
    helper: OnceLock<HelperThread>,
    /// The proxy's own data.
    data: Mutex<ProxyData>,
    /// Threads which are currently waiting for a token will wait on this condvar.
    wake_pending: Condvar,
}

impl Proxy {
    pub fn new() -> Arc<Self> {
        let proxy = Arc::new(Proxy {
            client: client(),
            // Assume that the main thread is actively doing work when it creates the proxy.
            data: Mutex::new(ProxyData { used: 1, pending: 0 }),
            wake_pending: Condvar::new(),
            helper: OnceLock::new(),
        });
        let proxy_ = Arc::clone(&proxy);
        let helper = proxy
            .client
            .clone()
            .into_helper_thread(move |token| {
                // Reminder: this callback runs when the helper acquires a token.
                if let Ok(token) = token {
                    let mut data = proxy_.data.lock();
                    if data.pending > 0 {
                        // The token is still needed, give it to one of the waiting threads.
                        token.drop_without_releasing();
                        assert!(data.used > 0);
                        data.used += 1;
                        data.pending -= 1;
                        proxy_.wake_pending.notify_one();
                    } else {
                        // The token is no longer needed, release it by dropping.
                        drop(data);
                        drop(token);
                    }
                }
            })
            .expect("failed to spawn helper thread");
        proxy.helper.set(helper).unwrap();
        proxy
    }

    /// Acquires a token, possibly using some buffered tokens as an optimization.
    /// May block and wait until the token is available.
    pub fn acquire_thread(&self) {
        let mut data = self.data.lock();

        if data.used == 0 {
            // No threads are doing any active work, but we are still holding the last token.
            // Give that token to the current thread.
            assert_eq!(data.pending, 0);
            data.used += 1;
        } else {
            // Request a token from the helper thread, this is a non-blocking operation.
            // Then wait until this or some other request succeeds.
            self.helper.get().unwrap().request_token();
            data.pending += 1;
            self.wake_pending.wait(&mut data);
        }
    }

    /// Releases a token, possibly immediately giving it to some other thread as an optimization.
    /// Makes sure that the last token is never actually released to the wrapped jobserver.
    pub fn release_thread(&self) {
        let mut data = self.data.lock();

        if data.pending > 0 {
            // Immediately give the released token to one of the waiting threads.
            data.pending -= 1;
            self.wake_pending.notify_one();
        } else {
            data.used -= 1;

            // Release the token to the wrapped jobserver, unless it's the last one in the process.
            if data.used > 0 {
                drop(data);
                self.client.release_raw().ok();
            }
        }
    }
}
