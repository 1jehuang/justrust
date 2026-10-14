//! Focused benchmark of the actual query_reads module, with no substituted recorder.
#![allow(dead_code)]
#[cfg_attr(feature = "baseline", path = "baseline-query_reads.rs")]
#[cfg_attr(
    not(feature = "baseline"),
    path = "../../toolchain/rustc/compiler/rustc_session/src/query_reads.rs"
)]
mod query_reads;

use query_reads::{DepReadOutcome, QueryReadObserver, RequestMode, RequestOutcome};
use std::sync::Barrier;
use std::time::Instant;

const CAP: u64 = 100_000;

fn field(json: &str, key: &str) -> u64 {
    let suffix = json.split_once(&format!("\"{key}\":")).unwrap().1;
    suffix.split([',', '}']).next().unwrap().parse().unwrap()
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let case = args.get(1).map(String::as_str).unwrap_or("mixed");
    let threads: u64 = args.get(2).unwrap().parse().unwrap();
    let events: u64 = args.get(3).unwrap().parse().unwrap();
    assert!(threads > 0);
    let accepted = case == "accepted";
    let mixed = case != "read";
    let events_per_iteration = if mixed { 5 } else { 1 };
    assert_eq!(events % (threads * events_per_iteration), 0);
    if accepted {
        assert!(events <= CAP);
    }
    let observer = QueryReadObserver::default();
    if !accepted {
        for _ in 0..CAP {
            observer.result_produced(1, 1, 42);
        }
    }
    let iterations = events / threads / events_per_iteration;
    let barrier = Barrier::new(threads as usize + 1);
    let elapsed = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..threads {
            let observer = &observer;
            let barrier = &barrier;
            handles.push(scope.spawn(move || {
                barrier.wait();
                for _ in 0..iterations {
                    if mixed {
                        let request = observer.begin(Some(1), Some(2), RequestMode::Get);
                        request.cache_hit(42);
                        observer.read(
                            Some(1),
                            Some(2),
                            Some(request.id()),
                            42,
                            DepReadOutcome::TrackedNew,
                        );
                        observer.result_produced(1, 2, 42);
                        request.finish(RequestOutcome::Value);
                    } else {
                        observer.read(None, None, None, 42, DepReadOutcome::Disabled);
                    }
                }
            }));
        }
        let start = Instant::now();
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        start.elapsed()
    });
    let mut json = String::new();
    observer.append_json(&mut json);
    let lost = field(&json, "lost_events");
    let expected_iterations = iterations * threads;
    assert_eq!(lost, if accepted { 0 } else { events });
    assert_eq!(
        field(&json, "expected_requests"),
        if mixed { expected_iterations } else { 0 }
    );
    assert_eq!(field(&json, "expected_reads"), expected_iterations);
    assert_eq!(
        field(&json, "expected_results"),
        if accepted { 0 } else { CAP } + if mixed { expected_iterations } else { 0 }
    );
    assert_eq!(
        json.matches("{\"kind\":").count() as u64,
        if accepted { events } else { CAP }
    );
    assert!(json.contains(if accepted {
        "\"records_complete\":true"
    } else {
        "\"records_complete\":false"
    }));
    println!(
        "{{\"case\":\"{case}\",\"threads\":{threads},\"events\":{events},\"seconds\":{},\"ns_per_event\":{},\"lost_events\":{lost},\"checked\":true}}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1e9 / events as f64
    );
}
