//! Borrowed current-engine substitution over already-read nominal definitions.
//! No graph request, source-slot fallback, interner entry or retained world lives
//! here. A closed graph type can only keep its exact original typed read.
use crate::compiler::{frontend, ir};
use crate::engine::EngineAuthority;
mod calls;

#[cfg(test)]
pub(in crate::compiler) fn replay_closed_signature(
    engine: EngineAuthority,
    target: &ir::Callee,
    canonical: &ir::Signature,
    mapped: &ir::Signature,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    replay_closed_signature_with(
        engine,
        target,
        canonical,
        mapped,
        caller,
        budget,
        &mut crate::compiler::traits::TraitScratch::default(),
    )
}

pub(in crate::compiler) fn replay_closed_signature_with(
    engine: EngineAuthority,
    target: &ir::Callee,
    canonical: &ir::Signature,
    mapped: &ir::Signature,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    calls::realize(
        None,
        engine,
        target,
        (canonical, mapped),
        caller,
        budget,
        scratch,
    )
    .map(|_| ())
}

pub(in crate::compiler) fn instantiate_checked_with_constants(
    template: &crate::compiler::ownership::VerifiedTemplateCfg,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    constants: &[std::sync::Arc<
        crate::compiler::associated_constant_graph::AssociatedConstantValueFact,
    >],
    engine: EngineAuthority,
) -> Result<ir::CheckedFunction, String> {
    let mut adapter = ReadSubstitution {
        context,
        engine: Some(engine),
    };
    let output = ir::instantiate_checked_with_constants(
        template,
        arguments,
        closed_facts,
        &mut adapter,
        budget,
        constants,
    )?;
    output
        .body_substitution
        .as_ref()
        .ok_or("closed body omitted substitution proof")?
        .validate_current_engine(engine)?;
    Ok(output)
}

pub(in crate::compiler) fn substitute_ty(
    ty: &ir::Ty,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    caller: ir::FactsCtx<'_>,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
) -> Result<ir::Ty, String> {
    let mut adapter = ReadSubstitution { context, engine };
    ir::substitute_ty(ty, env, arguments, caller, &mut adapter, budget)
}

pub(in crate::compiler) fn substitute_callee(
    callee: &ir::Callee,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
) -> Result<ir::Callee, String> {
    let mut adapter = ReadSubstitution { context, engine };
    ir::substitute_callee(callee, env, arguments, closed_facts, &mut adapter, budget)
}

struct ReadSubstitution<'a> {
    context: &'a mut frontend::PlanCtx,
    engine: Option<EngineAuthority>,
}

impl ir::SubstitutionCx for ReadSubstitution<'_> {
    fn current_engine(&self) -> Option<EngineAuthority> {
        self.engine
    }
    fn realize_canonical_call_signature(
        &mut self,
        target: &ir::Callee,
        canonical: &ir::Signature,
        mapped: &ir::Signature,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::Signature>, String> {
        let engine = self
            .engine
            .ok_or("closed call realization has no current engine")?;
        calls::realize(
            Some(self.context),
            engine,
            target,
            (canonical, mapped),
            caller,
            budget,
            &mut crate::compiler::traits::TraitScratch::default(),
        )
        .map(Some)
    }
    fn nominal_application(&self, id: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
        // The identity-only API remains source-only, even in this adapter.
        ir::SubstitutionCx::nominal_application(self.context, id)
    }

    fn retain_realized_nominal(
        &mut self,
        source: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let engine = self
            .engine
            .ok_or("graph nominal substitution has no current engine")?;
        let capability = match source {
            ir::Ty::Struct(value) => value.realization(),
            ir::Ty::Enum(value) => value.realization(),
            _ => return Err("typed nominal identity substitution requires a nominal".into()),
        }
        .ok_or("graph nominal substitution omitted its typed read")?;
        capability.retain_for_substitution(source, engine, caller, budget)
    }

    fn intern_nominal(
        &mut self,
        declaration: ir::DeclId,
        arguments: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        ir::SubstitutionCx::intern_nominal(self.context, declaration, arguments, caller, budget)
    }

    fn intern_substituted_nominal(
        &mut self,
        request: &ir::NominalSubstitutionRequest<'_>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        ir::SubstitutionCx::intern_substituted_nominal(self.context, request, caller, budget)
    }

    fn instantiate_signature(
        &mut self,
        target: &ir::CallTarget,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Signature, String> {
        ir::SubstitutionCx::instantiate_signature(self.context, target, caller, budget)
    }
}
