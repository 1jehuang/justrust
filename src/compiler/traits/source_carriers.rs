//! Reference-flow authority for authenticated source signatures. These views
//! prove carrier topology and obligations, never executable nominal storage.
use super::*;

/// One node's completed canonical reconstructions, not signature verdicts.
/// Every use still checks the actual signature and stored reference contract.
#[derive(Default)]
pub(super) struct CompletedSourceApplications {
    rows: Vec<CompletedSourceApplication>,
    storage: Vec<ir::PlanReservation>,
}

struct CompletedSourceApplication {
    source: body_validation::SourceKey,
    callee: Callee,
    signature: Arc<Signature>,
}

impl CompletedSourceApplications {
    fn find(
        &self,
        facts: &ValidatedFacts,
        callee: &Callee,
        inputs: &[Ty],
    ) -> Result<Option<Arc<Signature>>> {
        checked_module(
            facts.budget(),
            facts
                .world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        for row in &self.rows {
            charged(facts.budget(), facts.budget().charge_work(1))?;
            if same_callee(&row.callee, callee) && row.source.matches(facts, inputs)? {
                return Ok(Some(row.signature.clone()));
            }
        }
        Ok(None)
    }

    fn retain(
        &mut self,
        facts: &ValidatedFacts,
        callee: &Callee,
        inputs: &[Ty],
        signature: Arc<Signature>,
    ) -> Result<()> {
        if self.rows.len() < MAX_SCRATCH_OBLIGATIONS {
            let row = CompletedSourceApplication {
                source: body_validation::SourceKey::retain(facts, inputs)?,
                callee: callee.clone(),
                signature,
            };
            push_scratch_row(facts.budget(), &mut self.rows, &mut self.storage, row)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceCarrierAuthority {
    source: body_validation::SourceKey,
    inputs: Arc<[Ty]>,
}

/// A borrow of one completed canonical source reconstruction, not a cached
/// verdict. It cannot outlive any input and is never retained in proof scratch
/// or a fact. Equality replay consumes the already reconstructed topology of
/// this same invocation while still proving every equality row independently.
pub(super) struct SourceCarrierReplay<'a> {
    authority: &'a SourceCarrierAuthority,
    view: &'a CarrierContractView,
    facts: &'a ValidatedFacts,
}

impl SourceCarrierReplay<'_> {
    pub(super) fn validate_for(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        facts.world.check_facts(facts, true)?;
        if !std::ptr::eq(self.view, view)
            || !std::ptr::eq(self.facts, facts)
            || view.world != facts.world.id
            || view.registry != facts.registry_snapshot()
            || view.environment != facts.predicates.index
            || view.phase != facts.phase()
            || !view.budget.same_authority(facts.budget())
            || view.executable_storage
            || !matches!(&view.source_schema, Some(source) if std::ptr::eq(source.as_ref(), self.authority))
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "source reconstruction borrow changed exact view, caller or authority",
            ));
        }
        Ok(())
    }
}

