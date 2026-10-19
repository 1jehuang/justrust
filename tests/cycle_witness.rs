//! Live typed cycle evidence is independent of trace capture and preserves slots.
use justrust_engine::{Context, Engine, Error, Key, Provider, TaskFuture};
struct Cycle {
    mixed: bool,
}
impl Provider for Cycle {
    fn outputs(&self, _: &Key) -> usize {
        2
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        let mixed = self.mixed;
        Box::pin(async move {
            cx.publish(0, 7)?;
            let other = cx.request(Key::new(900, vec![1 - key.params[0]]));
            let slot = if mixed && key.params[0] == 0 { 0 } else { 1 };
            cx.read(other.get(slot)).await?;
            if mixed {
                cx.read(cx.request(Key::new(901, vec![key.params[0]])).get(1))
                    .await?;
            }
            cx.publish(1, 9)
        })
    }
}
#[test]
fn cycle_witness_uses_real_waits_without_trace_and_preserves_early_output() {
    for workers in [1, 8] {
        let engine = Engine::with_event_capacity(Cycle { mixed: false }, 0);
        let demand = engine.request(Key::new(900, vec![0]));
        let report = engine.run(workers).unwrap();
        assert!(!report.capture_complete);
        assert_eq!(demand.result(0).unwrap(), 7);
        assert!(demand.result(1).is_err());
        let edges = report.dependency_cycle().expect("real cycle").edges();
        assert_eq!(edges.len(), 2);
        for (index, edge) in edges.iter().enumerate() {
            assert_eq!(edge.output(), 1);
            assert_eq!(edge.producer(), edges[(index + 1) % edges.len()].consumer());
            assert_eq!(edge.consumer().kind, 900);
        }
    }
}
struct External;
impl Provider for External {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, _: Context, _: Key) -> TaskFuture {
        Box::pin(std::future::pending())
    }
}
struct Failure;
impl Provider for Failure {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, _: Context, _: Key) -> TaskFuture {
        Box::pin(async { Err(Error::new("resource limit fixture")) })
    }
}
#[test]
fn resource_and_external_wait_errors_have_no_cycle_witness() {
    let engine = Engine::new(External);
    let demand = engine.request(Key::new(901, vec![]));
    let report = engine.run(8).unwrap();
    assert!(demand.result(0).is_err());
    assert!(report.dependency_cycle().is_none());
    let engine = Engine::new(Failure);
    let demand = engine.request(Key::new(902, vec![]));
    let report = engine.run(8).unwrap();
    assert!(
        demand
            .result(0)
            .unwrap_err()
            .to_string()
            .contains("resource limit")
    );
    assert!(report.dependency_cycle().is_none());
}
struct Mixed;
impl Provider for Mixed {
    fn outputs(&self, _: &Key) -> usize {
        2
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        Box::pin(async move {
            if key.params[0] == 0 {
                cx.read(cx.request(Key::new(903, vec![1])).get(0)).await?;
            } else {
                cx.read(cx.request(Key::new(903, vec![0])).get(1)).await?;
            }
            cx.publish(0, 1)
        })
    }
}
#[test]
fn witness_keeps_mixed_output_slots_instead_of_calling_every_cycle_qualification() {
    let engine = Engine::new(Mixed);
    let demand = engine.request(Key::new(903, vec![0]));
    let report = engine.run(1).unwrap();
    assert!(demand.result(0).is_err());
    let edges = report.dependency_cycle().unwrap().edges();
    assert!(edges.iter().any(|edge| edge.output() == 0));
    assert!(edges.iter().any(|edge| edge.output() == 1));
}
