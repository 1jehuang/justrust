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
        active: &mut ProofScratch,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        let Ty::AssociatedType(projection) = &goal.self_ty else {
            return Ok(None);
        };
        let closed = self.goal_is_closed_in_with(&projection.trait_ref, context, active)?;
        if !closed
            && self
                .opaque_associated_binding(projection, context.env())?
                .is_none()
        {
            // A rigid projection is not a concrete normalization target, but
            // it may still satisfy an ordinary blanket implementation. Keep
            // its canonical owner and ordered arguments intact so the impl's
            // substituted predicates must be proved in this exact caller.
            // An assumption may discharge a blanket prerequisite without
            // authorizing a concrete layout for the projected type.
            self.validate_universal_projection_with(projection, context.env(), active)?;
            return Ok(None);
        }
        let (normal, reductions) = self.normalize_closed_projected_type_for_use(
            &goal.self_ty,
            context,
            !self.owner_normalization_finalized,
            active,
            context.executable_storage,
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
            || evidence.0.executable_storage != context.executable_storage
            || evidence.0.phase != context.phase
            || original.declaration != normalized.declaration
            || !self.eq_types(&original.args, &normalized.args, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected subject evidence lost caller or trait identity",
            ));
        }
        let (normal, replay) = self.normalize_closed_projected_type_for_use(
            &original.self_ty,
            context,
            !self.owner_normalization_finalized,
            &mut ProofScratch::default(),
            context.executable_storage,
        )?;
        if replay.len() != reductions.len() || !self.eq_ty(&normal, &normalized.self_ty, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "projected subject evidence lost exact normalization",
            ));
        }
        // Replay validates each reduction under the same staging it was
        // produced with (pre-finalization owner normalization is staged).
        for reduction in reductions.iter() {
            self.validate_staged_projected_dependency(
                reduction,
                context,
                !self.owner_normalization_finalized,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_predicate_ref_with_facts_for_use(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
        executable_storage: bool,
    ) -> Result<()> {
        if !matches!(goal.self_ty, Ty::AssociatedType(_)) {
            return self.validate_ref_with_facts_for_use(
                goal,
                env,
                maybe_unsized,
                replays,
                caller,
                executable_storage,
            );
        }
        self.validate_ty_metadata_with_facts(&goal.self_ty, env, 0, replays, caller)?;
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
            self.validate_ty_metadata_with_facts(argument, env, 0, replays, caller)?;
            if caller.is_none() && executable_storage {
                self.validate_actual(argument, env)?;
            } else if matches!(argument, Ty::Str) {
                charged(&self.budget, self.budget.charge_work(1))?;
            } else {
                self.validate_predicate_formation_for_use(
                    argument,
                    env,
                    TypeUse::Actual,
                    None,
                    replays,
                    caller,
                    executable_storage,
                )?;
            }
            charged(&self.budget, ir::reject_references(argument, &self.budget))?;
        }
        Ok(())
    }
}
