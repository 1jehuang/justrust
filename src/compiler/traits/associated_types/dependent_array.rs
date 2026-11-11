//! Implementation witnesses for caller-owned symbolic array applications.
use super::*;

impl ValidatedTraitWorld {
    /// Preserve the asserted operand and replay its genuine caller assignment.
    /// This comparison never turns a raw symbolic projection into a premise.
    pub(in crate::compiler::traits) fn dependent_array_equality_rhs_matches(
        &self,
        normal: &Ty,
        rhs: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<bool> {
        if self.eq_ty(normal, rhs, 0)? {
            return Ok(true);
        }
        let Ty::AssociatedType(projection) = rhs else {
            return Ok(false);
        };
        if context.phase != FactsPhase::Universal
            || context.source_metadata.is_some()
            || !matches!(projection.trait_ref.self_ty, Ty::DependentArray { .. })
        {
            return Ok(false);
        }
        if let Some(declared) =
            self.declared_equality_rhs(projection, &context.predicates.predicates, context.env())?
        {
            self.require_in(
                &Predicate::ProjectionEq {
                    projection: projection.as_ref().clone(),
                    ty: declared.clone(),
                },
                context,
                active,
            )?;
            return self.eq_ty(normal, &declared, 0);
        }
        let Some(membership) =
            self.dependent_array_implementation(&projection.trait_ref, context, active)?
        else {
            return Ok(false);
        };
        let assigned =
            self.implementation_equality_rhs_with(projection, &membership, context, active)?;
        self.validate_equality_domain_in(projection, &assigned, context, active)?;
        self.eq_ty(normal, &assigned, 0)
    }

    /// A symbolic extent is not a closed array and cannot use closed selection.
    /// Authenticate its exact declaration coordinate before asking the ordinary
    /// implementation prover for an assignment witness in this caller domain.
    pub(in crate::compiler::traits) fn dependent_array_implementation(
        &self,
        goal: &TraitRef,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<ProofEvidence>> {
        let Ty::DependentArray { length, .. } = &goal.self_ty else {
            return Ok(None);
        };
        if context.phase != FactsPhase::Universal || context.source_metadata.is_some() {
            return Ok(None);
        }
        let parameter = ir::ParamId {
            owner: length.owner,
            index: length.index,
        };
        if context.env().parameter_kind(parameter)
            != Some(ir::GenericParamKind::Const(ir::ConstType::Int(
                ir::IntTy::Usize,
            )))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "dependent array projection changed its caller extent coordinate",
            ));
        }
        self.validate_ty_metadata_in(&goal.self_ty, context.env(), 0)?;
        match self.prove_implementation_in(&Predicate::Implements(goal.clone()), context, active)? {
            ProofOutcome::NotProven => Ok(None),
            ProofOutcome::Proved(proof) => {
                if !matches!(&proof.0.origin, ProofOrigin::Implementation { .. }) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "dependent array projection requires an implementation assignment",
                    ));
                }
                Ok(Some(proof))
            }
        }
    }
}
