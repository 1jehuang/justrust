use justrust_engine::{Context, Engine, Key, Provider, TaskFuture};

struct Constant;
impl Provider for Constant {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        Box::pin(async move { cx.publish(0, key.params[0]) })
    }
}

#[test]
fn overflow_marks_capture_incomplete_without_changing_computation() {
    let engine = Engine::with_event_capacity(Constant, 1);
    let root = engine.request(Key::new(1, vec![7]));
    let report = engine.run(4).unwrap();
    assert_eq!(root.result(0).unwrap(), 7);
    assert!(root.completion().is_ok());
    assert_eq!(report.events.len(), 1);
    assert!(report.events_dropped > 0);
    assert!(!report.capture_complete);
}

#[test]
fn normal_trace_reconciles_execution_publication_and_demands() {
    let engine = Engine::new(Constant);
    let a = engine.request(Key::new(1, vec![7]));
    let b = engine.request(Key::new(1, vec![7]));
    b.cancel();
    let report = engine.run(4).unwrap();
    assert_eq!(report.nodes, 1);
    assert_eq!(report.tasks, 1);
    assert_eq!(report.polls, 1);
    assert_eq!(report.publications, 1);
    assert_eq!(report.demands, 2);
    assert_eq!(report.cancellations, 1);
    assert_eq!(
        report.events.iter().filter(|e| e.kind == "running").count(),
        report.polls
    );
    assert_eq!(
        report.events.iter().filter(|e| e.kind == "publish").count(),
        report.publications
    );
    assert_eq!(
        report
            .events
            .iter()
            .filter(|e| e.kind == "task-done")
            .count(),
        report.tasks
    );
    assert!(report.events.windows(2).all(|w| w[0].at_ns <= w[1].at_ns));
    assert!(!report.resource_sampling_available);
    assert_eq!(a.result(0).unwrap(), 7);
    assert!(a.completion().is_ok());
    assert!(b.completion().is_err());
}

#[test]
fn empty_graph_completes_without_fabricated_work() {
    let engine = Engine::new(Constant);
    let report = engine.run(8).unwrap();
    assert_eq!(report.nodes, 0);
    assert_eq!(report.tasks, 0);
    assert_eq!(report.polls, 0);
    assert_eq!(report.failures, 0);
    assert!(report.capture_complete);
}

#[test]
fn new_work_after_finished_run_is_terminal_not_stranded() {
    let engine = Engine::new(Constant);
    let a = engine.request(Key::new(1, vec![7]));
    engine.run(1).unwrap();
    assert_eq!(a.result(0).unwrap(), 7);
    let late = engine.request(Key::new(1, vec![8]));
    assert!(late.result(0).is_err());
    assert!(late.completion().is_err());
}
