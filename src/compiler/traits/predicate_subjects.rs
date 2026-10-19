//! Predicate subjects are not executable carrier types. A raw projection may
//! name a logical bound without authorizing residual projections in codegen.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn signature_contains_projection(&self, signature: &Signature) -> Result<bool> {
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            if self.type_contains_projection(ty, 0)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn prove_projected_subject(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        let Ty::AssociatedType(projection) = &goal.self_ty else {
            return Ok(None);
        };
        if !self.goal_is_closed(&projection.trait_ref, context.env())? {
            // An open subject can only be proved from exact contextual facts.
            // The assumption pass ran before this helper.
            return Ok(Some(ProofOutcome::NotProven));
        }
        let (normal, reductions) = self.normalize_closed_projected_type(
            &goal.self_ty,
            context,
            !self.owner_normalization_finalized,
        )?;
        let normalized = Predicate::Implements(TraitRef {
            declaration: goal.declaration,
            self_ty: normal,
            args: goal.args.clone(),
        });
        let outcome = match self.prove_in(&normalized, context, active)? {
            ProofOutcome::NotProven => ProofOutcome::NotProven,
            ProofOutcome::Proved(evidence) => self.evidence(
                predicate,
                context,
                ProofOrigin::ProjectedSubject {
                    evidence,
                    reductions: reductions.into(),
                },
            )?,
        };
        Ok(Some(outcome))
    }

    pub(super) fn validate_projected_subject_evidence(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        origin: &ProofOrigin,
    ) -> Result<()> {
        let ProofOrigin::ProjectedSubject {
            evidence,
            reductions,
        } = origin
        else {
            return Ok(());
        };
        let (Predicate::Implements(original), Predicate::Implements(normalized)) =
            (predicate, &evidence.0.goal)
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected subject evidence changed predicate kind",
            ));
        };
        if evidence.0.world != self.id
            || evidence.0.environment != context.predicates.index
            || evidence.0.phase != context.phase
            || original.declaration != normalized.declaration
            || !self.eq_types(&original.args, &normalized.args, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected subject evidence lost caller or trait identity",
            ));
        }
        let (normal, replay) = self.normalize_closed_projected_type(
            &original.self_ty,
            context,
            !self.owner_normalization_finalized,
        )?;
        if replay.len() != reductions.len() || !self.eq_ty(&normal, &normalized.self_ty, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "projected subject evidence lost exact normalization",
            ));
        }
        for reduction in reductions.iter() {
            self.validate_projected_dependency(reduction, context)?;
        }
        Ok(())
    }

    pub(super) fn validate_predicate_ref_in(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        if !matches!(goal.self_ty, Ty::AssociatedType(_)) {
            return self.validate_ref_in(goal, env, maybe_unsized);
        }
        self.validate_ty_metadata_in(&goal.self_ty, env, 0)?;
        let info = self.info(goal.declaration)?;
        let header = self.trait_header(goal.declaration)?;
        if info.kind != DeclKind::Trait
            || goal.args.len().checked_add(1) != Some(header.env.total_arity())
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected predicate trait identity or arity changed",
            ));
        }
        for argument in goal.args.iter() {
            self.validate_ty_metadata_in(argument, env, 0)?;
            self.validate_actual(argument, env)?;
            charged(&self.budget, ir::reject_references(argument, &self.budget))?;
        }
        Ok(())
    }
}
