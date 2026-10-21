//! Structural replay for a parent-minted closure under canonical owner actuals.
//!
//! A successful replay is metadata evidence, NOT Fn membership or permission to
//! invoke the owner. The caller must discharge the instantiated owner's
//! predicates in its real ProofContext before constructing callable evidence.
use super::*;

impl ValidatedTraitWorld {
    /// Replay one retained closure occurrence through the enclosing body's
    /// actual substitution. The original parent-minted seal is retained, not
    /// recreated from a mapping callback or compatible displayed signature.
    pub(super) fn validate_closure_rhs_substitution(
        &self,
        raw: &ir::ClosureTy,
        actual: &ir::ClosureTy,
        env: &GenericEnv,
        arguments: &[Ty],
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        // A copied seal on a changed raw type cannot become a new substitution
        // template, even when the requested result repeats that same forgery.
        self.replay_closure_owner_substitution(raw, env)?;
        let invalid = |message| FactsError::invalid(&self.budget, message);
        let (Some(left_seal), Some(right_seal)) = (raw.shape_seal(), actual.shape_seal()) else {
            return Err(invalid(
                "closure body substitution lost parent shape authority",
            ));
        };
        if !Arc::ptr_eq(left_seal, right_seal)
            || raw.owner() != actual.owner()
            || raw.provider() != actual.provider()
            || raw.site() != actual.site()
            || raw.kind() != actual.kind()
            || raw.sources() != actual.sources()
            || !raw.same_result_receipt_shape(actual)
            || raw.owner_args().len() != actual.owner_args().len()
            || raw.params().len() != actual.params().len()
            || raw.captures().len() != actual.captures().len()
        {
            return Err(invalid(
                "closure body substitution transplanted its source shape",
            ));
        }
        let recipe = self
            .schema
            .closure_recipe(raw.provider())?
            .ok_or_else(|| invalid("closure body substitution lacks a registered recipe"))?;
        if left_seal.template_for(&recipe).is_none() {
            return Err(invalid(
                "closure body substitution used a foreign source recipe",
            ));
        }
        for (left, right) in raw.types().zip(actual.types()) {
            self.validate_rhs_substitution(left, right, env, arguments, depth + 1)?;
        }
        Ok(())
    }

    pub(super) fn replay_closure_owner_substitution(
        &self,
        closure: &ir::ClosureTy,
        caller: &GenericEnv,
    ) -> Result<()> {
        let invalid = |message| FactsError::invalid(&self.budget, message);
        let recipe = self
            .schema
            .closure_recipe(closure.provider())?
            .ok_or_else(|| invalid("closure substitution has no retained source recipe"))?;
        let seal = closure
            .shape_seal()
            .ok_or_else(|| invalid("closure substitution lacks its parent-minted shape"))?;
        let template = seal.template_for(&recipe).ok_or_else(|| {
            invalid("closure source recipe or final shape changed during owner substitution")
        })?;
        if self.schema.closure_owner_site(closure.provider())?
            != Some((closure.owner(), closure.site()))
            || template.owner() != closure.owner()
            || template.provider() != closure.provider()
            || template.site() != closure.site()
            || template.kind() != closure.kind()
            || template.sources() != closure.sources()
            || !template.same_result_receipt_shape(closure)
            || template.params().len() != closure.params().len()
            || template.captures().len() != closure.captures().len()
        {
            return Err(invalid(
                "closure substitution changed its canonical source shape",
            ));
        }
        let owner = self.info(closure.owner())?;
        let env = &owner.env;
        if closure.owner_args().len() != env.total_arity()
            || template.owner_args().len() != env.total_arity()
        {
            return Err(invalid(
                "closure substitution changed its ordered owner arity",
            ));
        }
        for (parameter, argument) in env.ordered_params().zip(template.owner_args()) {
            charged(&self.budget, self.budget.charge_work(1))?;
            if !matches!(argument, Ty::Param(original) if *original == parameter) {
                return Err(invalid(
                    "closure template owner prefix is not its canonical binder order",
                ));
            }
        }
        for argument in closure.owner_args() {
            self.validate_ty_metadata_in(argument, caller, 0)?;
        }
        // Replaying the retained unsealed template is deliberately different
        // from trusting try_map's callback. Only this exact canonical owner
        // substitution may change a sealed signature or capture type.
        for (raw, actual) in template.types().zip(closure.types()) {
            self.validate_rhs_substitution(raw, actual, env, closure.owner_args(), 0)?;
        }
        Ok(())
    }

    /// Required in addition to structural replay before issuing Fn evidence.
    /// Reuse the caller's active obligation stack so a bound on the owner
    /// cannot circularly justify its own closure's builtin membership.
    pub(super) fn require_closure_owner_predicates(
        &self,
        closure: &ir::ClosureTy,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.replay_closure_owner_substitution(closure, context.env())?;
        let schema = self.callable_schema(closure.owner())?;
        let instantiated = self.instantiate_in(
            &schema,
            closure.owner_args(),
            context,
            Some(closure.owner()),
        )?;
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, context, active)?;
        }
        Ok(())
    }
}
