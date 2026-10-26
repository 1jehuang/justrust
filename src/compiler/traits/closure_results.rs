//! Closure-specific result views replay an original required-call authority.
//! No source body, world, facts or mutable schema is retained in the receipt.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn closure_result_witness(
        &self,
        receipt: &ir::ClosureResultReceipt,
    ) -> Result<(TraitCall, Ty)> {
        let closure = self.closure.as_ref().ok_or_else(|| {
            FactsError::invalid(
                self.budget(),
                "closure result source lacks its actual closure context",
            )
        })?;
        self.world.realize_closure_result_witness(closure, receipt)
    }
}

impl ValidatedTraitWorld {
    pub(super) fn realize_closure_result_witness(
        &self,
        closure: &ir::ClosureTy,
        receipt: &ir::ClosureResultReceipt,
    ) -> Result<(TraitCall, Ty)> {
        if receipt.owner() != closure.owner()
            || receipt.provider() != closure.provider()
            || receipt.site() != closure.site()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result witness belongs to another source",
            ));
        }
        let owner = self.info(closure.owner())?;
        if owner.env.total_arity() != closure.owner_args().len() {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result owner binding arity changed",
            ));
        }
        let binding = |parameter| {
            owner
                .env
                .ordered_params()
                .zip(closure.owner_args())
                .find_map(|(id, ty)| (id == parameter).then(|| ty.clone()))
                .ok_or_else(|| "closure result witness uses a foreign owner parameter".to_string())
        };
        Ok((
            charged(&self.budget, receipt.realize_call(&binding, &self.budget))?,
            charged(&self.budget, receipt.realize_result(&binding, &self.budget))?,
        ))
    }
    pub(super) fn closure_result_carrier_contract(
        &self,
        subject: SignatureSubject<'_>,
        facts: &ValidatedFacts,
    ) -> Result<Option<Arc<CarrierContractView>>> {
        let (closure, mode, signature) = match subject {
            SignatureSubject::Body if facts.closure.is_some() => {
                let closure = facts.closure.as_ref().expect("guarded closure");
                if closure.result_receipt().is_none() {
                    return Ok(None);
                }
                (
                    closure.clone(),
                    facts.closure_mode.ok_or_else(|| {
                        FactsError::invalid(&self.budget, "closure result lost its mode")
                    })?,
                    self.closure_body_signature(facts)?,
                )
            }
            SignatureSubject::Call(Callee::Direct(target))
                if self
                    .schema
                    .closure_owner_site(target.declaration)?
                    .is_some() =>
            {
                let adapter = self.decode_closure_adapter(target)?;
                if adapter.closure.result_receipt().is_none() {
                    return Ok(None);
                }
                let proof = self.discharge_direct(target, facts)?;
                (adapter.closure.clone(), adapter.mode, proof.signature)
            }
            _ => return Ok(None),
        };
        if closure.result_receipt().is_none()
            || charged(
                &self.budget,
                ir::carrier_locations(closure.result(), &self.budget),
            )?
            .is_empty()
        {
            return Ok(None);
        }
        let view = self.closure_result_carrier_parts(closure, mode, signature, subject, facts)?;
        Ok(Some(Arc::new(view)))
    }

    pub(super) fn closure_result_carrier_parts(
        &self,
        closure: Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
        signature: Signature,
        subject: SignatureSubject<'_>,
        facts: &ValidatedFacts,
    ) -> Result<CarrierContractView> {
        self.validate_closure_metadata(&closure, facts.env())?;
        if !closure.supports(mode) {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result has a foreign receiver capability",
            ));
        }
        self.require_closure_owner_predicates(
            &closure,
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        let receipt = closure.result_receipt().ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "closure result lacks its parent-sealed source receipt",
            )
        })?;
        if receipt.argument_origin().is_none() {
            let (required_call, _) = self.realize_closure_result_witness(&closure, receipt)?;
            let required_signature = self.instantiate_trait_signature(&required_call, facts)?;
            let original_call = Callee::TraitItem(required_call);
            let original_view =
                self.carrier_contract(SignatureSubject::Call(&original_call), facts)?;
            let locations = charged(
                &self.budget,
                ir::carrier_locations(&signature.result, &self.budget),
            )?;
            if locations.len() != 1 || !matches!(signature.result, Ty::Param(_)) {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "closure restricted result needs one rigid root equality-dependent family",
                ));
            }
            for location in locations.iter() {
                original_view.validate_restricted_result(
                    facts,
                    SignatureSubject::Call(&original_call),
                    &required_signature,
                    location,
                )?;
            }
        }
        let source_regions = self.validate_source_regions(closure.provider(), &signature)?;
        let (mut contract, regions) = self.derive_carriers(&signature, &source_regions)?;
        if let Some(input) = receipt.argument_origin() {
            let source_ty = closure.params().get(input.argument).ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "closure argument result names a foreign parameter",
                )
            })?;
            let mut ty = source_ty;
            for _ in 0..=input.depth {
                charged(&self.budget, self.budget.charge_work(1))?;
                let Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                } = ty
                else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "closure argument result changed its shared reference channel",
                    ));
                };
                ty = referent.as_ref();
            }
            retain::<ir::CarrierStep>(&self.budget, 1)?;
            charged(
                &self.budget,
                self.budget.retain_bytes(2 * std::mem::size_of::<usize>()),
            )?;
            let source = ir::InputCarrier {
                param: 1,
                location: ir::CarrierLocation {
                    path: ir::CarrierPath(Arc::from([ir::CarrierStep::Field(input.argument)])),
                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                        depth: input.depth,
                    }),
                },
            };
            charged(&self.budget, self.budget.charge_work(contract.inputs.len()))?;
            if !contract.inputs.contains(&source) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure argument result is outside the actual tuple carrier domain",
                ));
            }
            let callee =
                charged(&self.budget, receipt.argument_callee(&self.budget))?.ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "closure argument result lacks checked source callee",
                    )
                })?;
            let Callee::Direct(target) = &callee else {
                unreachable!()
            };
            let proof = self.discharge_direct(target, facts)?;
            let view = self.carrier_contract(SignatureSubject::Call(&callee), facts)?;
            view.validate_for(facts)?;
            charged(
                &self.budget,
                self.budget.retain_bytes(2 * std::mem::size_of::<usize>()),
            )?;
            let root = ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            };
            let incoming = ir::InputCarrier {
                param: 0,
                location: root.clone(),
            };
            if !self.eq_ty(&proof.signature.result, &signature.result, 0)?
                || view.contract().results.len() != 1
                || view.contract().results[0].location != root
                || view.contract().results[0].sources.as_ref() != [incoming]
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure argument result changed canonical callee return authority",
                ));
            }
            retain::<ir::CarrierReturn>(&self.budget, 1)?;
            retain::<ir::InputCarrier>(&self.budget, 1)?;
            charged(
                &self.budget,
                self.budget.retain_bytes(4 * std::mem::size_of::<usize>()),
            )?;
            contract.results = Arc::from([ir::CarrierReturn {
                location: root,
                sources: Arc::from([source]),
            }]);
        }
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&signature, facts.env(), &self.budget),
        )?;
        let mut arguments = Vec::new();
        let mut components = Vec::new();
        for parameter in facts.env().ordered_params() {
            let ty = Ty::Param(parameter);
            for location in charged(&self.budget, ir::carrier_locations(&ty, &self.budget))?.iter()
            {
                push(
                    &self.budget,
                    &mut components,
                    ir::CarrierComponentMapping {
                        original_family: ir::CarrierFamily::Parameter(parameter),
                        original: location.clone(),
                        substituted: location.clone(),
                    },
                )?;
            }
            push(&self.budget, &mut arguments, ty)?;
        }
        retain::<CarrierContractView>(&self.budget, 1)?;
        Ok(CarrierContractView {
            nominal: None,
            physical: None,
            closure_result: Some((closure.clone(), mode)),
            world: self.id,
            registry: self.registry,
            environment: facts.predicates.index,
            phase: facts.phase(),
            declaration: closure.provider(),
            arguments: arguments.into(),
            original_env: facts.env().clone(),
            original_contract: contract.clone(),
            contract,
            origin,
            fixed: signature.ref_contract.clone(),
            regions,
            source_regions,
            components: components.into(),
            nominals: Arc::from([]),
            projected: None,
            projected_subject: None,
            equality_required: None,
            equality_results: None,
            result_subject: self.equality_result_subject(subject, facts)?,
            result_signature: signature,
            budget: self.budget.clone(),
        })
    }

    pub(super) fn validate_closure_result_carrier(
        &self,
        view: &CarrierContractView,
        closure: &Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        // Reconstruct the exact actual subject, not a provider-length guess.
        let direct_callee =
            if let parameter_equality::EqualityResultSubject::Direct(call) = &view.result_subject {
                let target = self.closure_target(closure, mode)?;
                if call != &target {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "closure result changed its canonical target",
                    ));
                }
                Some(Callee::Direct(target))
            } else {
                None
            };
        let subject = match &view.result_subject {
            parameter_equality::EqualityResultSubject::Body { .. } => {
                if facts.closure_mode != Some(mode)
                    || facts
                        .closure
                        .as_ref()
                        .is_none_or(|actual| actual != closure)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "closure result body has foreign mode or shape",
                    ));
                }
                SignatureSubject::Body
            }
            parameter_equality::EqualityResultSubject::Direct(_) => SignatureSubject::Call(
                direct_callee
                    .as_ref()
                    .expect("checked direct result subject"),
            ),
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure result has a foreign result-flow subject",
                ));
            }
        };
        let signature = match subject {
            SignatureSubject::Body => self.closure_body_signature(facts)?,
            SignatureSubject::Call(Callee::Direct(target)) => {
                self.discharge_direct(target, facts)?.signature
            }
            _ => unreachable!(),
        };
        let actual_subject = self.equality_result_subject(subject, facts)?;
        let same_subject = match (&view.result_subject, &actual_subject) {
            (
                parameter_equality::EqualityResultSubject::Body {
                    declaration: a,
                    arguments: aa,
                },
                parameter_equality::EqualityResultSubject::Body {
                    declaration: b,
                    arguments: ba,
                },
            ) => a == b && self.eq_types(aa, ba, 0)?,
            (
                parameter_equality::EqualityResultSubject::Direct(a),
                parameter_equality::EqualityResultSubject::Direct(b),
            ) => a == b,
            _ => false,
        };
        if !same_subject {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result changed its actual body/call subject",
            ));
        }
        let expected =
            self.closure_result_carrier_parts(closure.clone(), mode, signature, subject, facts)?;
        if view.declaration != expected.declaration
            || view.arguments != expected.arguments
            || !self.same_env(&view.original_env, &expected.original_env)?
            || view.original_contract != expected.original_contract
            || view.contract != expected.contract
            || view.origin != expected.origin
            || view.fixed != expected.fixed
            || view.regions != expected.regions
            || view.source_regions != expected.source_regions
            || view.components != expected.components
            || view.nominals != expected.nominals
            || view.projected.is_some()
            || view.projected_subject.is_some()
            || view.equality_required.is_some()
            || view.equality_results.is_some()
            || !self.eq_signature(&view.result_signature, &expected.result_signature)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result carrier view differs from exact source-call replay",
            ));
        }
        Ok(())
    }
}
