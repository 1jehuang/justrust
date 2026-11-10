//! Invocation-local receipt reminting from provider source facts to final mode facts.
use super::*;

struct ModeRemint<'a> {
    plan: PlanCtx,
    source: &'a tr::ValidatedFacts,
    current: &'a tr::ValidatedFacts,
    retain_graph: &'a dyn Fn(&ir::Ty) -> Result<ir::Ty, String>,
    rows: Vec<(ir::Ty, Arc<[ir::Ty]>, ir::Ty)>,
}

impl ir::SubstitutionCx for ModeRemint<'_> {
    fn retain_realized_nominal(
        &mut self,
        source: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let facts = caller
            .validated()
            .ok_or("closure graph retention lacks final facts")?;
        if !facts.budget().same_authority(budget)
            || !self
                .current
                .parent_local_context()
                .map_err(|error| error.into_legacy_diagnostic(budget))?
                .matches(facts)
                .map_err(|error| error.into_legacy_diagnostic(budget))?
        {
            return Err("closure graph retention changed destination authority".into());
        }
        let retained = (self.retain_graph)(source)?;
        facts
            .validate_ty_metadata_with_budget(&retained, budget)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        Ok(retained)
    }

    fn nominal_application(&self, id: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
        ir::SubstitutionCx::nominal_application(&self.plan, id)
    }

    fn intern_nominal(
        &mut self,
        declaration: ir::DeclId,
        arguments: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.plan
            .intern_nominal(declaration, arguments, caller, budget)
    }

    fn intern_substituted_nominal(
        &mut self,
        request: &ir::NominalSubstitutionRequest<'_>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        ir::SubstitutionCx::intern_substituted_nominal(&mut self.plan, request, caller, budget)
    }

    fn instantiate_signature(
        &mut self,
        target: &ir::CallTarget,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Signature, String> {
        self.plan.instantiate_signature(target, caller, budget)
    }

    fn substitute_parent_local_nominal(
        &mut self,
        source: &ir::Ty,
        arguments: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let facts = caller
            .validated()
            .ok_or("closure remint requires finalized facts")?;
        if !self
            .current
            .parent_local_context()
            .map_err(|error| error.into_legacy_diagnostic(budget))?
            .matches(facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?
        {
            return Err("closure remint changed its exact destination facts".into());
        }
        let receipt = match source {
            ir::Ty::Struct(value) => value.parent_local_receipt(),
            ir::Ty::Enum(value) => value.parent_local_receipt(),
            _ => None,
        }
        .ok_or("closure mode remint requires sealed parent-local input")?;
        if receipt.matches_owning_context(facts)? {
            let original_arguments = match source {
                ir::Ty::Struct(value) => &value.def().type_args,
                ir::Ty::Enum(value) => &value.def().type_args,
                _ => return Err("closure mode remint lost nominal source kind".into()),
            };
            if original_arguments.len() != arguments.len() {
                return Err("closure mode remint changed finalized argument arity".into());
            }
            for (original, mapped) in original_arguments.iter().zip(arguments) {
                budget.charge_work(1)?;
                if original != mapped {
                    return Err("closure mode remint changed finalized ordered actuals".into());
                }
            }
            facts
                .validate_ty_metadata_with_budget(source, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            return Ok(source.clone());
        }
        for (original, actuals, output) in &self.rows {
            budget.charge_work(1)?;
            let original_receipt = match original {
                ir::Ty::Struct(value) => value.parent_local_receipt(),
                ir::Ty::Enum(value) => value.parent_local_receipt(),
                _ => None,
            };
            if !ir::identical_type(original, source, budget)?
                || !original_receipt.is_some_and(|old| Arc::ptr_eq(old, receipt))
                || actuals.len() != arguments.len()
            {
                continue;
            }
            let mut equal = true;
            for (a, b) in actuals.iter().zip(arguments) {
                if !ir::identical_type(a, b, budget)? {
                    equal = false;
                    break;
                }
            }
            if equal {
                facts
                    .validate_ty_metadata_with_budget(output, budget)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?;
                return Ok(output.clone());
            }
        }
        let output =
            receipt.remint_for_closure_mode(source, self.source, facts, arguments, budget)?;
        budget.retain_bytes(
            std::mem::size_of::<(ir::Ty, Arc<[ir::Ty]>, ir::Ty)>()
                .checked_add(std::mem::size_of_val(arguments))
                .ok_or("closure remint memo storage overflow")?,
        )?;
        self.rows
            .try_reserve_exact(1)
            .map_err(|_| "closure remint memo allocation failed")?;
        self.rows
            .push((source.clone(), arguments.into(), output.clone()));
        Ok(output)
    }
}

pub(super) fn body(
    checker: &Checker<'_>,
    plan: &PlanCtx,
    source: &tr::ValidatedFacts,
    current: &tr::ValidatedFacts,
    body: &ir::CheckedFunction,
) -> Result<ir::CheckedFunction, String> {
    let retain_graph = |ty: &ir::Ty| checker.retain_nominal_for_closure_mode(ty);
    let mut remint = ModeRemint {
        plan: plan.clone(),
        source,
        current,
        retain_graph: &retain_graph,
        rows: Vec::new(),
    };
    ir::remint_closure_mode_body(body, source, current, &mut remint, &plan.budget)
}
