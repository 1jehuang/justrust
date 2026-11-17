use super::*;
use std::sync::mpsc;
use std::time::Duration;

fn isolated(name: &str) -> bool {
    const ENV: &str = "JR_COMPILER_CLIENT_ISOLATED_TEST";
    if std::env::var(ENV).as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("jobserver::compiler_tests::{name}"), "--nocapture"])
        .env(ENV, name)
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .output()
        .unwrap();
    assert!(output.status.success(), "isolated compiler client test failed: {output:?}");
    true
}

#[test]
fn delegated_serial_slot_has_no_parallel_capacity_and_can_be_reused() {
    if isolated("delegated_serial_slot_has_no_parallel_capacity_and_can_be_reused") {
        return;
    }
    initialize(2, |error| panic!("unexpected ordinary initialization: {error}"));
    for _ in 0..2 {
        let request = resident::RequestLease::begin_serial().unwrap();
        request
            .with_current(|| {
                assert!(compiler_client().is_resident());
                let (send, receive) = mpsc::channel();
                let helper = compiler_client()
                    .into_helper_thread(move |token| {
                        let _ = send.send(token);
                    })
                    .unwrap();
                helper.request_token();
                assert!(matches!(
                    receive.recv_timeout(Duration::from_millis(30)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
                drop(helper);
            })
            .unwrap();
        request.finish().unwrap();
        assert!(!compiler_client().is_resident());
        assert_eq!(client().available().unwrap(), 1);
    }
}

#[test]
fn ordinary_codegen_helper_returns_actual_tokens() {
    let supplied = Client::new(1).unwrap();
    let (send, receive) = mpsc::channel();
    let helper = CompilerClient::Ordinary(supplied.clone())
        .into_helper_thread(move |token| send.send(token).unwrap())
        .unwrap();
    helper.request_token();
    let token = receive.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    assert_eq!(supplied.available().unwrap(), 0);
    drop(token);
    drop(helper);
    assert_eq!(supplied.available().unwrap(), 1);
}

#[test]
fn codegen_uses_current_request_and_retires_helper_before_completion() {
    if isolated("codegen_uses_current_request_and_retires_helper_before_completion") {
        return;
    }
    initialize(3, |error| panic!("unexpected ordinary initialization: {error}"));
    let supplied = Client::new(2).unwrap();
    let request = resident::RequestLease::begin(supplied.clone()).unwrap();
    request
        .with_current(|| {
            assert!(compiler_client().is_resident());
            assert!(std::thread::spawn(|| compiler_client().is_resident()).join().unwrap());
            let (send, receive) = mpsc::channel();
            let helper = compiler_client()
                .into_helper_thread(move |token| send.send(token).unwrap())
                .unwrap();
            helper.request_token();
            let token = receive.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            assert_eq!(supplied.available().unwrap(), 0);
            assert_eq!(client().available().unwrap(), 2);
            drop(token);
            drop(helper);
        })
        .unwrap();
    assert!(!compiler_client().is_resident());
    request.finish().unwrap();
    assert_eq!(supplied.available().unwrap(), 2);
    assert_eq!(client().available().unwrap(), 2);
}

#[test]
fn queued_codegen_token_keeps_request_owned_after_helper_shutdown() {
    if isolated("queued_codegen_token_keeps_request_owned_after_helper_shutdown") {
        return;
    }
    let supplied = Client::new(2).unwrap();
    let request = resident::RequestLease::begin(supplied.clone()).unwrap();
    let token = request
        .with_current(|| {
            let (send, receive) = mpsc::channel();
            let helper = compiler_client()
                .into_helper_thread(move |token| send.send(token).unwrap())
                .unwrap();
            helper.request_token();
            let token = receive.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
            drop(helper);
            token
        })
        .unwrap();
    assert!(matches!(request.finish(), Err(resident::BeginError::OutstandingHandles)));
    assert_eq!(supplied.available().unwrap(), 0);
    drop(token);
    assert_eq!(supplied.available().unwrap(), 2);
    assert!(matches!(
        resident::RequestLease::begin(supplied),
        Err(resident::BeginError::OutstandingHandles)
    ));
}

#[test]
fn escaped_codegen_client_cannot_acquire_after_request_completion() {
    if isolated("escaped_codegen_client_cannot_acquire_after_request_completion") {
        return;
    }
    let supplied = Client::new(2).unwrap();
    let request = resident::RequestLease::begin(supplied.clone()).unwrap();
    let escaped = request.with_current(compiler_client).unwrap();
    assert!(escaped.is_resident());
    assert!(matches!(request.finish(), Err(resident::BeginError::OutstandingHandles)));
    assert!(escaped.try_acquire().is_err());
    assert_eq!(supplied.available().unwrap(), 1);
    drop(escaped);
    assert_eq!(supplied.available().unwrap(), 2);
}

#[test]
fn unwinding_codegen_scope_restores_ordinary_client_without_poisoning() {
    if isolated("unwinding_codegen_scope_restores_ordinary_client_without_poisoning") {
        return;
    }
    initialize(2, |error| panic!("unexpected ordinary initialization: {error}"));
    let supplied = Client::new(1).unwrap();
    let request = resident::RequestLease::begin(supplied.clone()).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = request.with_current(|| {
                assert!(compiler_client().is_resident());
                panic!("deliberate codegen-scope failure");
            });
        }))
        .is_err()
    );
    assert!(!compiler_client().is_resident());
    request.finish().unwrap();
    assert_eq!(supplied.available().unwrap(), 1);
    assert_eq!(client().available().unwrap(), 1);
}
