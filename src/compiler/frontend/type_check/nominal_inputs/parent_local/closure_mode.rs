//! Reissue exact parent-local source applications for a finalized closure mode.
use super::*;

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn retain_nominal_for_closure_mode(
        &self,
        source: &ir::Ty,
    ) -> Result<ir::Ty, String> {
        let id = match source {
            ir::Ty::Struct(value) => value.id(),
            ir::Ty::Enum(value) => value.id(),
            _ => return Err("closure mode graph retention requires a nominal".into()),
        };
        if !id.index.is_graph() || self.nominal_inputs.is_none() {
            return Err("closure mode graph retention requires original typed read inputs".into());
        }
        // This reuses the owning computation's typed-read callback and its
        // exact-engine replay, never an identity-only source interner lookup.
        let retained = self.realize_constructor_source(source)?;
        let budget = &self.signatures.plan()?.budget;
        if !ir::identical_type(source, &retained, budget)? {
            return Err("closure mode graph retention changed original definition".into());
        }
        Ok(retained)
    }
}

impl ParentLocalReceipt {
    pub(in crate::compiler) fn remint_for_closure_mode(
        &self,
        definition: &ir::Ty,
        source_facts: &tr::ValidatedFacts,
        current_facts: &tr::ValidatedFacts,
        actuals: &[ir::Ty],
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        current_facts
            .validate_parent_local_closure_source(source_facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        self.replay(definition, &self.template, source_facts, budget)?;
        if self.actuals.len() != actuals.len() || !same_actuals(&self.actuals, actuals, budget)? {
            return Err("closure mode remint changed canonical ordered actuals".into());
        }
        let mut scratch = tr::TraitScratch::default();
        for actual in actuals {
            current_facts
                .validate_ty_metadata_scratch(actual, budget, &mut scratch)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        budget.retain_bytes(std::mem::size_of_val(actuals))?;
        let actuals: Arc<[ir::Ty]> = actuals.into();
        let mut fields = Vec::new();
        let mut collect = |variant, source: &ir::NominalFields| -> Result<(), String> {
            let source = source.read(budget)?;
            for field in 0..source.len() {
                budget.charge_work(1)?;
                budget.retain_bytes(std::mem::size_of::<tr::ParentLocalField>())?;
                fields
                    .try_reserve_exact(1)
                    .map_err(|_| "closure mode field receipt allocation failed")?;
                fields.push(
                    current_facts
                        .parent_local_field(&self.template, &self.env, &actuals, variant, field)
                        .map_err(|error| error.into_legacy_diagnostic(budget))?,
                );
            }
            Ok(())
        };
        match &self.template {
            ir::Ty::Struct(value) => collect(None, &value.def().fields)?,
            ir::Ty::Enum(value) => {
                for (variant, source) in value.def().variants.iter().enumerate() {
                    collect(Some(variant), &source.fields)?;
                }
            }
            _ => return Err("closure mode remint lost canonical nominal template".into()),
        }
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        let receipt = Arc::new(Self {
            context: current_facts
                .parent_local_context()
                .map_err(|error| error.into_legacy_diagnostic(budget))?,
            template: self.template.clone(),
            env: self.env.clone(),
            id: self.id,
            actuals,
            fields: fields.into(),
        });
        let reminted = match receipt.definition(budget)? {
            ir::Ty::Struct(value) => {
                ir::Ty::Struct(value.with_parent_local_receipt(receipt.clone()))
            }
            ir::Ty::Enum(value) => ir::Ty::Enum(value.with_parent_local_receipt(receipt.clone())),
            _ => return Err("closure mode remint changed nominal kind".into()),
        };
        receipt.replay(&reminted, &self.template, current_facts, budget)?;
        Ok(reminted)
    }
}
