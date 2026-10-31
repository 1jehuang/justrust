//! Opaque return bounds are authenticated against the defining callable schema.
//! This module never reads a defining body or its hidden representation.
use super::*;

impl ValidatedTraitWorld {
    /// A normalized opaque descriptor remains tied to its original source
    /// equation. This bounded replay uses no caller assumptions: only exact
    /// source substitution followed by independently proven closed reductions
    /// may differ from structural correspondence. In particular generic
    /// membership is never substituted for a projection assignment.
    fn validate_opaque_payload_substitution(
        &self,
        raw: &Ty,
        actual: &Ty,
        source_env: &GenericEnv,
        arguments: &[Ty],
        ambient_env: &GenericEnv,
    ) -> Result<()> {
        let structural = self.validate_rhs_substitution(raw, actual, source_env, arguments, 0);
        let error = match structural {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => error,
            Err(error) => return Err(error),
        };
        if !self.type_contains_projection(raw, 0)? {
            return Err(error);
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        retain::<GenericEnv>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: Arc::new(source_env.clone()),
            signature: Signature {
                params: Vec::new(),
                result: raw.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let substituted =
            self.schema
                .instantiate_schema(&schema, arguments, ambient_env, &self.budget)?;
        self.validate_rhs_substitution(
            raw,
            &substituted.signature.result,
            source_env,
            arguments,
            0,
        )?;
        let proof = ProofContext {
            original: None,
            predicates: &self.empty,
            phase: FactsPhase::Schema,
        };
        let (normal, dependencies) =
            self.normalize_closed_projected_type(&substituted.signature.result, proof, false)?;
        // The closed replay is not allowed to bless an unchanged residual
        // family, even when its ordinary membership happened to be admissible.
        if dependencies.is_empty() || self.type_contains_projection(&normal, 0)? {
            return Err(error);
        }
        for dependency in &dependencies {
            self.validate_projected_dependency(dependency, proof)?;
        }
        if !self.eq_ty(&normal, actual, 0)? {
            return Err(error);
        }
        Ok(())
    }

    /// Public associated assignments are authenticated declaration metadata,
    /// never a query for the defining body's hidden representation.
    pub(super) fn opaque_associated_binding(
        &self,
        projection: &ir::AssociatedTypeProjection,
        env: &GenericEnv,
    ) -> Result<Option<Ty>> {
        let Ty::Opaque(opaque) = &projection.trait_ref.self_ty else {
            return Ok(None);
        };
        self.validate_opaque_metadata(opaque, env)?;
        if self.associated_header(projection.item)?.trait_decl != projection.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque projection changed its associated owner",
            ));
        }
        let mut result = None;
        for bound in opaque.bounds() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let application = bound.application(projection.trait_ref.self_ty.clone());
            if !self.eq_predicate(
                &Predicate::Implements(application),
                &Predicate::Implements(projection.trait_ref.clone()),
            )? {
                continue;
            }
            for (item, rhs) in bound.equalities() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if *item != projection.item {
                    continue;
                }
                self.validate_ty_metadata_in(rhs, env, 0)?;
                if let Some(previous) = &result
                    && !self.eq_ty(previous, rhs, 0)?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque associated assignments conflict",
                    ));
                }
                result = Some(rhs.clone());
            }
        }
        Ok(result)
    }
    pub(super) fn realize_opaque_family(
        &self,
        opaque: &Arc<ir::OpaqueTy>,
        env: &GenericEnv,
        args: &[Ty],
        facts: &ValidatedFacts,
    ) -> Result<Ty> {
        self.check_facts(facts, false)?;
        self.validate_opaque_metadata(opaque, env)?;
        let schema = GenericSchema {
            env: Arc::new(env.clone()),
            signature: Signature {
                params: Vec::new(),
                result: Ty::Opaque(opaque.clone()),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let actual = self
            .substitute_projected_schema(&schema, args, facts.env())?
            .signature
            .result;
        let actual = facts.normalize_substituted_projection_type(&actual, &self.budget)?;
        let Ty::Opaque(opaque) = &actual else {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque family substitution changed its outer declaration identity",
            ));
        };
        self.validate_opaque_metadata(opaque, facts.env())?;
        Ok(actual)
    }
    pub(super) fn prove_opaque(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
    ) -> Result<Option<ProofOutcome>> {
        let Ty::Opaque(opaque) = &predicate.trait_ref().self_ty else {
            return Ok(None);
        };
        self.validate_opaque_metadata(opaque, context.env())?;
        let mut declared = Vec::new();
        for bound in opaque.bounds() {
            let application = bound.application(predicate.trait_ref().self_ty.clone());
            self.append_predicate(&mut declared, &Predicate::Implements(application.clone()))?;
            for (item, ty) in bound.equalities() {
                self.append_predicate(
                    &mut declared,
                    &Predicate::ProjectionEq {
                        projection: ir::AssociatedTypeProjection {
                            trait_ref: application.clone(),
                            item: *item,
                        },
                        ty: ty.clone(),
                    },
                )?;
            }
        }
        // Return-position opaque types are Sized unless an explicit relaxed
        // bound is retained. That syntax is rejected until represented here.
        self.append_predicate(
            &mut declared,
            &Predicate::Implements(TraitRef {
                declaration: self.catalog.id(CatalogItem::Sized),
                self_ty: predicate.trait_ref().self_ty.clone(),
                args: Arc::from([]),
            }),
        )?;
        self.close_predicates(&mut declared, context.env())?;
        for requirement in &declared {
            if self.eq_predicate(requirement, predicate)? {
                return self
                    .evidence(predicate, context, ProofOrigin::OpaqueDeclared)
                    .map(Some);
            }
        }
        // A blanket impl may still establish a trait from the public bounds.
        // Ordinary implementation selection never receives the hidden type.
        Ok(None)
    }

    pub(super) fn validate_opaque_metadata(
        &self,
        opaque: &ir::OpaqueTy,
        env: &GenericEnv,
    ) -> Result<()> {
        let owner = self.info(opaque.owner())?;
        if !matches!(
            owner.kind,
            DeclKind::Function | DeclKind::AssociatedFunction
        ) || owner.env.total_arity() != opaque.owner_args().len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque return changed its defining callable or argument arity",
            ));
        }
        for actual in opaque.owner_args() {
            self.validate_ty_metadata_in(actual, env, 0)?;
        }
        let schema = self.schema.declaration_schema(opaque.owner())?;
        let original = self
            .opaque_occurrence(&schema.signature.result, opaque.owner(), opaque.site(), 0)?
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "opaque return occurrence is absent from its defining result schema",
                )
            })?;
        if !original.try_eq(opaque, |raw, actual| {
            self.validate_opaque_payload_substitution(
                raw,
                actual,
                &owner.env,
                opaque.owner_args(),
                env,
            )?;
            Ok::<_, FactsError>(true)
        })? {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque return changed its declaration-owned bounds or capture identities",
            ));
        }
        // The enclosing type walk validates every bound child separately.
        // Revalidating a complete bound application here would revisit its
        // opaque Self and recurse into this same declaration check.
        Ok(())
    }

    /// Declaration-only eligibility. This reads no defining body or hidden type.
    /// The current edition-2024 registry captures its complete ordered generic
    /// environment. A substituted argument is eligible only through its original
    /// declaration-owned parameter and exact schema substitution correspondence.
    pub(super) fn opaque_captures_parameter(
        &self,
        opaque: &ir::OpaqueTy,
        parameter: ir::ParamId,
    ) -> Result<bool> {
        let owner = self.info(opaque.owner())?;
        if !matches!(
            owner.kind,
            DeclKind::Function | DeclKind::AssociatedFunction
        ) || owner.env.total_arity() != opaque.owner_args().len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque capture owner or argument arity changed",
            ));
        }
        let schema = self.schema.declaration_schema(opaque.owner())?;
        let original = self
            .opaque_occurrence(&schema.signature.result, opaque.owner(), opaque.site(), 0)?
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "opaque capture occurrence is absent from its declaration",
                )
            })?;
        // Capture eligibility authenticates only exact structural substitution.
        // It has no ambient caller environment to authorize normalized payloads.
        // Closed normalization remains a separate validate_opaque_metadata check
        // under its genuine ambient environment, not the defining owner here.
        if !original.try_eq(opaque, |raw, actual| {
            self.validate_rhs_substitution(raw, actual, &owner.env, opaque.owner_args(), 0)?;
            Ok::<_, FactsError>(true)
        })? {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque capture descriptor changed its declared bounds or regions",
            ));
        }
        let mut eligible = false;
        for ((formal, raw), actual) in owner
            .env
            .ordered_params()
            .zip(original.owner_args())
            .zip(opaque.owner_args())
        {
            charged(&self.budget, self.budget.charge_work(1))?;
            if raw != &Ty::Param(formal) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "opaque capture registry changed its ordered formal identity",
                ));
            }
            eligible |= actual == &Ty::Param(parameter);
        }
        Ok(eligible)
    }

    fn opaque_occurrence<'a>(
        &self,
        ty: &'a Ty,
        owner: DeclId,
        site: u32,
        depth: usize,
    ) -> Result<Option<&'a ir::OpaqueTy>> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Opaque(opaque) => {
                if opaque.owner() == owner && opaque.site() == site {
                    return Ok(Some(opaque));
                }
                for child in opaque.types() {
                    if let Some(found) = self.opaque_occurrence(child, owner, site, depth + 1)? {
                        return Ok(Some(found));
                    }
                }
            }
            Ty::Ref { referent, .. } => {
                return self.opaque_occurrence(referent, owner, site, depth + 1);
            }
            Ty::RawPtr { pointee, .. } => {
                return self.opaque_occurrence(pointee, owner, site, depth + 1);
            }
            Ty::Array { element, .. } | Ty::Slice { element } => {
                return self.opaque_occurrence(element, owner, site, depth + 1);
            }
            Ty::Tuple(_) | Ty::Struct(_) | Ty::Enum(_) => {
                for child in type_arguments(ty) {
                    if let Some(found) = self.opaque_occurrence(child, owner, site, depth + 1)? {
                        return Ok(Some(found));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod capture_declaration_tests {
    use super::*;
    use crate::compiler as c;

    #[test]
    fn opaque_capture_envelope_replays_raw_formals_not_arbitrary_parameters() {
        let snapshot = Arc::new(c::Snapshot {
            input: c::Input::Text("fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move |x:A|g(f(x))}fn main(){}".into()),
            source_options: c::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = c::Engine::new(c::CompilerProvider(snapshot));
        let key = crate::compiler::keys::item(c::TEMPLATE_BODY, 0);
        let requested = engine.request(key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        let body = engine.fact_of::<c::TemplateBodyFact>(&key, 0).unwrap();
        let types = engine
            .fact_of::<c::frontend::PlanCtx>(&crate::compiler::keys::SignaturesDebtRoot.key(), 0)
            .unwrap();
        let world = types.trait_world().unwrap();
        let (opaque, hidden) = body
            .checked
            .exprs
            .iter()
            .find_map(|expression| {
                let ir::ExprKind::OpaqueWrap { opaque, value } = &expression.kind else {
                    return None;
                };
                Some((opaque.clone(), body.checked.exprs[value.0].ty.clone()))
            })
            .unwrap();
        let Ty::Closure(closure) = hidden else {
            panic!("expected captured closure")
        };
        let Ty::Param(parameter) = closure.captures()[0] else {
            panic!("expected captured family")
        };
        assert!(world.opaque_captures_parameter(&opaque, parameter).unwrap());
        let mut foreign = parameter;
        foreign.index = u16::MAX;
        assert!(!world.opaque_captures_parameter(&opaque, foreign).unwrap());
        foreign = parameter;
        foreign.owner.declaration = types.callable_decl(1).unwrap();
        assert!(!world.opaque_captures_parameter(&opaque, foreign).unwrap());
        let signature = world
            .normalized_callable_schema(opaque.owner())
            .unwrap()
            .signature;
        let input = ir::InputCarrier {
            param: 0,
            location: ir::CarrierLocation {
                path: ir::CarrierPath::default(),
                channel: ir::CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                },
            },
        };
        let bad_destination = ir::CarrierLocation {
            path: ir::CarrierPath::default(),
            channel: ir::CarrierChannel::OpaqueTail {
                opaque: opaque.clone(),
                prefix: 1,
            },
        };
        assert!(
            world
                .carrier_source_eligible(
                    &signature,
                    &bad_destination,
                    &signature.result,
                    &input,
                    None
                )
                .is_err()
        );
        let mut args = opaque.owner_args().to_vec();
        args.swap(0, 1);
        for changed in [
            ir::OpaqueTy::new(
                types.callable_decl(1).unwrap(),
                opaque.owner_args().into(),
                opaque.site(),
                opaque.bounds().into(),
                opaque.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                args.into(),
                opaque.site(),
                opaque.bounds().into(),
                opaque.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                Arc::from([]),
                opaque.site(),
                opaque.bounds().into(),
                opaque.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                opaque.owner_args().into(),
                opaque.site() + 1,
                opaque.bounds().into(),
                opaque.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                opaque.owner_args().into(),
                opaque.site(),
                opaque.bounds().into(),
                Arc::from([ir::SourceRegionId {
                    declaration: opaque.owner(),
                    index: u16::MAX,
                }]),
            ),
        ] {
            assert!(
                world
                    .opaque_captures_parameter(&changed, parameter)
                    .is_err()
            );
        }
    }
}
