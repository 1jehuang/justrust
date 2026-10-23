//! Classify a parent-authenticated builtin callable Output projection.
//! Generic nominal input projections require checked normalization before tuple equality.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn checked_closure_projection(
        &self,
        projection: &ir::AssociatedTypeProjection,
        env: &GenericEnv,
    ) -> Result<bool> {
        let Ty::Closure(closure) = &projection.trait_ref.self_ty else {
            return Ok(false);
        };
        let Some(mode) = self.closure_trait_kind(projection.trait_ref.declaration)? else {
            return Ok(false);
        };
        self.validate_closure_metadata(closure, env)?;
        let mut arguments_match = projection.trait_ref.args.len() == 1
            && self.eq_ty(
                &projection.trait_ref.args[0],
                &self.closure_arguments(closure)?,
                0,
            )?;
        if !arguments_match && projection.trait_ref.args.len() == 1 && env.is_generic() {
            let scope = env.scopes().last().ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "generic closure projection lacks caller scope",
                )
            })?;
            let caller = self.predicate_env(scope.binder().declaration)?;
            let context = ProofContext {
                predicates: &caller,
                phase: FactsPhase::Schema,
            };
            let mut active = ProofScratch::default();
            let arguments = self.map_universal_projection_type(
                &projection.trait_ref.args[0],
                env,
                0,
                &mut |projection| {
                    if matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_)) {
                        self.normalize_closed_projected_type_with(
                            &Ty::AssociatedType(Arc::new(projection.clone())),
                            context,
                            false,
                            &mut active,
                        )
                        .map(|(normal, _)| normal)
                    } else {
                        Ok(Ty::AssociatedType(Arc::new(projection.clone())))
                    }
                },
            )?;
            arguments_match = self.eq_ty(&arguments, &self.closure_arguments(closure)?, 0)?;
        }
        if self.associated_header(projection.item)?.trait_decl != projection.trait_ref.declaration
            || !closure.supports(mode)
            || !arguments_match
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure Output projection changed its canonical item, capability or argument tuple",
            ));
        }
        Ok(true)
    }
}
