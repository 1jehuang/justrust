use std::sync::{
    atomic::{AtomicUsize, Ordering::SeqCst},
    mpsc, Arc, Barrier,
};
use std::time::{Duration, Instant};

#[allow(unused_imports)]
mod native {
    include!("../archive/baseline-jobserver.rs");
    // Read-only synchronization probe, not a grant/dequeue or capacity oracle.
    pub fn pending(proxy: &Proxy) -> u16 {
        proxy.data.lock().pending
    }
    pub fn used(proxy: &Proxy) -> u16 {
        proxy.data.lock().used
    }
}

fn wait_pending(proxy: &native::Proxy) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while native::pending(proxy) == 0 {
        assert!(Instant::now() < deadline, "waiter never registered");
        std::thread::yield_now();
    }
}

fn contention(capacity: usize, operations: usize) {
    native::initialize(capacity, |msg| panic!("{msg}"));
    let proxy = native::Proxy::new();
    // new() accounts for main as used=1. Main is idle before workers start.
    proxy.release_thread();
    let start = Arc::new(Barrier::new(9));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let mut joins = Vec::new();
    for _ in 0..8 {
        let (proxy, start, active, peak, completed) = (
            proxy.clone(),
            start.clone(),
            active.clone(),
            peak.clone(),
            completed.clone(),
        );
        joins.push(std::thread::spawn(move || {
            start.wait();
            for _ in 0..operations {
                proxy.acquire_thread();
                let n = active.fetch_add(1, SeqCst) + 1;
                peak.fetch_max(n, SeqCst);
                assert!(
                    n <= capacity,
                    "external active {n} exceeds capacity {capacity}"
                );
                std::hint::black_box(n.wrapping_mul(17));
                active.fetch_sub(1, SeqCst);
                completed.fetch_add(1, SeqCst);
                proxy.release_thread();
            }
        }));
    }
    let now = Instant::now();
    start.wait();
    for join in joins {
        join.join().unwrap();
    }
    let ns = now.elapsed().as_nanos();
    assert_eq!(active.load(SeqCst), 0);
    assert_eq!(completed.load(SeqCst), 8 * operations);
    assert_eq!(native::used(&proxy), 0);
    println!("{{\"scenario\":\"contention\",\"capacity\":{capacity},\"threads\":8,\"operations\":{},\"peak_active\":{},\"elapsed_ns\":{ns}}}", completed.load(SeqCst), peak.load(SeqCst));
}

fn immediate(operations: usize) {
    native::initialize(1, |msg| panic!("{msg}"));
    let proxy = native::Proxy::new();
    proxy.release_thread();
    let now = Instant::now();
    for _ in 0..operations {
        proxy.acquire_thread();
        proxy.release_thread();
    }
    assert_eq!(native::used(&proxy), 0);
    assert_eq!(
        native::client().available().unwrap(),
        0,
        "last token must remain held"
    );
    println!(
        "{{\"scenario\":\"immediate\",\"operations\":{operations},\"elapsed_ns\":{}}}",
        now.elapsed().as_nanos()
    );
}

fn surplus() {
    native::initialize(2, |msg| panic!("{msg}"));
    let proxy = native::Proxy::new(); // main holds retained entitlement
    let external = native::client().acquire().unwrap(); // exhaust real capacity
    let (granted_tx, granted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_proxy = proxy.clone();
    let join = std::thread::spawn(move || {
        worker_proxy.acquire_thread();
        granted_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker_proxy.release_thread();
    });
    wait_pending(&proxy);
    proxy.release_thread(); // transfer main's entitlement, never release last real token
    granted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(native::client().available().unwrap(), 0);
    release_tx.send(()).unwrap();
    join.join().unwrap();
    assert_eq!(native::used(&proxy), 0);
    drop(external); // outstanding helper request can now acquire surplus
                    // Availability is a snapshot, not proof the helper callback completed.
                    // The immutable API exposes no callback completion acknowledgment.
    let deadline = Instant::now() + Duration::from_secs(5);
    while native::client().available().unwrap() != 1 {
        assert!(Instant::now() < deadline, "helper failed to return surplus");
        std::thread::yield_now();
    }
    proxy.acquire_thread();
    proxy.release_thread();
    assert_eq!(native::used(&proxy), 0);
    println!("{{\"scenario\":\"surplus\",\"operations\":2,\"available_after\":1}}");
}

fn main() {
    // Covers blocked native acquires and joins, including panic paths.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        eprintln!("fixture watchdog expired");
        std::process::exit(124);
    });
    let args: Vec<_> = std::env::args().collect();
    let operations = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(2000);
    match args.get(1).map(String::as_str).unwrap_or("help") {
        "one" => contention(1, operations),
        "multi" => contention(4, operations),
        "immediate" => immediate(operations),
        "surplus" => surplus(),
        _ => {
            eprintln!("usage: allowance-tickets-baseline one|multi|immediate|surplus [operations]");
            std::process::exit(2);
        }
    }
}
