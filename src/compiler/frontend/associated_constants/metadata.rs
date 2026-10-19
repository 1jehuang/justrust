//! Canonical metadata substitution for non-callable constant members.
use super::*;

impl SchemaAdapter {
    pub(in crate::compiler::frontend) fn substitute_constant_metadata(
        &self,
        declaration: ir::DeclId,
        args: &[ir::Ty],
        caller: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> Result<(Option<ir::Ty>, ir::Ty, Arc<[ir::Predicate]>), tr::FactsError> {
        if !budget.same_authority(&self.budget) {
            return Err(tr::FactsError::invalid(
                budget,
                "constant metadata budget authority mismatch",
            ));
        }
        let mut plan = self.raw_plan().map_err(|e| self.error(e))?;
        let header = plan
            .associated_constant_schema(declaration, budget)
            .map_err(|e| self.error(e))?;
        let mut substitute = |ty: &ir::Ty| {
            ir::substitute_schema_metadata_ty(
                ty,
                &header.env,
                args,
                ir::FactsCtx::Universal(caller),
                &mut plan,
                self,
                budget,
            )
            .map_err(|e| self.error(e))
        };
        let owner = header.owner_ty.as_ref().map(&mut substitute).transpose()?;
        let ty = substitute(&header.ty)?;
        let mut predicates = Vec::new();
        for predicate in header.predicates.iter() {
            let application = predicate.trait_ref();
            let self_ty = substitute(&application.self_ty)?;
            let mut actuals = Vec::new();
            for actual in application.args.iter() {
                let value = substitute(actual)?;
                sr::push(&mut actuals, value, budget)
                    .map_err(|e| self.error(e.into_legacy_diagnostic(budget)))?;
            }
            let application = ir::TraitRef {
                declaration: application.declaration,
                self_ty,
                args: actuals.into(),
            };
            let value = match predicate {
                ir::Predicate::Implements(_) => ir::Predicate::Implements(application),
                ir::Predicate::ProjectionEq { projection, ty } => ir::Predicate::ProjectionEq {
                    projection: ir::AssociatedTypeProjection {
                        trait_ref: application,
                        item: projection.item,
                    },
                    ty: substitute(ty)?,
                },
            };
            sr::push(&mut predicates, value, budget)
                .map_err(|e| self.error(e.into_legacy_diagnostic(budget)))?;
        }
        Ok((owner, ty, predicates.into()))
    }
}