impl SourceCarrierAuthority {
    /// Graph carrier transport changes occurrence coordinates and may expose
    /// sealed nominal inputs absent from the declaration's symbolic signature.
    /// Authenticate the old capsule and the complete new source signature, then
    /// issue a new exact input key. The original capsule never changes domain.
    pub(super) fn reissue_for_signature(
        &self,
        original: &CarrierContractView,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        facts: &ValidatedFacts,
        original_replay: &SourceCarrierReplay<'_>,
        scratch: &mut TraitScratch,
    ) -> Result<Arc<Self>> {
        if !matches!(&original.source_schema, Some(source) if std::ptr::eq(self, source.as_ref())) {
            return Err(FactsError::invalid(
                facts.budget(),
                "source transport changed its original capsule",
            ));
        }
        // The caller just completed the original view's full validators in
        // this operation. Borrow that exact reconstruction, never an owned
        // success row or an equal-content view from another invocation.
        original_replay.validate_for(original, facts)?;
        let result_subject = facts.world.equality_result_subject(subject, facts)?;
        if !facts
            .world
            .same_equality_result_subject(&original.result_subject, &result_subject)?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "source transport changed its original subject",
            ));
        }
        facts.validate_source_flow_signature_with_scratch(subject, signature, scratch)?;
        let inputs = match subject {
            SignatureSubject::Body => {
                facts.parent_local_signature_inputs_with_scratch(signature, scratch)?
            }
            SignatureSubject::Call(callee) => {
                facts.parent_local_call_inputs_with_scratch(callee, signature, scratch)?
            }
        };
        retain::<Self>(facts.budget(), 1)?;
        retain::<Ty>(facts.budget(), inputs.len())?;
        Ok(Arc::new(Self {
            source: body_validation::SourceKey::retain(facts, &inputs)?,
            inputs: inputs.into(),
        }))
    }

    pub(super) fn same_authority(&self, other: &Self, budget: &PlanBudget) -> Result<bool> {
        Ok(self.source.matches_retained(&other.source, budget)?
            && schema_instantiation::same_transport(&self.inputs, &other.inputs, budget, 0)?)
    }

    pub(super) fn transport(&self) -> ParentLocalTransport<'_> {
        ParentLocalTransport::new(&self.inputs)
    }
    pub(super) fn replay_with_scratch<'a>(
        &'a self,
        view: &'a CarrierContractView,
        facts: &'a ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<SourceCarrierReplay<'a>> {
        facts.world.check_facts(facts, true)?;
        if !matches!(&view.source_schema, Some(source) if std::ptr::eq(source.as_ref(), self)) {
            return Err(FactsError::invalid(
                facts.budget(),
                "source reconstruction changed its immutable authority",
            ));
        }
        if view.executable_storage {
            return Err(FactsError::invalid(
                facts.budget(),
                "source carrier authority cannot authorize executable storage",
            ));
        }
        let inputs = match &view.result_subject {
            parameter_equality::EqualityResultSubject::Body { .. } => {
                facts.parent_local_signature_inputs_with_scratch(&view.result_signature, scratch)?
            }
            parameter_equality::EqualityResultSubject::Direct(target) => facts
                .parent_local_call_inputs_with_scratch(
                    &Callee::Direct(target.clone()),
                    &view.result_signature,
                    scratch,
                )?,
            parameter_equality::EqualityResultSubject::Required(call) => facts
                .parent_local_call_inputs_with_scratch(
                    &Callee::TraitItem(call.clone()),
                    &view.result_signature,
                    scratch,
                )?,
        };
        if !self.source.matches(facts, &inputs)?
            || !schema_instantiation::same_transport(&self.inputs, &inputs, facts.budget(), 0)?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "source carrier view changed exact caller or ordered sealed inputs",
            ));
        }
        let context = ProofContext {
            transport: ParentLocalTransport::new(&inputs),
            ..ProofContext::from_facts(facts).for_storage(false)
        };
        context
            .transport
            .validate_with_scratch(&facts.world, facts, scratch)?;
        let (declaration, arguments, application) = facts.world.source_carrier_application(
            &view.result_subject,
            facts,
            context,
            scratch.for_world(facts.world.id),
        )?;
        if declaration != view.declaration
            || !facts.world.eq_types(&arguments, &view.arguments, 0)?
            || !facts
                .world
                .eq_signature(&application.signature, &view.result_signature)?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "source carrier view changed canonical declaration, arguments or signature",
            ));
        }
        let world = facts.world();
        let raw = world.normalized_declaration_schema(declaration)?;
        let retained = world.declaration_carriers(declaration)?;
        let origin = world
            .origin_index
            .get(facts.budget(), declaration)?
            .map(|position| world.origins[position].1.clone())
            .ok_or_else(|| {
                FactsError::invalid(
                    facts.budget(),
                    "source carrier replay lost original origins",
                )
            })?;
        let storage_arguments =
            world.carrier_storage_arguments(declaration, &arguments, &view.result_signature)?;
        retain::<Ty>(facts.budget(), view.result_signature.params.len())?;
        let expected = world.transport_carrier_parts_with_scratch(
            declaration,
            storage_arguments,
            view.result_signature.clone(),
            facts,
            raw,
            retained,
            origin,
            None,
            view.result_subject.clone(),
            scratch,
        )?;
        if !world.same_env(&view.original_env, &expected.original_env)?
            || view.original_contract != expected.original_contract
            || view.contract != expected.contract
            || view.origin != expected.origin
            || view.fixed != expected.fixed
            || view.regions != expected.regions
            || view.source_regions != expected.source_regions
            || view.components != expected.components
            || view.nominals != expected.nominals
            || view.native_heap_invariants.len() != expected.native_heap_invariants.len()
            || view
                .native_heap_invariants
                .iter()
                .zip(expected.native_heap_invariants.iter())
                .any(|(actual, expected)| {
                    actual.storage != expected.storage || actual.sources != expected.sources
                })
            || view.physical.is_some()
            || view.closure_result.is_some()
            || view.projected.is_some()
            || view.equality_required.is_some()
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "source carrier view changed original reference, write, result or region topology",
            ));
        }
        Ok(SourceCarrierReplay {
            authority: self,
            view,
            facts,
        })
    }
}

