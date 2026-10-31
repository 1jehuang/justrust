//! Canonical lowering of a final closure body read from its parent graph fact.
use super::*;

pub(in crate::compiler) fn lower_closure_template(
    source: std::sync::Arc<ir::CheckedFunction>,
    authority: std::sync::Arc<super::super::traits::ValidatedFacts>,
    original: &super::super::frontend::closures::check::CheckedClosure,
    mode: ir::ClosureKind,
    budget: &ir::PlanBudget,
) -> Result<TemplateCfg, String> {
    require_facts(
        &authority,
        super::super::traits::FactsPhase::Universal,
        budget,
    )?;
    let checked = original.body_for(mode)?;
    if !std::sync::Arc::ptr_eq(&source, &checked) {
        return Err("closure lowering changed its parent-body checked allocation".into());
    }
    authority
        .validate_closure_body_source(&original.ty, mode, &source)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    budget.charge_nodes(source.exprs.len().saturating_add(source.locals.len()))?;
    let facts = ir::FactsCtx::from_validated(&authority);
    let closure_origin = super::super::source_replay::OriginalBodyId::new(budget)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    let mut lowered = lower_internal_with_source(
        &source,
        facts,
        Some(budget),
        SourceLowering::Closure(&closure_origin),
    )?;
    lowered.call_evidence = ir::bind_call_evidence(source.clone(), authority.clone(), budget)?;
    let mut shape = GeneratedCfgShape::capture(&lowered, &authority, budget)?;
    shape.closure_origin = Some(closure_origin);
    lowered.lowering_proof = Some(std::sync::Arc::new(shape));
    let function = std::sync::Arc::new(lowered);
    function.preflight_plan(facts, budget)?;
    function.validate_with_facts(facts)?;
    Ok(TemplateCfg {
        source,
        facts: authority,
        function,
        budget: budget.clone(),
    })
}
