use super::*;

impl ValidatedTraitWorld {
    /// A structural implementation can be selected without closing the caller's
    /// own parameters. Assumption membership alone never selects a provider.
    pub(super) fn universal_implementation_dispatch_evidence(
        &self,
        call: &TraitCall,
        caller: ProofContext<'_>,
        scratch: &mut ProofScratch,
    ) -> Result<Option<ProofEvidence>> {
        if caller.phase != FactsPhase::Universal
            || caller.source_metadata.is_some()
            || matches!(call.trait_ref.self_ty, Ty::Dyn(_) | Ty::Opaque(_))
            || (self.goal_is_closed_in(&call.trait_ref, caller)?
                && self.method_args_are_closed_in(call, caller)?)
        {
            return Ok(None);
        }
        match self.prove_implementation_in(
            &Predicate::Implements(call.trait_ref.clone()),
            caller,
            scratch,
        )? {
            ProofOutcome::NotProven => Ok(None),
            ProofOutcome::Proved(evidence) => {
                self.validate_universal_implementation_dispatch(call, caller, &evidence)?;
                Ok(Some(evidence))
            }
        }
    }

    pub(super) fn validate_universal_implementation_dispatch(
        &self,
        call: &TraitCall,
        caller: ProofContext<'_>,
        evidence: &ProofEvidence,
    ) -> Result<()> {
        self.check_proof_context_caller(caller)?;
        if caller.phase != FactsPhase::Universal
            || caller.original.is_none()
            || caller.source_metadata.is_some()
            || evidence.0.world != self.id
            || evidence.0.phase != caller.phase
            || evidence.0.environment != caller.predicates.index
            || evidence.0.executable_storage != caller.executable_storage
            || !caller.matches_source(&evidence.0.source_metadata)?
            || !matches!(evidence.0.origin, ProofOrigin::Implementation { .. })
            || !self.eq_predicate(
                &evidence.0.goal,
                &Predicate::Implements(call.trait_ref.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal implementation dispatch changed exact caller or canonical goal",
            ));
        }
        Ok(())
    }

    pub(super) fn seal_universal_implementation_dispatch(
        &self,
        call: &TraitCall,
        caller: ProofContext<'_>,
        facts: &ValidatedFacts,
        callee: &Callee,
        membership: ProofEvidence,
    ) -> Result<TraitDispatchEvidence> {
        self.validate_universal_implementation_dispatch(call, caller, &membership)?;
        let target = self.selected_trait_target(
            call,
            facts,
            caller,
            &membership,
            caller.executable_storage,
        )?;
        if !same_callee(callee, &Callee::Direct(target)) {
            return Err(FactsError::invalid(
                &self.budget,
                "universal implementation dispatch changed canonical provider or ordered arguments",
            ));
        }
        Ok(TraitDispatchEvidence::Ordinary(membership))
    }
}
