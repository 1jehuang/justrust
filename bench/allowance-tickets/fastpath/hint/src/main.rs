use std::sync::{
    atomic::{AtomicUsize, Ordering::SeqCst},
    mpsc, Arc, Barrier,
};
use std::time::{Duration, Instant};

#[allow(dead_code, unused_imports)]
mod native {
    #[cfg(not(any(feature = "fast", feature = "hint")))]
    include!("../archive/original/jobserver.rs");
    #[cfg(all(feature = "fast", not(feature = "hint")))]
    include!("../archive/fast/jobserver.rs");
    #[cfg(feature = "hint")]
    include!("../archive/hint/jobserver.rs");
    pub type BenchProxy = TicketedProxy;
    pub fn new_proxy() -> Arc<BenchProxy> {
        let capacity = std::env::var("ALLOWANCE_CAPTURE_CAPACITY")
            .expect("candidate requires explicit capture capacity")
            .parse()
            .unwrap();
        TicketedProxy::new_with_event_capacity(capacity)
    }
    pub fn pending(proxy: &BenchProxy) -> usize {
        proxy.debug_snapshot().pending
    }
    pub fn used(proxy: &BenchProxy) -> usize {
        proxy.debug_snapshot().used
    }
    pub fn await_surplus(proxy: &BenchProxy) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while proxy.debug_snapshot().inventory.callbacks_completed < 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "surplus callback not acknowledged"
            );
            std::thread::yield_now();
        }
        let snapshot = proxy.debug_snapshot();
        assert_eq!(snapshot.inventory.supplies, 1);
        assert_eq!(snapshot.inventory.helper_errors, 0);
    }
    pub fn evidence(proxy: &BenchProxy, completed: usize) {
        let s = proxy.debug_snapshot();
        let i = &s.inventory;
        assert_eq!((s.used, s.pending), (0, 0));
        assert_eq!(
            (i.requests, i.grants, i.acks),
            (completed as u64, completed as u64, completed as u64)
        );
        assert_eq!(
            i.releases,
            completed as u64 + 1,
            "initial main release plus worker releases"
        );
        assert_eq!(i.helper_errors, 0);
        assert!(!i.overflow);
        let capacity: usize = std::env::var("ALLOWANCE_CAPTURE_CAPACITY")
            .unwrap()
            .parse()
            .unwrap();
        let capture = if capacity == 0 { "disabled" } else { "enabled" };
        let variant = if cfg!(feature = "hint") {
            "hint"
        } else if cfg!(feature = "fast") {
            "fast"
        } else {
            "original"
        };
        println!("{{\"variant\":\"{variant}\",\"capture\":\"{capture}\",\"capture_capacity\":{capacity},\"completed_pairs\":{completed},\"used\":{},\"pending\":{},\"requests\":{},\"grants\":{},\"acks\":{},\"releases\":{},\"supplies\":{},\"callbacks_completed\":{},\"helper_errors\":{},\"lost_events\":{},\"overflow\":{},\"events_count\":{},\"events_debug\":{:?}}}", s.used, s.pending, i.requests, i.grants, i.acks, i.releases, i.supplies, i.callbacks_completed, i.helper_errors, i.lost_events, i.overflow, s.events.len(), if completed <= 2 { format!("{:?}", s.events) } else { String::new() });
    }
}

fn wait_pending(proxy: &native::BenchProxy) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while native::pending(proxy) == 0 {
        assert!(Instant::now() < deadline, "waiter never registered");
        std::thread::yield_now();
    }
}

fn contention(capacity: usize, operations: usize) {
    native::initialize(capacity, |msg| panic!("{msg}"));
    let proxy = native::new_proxy();
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
    native::evidence(&proxy, 8 * operations);
}

fn immediate(operations: usize) {
    native::initialize(1, |msg| panic!("{msg}"));
    let proxy = native::new_proxy();
    proxy.release_thread();
    let now = Instant::now();
    for _ in 0..operations {
        proxy.acquire_thread();
        proxy.release_thread();
    }
    let elapsed_ns = now.elapsed().as_nanos();
    assert_eq!(native::used(&proxy), 0);
    assert_eq!(
        native::client().available().unwrap(),
        0,
        "last token must remain held"
    );
    println!(
        "{{\"scenario\":\"immediate\",\"operations\":{operations},\"elapsed_ns\":{}}}",
        elapsed_ns
    );
    native::evidence(&proxy, operations);
}

fn surplus() {
    native::initialize(2, |msg| panic!("{msg}"));
    let proxy = native::new_proxy(); // main holds retained entitlement
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
    native::await_surplus(&proxy);
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
    native::evidence(&proxy, 2);
}

fn forced_pending(capacity: usize, rounds: usize) {
    native::initialize(capacity, |msg| panic!("{msg}"));
    let proxy = native::new_proxy();
    proxy.release_thread();
    let phase = Arc::new(Barrier::new(9));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let (granted_tx, granted_rx) = mpsc::channel();
    let mut release_txs = Vec::new();
    let mut joins = Vec::new();
    for _ in 0..8 {
        let (release_tx, release_rx) = mpsc::channel();
        release_txs.push(release_tx);
        let (proxy, phase, active, peak, completed, granted_tx) = (
            proxy.clone(),
            phase.clone(),
            active.clone(),
            peak.clone(),
            completed.clone(),
            granted_tx.clone(),
        );
        joins.push(std::thread::spawn(move || {
            for _ in 0..rounds {
                phase.wait();
                proxy.acquire_thread();
                let n = active.fetch_add(1, SeqCst) + 1;
                peak.fetch_max(n, SeqCst);
                assert!(n <= capacity, "external active exceeds real capacity");
                granted_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                active.fetch_sub(1, SeqCst);
                completed.fetch_add(1, SeqCst);
                proxy.release_thread();
                phase.wait();
            }
        }));
    }
    let now = Instant::now();
    for _ in 0..rounds {
        phase.wait();
        // Hold externally acknowledged active grants until every other worker
        // registered Pending. Queue state is setup only, never a grant oracle.
        for _ in 0..capacity {
            granted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while native::pending(&proxy) != 8 - capacity {
            assert!(Instant::now() < deadline, "forced pending setup timed out");
            std::thread::yield_now();
        }
        assert_eq!(active.load(SeqCst), capacity);
        for tx in &release_txs {
            tx.send(()).unwrap();
        }
        phase.wait();
        for _ in capacity..8 {
            granted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(active.load(SeqCst), 0);
    }
    for join in joins {
        join.join().unwrap();
    }
    let elapsed_ns = now.elapsed().as_nanos();
    assert_eq!(completed.load(SeqCst), 8 * rounds);
    assert_eq!(peak.load(SeqCst), capacity);
    assert_eq!(native::used(&proxy), 0);
    println!("{{\"scenario\":\"forced-pending\",\"capacity\":{capacity},\"threads\":8,\"operations\":{},\"pending_episodes\":{rounds},\"minimum_pending_each_episode\":{},\"peak_active\":{},\"elapsed_ns\":{elapsed_ns}}}", completed.load(SeqCst), 8 - capacity, peak.load(SeqCst));
    native::evidence(&proxy, 8 * rounds);
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
        "pending-one" => forced_pending(1, operations),
        "pending-multi" => forced_pending(4, operations),
        "surplus" => surplus(),
        _ => {
            eprintln!("usage: allowance-tickets-baseline one|multi|immediate|surplus [operations]");
            std::process::exit(2);
        }
    }
}
