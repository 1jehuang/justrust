//! Builtin callable evidence for zero-sized function items. The item's exact
//! declaration and ordered actuals select its instantiated signature; the
//! callable tuple is the ordered parameter list and `Output` is the result.
use super::*;

impl ValidatedTraitWorld {
    /// The instantiated signature of a function item. Unsafe functions and
    /// reference-bearing signatures are not admitted as callable values yet.
    pub(super) fn fn_item_signature(
        &self,
        item: &CallTarget,
        context: ProofContext<'_>,
    ) -> Result<Signature> {
        let proof = self.discharge_in(item, context, &mut ProofScratch::default())?;
        let signature = proof.signature().clone();
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            if !charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.is_empty() {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: function items with reference-bearing signatures as callable values",
                ));
            }
        }
        Ok(signature)
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
