//! Parent-private final shape minting, distinct from finalized publication proof.
use super::super::type_check::Checker;
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureShapeSeal {
    recipe: ir::ClosureRecipeId,
    authored_source: Option<sr::OriginalBodyId>,
    // This is an unsealed immutable value, so the certificate cannot form a
    // self-cycle through the closure it authorizes.
    shape: ir::ClosureTy,
}
impl std::hash::Hash for ClosureShapeSeal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.recipe, state);
        std::hash::Hash::hash(&self.shape, state);
    }
}
impl ClosureShapeSeal {
    pub(in crate::compiler) fn validates<E>(
        &self,
        recipe: &ir::ClosureRecipeId,
        actual: &ir::ClosureTy,
        equal: impl FnMut(&ir::Ty, &ir::Ty) -> Result<bool, E>,
    ) -> Result<bool, E> {
        if &self.recipe != recipe {
            return Ok(false);
        }
        let unsealed = actual.clone().with_shape_seal(None);
        self.shape.try_eq(&unsealed, equal)
    }
    pub(super) fn authored_source(&self) -> Option<&sr::OriginalBodyId> {
        self.authored_source.as_ref()
    }
}

impl Checker<'_> {
    /// Only the parent checker, after inference and capture analysis, can mint
    /// a final shape. A public raw ClosureTy constructor never grants authority.
    pub(super) fn seal_closure_shape(&self, shape: ir::ClosureTy) -> Result<ir::ClosureTy, String> {
        let plan = self.signatures.plan()?;
        if self.declaration != Some(shape.owner())
            || plan.stored_closure_owner(shape.provider())? != Some((shape.owner(), shape.site()))
            || shape.shape_seal().is_some()
        {
            return Err("closure shape mint has no exact checked source owner".into());
        }
        let recipe = plan
            .registry
            .closure_recipes
            .get(&shape.provider())
            .ok_or("closure shape mint lacks its source recipe")?
            .clone();
        plan.budget.charge_nodes(1)?;
        plan.budget
            .retain_bytes(std::mem::size_of::<ClosureShapeSeal>() + 32)?;
        let seal = Arc::new(ClosureShapeSeal {
            recipe,
            authored_source: self.closure_source_recipe.clone(),
            shape: shape.clone(),
        });
        Ok(shape.with_shape_seal(Some(seal)))
    }
}
