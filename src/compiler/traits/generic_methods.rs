//! Generic trait methods keep an owner binder and a distinct method binder.
//! Substitution is simultaneous and checked against the canonical raw schema.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn alpha_rename_method_schema(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        method_args: &[Ty],
        caller: &GenericEnv,
    ) -> Result<InstantiatedSchema> {
        self.validate_ref_in(goal, caller, None)?;
        if item.trait_decl != goal.declaration
            || self.info(item.declaration)?.own_env.arity() != method_args.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "generic default alpha-renaming changed owner or arity",
            ));
        }
        let mut arguments = self.trait_arguments(goal)?;
        for argument in method_args {
            self.validate_ty_metadata_in(argument, caller, 0)?;
            push(&self.budget, &mut arguments, argument.clone())?;
        }
        let result =
            self.schema
                .instantiate_schema(&item.schema, &arguments, caller, &self.budget)?;
        if result.signature.params.len() != item.schema.signature.params.len()
            || result.predicates.len() != item.schema.predicates.len()
            || !self.eq_contract(
                &item.schema.signature.ref_contract,
                &result.signature.ref_contract,
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "generic default alpha-renaming changed schema shape",
            ));
        }
        for (raw, actual) in item
            .schema
            .signature
            .params
            .iter()
            .zip(&result.signature.params)
        {
            self.validate_rhs_substitution(raw, actual, &item.schema.env, &arguments, 0)?;
        }
        self.validate_rhs_substitution(
            &item.schema.signature.result,
            &result.signature.result,
            &item.schema.env,
            &arguments,
            0,
        )?;
        for (raw, actual) in item.schema.predicates.iter().zip(result.predicates.iter()) {
            self.validate_predicate_substitution(raw, actual, &item.schema.env, &arguments)?;
        }
        // All substitution checks above ran against the raw contract. Realize
        // reference topology for the full ordered actuals with the same kernel
        // ordinary instantiation uses, so a reference-bearing Self (or method
        // actual) gets its per-layer rows from the item's own origin tails.
        let mut result = result;
        result.signature.ref_contract = self.realize_ref_contract(
            &item.schema,
            &arguments,
            &result.signature,
            Some(item.declaration),
        )?;
        Ok(result)
    }

    pub(super) fn method_environment_matches(
        &self,
        environment: &GenericEnv,
        owner: &GenericEnv,
        own: &ir::ParamEnv,
    ) -> Result<bool> {
        charged(
            &self.budget,
            self.budget
                .charge_work(environment.scopes().len() + owner.scopes().len() + 1),
        )?;
        if environment.scopes().len() != owner.scopes().len() + 1 {
            return Ok(false);
        }
        for (actual, expected) in environment.scopes().iter().zip(owner.scopes()) {
            if actual.binder() != expected.binder() || actual.arity() != expected.arity() {
                return Ok(false);
            }
        }
        let method = environment.scopes().last().expect("checked method scope");
        Ok(method.binder() == own.binder() && method.arity() == own.arity())
    }

    pub(super) fn method_arguments(&self, declaration: DeclId) -> Result<Vec<Ty>> {
        let scope = &self.info(declaration)?.own_env;
        let mut args = Vec::new();
        for index in 0..scope.arity() {
            push(
                &self.budget,
                &mut args,
                Ty::Param(ParamId {
                    owner: scope.binder(),
                    index: index as u16,
                }),
            )?;
        }
        Ok(args)
    }

    #[cfg(test)]
    pub(super) fn instantiate_generic_method(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        method_args: &[Ty],
        context: ProofContext<'_>,
    ) -> Result<InstantiatedSchema> {
        self.instantiate_generic_method_with_scratch(
            item,
            goal,
            method_args,
            context,
            &mut ProofScratch::default(),
        )
    }

    pub(super) fn instantiate_generic_method_with_scratch(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        method_args: &[Ty],
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        self.check_context(context)?;
        self.validate_ref_with(goal, context.env(), None, &mut scratch.replays)?;
        if item.trait_decl != goal.declaration
            || self.info(item.declaration)?.own_env.arity() != method_args.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "generic trait method owner or argument arity mismatch",
            ));
        }
        // Validate the raw simultaneous substitution before normalizing any
        // projected ABI types. Comparing a normalized Item to the raw projection
        // would discard the very substitution witness we need to authenticate.
        let mut arguments = self.trait_arguments(goal)?;
        for argument in method_args {
            push(&self.budget, &mut arguments, argument.clone())?;
        }
        let projected = self.signature_contains_projection(&item.schema.signature)?;
        let result = if projected {
            self.schema
                .instantiate_schema(&item.schema, &arguments, context.env(), &self.budget)?
        } else {
            self.instantiate_scratch_with_facts(
                &item.schema,
                &arguments,
                context,
                Some(item.declaration),
                scratch,
                context.original,
            )?
        };
        if item.schema.signature.params.len() != result.signature.params.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "generic trait method substitution changed parameter count",
            ));
        }
        for (raw, actual) in item
            .schema
            .signature
            .params
            .iter()
            .zip(&result.signature.params)
        {
            self.validate_rhs_substitution(raw, actual, &item.schema.env, &arguments, 0)?;
        }
        self.validate_rhs_substitution(
            &item.schema.signature.result,
            &result.signature.result,
            &item.schema.env,
            &arguments,
            0,
        )?;
        let result = if projected {
            self.instantiate_scratch_with_facts(
                &item.schema,
                &arguments,
                context,
                Some(item.declaration),
                scratch,
                context.original,
            )?
        } else {
            result
        };
        // Universal checking retains authenticated caller-bound projections.
        // Only a closed executable ABI must contain no symbolic projection.
        if context.phase == FactsPhase::Closed
            && self.signature_contains_projection(&result.signature)?
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: generic projected trait method executable ABI",
            ));
        }
        Ok(result)
    }
}
