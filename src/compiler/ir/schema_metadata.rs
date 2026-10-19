//! Unsealed structural substitution for canonical schema producers only.
//! Caller membership and executable type/carrier authority are checked by the
//! trait world before and after the schema adapter, never granted here.
use super::*;

pub(in crate::compiler) fn substitute_schema_metadata_ty(
    ty: &Ty,
    env: &GenericEnv,
    args: &[Ty],
    caller: FactsCtx<'_>,
    cx: &mut impl SubstitutionCx,
    registry: &dyn super::super::traits::SchemaCx,
    budget: &PlanBudget,
) -> Result<Ty, String> {
    if !registry.budget().same_authority(budget) || args.len() != env.total_arity() {
        return Err("schema metadata substitution authority or arity mismatch".into());
    }
    for scope in env.scopes() {
        budget.charge_work(1)?;
        let declaration = registry
            .declaration_info(scope.binder().declaration)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        if declaration.own_env.binder() != scope.binder()
            || declaration.own_env.arity() != scope.arity()
        {
            return Err("schema metadata substitution changed canonical binder scope".into());
        }
    }
    FactsCtx::Universal(env).validate_ty_metadata_with_budget(ty, budget)?;
    for argument in args {
        caller.validate_ty_metadata_with_budget(argument, budget)?;
    }
    let result = substitute_ty_walk(ty, env, args, caller, cx, budget, 0)?;
    caller.validate_ty_metadata_with_budget(&result, budget)?;
    Ok(result)
}
