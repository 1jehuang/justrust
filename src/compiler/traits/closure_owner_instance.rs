//! Borrowed reconstruction of a sealed closure under canonical owner actuals.
//!
//! This is metadata substitution authority, not a replacement for graph-reading
//! and replaying the original parent publication and mode-specific checked body.
use super::*;

#[cfg(test)]
#[path = "closure_owner_instance_tests.rs"]
mod tests;

impl ValidatedFacts {
    pub(in crate::compiler) fn instantiate_closure_owner(
        &self,
        original: &Arc<ir::ClosureTy>,
        owner_arguments: &[Ty],
    ) -> Result<Arc<ir::ClosureTy>> {
        self.instantiate_closure_owner_inner(original, owner_arguments, None)
    }

    pub(in crate::compiler) fn instantiate_closure_owner_with_reads(
        &self,
        original: &Arc<ir::ClosureTy>,
        owner_arguments: &[Ty],
        context: &mut crate::compiler::frontend::PlanCtx,
        engine: crate::engine::EngineAuthority,
    ) -> Result<Arc<ir::ClosureTy>> {
        let context_world = context
            .trait_world()
            .map_err(|error| FactsError::from_message(&self.world.budget, &error))?;
        if !Arc::ptr_eq(&context_world, &self.world) {
            return Err(FactsError::invalid(
                &self.world.budget,
                "closure owner typed reads changed their source world",
            ));
        }
        self.instantiate_closure_owner_inner(original, owner_arguments, Some((context, engine)))
    }

    fn instantiate_closure_owner_inner(
        &self,
        original: &Arc<ir::ClosureTy>,
        owner_arguments: &[Ty],
        reads: Option<(
            &mut crate::compiler::frontend::PlanCtx,
            crate::engine::EngineAuthority,
        )>,
    ) -> Result<Arc<ir::ClosureTy>> {
        let world = &self.world;
        world.check_facts(self, true)?;
        if self.phase() != FactsPhase::Closed {
            return Err(FactsError::invalid(
                &world.budget,
                "closed closure reconstruction requires actual closed caller facts",
            ));
        }
        let owner = world.info(original.owner())?;
        world.replay_closure_owner_substitution(original, &owner.env)?;
        if original.owner_args().len() != owner.env.total_arity()
            || owner_arguments.len() != owner.env.total_arity()
        {
            return Err(FactsError::invalid(
                &world.budget,
                "closed closure reconstruction changed its ordered owner arity",
            ));
        }
        for (parameter, original_argument) in owner.env.ordered_params().zip(original.owner_args())
        {
            charged(&world.budget, world.budget.charge_work(1))?;
            if !matches!(original_argument, Ty::Param(actual) if *actual == parameter) {
                return Err(FactsError::invalid(
                    &world.budget,
                    "closed closure reconstruction requires the original owner template",
                ));
            }
        }
        let mut read_scratch = TraitScratch::default();
        for argument in owner_arguments {
            world.validate_executable_actual(argument, self.env())?;
            if let Some((_, engine)) = reads.as_ref() {
                crate::compiler::nominal_realization::substitution::replay_closed_storage_with(
                    *engine,
                    argument,
                    self,
                    &world.budget,
                    &mut read_scratch,
                )
                .map_err(|error| FactsError::from_message(&world.budget, &error))?;
            }
        }
        if owner.env.total_arity() == 0 && reads.is_none() {
            // A non-generic owner has no parameters to substitute: the sealed
            // template is already the exact closed type. Re-running the kernel
            // would only re-derive closed graph nominals it may not retain.
            world.validate_closure_metadata(original, self.env())?;
            world.require_closure_owner_predicates(
                original,
                ProofContext::from_facts(self),
                &mut ProofScratch::default(),
            )?;
            return Ok(original.clone());
        }
        // This local schema is only a container for the existing simultaneous
        // metadata substitution kernel. It is never published as a declaration,
        // signature or executable proof. That kernel independently replays each
        // resulting type against the retained source template and ordered args.
        let schema = GenericSchema {
            env: owner.env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: Ty::Closure(original.clone()),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let substituted = match reads {
            Some((context, engine)) => {
                crate::compiler::nominal_realization::substitution::substitute_ty_with_scratch(
                    &schema.signature.result,
                    &schema.env,
                    owner_arguments,
                    ir::FactsCtx::Validated(self),
                    context,
                    &world.budget,
                    Some(engine),
                    &mut TraitScratch::default(),
                )
                .map_err(|error| FactsError::from_message(&world.budget, &error))?
            }
            None => {
                world
                    .substitute_projected_schema(&schema, owner_arguments, self.env())?
                    .signature
                    .result
            }
        };
        let Ty::Closure(actual) = substituted else {
            return Err(FactsError::invalid(
                &world.budget,
                "closed closure reconstruction lost its sealed type",
            ));
        };
        // Substituting a caller projection (`&C::Item` with `C := S`) leaves a
        // closed family `<S as K>::Item` in the closure's signature/captures.
        // A closed instance names the solver-selected normal form, exactly as
        // every closed call contract does. Residual or open families stay as
        // they are and remain rejected by realization.
        let actual = if world.type_contains_projection(&Ty::Closure(actual.clone()), 0)?
            && world.raw_associated_ty_is_closed(&Ty::Closure(actual.clone()), 0)?
        {
            let (normal, _) = world.normalize_closed_projected_type(
                &Ty::Closure(actual.clone()),
                ProofContext::from_facts(self).closed_selection(world),
                false,
            )?;
            let Ty::Closure(normal) = normal else {
                return Err(FactsError::invalid(
                    &world.budget,
                    "closed closure normalization changed its type kind",
                ));
            };
            normal
        } else {
            actual
        };
        world.validate_closure_rhs_substitution(
            original,
            &actual,
            &owner.env,
            owner_arguments,
            0,
        )?;
        world.validate_closure_metadata(&actual, self.env())?;
        world.require_closure_owner_predicates(
            &actual,
            ProofContext::from_facts(self),
            &mut ProofScratch::default(),
        )?;
        Ok(actual)
    }
}
