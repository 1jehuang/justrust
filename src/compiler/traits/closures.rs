//! Builtin callable evidence is keyed by the source closure's canonical owner,
//! ordered owner actuals, site and immutable checked signature.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn closure_lowering_identity(
        &self,
    ) -> Option<(Arc<ir::ClosureTy>, ir::ClosureKind)> {
        self.closure.clone().zip(self.closure_mode)
    }
    pub(in crate::compiler) fn validate_closure_body_source(
        &self,
        closure: &Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
        source: &ir::CheckedFunction,
    ) -> Result<()> {
        self.world.check_facts(self, true)?;
        if self.closure_mode != Some(mode) || self.declaration() != closure.provider() {
            return Err(FactsError::invalid(
                self.budget(),
                "closure lowering changed provider or receiver mode",
            ));
        }
        let original = self.closure.as_ref().ok_or_else(|| {
            FactsError::invalid(
                self.budget(),
                "closure lowering has no checked parent metadata",
            )
        })?;
        if !self.world.eq_ty(
            &Ty::Closure(original.clone()),
            &Ty::Closure(closure.clone()),
            0,
        )? {
            return Err(FactsError::invalid(
                self.budget(),
                "closure lowering changed canonical parent metadata",
            ));
        }
        self.world.validate_closure_metadata(closure, self.env())?;
        let mut params = Vec::new();
        for local in &source.params {
            let ty = source.locals.get(local.0).ok_or_else(|| {
                FactsError::invalid(self.budget(), "closure lowering parameter local is invalid")
            })?;
            push(self.budget(), &mut params, ty.ty.clone())?;
        }
        self.validate_signature(
            SignatureSubject::Body,
            &Signature {
                params,
                result: source.result.clone(),
                ref_contract: source.ref_contract.clone(),
            },
        )?;
        if let Some(receipt) = closure.result_receipt() {
            super::super::frontend::closures::result_contract::validate_source(
                receipt, source, self,
            )
            .map_err(|message| FactsError::invalid(self.budget(), &message))?;
        }
        charged(
            self.budget(),
            source.validate_with_facts(ir::FactsCtx::from_validated(self)),
        )?;
        Ok(())
    }
}

