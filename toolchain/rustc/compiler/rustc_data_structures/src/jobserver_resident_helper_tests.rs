use std::sync::mpsc;
use std::time::Duration;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

fn admission() -> &'static Admission {
    Box::leak(Box::new(Admission::new()))
}

fn await_retirement(receipt: &HelperRetirement) {
    assert!(receipt.wait_timeout(TIMEOUT), "helper did not retire");
    assert!(receipt.is_retired());
}

#[test]
fn sequential_helpers_return_tokens_to_distinct_clients() {
    let admission = admission();
    for capacity in [2, 4] {
        let supplied = Client::new(capacity).unwrap();
        let request = admission.begin(supplied.clone()).unwrap();
        let (tx, rx) = mpsc::channel();
        let helper =
            request.client().into_helper_thread(move |token| tx.send(token).unwrap()).unwrap();
        helper.request_token().unwrap();
        let token = rx.recv_timeout(TIMEOUT).unwrap().unwrap();
        assert_eq!(supplied.available().unwrap(), capacity - 2);
        drop(token);
        let retired = helper.retire();
        await_retirement(&retired);
        request.finish().unwrap();
        assert_eq!(supplied.available().unwrap(), capacity);
    }
}

#[test]
fn retired_helper_does_not_hide_queued_token_ownership() {
    let admission = admission();
    let supplied = Client::new(2).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let (tx, rx) = mpsc::channel();
    let helper = request.client().into_helper_thread(move |token| tx.send(token).unwrap()).unwrap();
    helper.request_token().unwrap();
    let token = rx.recv_timeout(TIMEOUT).unwrap().unwrap();
    let retired = helper.retire();
    await_retirement(&retired);
    assert!(matches!(request.finish(), Err(BeginError::OutstandingHandles)));
    assert_eq!(supplied.available().unwrap(), 0);
    drop(token);
    assert_eq!(supplied.available().unwrap(), 2);
    assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
}

#[test]
fn live_helper_rejects_requests_after_lease_end() {
    let admission = admission();
    let supplied = Client::new(1).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let helper = request.client().into_helper_thread(|_| panic!("unexpected callback")).unwrap();
    assert!(matches!(request.finish(), Err(BeginError::OutstandingHandles)));
    assert!(helper.request_token().is_err());
    assert_eq!(supplied.available().unwrap(), 0);
    let retired = helper.retire();
    await_retirement(&retired);
    assert_eq!(supplied.available().unwrap(), 1);
    assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
}

#[test]
fn ended_client_cannot_spawn_helper() {
    let admission = admission();
    let request = admission.begin(Client::new(1).unwrap()).unwrap();
    let client = request.client();
    drop(request);
    assert!(client.into_helper_thread(|_| panic!("unexpected callback")).is_err());
}

#[test]
fn pending_helper_request_retires_without_releasing_base_token_early() {
    let admission = admission();
    let supplied = Client::new(1).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let helper = request.client().into_helper_thread(|_| panic!("no token available")).unwrap();
    helper.request_token().unwrap();
    let retired = helper.retire();
    await_retirement(&retired);
    assert_eq!(supplied.available().unwrap(), 0);
    request.finish().unwrap();
    assert_eq!(supplied.available().unwrap(), 1);
}

// Unix helper shutdown may detach a worker that is still executing user code. Windows instead
// joins it, so this deliberate hold-across-retire scenario is Unix-specific.
#[cfg(unix)]
#[test]
fn detached_callback_retains_request_ownership() {
    let admission = admission();
    let supplied = Client::new(2).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let helper = request
        .client()
        .into_helper_thread(move |token| {
            let _token = token.unwrap();
            entered_tx.send(()).unwrap();
            resume_rx.recv_timeout(TIMEOUT).unwrap();
        })
        .unwrap();
    helper.request_token().unwrap();
    entered_rx.recv_timeout(TIMEOUT).unwrap();
    let retired = helper.retire();
    assert!(!retired.is_retired(), "blocked callback must not report retirement");
    assert!(!retired.wait_timeout(Duration::ZERO));
    assert!(!retired.wait_timeout(Duration::from_millis(5)));
    assert!(matches!(request.finish(), Err(BeginError::OutstandingHandles)));
    assert_eq!(supplied.available().unwrap(), 0);
    resume_tx.send(()).unwrap();
    await_retirement(&retired);
    assert_eq!(supplied.available().unwrap(), 2);
    assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
}

#[test]
fn callback_panic_releases_its_token_and_worker_ownership() {
    let admission = admission();
    let supplied = Client::new(2).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let helper = request
        .client()
        .into_helper_thread(move |token| {
            let _token = token.unwrap();
            entered_tx.send(()).unwrap();
            panic!("intentional helper callback failure");
        })
        .unwrap();
    helper.request_token().unwrap();
    entered_rx.recv_timeout(TIMEOUT).unwrap();
    let retired = helper.retire();
    await_retirement(&retired);
    request.finish().unwrap();
    assert_eq!(supplied.available().unwrap(), 2);
    admission.begin(supplied).unwrap().finish().unwrap();
}

#[test]
fn callback_destructor_precedes_retirement_and_ownership_release() {
    struct DropProbe {
        owner: std::sync::Weak<RequestInner>,
        retired: Arc<RetirementState>,
        observed: Arc<AtomicBool>,
    }
    impl Drop for DropProbe {
        fn drop(&mut self) {
            let owner = self.owner.upgrade().unwrap();
            // Lease + callback owner + this upgraded observation. A lease alone is insufficient.
            assert_eq!(Arc::strong_count(&owner), 3);
            assert!(!self.retired.is_retired());
            self.observed.store(true, Ordering::Release);
        }
    }
    impl DropProbe {
        fn touch(&self) {}
    }
    let admission = admission();
    let request = admission.begin(Client::new(1).unwrap()).unwrap();
    let owner = Arc::clone(request.inner.as_ref().unwrap());
    let retired = Arc::new(RetirementState::default());
    let observed = Arc::new(AtomicBool::new(false));
    let probe = DropProbe {
        owner: Arc::downgrade(&owner),
        retired: Arc::clone(&retired),
        observed: Arc::clone(&observed),
    };
    // Exercise exactly the callback object captured by the native worker, including field order.
    let callback = TrackedCallback {
        callback: move |_: io::Result<RequestToken>| probe.touch(),
        owner,
        _completion: RetirementNotifier(Arc::clone(&retired)),
    };
    drop(callback);
    assert!(observed.load(Ordering::Acquire));
    assert!(retired.is_retired());
    request.finish().unwrap();
}

#[test]
fn late_callback_releases_raw_token_without_entering_user_code() {
    let admission = admission();
    let supplied = Client::new(2).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let retired = Arc::new(RetirementState::default());
    let mut callback = TrackedCallback {
        callback: |_: io::Result<RequestToken>| panic!("ended request entered callback"),
        owner: Arc::clone(request.inner.as_ref().unwrap()),
        _completion: RetirementNotifier(Arc::clone(&retired)),
    };
    let token = supplied.acquire().unwrap();
    assert!(matches!(request.finish(), Err(BeginError::OutstandingHandles)));
    callback.deliver(Ok(token));
    assert_eq!(supplied.available().unwrap(), 1);
    assert!(!retired.is_retired());
    drop(callback);
    assert!(retired.is_retired());
    assert_eq!(supplied.available().unwrap(), 2);
}