impl ValidatedFacts {
    #[cfg(test)]
    pub(in crate::compiler) fn source_carrier_contract(
        &self,
        subject: SignatureSubject<'_>,
    ) -> Result<Arc<CarrierContractView>> {
        self.source_carrier_contract_with_scratch(subject, &mut TraitScratch::default())
    }

    pub(in crate::compiler) fn source_carrier_contract_with_scratch(
        &self,
        subject: SignatureSubject<'_>,
        scratch: &mut TraitScratch,
    ) -> Result<Arc<CarrierContractView>> {
        self.world.check_facts(self, true)?;
        // Parent-sealed closure result witnesses and untupled adapters have
        // independent topology producers. They retain their exact validators.
        if let Some(view) = self.world.closure_result_carrier_contract(subject, self)? {
            return Ok(view);
        }
        if matches!(subject, SignatureSubject::Call(Callee::Untupled(_))) {
            return self.carrier_contract(subject);
        }
        let result_subject = self.world.equality_result_subject(subject, self)?;
        let (_, _, initial) = self.world.source_carrier_application(
            &result_subject,
            self,
            ProofContext::from_facts(self).for_storage(false),
            scratch.for_world(self.world.id),
        )?;
        let inputs = match subject {
            SignatureSubject::Body => {
                self.parent_local_signature_inputs_with_scratch(&initial.signature, scratch)?
            }
            SignatureSubject::Call(callee) => {
                self.parent_local_call_inputs_with_scratch(callee, &initial.signature, scratch)?
            }
        };
        let context = ProofContext {
            transport: ParentLocalTransport::new(&inputs),
            ..ProofContext::from_facts(self).for_storage(false)
        };
        context
            .transport
            .validate_with_scratch(&self.world, self, scratch)?;
        let (declaration, arguments, application) = self.world.source_carrier_application(
            &result_subject,
            self,
            context,
            scratch.for_world(self.world.id),
        )?;
        let raw = self.world.normalized_declaration_schema(declaration)?;
        let retained = self.world.declaration_carriers(declaration)?;
        let origin = self
            .world
            .origin_index
            .get(self.budget(), declaration)?
            .map(|position| self.world.origins[position].1.clone())
            .ok_or_else(|| {
                FactsError::invalid(
                    self.budget(),
                    "source carrier subject lost its original origin schema",
                )
            })?;
        retain::<SourceCarrierAuthority>(self.budget(), 1)?;
        retain::<Ty>(self.budget(), inputs.len())?;
        let authority = Arc::new(SourceCarrierAuthority {
            source: body_validation::SourceKey::retain(self, &inputs)?,
            inputs: inputs.into(),
        });
        let storage_arguments = self.world.carrier_storage_arguments(
            declaration,
            &arguments,
            &application.signature,
        )?;
        let mut view = self.world.transport_carrier_parts_with_scratch(
            declaration,
            storage_arguments,
            application.signature,
            self,
            raw,
            retained,
            origin,
            None,
            result_subject,
            scratch,
        )?;
        // Typed occurrences determine expansion, not the canonical ordered
        // actuals retained by source identity and all subsequent replays.
        view.arguments = arguments;
        view.source_schema = Some(authority);
        view.executable_storage = false;
        self.world
            .seal_equality_results_with_scratch(&mut view, self, scratch)?;
        view.validate_for_with_scratch(self, scratch)?;
        Ok(Arc::new(view))
    }

