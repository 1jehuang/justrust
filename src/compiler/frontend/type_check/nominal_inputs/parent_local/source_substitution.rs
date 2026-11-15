//! Generic source actuals are replayed independently before closing a receipt.
use super::*;

impl ParentLocalReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn remint_for_generic_source_substitution(
        &self,
        definition: &ir::Ty,
        source_facts: &tr::ValidatedFacts,
        current_facts: &tr::ValidatedFacts,
        env: &ir::GenericEnv,
        parent_arguments: &[ir::Ty],
        mapped_actuals: &[ir::Ty],
        cx: &mut impl ir::SubstitutionCx,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let ir::NominalIndex::ParentLocal { owner, .. } = self.id.index else {
            return Err("generic source substitution requires a parent-local receipt".into());
        };
        current_facts
            .validate_parent_local_closing_source(source_facts, owner, env, parent_arguments)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        self.replay(definition, &self.template, source_facts, budget)?;
        if self.actuals.len() != mapped_actuals.len() {
            return Err("generic source substitution changed canonical actual arity".into());
        }
        budget.retain_bytes(std::mem::size_of_val(mapped_actuals))?;
        let mut expected = Vec::new();
        expected
            .try_reserve_exact(self.actuals.len())
            .map_err(|_| "generic source actual replay allocation failed")?;
        let mut source_scratch = tr::TraitScratch::default();
        let mut current_scratch = tr::TraitScratch::default();
        for actual in self.actuals.iter() {
            budget.charge_work(1)?;
            expected.push(ir::substitute_source_ty_with_scratch(
                actual,
                env,
                parent_arguments,
                source_facts,
                current_facts,
                cx,
                budget,
                &mut source_scratch,
                &mut current_scratch,
            )?);
        }
        // Compare complete canonical types, including every closure owner, site,
        // provider, ordered owner actual and all signature/capture children.
        if !same_actuals(&expected, mapped_actuals, budget)? {
            return Err("generic source substitution changed canonical mapped actuals".into());
        }
        self.remint_source_actuals(
            definition,
            source_facts,
            current_facts,
            env,
            parent_arguments,
            mapped_actuals,
            budget,
        )
    }
}
