use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn admission() -> &'static Admission {
    Box::leak(Box::new(Admission::new()))
}

fn available(client: &Client) -> usize {
    client.available().unwrap()
}

#[test]
fn admission_is_reserved_while_acquisition_waits() {
    let admission = admission();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        admission
            .begin_with(Client::new(1).unwrap(), |client| {
                entered_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                client.acquire()
            })
            .unwrap()
    });
    entered_rx.recv().unwrap();
    assert!(matches!(
        admission.begin(Client::new(1).unwrap()),
        Err(BeginError::OverlappingRequest)
    ));
    resume_tx.send(()).unwrap();
    worker.join().unwrap().finish().unwrap();
    admission.begin(Client::new(1).unwrap()).unwrap().finish().unwrap();
}

#[test]
fn acquisition_panic_restores_admission_without_poisoning_mutex() {
    let admission = admission();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _ = admission.begin_with(Client::new(1).unwrap(), |_| panic!("acquire panic"));
        }))
        .is_err()
    );
    admission.begin(Client::new(1).unwrap()).unwrap().finish().unwrap();
}

#[test]
fn sequential_requests_use_distinct_supplied_clients() {
    let admission = admission();
    let first = Client::new(2).unwrap();
    let second = Client::new(4).unwrap();
    {
        let request = admission.begin(first.clone()).unwrap();
        assert_eq!(available(&first), 1);
        assert_eq!(available(&second), 4);
        let client = request.client();
        let token = client.acquire().unwrap();
        assert_eq!(available(&first), 0);
        drop(token);
        drop(client);
        request.finish().unwrap();
    }
    assert_eq!(available(&first), 2);
    let request = admission.begin(second.clone()).unwrap();
    assert_eq!(available(&second), 3);
    assert_eq!(available(&first), 2);
    request.finish().unwrap();
    assert_eq!(available(&second), 4);
}

#[test]
fn overlapping_requests_rejected_without_consuming_tokens() {
    let admission = admission();
    let first = admission.begin(Client::new(1).unwrap()).unwrap();
    let second = Client::new(1).unwrap();
    assert!(matches!(admission.begin(second.clone()), Err(BeginError::OverlappingRequest)));
    assert_eq!(available(&second), 1);
    drop(first);
    admission.begin(second).unwrap().finish().unwrap();
}

#[test]
fn failed_acquisition_does_not_claim_admission() {
    let admission = admission();
    let result = admission.begin_with(Client::new(0).unwrap(), |_| {
        Err(io::Error::other("injected acquisition failure"))
    });
    assert!(matches!(result, Err(BeginError::Acquire(_))));
    admission.begin(Client::new(1).unwrap()).unwrap().finish().unwrap();
}

#[test]
fn error_cleanup_returns_implicit_token_and_admission() {
    let admission = admission();
    let supplied = Client::new(1).unwrap();
    let result: Result<(), ()> = (|| {
        let _request = admission.begin(supplied.clone()).unwrap();
        Err(())
    })();
    assert!(result.is_err());
    assert_eq!(available(&supplied), 1);
    admission.begin(supplied).unwrap().finish().unwrap();
}

#[test]
fn panic_cleanup_returns_implicit_token_and_admission() {
    let admission = admission();
    let supplied = Client::new(1).unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let request = admission.begin(supplied.clone()).unwrap();
            let _client = request.client();
            panic!("request failed");
        }))
        .is_err()
    );
    assert_eq!(available(&supplied), 1);
    admission.begin(supplied).unwrap().finish().unwrap();
}

#[test]
fn outstanding_client_closes_admission_and_keeps_implicit_token() {
    let admission = admission();
    let supplied = Client::new(1).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let escaped = request.client();
    assert!(matches!(request.finish(), Err(BeginError::OutstandingHandles)));
    assert!(escaped.try_acquire().is_err());
    assert!(escaped.acquire().is_err());
    assert_eq!(available(&supplied), 0);
    assert!(matches!(
        admission.begin(Client::new(1).unwrap()),
        Err(BeginError::OutstandingHandles)
    ));
    drop(escaped);
    assert_eq!(available(&supplied), 1);
    assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
}

#[test]
fn outstanding_token_alone_closes_admission() {
    let admission = admission();
    let supplied = Client::new(2).unwrap();
    let request = admission.begin(supplied.clone()).unwrap();
    let token = request.client().acquire().unwrap();
    drop(request);
    assert_eq!(available(&supplied), 0);
    drop(token);
    assert_eq!(available(&supplied), 2);
    assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
}

#[test]
fn panic_with_escaped_handle_fails_closed() {
    let admission = admission();
    let mut escaped = None;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let request = admission.begin(Client::new(1).unwrap()).unwrap();
            escaped = Some(request.client());
            panic!("escaped request failed");
        }))
        .is_err()
    );
    assert!(escaped.unwrap().try_acquire().is_err());
    assert!(matches!(
        admission.begin(Client::new(1).unwrap()),
        Err(BeginError::OutstandingHandles)
    ));
}

#[test]
fn concurrent_admission_is_rejected() {
    let admission = admission();
    let request = admission.begin(Client::new(1).unwrap()).unwrap();
    std::thread::spawn(move || {
        assert!(matches!(
            admission.begin(Client::new(1).unwrap()),
            Err(BeginError::OverlappingRequest)
        ));
    })
    .join()
    .unwrap();
    request.finish().unwrap();
}

#[test]
fn resident_requests_do_not_initialize_or_replace_ordinary_client() {
    let before = super::super::CLIENT.get().map(|client| client as *const Client);
    let request = RequestLease::begin(Client::new(1).unwrap()).unwrap();
    request.finish().unwrap();
    let after = super::super::CLIENT.get().map(|client| client as *const Client);
    assert_eq!(before, after);
}

#[test]
fn ordinary_initialization_is_isolated_in_fresh_process() {
    const CHILD: &str = "JR_JOBSERVER_RESIDENT_ORDINARY_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "jobserver::resident::tests::ordinary_initialization_is_isolated_in_fresh_process",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("CARGO_MAKEFLAGS")
            .env_remove("MAKEFLAGS")
            .env_remove("MFLAGS")
            .output()
            .unwrap();
        assert!(output.status.success(), "child failed: {output:?}");
        return;
    }
    assert!(super::super::CLIENT.get().is_none());
    RequestLease::begin(Client::new(1).unwrap()).unwrap().finish().unwrap();
    assert!(super::super::CLIENT.get().is_none());
    super::super::initialize(3, |error| panic!("unexpected ordinary init error: {error}"));
    let ordinary = super::super::client();
    assert_eq!(available(&ordinary), 2);
    let supplied = Client::new(4).unwrap();
    let request = RequestLease::begin(supplied.clone()).unwrap();
    assert_eq!(available(&supplied), 3);
    assert_eq!(available(&super::super::client()), 2);
    super::super::initialize(99, |_| panic!("ordinary initialization must remain once-only"));
    request.finish().unwrap();
    assert_eq!(available(&ordinary), 2);
    assert_eq!(available(&supplied), 4);
}
