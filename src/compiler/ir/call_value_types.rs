//! Type shape of a call whose canonical callee/signature was already validated.
use super::*;

fn exact(left: &Ty, right: &Ty, facts: FactsCtx<'_>, budget: &PlanBudget) -> Result<bool, String> {
    if let Some(authority) = facts
        .validated()
        .filter(|f| f.phase() == super::super::traits::FactsPhase::Universal)
    {
        authority
            .validate_ty(left)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        authority
            .validate_ty(right)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        return authority
            .world()
            .types_equal(std::slice::from_ref(left), std::slice::from_ref(right))
            .map_err(|e| e.into_legacy_diagnostic(budget));
    }
    if identical_type(left, right, budget)? {
        return Ok(true);
    }
    ref_types_equal(left, right, budget, 0)
}

impl CheckedFunction {
    pub(super) fn validate_call_value_types(
        &self,
        value: &TypedExpr,
        args: &[ExprId],
        signature: &Signature,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if !exact(&value.ty, &signature.result, facts, budget)? {
            return Err("call result type differs from canonical signature".into());
        }
        if args.len() != signature.params.len() {
            return Err("call value type arity mismatch".into());
        }
        for (id, formal) in args.iter().zip(signature.params.iter()) {
            budget.charge_work(1)?;
            let actual = self.exprs.get(id.0).ok_or("call argument outside arena")?;
            if actual.ty == Ty::Never && actual.completion != Completion::Never {
                return Err("call never argument may not complete".into());
            }
            if !exact(&actual.ty, formal, facts, budget)?
                && !Self::conditional_join_matches(&actual.ty, formal, formal, budget)?
            {
                return Err("call argument type differs from canonical signature".into());
            }
        }
        Ok(())
    }
}
