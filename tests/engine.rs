use justrust_engine::fixtures::{FixtureProvider, Shape};
use justrust_engine::{Context, Engine, Error, Key, Provider, TaskFuture};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

struct Cases;

fn key(kind: u32) -> Key {
    Key::new(kind, vec![])
}

impl Provider for Cases {
    fn outputs(&self, k: &Key) -> usize {
        match k.kind {
            10 | 12 | 19 => 2,
            _ => 1,
        }
    }

    fn start(&self, cx: Context, k: Key) -> TaskFuture {
        Box::pin(async move {
            match k.kind {
                0 => cx.publish(0, k.params.first().copied().unwrap_or(1)),
                1 => {
                    let a = cx.request(key(2));
                    let b = cx.request(key(3));
                    let x = cx.read(a.get(0)).await?;
                    let y = cx.read(b.get(0)).await?;
                    cx.publish(0, x + y)
                }
                2 | 3 => {
                    let shared = cx.request(key(4));
                    let value = cx.read(shared.get(0)).await?;
                    cx.publish(0, value + k.kind as u64)
                }
                4 => cx.publish(0, 10),
                10 => {
                    // Body depends on a consumer of our signature. Waiting for
                    // whole-producer completion here would cause a cycle.
                    cx.publish(0, 7)?;
                    let branch = cx.clone();
                    cx.fork(Box::pin(async move {
                        let release = branch.request(key(11));
                        assert_eq!(branch.read(release.get(0)).await?, 7);
                        branch.publish(1, 11)
                    }))?;
                    Ok(())
                }
                11 => {
                    let signature = cx.request(key(10));
                    let value = cx.read(signature.get(0)).await?;
                    cx.publish(0, value)
                }
                12 => {
                    cx.publish(0, 7)?;
                    Err(Error::new("body invalid"))
                }
                13 => {
                    let inputs = cx.request(key(12));
                    let artifact = cx.read(inputs.get(0)).await?;
                    let _mandatory_check = cx.read(inputs.get(1)).await?;
                    cx.publish(0, artifact)
                }
                14 => {
                    cx.publish(0, 1)?;
                    assert!(cx.publish(0, 2).is_err());
                    Ok(())
                }
                15 => Ok(()), // Missing output must fail, not succeed or hang.
                16 => panic!("injected provider fault"),
                17 | 18 => {
                    let other = cx.request(key(if k.kind == 17 { 18 } else { 17 }));
                    let value = cx.read(other.get(0)).await?;
                    cx.publish(0, value)
                }
                19 => {
                    let branch = cx.clone();
                    cx.fork(Box::pin(async move { branch.publish(1, 22) }))?;
                    cx.publish(0, 11)
                }
                20 => {
                    let mut remaining = 20;
                    let polling = Arc::new(AtomicBool::new(false));
                    std::future::poll_fn(move |w| {
                        assert!(!polling.swap(true, Ordering::SeqCst), "concurrent poll");
                        for _ in 0..8 {
                            w.waker().wake_by_ref();
                        }
                        let result = if remaining == 0 {
                            Poll::Ready(())
                        } else {
                            remaining -= 1;
                            Poll::Pending
                        };
                        polling.store(false, Ordering::SeqCst);
                        result
                    })
                    .await;
                    cx.publish(0, 99)
                }
                21 => std::future::pending::<Result<(), Error>>().await,
                _ => Err(Error::new("unknown test kind")),
            }
        })
    }
}

#[test]
fn diamond_shared_once_and_output_edges_retained() {
    let engine = Engine::new(Cases);
    let root = engine.request(key(1));
    let report = engine.run(4).unwrap();
    assert_eq!(root.result(0).unwrap(), 25);
    assert_eq!(report.nodes, 4);
    assert_eq!(report.publications, 4);
    assert_eq!(report.edges, 4);
    assert_eq!(report.retained_outputs, 4);
    assert!(report.intern_hits >= 1);
    assert!(report.node_evaluations.iter().all(|(_, n)| *n == 1));
    assert!(report.capture_complete);
}

