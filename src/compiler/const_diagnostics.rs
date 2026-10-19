//! Diagnostic-only interpretation of exact live engine cycle witnesses.
//! No compiler scheduling, acceptance, or value computation reads this path.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    struct Control {
        compiler: CompilerProvider,
        mixed: bool,
        error: &'static str,
    }
    impl Provider for Control {
        fn outputs(&self, key: &Key) -> usize {
            self.compiler.outputs(key)
        }
        fn start(&self, cx: Context, key: Key) -> TaskFuture {
            if key.kind == 900 {
                let error = self.error;
                return Box::pin(async move { Err(Error::new(error)) });
            }
            if self.mixed && matches!(key.kind, CONST_BODY | CONST_VALUE) {
                return Box::pin(async move {
                    let other = if key.kind == CONST_BODY {
                        CONST_VALUE
                    } else {
                        CONST_BODY
                    };
                    cx.read(
                        cx.request(Key::new(other, vec![0]))
                            .get(if other == CONST_BODY { 1 } else { 0 }),
                    )
                    .await?;
                    cx.publish(0, 1)
                });
            }
            self.compiler.start(cx, key)
        }
    }
    #[test]
    fn const_cycle_witness_without_capture_preserves_independent_resource_and_type_errors() {
        for workers in [1, 8] {
            for error in [
                "resource limit independent control",
                "E0308 independent type control",
            ] {
                let budget = ir::PlanBudget::default();
                let compiler = CompilerProvider(Arc::new(Snapshot {
                    input: Input::Text("const A:bool=false&&A;fn main(){}".into()),
                    source_options: Default::default(),
                    budget: budget.clone(),
                }));
                let engine = Engine::with_event_capacity(
                    Control {
                        compiler,
                        mixed: false,
                        error,
                    },
                    0,
                );
                let cycle = engine.request(Facts::root(CHECK));
                let failure = engine.request(Key::new(900, vec![]));
                let report = engine.run(workers).unwrap();
                assert!(report.dependency_cycle().is_some());
                assert!(!report.capture_complete);
                let plan = engine
                    .fact_of::<frontend::PlanCtx>(&Facts::root(SIGNATURES), 0)
                    .unwrap();
                let id = plan.constant_decl(0).unwrap();
                let key = Facts::item(CONST_BODY, id.index as usize);
                assert!(
                    engine
                        .fact_of::<Arc<const_check::BodyFact>>(&key, 0)
                        .is_some(),
                    "typed body may publish independently"
                );
                let cycle_error = cycle.result(0).unwrap_err();
                assert!(
                    qualification_cycle_error(&engine, &report, &cycle_error, &budget)
                        .unwrap()
                        .to_string()
                        .contains("E0391")
                );
                let original = failure.result(0).unwrap_err();
                assert_eq!(original.to_string(), error);
                assert!(qualification_cycle_error(&engine, &report, &original, &budget).is_none());
                assert!(
                    engine
                        .fact_of::<const_check::ValueFact>(
                            &Facts::item(CONST_VALUE, id.index as usize),
                            0
                        )
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn const_mixed_body_value_cycle_is_not_language_qualification_cycle() {
        for workers in [1, 8] {
            let budget = ir::PlanBudget::default();
            let compiler = CompilerProvider(Arc::new(Snapshot {
                input: Input::Text("const A:bool=true;fn main(){}".into()),
                source_options: Default::default(),
                budget: budget.clone(),
            }));
            let engine = Engine::with_event_capacity(
                Control {
                    compiler,
                    mixed: true,
                    error: "unused",
                },
                0,
            );
            let _plan = engine.request(Facts::root(SIGNATURES));
            let demand = engine.request(Facts::item(CONST_BODY, 0));
            let report = engine.run(workers).unwrap();
            let witness = report.dependency_cycle().unwrap();
            assert!(
                witness
                    .edges()
                    .iter()
                    .any(|edge| edge.consumer().kind == CONST_VALUE)
            );
            let original = demand.result(1).unwrap_err();
            assert!(qualification_cycle_error(&engine, &report, &original, &budget).is_none());
            assert!(!original.to_string().contains("E0391"));
        }
    }
}

pub(super) fn qualification_cycle_error(
    engine: &Engine,
    report: &Report,
    original: &Error,
    budget: &ir::PlanBudget,
) -> Option<Error> {
    let witness = report.dependency_cycle()?;
    let edges = witness.edges();
    if edges.is_empty() {
        return None;
    }
    let bytes = edges.iter().try_fold(0usize, |bytes, edge| {
        edge.consumer()
            .params
            .len()
            .checked_add(edge.producer().params.len())?
            .checked_mul(std::mem::size_of::<u64>())?
            .checked_add(std::mem::size_of::<crate::engine::DependencyCycleEdge>())?
            .checked_add(bytes)
    })?;
    budget.charge_work(edges.len()).ok()?;
    let _admission = budget.reserve_bytes(bytes).ok()?;
    let plan = engine.fact_of::<frontend::PlanCtx>(&Facts::root(SIGNATURES), 0)?;
    for (position, edge) in edges.iter().enumerate() {
        let consumer = edge.consumer();
        let producer = edge.producer();
        if consumer.kind != CONST_BODY
            || producer.kind != CONST_BODY
            || consumer.version != 1
            || producer.version != 1
            || consumer.params.len() != 1
            || producer.params.len() != 1
            || edge.output() != 1
            || producer != edges[(position + 1) % edges.len()].consumer()
        {
            return None;
        }
        let consumer_id = plan
            .constant_key_decl(usize::try_from(consumer.params[0]).ok()?)
            .ok()?;
        let producer_id = plan
            .constant_key_decl(usize::try_from(producer.params[0]).ok()?)
            .ok()?;
        let body = engine.fact_of::<Arc<const_check::BodyFact>>(consumer, 0)?;
        if body.source().declaration != consumer_id {
            return None;
        }
        budget.charge_work(body.source().expressions.len()).ok()?;
        if !body.source().expressions.iter().any(
            |expr| matches!(expr.kind,const_source::SyntaxKind::Reference(id) if id==producer_id),
        ) {
            return None;
        }
        // Preserve an unrelated resource/value/type error even if another
        // component of the demanded graph independently has a real cycle.
        budget
            .charge_work(
                report
                    .node_evaluations
                    .len()
                    .checked_add(report.node_errors.len())?
                    .checked_add(report.node_ids.len())?,
            )
            .ok()?;
        let node = report
            .node_ids
            .iter()
            .zip(&report.node_evaluations)
            .find_map(|(id, (key, _))| (key == consumer).then_some(*id))?;
        if !report
            .node_errors
            .iter()
            .any(|(id, error)| *id == node && error == original)
        {
            return None;
        }
    }
    let message = "E0391: cycle detected while qualifying source constant initializers";
    budget.charge_diagnostic_bytes(message.len()).ok()?;
    budget.retain_bytes(message.len()).ok()?;
    Some(Error::new(message))
}
