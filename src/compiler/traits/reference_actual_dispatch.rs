//! Executable body selection for reference-actual trait item calls.
//!
//! A universal caller may call a trait item whose trait goal is closed (an
//! exact `Range<i32>: Iterator`) while its method actuals still mention the
//! caller's own rigid types (`filter_map::<&C::Item, _>`). Such a call has the
//! same unique executable provider as a Direct call: the selected
//! implementation item or the trait's provided default body. Reference-actual
//! admission reuses exactly that selection so the callee's complete universal
//! family proof is demanded through the same path as Direct callees. Open,
//! object, opaque and callable-builtin goals select no body here and remain
//! fail-closed for their callers.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn reference_actual_dispatch_target(
        &self,
        callee: &Callee,
    ) -> Result<Option<CallTarget>> {
        let call = match callee {
            Callee::Direct(target) => return Ok(Some(target.clone())),
            Callee::TraitItem(call) => call,
            Callee::Virtual(_) | Callee::Untupled(_) => return Ok(None),
        };
        self.world.check_facts(self, true)?;
        if matches!(
            &call.trait_ref.self_ty,
            Ty::Dyn(_)
                | Ty::Opaque(_)
                | Ty::Closure(_)
                | Ty::FnItem(_)
                | Ty::FnPointer(_)
                | Ty::DependentArray { .. }
        ) {
            return Ok(None);
        }
        let world = &self.world;
        let item = world.item_header(call.item)?;
        if item.trait_decl != call.trait_ref.declaration
            || call.method_args.len() != world.info(item.declaration)?.own_env.arity()
        {
            return Err(FactsError::invalid(
                self.budget(),
                "reference-actual trait call changed item owner or method arity",
            ));
        }
        let caller = ProofContext::from_facts(self).for_storage(false);
        if !world.goal_is_closed_in(&call.trait_ref, caller)? {
            return Ok(None);
        }
        let closed = caller.closed_selection(world);
        let predicate = Predicate::Implements(call.trait_ref.clone());
        let mut scratch = ProofScratch::default();
        let evidence = world.require_in(&predicate, closed, &mut scratch)?;
        if evidence.0.world != world.id
            || evidence.0.environment != world.empty.index
            || evidence.0.phase != FactsPhase::Closed
            || !world.eq_predicate(&evidence.0.goal, &predicate)?
        {
            return Err(FactsError::invalid(
                self.budget(),
                "reference-actual dispatch evidence changed the exact closed goal",
            ));
        }
        if !matches!(&evidence.0.origin, ProofOrigin::Implementation { .. }) {
            return Ok(None);
        }
        world
            .selected_trait_target(call, self, closed, &evidence, false)
            .map(Some)
    }
}
