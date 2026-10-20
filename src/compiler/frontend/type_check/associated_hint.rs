//! Associated result hints use the same owner and method-argument inference as
//! ordinary checked calls. A hint does not select a weaker arity-only schema.
use super::*;

#[cfg(test)]
#[path = "associated_hint_tests.rs"]
mod tests;

impl Checker<'_> {
    pub(super) fn associated_result_hint(
        &self,
        owner: &Type,
        name: &AstIdent,
        type_args: &[Type],
        arguments: &[Expr],
    ) -> Result<Option<ir::Ty>, String> {
        let owner = if inferred_owner::has_inferred_owner(owner) {
            match self.infer_associated_owner(owner, name, type_args.len(), arguments, None) {
                Ok(owner) => owner,
                // An unresolved owner has no useful scalar hint. The ordinary
                // expression path still diagnoses insufficient owner evidence.
                Err(_) => return Ok(None),
            }
        } else {
            checked_local_ty(owner, self.signatures)?
        };
        let actuals = self.resolved_method_actuals(&owner, name, type_args, arguments, None)?;
        let plan = self.signatures.plan()?;
        let (target, signature, _) = plan.select_associated(
            &owner,
            name,
            &actuals,
            self.facts(),
            &self.module,
            false,
            &plan.budget,
        )?;
        // A chained receiver must use the same representation as the mapped
        // method signature. Replay the complete genuine call before extracting
        // its result hint, rather than converting an unproven nominal shell.
        // Final call construction still owns the original/mapped source receipt.
        let signature = match self.nominal_call_signature(&target, signature)? {
            nominal_calls::NominalCallSignature::Ready { signature, .. } => signature,
            nominal_calls::NominalCallSignature::Blocked(blocked) => {
                *self.nominal_blocked.borrow_mut() = Some(blocked);
                return Err(
                    "missing graph input: associated result hint signature realization".into(),
                );
            }
        };
        Ok(Some(signature.result))
    }
}
