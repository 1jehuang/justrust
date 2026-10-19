//! Classify only a parent-authenticated builtin callable Output projection.
//! This is metadata classification, not trait membership or a reduction proof.
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
        if self.associated_header(projection.item)?.trait_decl != projection.trait_ref.declaration
            || !closure.supports(mode)
            || projection.trait_ref.args.len() != 1
            || !self.eq_ty(
                &projection.trait_ref.args[0],
                &self.closure_arguments(closure)?,
                0,
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure Output projection changed its canonical item, capability or argument tuple",
            ));
        }
        Ok(true)
    }
}
