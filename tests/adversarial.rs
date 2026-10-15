use justrust_engine::{Context, Engine, Error, Handle, Key, Provider, TaskFuture};
use std::future::Future;
use std::task::Poll;

struct Adversarial {
    foreign: Option<Handle>,
}

fn key(kind: u32) -> Key {
    Key::new(kind, vec![])
}

#[test]
fn dropping_one_same_task_read_preserves_other_registered_read() {
    let engine = Engine::new(Adversarial { foreign: None });
    let root = engine.request(key(9));
    let report = engine.run(1).unwrap();
    assert_eq!(root.result(0).unwrap(), 91);
    assert!(root.completion().is_ok());
    assert_eq!(
        report.edges, 1,
        "semantic edges deduplicate task/slot pairs"
    );
    assert_eq!(report.publications, 2);
    assert_eq!(report.failures, 0);
}

impl Provider for Adversarial {
    fn outputs(&self, k: &Key) -> usize {
        if k.kind == 3 { 2 } else { 1 }
    }

    fn start(&self, cx: Context, k: Key) -> TaskFuture {
        let foreign = self.foreign.clone();
        Box::pin(async move {
            match k.kind {
                0 => cx.publish(0, 91),
                1 => {
                    let error = cx
                        .read(foreign.expect("foreign test handle"))
                        .await
                        .unwrap_err();
                    assert!(
                        error.to_string().contains("engine") || error.to_string().contains("graph")
                    );
                    cx.publish(0, 1)
                }
                2 => {
                    let source = cx.request(key(0));
                    assert!(cx.read(source.get(usize::MAX)).await.is_err());
                    assert!(cx.publish(usize::MAX, 2).is_err());
                    cx.publish(0, cx.read(source.get(0)).await?)
                }
                3 => {
                    cx.publish(0, 17)?;
                    cx.fork(Box::pin(async { Err(Error::new("fork rejected body")) }))?;
                    Ok(())
                }
                4 => {
                    let source = cx.request(key(3));
                    cx.publish(0, cx.read(source.get(0)).await?)
                }
                5 => {
                    let source = cx.request(key(3));
                    let value = cx.read(source.get(0)).await?;
                    cx.read(source.get(1)).await?;
                    cx.publish(0, value)
                }
                6 => cx.publish(0, k.params[0] * 100 + k.params[1]),
                7 => {
                    let source = cx.request(key(8));
                    // One producer, many independently suspended reader branches.
                    for _ in 0..128 {
                        let branch = cx.clone();
                        let handle = source.get(0);
                        cx.fork(Box::pin(async move {
                            assert_eq!(branch.read(handle.clone()).await?, 73);
                            // Re-reading a retained fact must not restart its producer.
                            assert_eq!(branch.read(handle).await?, 73);
                            Ok(())
                        }))?;
                    }
                    cx.publish(0, 1)
                }
                8 => {
                    let mut remaining = 32;
                    std::future::poll_fn(move |cx| {
                        if remaining == 0 {
                            Poll::Ready(())
                        } else {
                            remaining -= 1;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                    })
                    .await;
                    cx.publish(0, 73)
                }
                9 => {
                    let source = cx.request(key(0));
                    let mut discarded = Some(Box::pin(cx.read(source.get(0))));
                    let mut retained = Box::pin(cx.read(source.get(0)));
                    let value = std::future::poll_fn(move |poll_cx| {
                        if let Some(first) = discarded.as_mut() {
                            // One worker guarantees the producer is still queued.
                            assert!(first.as_mut().poll(poll_cx).is_pending());
                            assert!(retained.as_mut().poll(poll_cx).is_pending());
                            // One dropped read must not remove another read's wakeup.
                            drop(discarded.take());
                            Poll::Pending
                        } else {
                            retained.as_mut().poll(poll_cx)
                        }
                    })
                    .await?;
                    cx.publish(0, value)
                }
                _ => Err(Error::new("unknown adversarial case")),
            }
        })
    }
}

#[test]
fn foreign_handles_rejected_even_when_node_ids_and_values_match() {
    for finalized in [false, true] {
        let foreign_engine = Engine::new(Adversarial { foreign: None });
        let source = foreign_engine.request(key(0));
        if finalized {
            foreign_engine.run(1).unwrap();
            assert_eq!(source.result(0).unwrap(), 91);
        }
        let engine = Engine::new(Adversarial {
            foreign: Some(source.outputs().get(0)),
        });
        let root = engine.request(key(1));
        assert_eq!(root.outputs().node_id(), source.outputs().node_id());
        let report = engine.run(2).unwrap();
        assert_eq!(root.result(0).unwrap(), 1);
        assert!(root.completion().is_ok());
        assert_eq!(
            report.edges, 0,
            "foreign graph must not enter local dependencies"
        );
    }
}

#[test]
fn invalid_output_indices_are_errors_without_corrupting_valid_slots() {
    let engine = Engine::new(Adversarial { foreign: None });
    let root = engine.request(key(2));
    assert!(root.result(usize::MAX).is_err());
    let report = engine.run(2).unwrap();
    assert_eq!(root.result(0).unwrap(), 91);
    assert!(root.result(1).is_err());
    assert!(root.completion().is_ok());
    assert_eq!(report.publications, 2);
    assert_eq!(report.failures, 0);
}

#[test]
fn fork_failure_preserves_early_fact_but_fails_producer_completion() {
    for workers in [1, 2, 8] {
        let engine = Engine::new(Adversarial { foreign: None });
        let producer = engine.request(key(3));
        let signature_only = engine.request(key(4));
        let checked = engine.request(key(5));
        let report = engine.run(workers).unwrap();
        assert_eq!(producer.result(0).unwrap(), 17);
        assert!(
            producer
                .result(1)
                .unwrap_err()
                .to_string()
                .contains("fork rejected body")
        );
        assert!(
            producer
                .completion()
                .unwrap_err()
                .to_string()
                .contains("fork rejected body")
        );
        assert_eq!(signature_only.result(0).unwrap(), 17);
        assert!(signature_only.completion().is_ok());
        assert!(checked.result(0).is_err());
        assert!(checked.completion().is_err());
        assert_eq!(report.retained_outputs, 2);
        assert!(report.capture_complete);
    }
}

#[test]
fn parameter_order_is_semantic_even_with_total_hash_collision() {
    let engine = Engine::with_hash_bits(Adversarial { foreign: None }, 0);
    let a = engine.request(Key::new(6, vec![2, 9]));
    let b = engine.request(Key::new(6, vec![9, 2]));
    let same = engine.request(Key::new(6, vec![2, 9]));
    assert_ne!(a.outputs().node_id(), b.outputs().node_id());
    assert_eq!(a.outputs().node_id(), same.outputs().node_id());
    let report = engine.run(4).unwrap();
    assert_eq!(a.result(0).unwrap(), 209);
    assert_eq!(b.result(0).unwrap(), 902);
    assert_eq!(report.nodes, 2);
    assert!(report.node_evaluations.iter().all(|(_, count)| *count == 1));
}

#[test]
fn high_fan_in_readers_all_resume_and_share_one_producer() {
    for workers in [1, 2, 8] {
        for repetition in 0..8 {
            let engine = Engine::new(Adversarial { foreign: None });
            let root = engine.request(key(7));
            let report = engine.run(workers).unwrap();
            assert_eq!(root.result(0).unwrap(), 1);
            assert!(
                root.completion().is_ok(),
                "workers={workers} repetition={repetition}"
            );
            assert_eq!(report.nodes, 2);
            assert_eq!(report.tasks, 130);
            assert_eq!(report.edges, 128);
            assert_eq!(report.publications, 2);
            assert!(report.node_evaluations.iter().all(|(_, count)| *count == 1));
            assert!(report.capture_complete);
        }
    }
}
