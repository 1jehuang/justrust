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
        for argument in owner_arguments {
            world.validate_executable_actual(argument, self.env())?;
        }
        if owner.env.total_arity() == 0 {
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
        let substituted =
            world.substitute_projected_schema(&schema, owner_arguments, self.env())?;
        let Ty::Closure(actual) = substituted.signature.result else {
            return Err(FactsError::invalid(
                &world.budget,
                "closed closure reconstruction lost its sealed type",
            ));
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
