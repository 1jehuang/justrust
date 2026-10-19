//! Diagnostic-only interpretation of exact live engine cycle witnesses.
//! No compiler scheduling, acceptance, or value computation reads this path.
use super::*;

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
                    .checked_add(report.node_errors.len())?,
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