#[test]
fn equivalent_concurrent_requests_join_one_evaluation() {
    let engine = Engine::new(Cases);
    let demands = std::thread::scope(|scope| {
        (0..16)
            .map(|_| scope.spawn(|| engine.request(key(1))))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let id = demands[0].outputs().node_id();
    assert!(demands.iter().all(|d| d.outputs().node_id() == id));
    let report = engine.run(8).unwrap();
    assert_eq!(report.nodes, 4);
    for demand in demands {
        assert_eq!(demand.result(0).unwrap(), 25);
    }
}

#[test]
fn collisions_compare_full_keys_and_versions() {
    let engine = Engine::with_hash_bits(Cases, 0);
    let a = engine.request(Key::new(0, vec![8]));
    let again = engine.request(Key::new(0, vec![8]));
    let b = engine.request(Key::new(0, vec![9]));
    let mut versioned = Key::new(0, vec![8]);
    versioned.version = 2;
    let c = engine.request(versioned);
    assert_eq!(a.outputs().node_id(), again.outputs().node_id());
    assert_ne!(a.outputs().node_id(), b.outputs().node_id());
    assert_ne!(a.outputs().node_id(), c.outputs().node_id());
    let report = engine.run(4).unwrap();
    assert_eq!(report.nodes, 3);
    assert_eq!(a.result(0).unwrap(), 8);
    assert_eq!(b.result(0).unwrap(), 9);
    assert_eq!(c.result(0).unwrap(), 8);
}

#[test]
fn early_output_unblocks_consumer_before_body_finishes() {
    for workers in [1, 2, 8, 96] {
        let engine = Engine::new(Cases);
        let root = engine.request(key(10));
        let report = engine.run(workers).unwrap();
        assert_eq!(root.result(0).unwrap(), 7);
        assert_eq!(root.result(1).unwrap(), 11);
        assert_eq!(report.nodes, 2);
        assert_eq!(report.tasks, 3);
        assert!(report.capture_complete);
    }
}

#[test]
fn published_output_survives_failure_but_required_checks_do_not() {
    let engine = Engine::new(Cases);
    let partial = engine.request(key(12));
    let checked = engine.request(key(13));
    let report = engine.run(4).unwrap();
    assert_eq!(partial.result(0).unwrap(), 7);
    assert!(
        partial
            .result(1)
            .unwrap_err()
            .to_string()
            .contains("body invalid")
    );
    assert!(checked.result(0).is_err());
    assert!(report.failures >= 1);
}

#[test]
fn duplicate_publication_cannot_mutate_value() {
    let engine = Engine::new(Cases);
    let root = engine.request(key(14));
    engine.run(2).unwrap();
    assert_eq!(root.result(0).unwrap(), 1);
}

#[test]
fn missing_output_and_provider_panic_are_terminal() {
    for kind in [15, 16] {
        let engine = Engine::new(Cases);
        let root = engine.request(key(kind));
        let report = engine.run(4).unwrap();
        assert!(root.result(0).is_err());
        assert!(report.failures > 0);
    }
}

#[test]
fn cancellation_detaches_only_its_demand() {
    let engine = Engine::new(Cases);
    let a = engine.request(key(1));
    let b = engine.request(key(1));
    a.cancel();
    a.cancel();
    let report = engine.run(4).unwrap();
    assert!(a.result(0).is_err());
    assert_eq!(b.result(0).unwrap(), 25);
    assert_eq!(report.nodes, 4);
}

#[test]
fn independent_forks_finish_all_required_outputs() {
    let engine = Engine::new(Cases);
    let root = engine.request(key(19));
    let report = engine.run(4).unwrap();
    assert_eq!(root.result(0).unwrap(), 11);
    assert_eq!(root.result(1).unwrap(), 22);
    assert_eq!(report.tasks, 2);
}

#[test]
fn self_wake_stress_never_overlaps_or_strands_a_continuation() {
    for workers in [1, 2, 8, 96] {
        for _ in 0..20 {
            let engine = Engine::new(Cases);
            let root = engine.request(key(20));
            let report = engine.run(workers).unwrap();
            assert_eq!(root.result(0).unwrap(), 99);
            assert_eq!(report.publications, 1);
            assert_eq!(report.polls, 21);
        }
    }
}

#[test]
fn unproductive_cycle_has_diagnostic_not_a_hang() {
    let engine = Engine::new(Cases);
    let root = engine.request(key(17));
    let _report = engine.run(4);
    let error = root.result(0).unwrap_err().to_string();
    assert!(
        error.contains("cycle") || error.contains("quiescen"),
        "{error}"
    );
}

#[test]
fn unsupported_external_pending_is_explicit_not_success() {
    let engine = Engine::new(Cases);
    let root = engine.request(key(21));
    let result = engine.run(2);
    assert!(root.result(0).is_err());
    if let Ok(report) = result {
        assert!(!report.capture_complete || report.failures > 0);
    }
}

#[test]
fn build_check_test_share_common_work() {
    let engine = Engine::new(FixtureProvider);
    let shape = Shape::BuildCheckTest;
    let demands: Vec<_> = shape
        .roots(5, 20, 7)
        .into_iter()
        .map(|k| engine.request(k))
        .collect();
    let report = engine.run(8).unwrap();
    let actual: Vec<_> = demands.iter().map(|d| d.result(0).unwrap()).collect();
    assert_eq!(actual, shape.reference(5, 20, 7));
    assert_eq!(report.nodes, 8);
    assert!(report.node_evaluations.iter().all(|(_, n)| *n == 1));
}

#[test]
fn deterministic_shapes_match_sequential_reference_across_schedules() {
    for shape in [
        Shape::Wide,
        Shape::Chain,
        Shape::Diamond,
        Shape::Dynamic,
        Shape::FixedPoint,
        Shape::MultiOutput,
    ] {
        for workers in [1, 2, 4, 8] {
            for seed in 0..8 {
                let size = 2 + seed * 3;
                let engine = Engine::new(FixtureProvider);
                let root = engine.request(shape.roots(size, 10, seed).remove(0));
                let report = engine.run(workers).unwrap();
                assert_eq!(
                    root.result(0).unwrap(),
                    shape.reference(size, 10, seed)[0],
                    "{shape:?} workers={workers} seed={seed}"
                );
                assert!(report.capture_complete);
                assert_eq!(report.events_dropped, 0);
                assert!(report.node_evaluations.iter().all(|(_, n)| *n == 1));
            }
        }
    }
}

#[test]
fn invalid_worker_count_and_repeat_run_rejected() {
    let engine = Engine::new(Cases);
    assert!(engine.run(0).is_err());
    let root = engine.request(key(0));
    engine.run(1).unwrap();
    assert_eq!(root.result(0).unwrap(), 1);
    assert!(engine.run(1).is_err());
}

struct Facts;
impl Provider for Facts {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, cx: Context, k: Key) -> TaskFuture {
        Box::pin(async move {
            match k.kind {
                // Leaf publishes a typed fact.
                0 => cx.publish_fact(0, vec![k.params[0]; 3]),
                // Consumer reads two typed facts through recorded edges.
                1 => {
                    let a = cx.request(Key::new(0, vec![2]));
                    let b = cx.request(Key::new(0, vec![5]));
                    let a: Arc<Vec<u64>> = cx.read_fact(a.get(0)).await?;
                    let b: Arc<Vec<u64>> = cx.read_fact(b.get(0)).await?;
                    cx.publish_fact(0, a.iter().chain(b.iter()).sum::<u64>())
                }
                // Wrong-type and missing-fact reads are explicit errors.
                2 => {
                    let a = cx.request(Key::new(0, vec![1]));
                    match cx.read_fact::<String>(a.get(0)).await {
                        Err(error) if error.0.contains("different type") => {}
                        other => return Err(Error::new(format!("unexpected {:?}", other.err()))),
                    }
                    let plain = cx.request(Key::new(3, vec![]));
                    match cx.read_fact::<u64>(plain.get(0)).await {
                        Err(error) if error.0.contains("no fact") => cx.publish(0, 1),
                        other => Err(Error::new(format!("unexpected {:?}", other.err()))),
                    }
                }
                3 => cx.publish(0, 9),
                _ => Err(Error::new("unknown")),
            }
        })
    }
}

#[test]
fn typed_facts_flow_through_recorded_dependency_edges() {
    for workers in [1, 4] {
        let engine = Engine::new(Facts);
        let sum = engine.request(Key::new(1, vec![]));
        let checks = engine.request(Key::new(2, vec![]));
        let report = engine.run(workers).unwrap();
        assert_eq!(*sum.fact::<u64>(0).unwrap(), 21);
        assert_eq!(checks.result(0).unwrap(), 1);
        assert!(checks.completion().is_ok());
        // Two fact reads by the consumer are visible as graph edges.
        assert!(report.edges >= 2);
        assert!(sum.fact::<String>(0).is_err());
    }
}
