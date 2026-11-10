//! Source-closing transport independently derives every ordered mapped actual.
use super::*;

impl ParentLocalReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn remint_from_mapped_source(
        &self,
        definition: &ir::Ty,
        source: &tr::ValidatedFacts,
        current: &tr::ValidatedFacts,
        env: &ir::GenericEnv,
        parent_arguments: &[ir::Ty],
        mapped_actuals: &[ir::Ty],
        cx: &mut impl ir::SubstitutionCx,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let ir::NominalIndex::ParentLocal { owner, .. } = self.id.index else {
            return Err("source remint requires a parent-local source receipt".into());
        };
        current
            .validate_parent_local_closing_source(source, owner, env, parent_arguments)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        self.replay(definition, &self.template, source, budget)?;
        if self.actuals.len() != mapped_actuals.len() {
            return Err("source remint changed canonical actual arity".into());
        }
        budget.retain_bytes(std::mem::size_of_val(mapped_actuals))?;
        let mut expected = Vec::new();
        expected
            .try_reserve_exact(self.actuals.len())
            .map_err(|_| "source remint expected actuals allocation failed")?;
        let mut source_scratch = tr::TraitScratch::default();
        let mut current_scratch = tr::TraitScratch::default();
        for actual in self.actuals.iter() {
            budget.charge_work(1)?;
            expected.push(ir::substitute_source_ty_with_scratch(
                actual,
                env,
                parent_arguments,
                source,
                current,
                cx,
                budget,
                &mut source_scratch,
                &mut current_scratch,
            )?);
        }
        // Full structural canonical types, not display names or shallow kinds:
        // closure owner, site, provider, owner arguments and all children stay.
        if !same_actuals(&expected, mapped_actuals, budget)? {
            return Err("source remint changed canonical mapped actuals".into());
        }
        self.remint_source_actuals(
            definition,
            source,
            current,
            env,
            parent_arguments,
            mapped_actuals,
            budget,
        )
    }
}