    #[cfg(test)]
    pub(in crate::compiler) fn validate_source_flow_signature(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
    ) -> Result<()> {
        self.validate_source_flow_signature_with_scratch(
            subject,
            signature,
            &mut TraitScratch::default(),
        )
    }

    pub(in crate::compiler) fn validate_source_flow_signature_with_scratch(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.validate_source_flow_signatures_with_scratch(subject, &[signature], scratch)
    }

    /// One receipt operation may compare several realizations of one source call.
    /// Canonical applications may be retained only by this node's complete
    /// source context. Every operand still authenticates its transport, type
    /// metadata, signature equality and stored reference contract on every use.
    pub(in crate::compiler) fn validate_source_flow_signatures_with_scratch(
        &self,
        subject: SignatureSubject<'_>,
        signatures: &[&Signature],
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.world.check_facts(self, true)?;
        let result_subject = self.world.equality_result_subject(subject, self)?;
        let mut reconstruction: Option<(Vec<Ty>, Arc<Signature>)> = None;
        for signature in signatures {
            self.world.check_facts(self, true)?;
            let inputs = match subject {
                SignatureSubject::Body => {
                    self.parent_local_signature_inputs_with_scratch(signature, scratch)?
                }
                SignatureSubject::Call(callee) => {
                    self.parent_local_call_inputs_with_scratch(callee, signature, scratch)?
                }
            };
            let context = ProofContext {
                transport: ParentLocalTransport::new(&inputs),
                ..ProofContext::from_facts(self).for_storage(false)
            };
            context
                .transport
                .validate_with_scratch(&self.world, self, scratch)?;
            let same_inputs = match &reconstruction {
                Some((previous, _)) => {
                    schema_instantiation::same_transport(previous, &inputs, self.budget(), 0)?
                }
                None => false,
            };
            if !same_inputs {
                let completed = match subject {
                    SignatureSubject::Call(callee) => {
                        scratch.source_applications.find(self, callee, &inputs)?
                    }
                    SignatureSubject::Body => None,
                };
                let expected = if let Some(expected) = completed {
                    expected
                } else {
                    let (_, _, application) = self.world.source_carrier_application(
                        &result_subject,
                        self,
                        context,
                        scratch.for_world(self.world.id),
                    )?;
                    retain::<Signature>(self.budget(), 1)?;
                    let expected = Arc::new(application.signature);
                    if let SignatureSubject::Call(callee) = subject {
                        scratch.source_applications.retain(
                            self,
                            callee,
                            &inputs,
                            expected.clone(),
                        )?;
                    }
                    expected
                };
                reconstruction = Some((inputs, expected));
            }
            let expected = &reconstruction.as_ref().unwrap().1;
            if !self.world.eq_signature(expected, signature)? {
                return Err(FactsError::invalid(
                    self.budget(),
                    "source flow changed its exact canonical signature or reference contract",
                ));
            }
            for ty in signature
                .params
                .iter()
                .chain(std::iter::once(&signature.result))
            {
                self.validate_ty_metadata_scratch(ty, self.budget(), scratch)?;
            }
            charged(
                self.budget(),
                ir::validate_stored_ref_contract(
                    &signature.params,
                    &signature.result,
                    &signature.ref_contract,
                    self.budget(),
                ),
            )?;
        }
        Ok(())
    }
}

