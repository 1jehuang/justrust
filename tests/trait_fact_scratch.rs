//! Real compiler-path regressions for invocation-local trait proof scratch.
use justrust_engine::{Key, compiler};

#[test]
fn independent_tasks_get_exact_local_obligation_limits_and_typed_failures() {
    use compiler::ir::{PlanBudget, ResourceBudgetKind};
    use justrust_engine::{Context, Engine, Error, Provider, TaskFuture};
    struct Obligations(PlanBudget);
    impl Provider for Obligations {
        fn outputs(&self, _: &Key) -> usize {
            1
        }
        fn start(&self, cx: Context, key: Key) -> TaskFuture {
            let budget = self.0.clone();
            Box::pin(async move {
                for _ in 0..key.params[0] {
                    budget.charge_obligation().map_err(Error::new)?;
                }
                cx.publish_fact(0, key.params[0])
            })
        }
    }
    for workers in [1, 8] {
        let budget = PlanBudget::default();
        let engine = Engine::new(Obligations(budget.clone()));
        let demands = (0..8)
            .map(|i| engine.request(Key::new(0, vec![4096, i])))
            .collect::<Vec<_>>();
        let report = engine.run(workers).unwrap();
        for demand in demands {
            demand.completion().unwrap();
            assert_eq!(*demand.fact::<u64>(0).unwrap(), 4096);
        }
        assert_eq!(report.metered_obligations, 8 * 4096);
        assert!(budget.first_resource_failure().is_none());
        assert_eq!(budget.usage_snapshot().obligations, 0);

        let budget = PlanBudget::default();
        let engine = Engine::new(Obligations(budget.clone()));
        let demand = engine.request(Key::new(0, vec![4097]));
        let report = engine.run(workers).unwrap();
        assert!(demand.completion().is_err());
        assert!(demand.fact::<u64>(0).is_err());
        assert_eq!(report.metered_obligations, 4096);
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Obligations);
        assert_eq!(
            (failure.requested, failure.used, failure.limit),
            (1, 4096, 4096)
        );
        assert!(failure.graph_id.is_some());
        assert!(failure.task_id.is_some());
        // Failure evidence is sticky even if a later direct-ledger operation
        // succeeds. It must not be replaced by text classification or a peer.
        let _ = budget.charge_obligation();
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }
}

const SOURCE: &str = "struct S{value:i32}\
trait Has:Sized{type Out;fn get(self)->Self::Out;}\
impl Has for S{type Out=i32;fn get(self)->i32{self.value}}\
fn produce<T:Has<Out=U>,U:Copy>(x:T)->U{x.get()}\
fn unused<T:Copy>(x:T)->(T,T){(x,x)}\
fn main(){let a=produce::<S,i32>(S{value:21});if a==21{println!(\"immutable trait facts\");}}";

#[test]
fn equality_proofs_keep_signature_dependencies_and_worker_independent_artifacts() {
    let one = compiler::compile(SOURCE.to_owned(), 1, false).unwrap();
    let eight = compiler::compile(SOURCE.to_owned(), 8, false).unwrap();
    assert_eq!(one.executable, eight.executable);
    for compiled in [&one, &eight] {
        let report = &compiled.report;
        assert!(report.capture_complete);
        assert_eq!(report.failures, 0);
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        let signature = report
            .node_evaluations
            .iter()
            .position(|(key, _)| *key == Key::new(101, vec![]))
            .unwrap();
        let signature = report.node_ids[signature];
        // Every scheduled source/recipe body must read the immutable signature
        // publication. A previously completed proof cannot replace that edge.
        for (position, (key, count)) in report.node_evaluations.iter().enumerate() {
            assert_eq!(*count, 1);
            if key.kind == 110 {
                let node = report.node_ids[position];
                let tasks = report
                    .events
                    .iter()
                    .filter(|event| event.kind == "running" && event.node == Some(node))
                    .filter_map(|event| event.task)
                    .collect::<Vec<_>>();
                assert!(
                    report
                        .dependency_edges
                        .iter()
                        .any(|(consumer, producer, slot)| {
                            tasks.contains(consumer) && *producer == signature && *slot == 0
                        }),
                    "body {key:?} bypassed its signature fact"
                );
            }
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("jr-trait-scratch-{}", std::process::id()));
        std::fs::write(&path, one.executable.as_ref().unwrap().as_slice()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&path).output().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"immutable trait facts\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn unused_generic_body_is_not_authorized_by_a_successful_other_proof() {
    let source = SOURCE.replace(
        "fn unused<T:Copy>(x:T)->(T,T){(x,x)}",
        "fn unused<T:Has<Out=U>,U>(x:T)->(U,U){let y=x.get();(y,y)}",
    );
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.clone(), workers, check)
                .err()
                .unwrap();
            assert!(
                failure
                    .analyses
                    .iter()
                    .any(|(_, analysis)| !analysis.passed())
            );
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(!report.published_kind(105));
            assert!(
                report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 112)
            );
        }
    }
}