impl ValidatedTraitWorld {
    pub(in crate::compiler) fn closure_trait_kind(
        &self,
        declaration: DeclId,
    ) -> Result<Option<ir::ClosureKind>> {
        self.schema.closure_trait_kind(declaration)
    }
    pub(super) fn validate_closure_metadata(
        &self,
        closure: &ir::ClosureTy,
        env: &GenericEnv,
    ) -> Result<()> {
        self.replay_closure_owner_substitution(closure, env)?;
        if self.schema.closure_owner_site(closure.provider())?
            != Some((closure.owner(), closure.site()))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure type changed its original parent or source site",
            ));
        }
        let owner = self.info(closure.owner())?;
        if owner.env.total_arity() != closure.owner_args().len() {
            return Err(FactsError::invalid(
                &self.budget,
                "closure owner instantiation arity mismatch",
            ));
        }
        self.closure_provider_prefix(closure.provider())?;
        if closure.sources().len() != closure.captures().len() {
            return Err(FactsError::invalid(
                &self.budget,
                "closure capture authorization arity mismatch",
            ));
        }
        for (source, ty) in closure.sources().iter().zip(closure.captures()) {
            if source.place.projection.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure capture projection depth exceeds limit",
                ));
            }
            let valid = match source.kind {
                ir::CaptureKind::Value => true,
                ir::CaptureKind::Shared => matches!(
                    ty,
                    Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        ..
                    }
                ),
                ir::CaptureKind::Mut => matches!(
                    ty,
                    Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        ..
                    }
                ),
            };
            if !valid {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure capture mode differs from environment field",
                ));
            }
        }
        let mut replays = NominalReplays::default();
        for ty in closure.types() {
            self.validate_ty_metadata_with(ty, env, 0, &mut replays)?;
        }
        if let Some(receipt) = closure.result_receipt() {
            if receipt.owner() != closure.owner()
                || receipt.provider() != closure.provider()
                || receipt.site() != closure.site()
                || !self.eq_ty(
                    &self.realize_closure_result_witness(closure, receipt)?.1,
                    closure.result(),
                    0,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure result receipt changed its source shape",
                ));
            }
            let result_channels = charged(
                &self.budget,
                ir::carrier_locations(closure.result(), &self.budget),
            )?;
            if !result_channels.is_empty() && !matches!(closure.result(), Ty::Param(_)) {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "closure restricted result cannot admit reference-bearing executable substitution",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn closure_arguments(&self, closure: &ir::ClosureTy) -> Result<Ty> {
        retain::<Ty>(&self.budget, closure.params().len())?;
        Ok(if closure.params().is_empty() {
            Ty::Unit
        } else {
            Ty::Tuple(closure.params().into())
        })
    }

    pub(super) fn prove_closure(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        let Ty::Closure(closure) = &goal.self_ty else {
            return Ok(None);
        };
        let Some(kind) = self.schema.closure_trait_kind(goal.declaration)? else {
            return Ok(None);
        };
        self.validate_closure_metadata(closure, context.env())?;
        if !closure.supports(kind) || goal.args.len() != 1 {
            return Ok(Some(ProofOutcome::NotProven));
        }
        let arguments = self.map_universal_projection_type(
            &goal.args[0],
            context.env(),
            0,
            &mut |projection| {
                if let Some(ty) = self.declared_equality_rhs(
                    projection,
                    &context.predicates.predicates,
                    context.env(),
                )? {
                    self.require_in(
                        &Predicate::ProjectionEq {
                            projection: projection.clone(),
                            ty: ty.clone(),
                        },
                        context,
                        active,
                    )?;
                    Ok(ty)
                } else if matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_)) {
                    self.normalize_closed_projected_type_with(
                        &Ty::AssociatedType(Arc::new(projection.clone())),
                        context,
                        false,
                        active,
                    )
                    .map(|(normal, _)| normal)
                } else {
                    Ok(Ty::AssociatedType(Arc::new(projection.clone())))
                }
            },
        )?;
        if !self.eq_ty(&arguments, &self.closure_arguments(closure)?, 0)? {
            return Ok(Some(ProofOutcome::NotProven));
        }
        self.require_closure_owner_predicates(closure, context, active)?;
        Ok(Some(self.evidence(
            predicate,
            context,
            ProofOrigin::Closure { kind },
        )?))
    }

    pub(super) fn closure_target(
        &self,
        closure: &Arc<ir::ClosureTy>,
        kind: ir::ClosureKind,
    ) -> Result<CallTarget> {
        self.closure_target_in_owner_domain(closure, kind)
    }

    pub(super) fn validate_closure_direct(
        &self,
        target: &CallTarget,
        context: ProofContext<'_>,
    ) -> Result<()> {
        if self
            .schema
            .closure_owner_site(target.declaration)?
            .is_none()
        {
            return Ok(());
        }
        let adapter = self.decode_closure_adapter(target)?;
        self.validate_closure_metadata(adapter.closure, context.env())?;
        // discharge_in separately instantiates this provider's full inherited
        // schema and requires every predicate, including the parent's bounds.
        Ok(())
    }

    pub(in crate::compiler) fn closure_callable_facts_for(
        self: &Arc<Self>,
        closure: Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
    ) -> Result<Arc<ValidatedFacts>> {
        let mut facts = self.callable_facts(closure.provider())?.as_ref().clone();
        self.validate_closure_metadata(&closure, facts.env())?;
        if !closure.supports(mode) {
            return Err(FactsError::invalid(
                &self.budget,
                "closure receiver capability is unsupported",
            ));
        }
        facts.closure_mode = Some(mode);
        facts.closure = Some(closure);
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(facts))
    }

    pub(in crate::compiler) fn closure_body_signature(
        &self,
        facts: &ValidatedFacts,
    ) -> Result<Signature> {
        let closure = facts.closure.as_ref().ok_or_else(|| {
            FactsError::invalid(&self.budget, "missing checked closure body metadata")
        })?;
        let arguments = self
            .closure_body_arguments(facts)?
            .ok_or_else(|| FactsError::invalid(&self.budget, "missing closure body arguments"))?;
        let header = self.callable_header(closure.provider())?;
        Ok(self
            .instantiate_in(
                &header.schema,
                &arguments,
                ProofContext::from_facts(facts),
                Some(closure.provider()),
            )?
            .signature)
    }

    pub(super) fn closure_body_arguments(
        &self,
        facts: &ValidatedFacts,
    ) -> Result<Option<Arc<[Ty]>>> {
        let Some(closure) = &facts.closure else {
            return Ok(None);
        };
        let mode = facts.closure_mode.ok_or_else(|| {
            FactsError::invalid(&self.budget, "closure body lacks receiver capability")
        })?;
        Ok(Some(self.closure_target(closure, mode)?.type_args))
    }
}