impl ValidatedTraitWorld {
    pub(super) fn source_carrier_application(
        &self,
        subject: &parameter_equality::EqualityResultSubject,
        facts: &ValidatedFacts,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<(DeclId, Arc<[Ty]>, InstantiatedSchema)> {
        // Every consumer reconstructs the same declaration-only application,
        // including equality replay entered from an executable caller.
        let context = context.for_storage(false);
        self.check_facts(facts, true)?;
        self.check_proof_context_caller(context)?;
        let (declaration, arguments, schema) = match subject {
            parameter_equality::EqualityResultSubject::Body {
                declaration,
                arguments,
            } => {
                if *declaration != facts.declaration() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "source body flow changed owning declaration",
                    ));
                }
                (
                    *declaration,
                    arguments.clone(),
                    self.callable_schema(*declaration)?,
                )
            }
            parameter_equality::EqualityResultSubject::Direct(target) => {
                self.ensure_provider(target.declaration)?;
                self.validate_closure_direct(target, context)?;
                (
                    target.declaration,
                    target.type_args.clone(),
                    self.callable_schema(target.declaration)?,
                )
            }
            parameter_equality::EqualityResultSubject::Required(call) => {
                let item = self.item_header(call.item)?;
                if item.trait_decl != call.trait_ref.declaration
                    || call.method_args.len() != self.info(item.declaration)?.own_env.arity()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "source flow changed required item or method argument arity",
                    ));
                }
                self.validate_ref_with_facts_for_use(
                    &call.trait_ref,
                    context.env(),
                    None,
                    &mut active.replays,
                    Some(facts),
                    false,
                )?;
                self.require_in(
                    &Predicate::Implements(call.trait_ref.clone()),
                    context,
                    active,
                )?;
                let mut arguments = self.trait_arguments(&call.trait_ref)?;
                for argument in call.method_args.iter() {
                    push(&self.budget, &mut arguments, argument.clone())?;
                }
                (item.declaration, arguments.into(), item.schema.clone())
            }
        };
        let mut application = self.instantiate_scratch_for_use(
            &schema,
            &arguments,
            context,
            Some(declaration),
            active,
            Some(facts),
            false,
        )?;
        for (original, predicate) in schema.predicates.iter().zip(application.predicates.iter()) {
            // A body applied to its own declaration's identity arguments
            // assumes its elaborated where-clauses (rustc's param-env). A
            // `<T as Source>::Out: Copy` row normalized through `Out = String`
            // to `String: Copy` is then an assumption, not a new obligation.
            if matches!(
                subject,
                parameter_equality::EqualityResultSubject::Body { .. }
            ) {
                let mut assumed = false;
                for assumption in context.predicates.predicates.iter() {
                    if self.eq_predicate(assumption, original)? {
                        assumed = true;
                        break;
                    }
                }
                if assumed {
                    continue;
                }
            }
            self.require_in(predicate, context, active)?;
        }
        if let parameter_equality::EqualityResultSubject::Direct(target) = subject {
            // The same sealed adapter owns direct source and executable result
            // topology. Normalize only after its exact schema and obligations
            // have been authenticated, never by relaxing contract equality.
            self.canonicalize_projected_closure_direct_with(
                target,
                &mut application.signature,
                context,
                active,
            )?;
        }
        if let parameter_equality::EqualityResultSubject::Required(call) = subject
            && matches!(call.trait_ref.self_ty, Ty::FnPointer(_))
            && self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_some()
        {
            // The canonical builtin signature normalizes Output even when its
            // pointer inputs are rigid caller parameters. Replay that same
            // proved application here, not a structurally relaxed signature.
            for ty in &mut application.signature.params {
                *ty = self
                    .normalize_closed_projected_type_with(ty, context, false, active)?
                    .0;
            }
            application.signature.result = self
                .normalize_closed_projected_type_with(
                    &application.signature.result,
                    context,
                    false,
                    active,
                )?
                .0;
        }
        if let parameter_equality::EqualityResultSubject::Required(call) = subject
            && self
                .dependent_array_implementation(&call.trait_ref, context, active)?
                .is_some()
        {
            // Reconstruct the same authenticated caller-array assignments used
            // by required-call realization, before comparing the exact ABI.
            for ty in &mut application.signature.params {
                *ty = self
                    .normalize_closed_projected_type_for_use(ty, context, false, active, false)?
                    .0;
            }
            application.signature.result = self
                .normalize_closed_projected_type_for_use(
                    &application.signature.result,
                    context,
                    false,
                    active,
                    false,
                )?
                .0;
        }
        Ok((declaration, arguments, application))
    }
}
