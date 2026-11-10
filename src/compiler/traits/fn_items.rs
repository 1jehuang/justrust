//! Builtin callable evidence for zero-sized function items. The item's exact
//! declaration and ordered actuals select its instantiated signature; the
//! callable tuple is the ordered parameter list and `Output` is the result.
use super::*;

impl ValidatedTraitWorld {
    /// The instantiated signature of a function item, with its reference
    /// contract. Reference-bearing parameters are admitted; a result that
    /// borrows from an input is still rejected downstream with an explicit
    /// diagnostic.
    pub(super) fn fn_item_signature(
        &self,
        item: &CallTarget,
        context: ProofContext<'_>,
    ) -> Result<Signature> {
        let proof = self.discharge_in(item, context, &mut ProofScratch::default())?;
        Ok(proof.signature().clone())
    }

    pub(super) fn fn_item_arguments(&self, signature: &Signature) -> Result<Ty> {
        retain::<Ty>(&self.budget, signature.params.len())?;
        Ok(if signature.params.is_empty() {
            Ty::Unit
        } else {
            Ty::Tuple(signature.params.clone().into())
        })
    }

    pub(super) fn prove_fn_item(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        let Ty::FnItem(item) = &goal.self_ty else {
            return Ok(None);
        };
        let Some(kind) = self.schema.closure_trait_kind(goal.declaration)? else {
            return Ok(None);
        };
        let signature = self.fn_item_signature(item, context)?;
        if goal.args.len() != 1
            || !self.eq_ty(&goal.args[0], &self.fn_item_arguments(&signature)?, 0)?
        {
            return Ok(Some(ProofOutcome::NotProven));
        }
        Ok(Some(self.evidence(
            predicate,
            context,
            ProofOrigin::FnItem { kind },
        )?))
    }
}

impl ValidatedTraitWorld {
    /// Builtin untupled dispatch: the exact `FnItem: Fn*<(A..)>` goal is
    /// proved, the trait item signature `(receiver, (A..)) -> R` is returned,
    /// and the executed item's signature must be exactly `(A..) -> R`.
    pub(super) fn validated_untupled(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<(Signature, ir::DirectCallProofView)> {
        let Ty::FnItem(item) = &call.trait_ref.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call receiver is not a function item",
            ));
        };
        if !call.method_args.is_empty()
            || self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_none()
            || self.item_header(call.item)?.trait_decl != call.trait_ref.declaration
        {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call is not a builtin callable trait item",
            ));
        }
        let context = ProofContext::from_facts(facts);
        match self.prove_fn_item(&Predicate::Implements(call.trait_ref.clone()), context)? {
            Some(ProofOutcome::Proved(_)) => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "untupled call lacks function item callable evidence",
                ));
            }
        }
        let direct = self.discharge_in(item, context, &mut ProofScratch::default())?;
        let trait_signature = self.instantiate_trait_signature(call, facts)?;
        let arguments = self.fn_item_arguments(direct.signature())?;
        if trait_signature.params.len() != 2
            || !self.eq_ty(&trait_signature.params[1], &arguments, 0)?
            || !self.eq_ty(&trait_signature.result, &direct.signature().result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call signature differs from its function item",
            ));
        }
        Ok((
            trait_signature,
            ir::DirectCallProofView {
                target: direct.target().clone(),
                signature: direct.signature().clone(),
            },
        ))
    }
}

impl ValidatedTraitWorld {
    /// The callable tuple input of a function item is exactly its ordered
    /// parameter tuple, references included.
    pub(super) fn closed_fn_item_tuple_input(
        &self,
        goal: &TraitRef,
        signature: &Signature,
        index: usize,
        context: ProofContext<'_>,
    ) -> Result<bool> {
        let Ty::FnItem(item) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "function item tuple proof lost its item type",
            ));
        };
        let direct = self.fn_item_signature(item, context)?;
        let tuple = self.fn_item_arguments(&direct)?;
        if goal.args.len() != 1
            || signature.params.get(index).is_none()
            || !self.eq_ty(&signature.params[index], &tuple, 0)?
            || !self.eq_ty(&goal.args[0], &tuple, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item callable tuple differs from its signature",
            ));
        }
        Ok(true)
    }
}
