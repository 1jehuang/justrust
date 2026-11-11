//! Dependency-only dispatch over authenticated source call schemas.
//! No executable signature, topology or DirectCallProof can be obtained here.
use super::*;

impl ValidatedFacts {
    /// Canonical source publication metadata. Selection and every source
    /// obligation are checked, but no executable call proof is issued.
    pub(in crate::compiler) fn canonical_source_call_with_scratch(
        &self,
        callee: &Callee,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<(Callee, Signature)> {
        self.validate_source_flow_signature_with_scratch(
            SignatureSubject::Call(callee),
            signature,
            scratch,
        )?;
        let inputs = self.parent_local_call_inputs_with_scratch(callee, signature, scratch)?;
        let selected =
            if self.phase() == FactsPhase::Closed && !matches!(callee, Callee::Untupled(_)) {
                self.source_callee_entry(callee, &inputs, scratch)?
            } else {
                callee.clone()
            };
        let validation =
            std::cell::RefCell::new(nominal_normalization::ParentLocalValidation::default());
        let transport = ParentLocalTransport::new(&inputs).with_validation(&validation);
        transport.validate_with_scratch(&self.world, self, scratch)?;
        let context = ProofContext {
            transport,
            ..ProofContext::from_facts(self).for_storage(false)
        };
        let subject = self
            .world
            .equality_result_subject(SignatureSubject::Call(&selected), self)?;
        let (_, _, application) = self.world.source_carrier_application(
            &subject,
            self,
            context,
            scratch.for_world(self.world.id),
        )?;
        self.validate_source_flow_signature_with_scratch(
            SignatureSubject::Call(&selected),
            &application.signature,
            scratch,
        )?;
        Ok((selected, application.signature))
    }

    /// Fully checked declaration metadata for source inference. This is not a
    /// DirectCallProof and grants no field layout or executable call authority.
    pub(in crate::compiler) fn source_direct_call_signature_with_scratch(
        &self,
        target: &CallTarget,
        scratch: &mut TraitScratch,
    ) -> Result<Signature> {
        self.world.check_facts(self, true)?;
        scratch.for_facts(self);
        let mut inputs = Vec::new();
        for argument in target.type_args.iter() {
            for input in self.parent_local_receiver_inputs_with_scratch(argument, scratch)? {
                push(self.budget(), &mut inputs, input)?;
            }
        }
        let validation =
            std::cell::RefCell::new(nominal_normalization::ParentLocalValidation::default());
        let transport = ParentLocalTransport::new(&inputs).with_validation(&validation);
        transport.validate_with_scratch(&self.world, self, scratch)?;
        self.world.validate_declaration_schema(
            target.declaration,
            &self
                .world
                .original_callable_header(target.declaration)?
                .schema,
        )?;
        let context = ProofContext {
            transport,
            ..ProofContext::from_facts(self).for_storage(false)
        };
        let (_, _, application) = self.world.source_carrier_application(
            &parameter_equality::EqualityResultSubject::Direct(target.clone()),
            self,
            context,
            scratch.for_world(self.world.id),
        )?;
        charged(
            self.budget(),
            ir::validate_stored_ref_contract(
                &application.signature.params,
                &application.signature.result,
                &application.signature.ref_contract,
                self.budget(),
            ),
        )?;
        Ok(application.signature)
    }

    /// Authenticate receiver metadata against the exact source call signature.
    /// Source CFG loan validation does not request executable field layout.
    pub(in crate::compiler) fn source_callee_receiver_for_signature(
        &self,
        callee: &Callee,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<ReceiverMode> {
        self.world.check_facts(self, true)?;
        let nominals = self.parent_local_call_inputs_with_scratch(callee, signature, scratch)?;
        let validation =
            std::cell::RefCell::new(nominal_normalization::ParentLocalValidation::default());
        let transport = ParentLocalTransport::new(&nominals).with_validation(&validation);
        transport.validate_with_scratch(&self.world, self, scratch)?;
        let context = ProofContext {
            transport,
            ..ProofContext::from_facts(self).for_storage(false)
        };
        let receiver = match callee {
            Callee::Direct(target) => {
                self.world.ensure_provider(target.declaration)?;
                self.world.validate_closure_direct(target, context)?;
                self.world
                    .closure_dispatch_receiver(target)?
                    .unwrap_or(self.world.info(target.declaration)?.receiver)
            }
            Callee::TraitItem(call) | Callee::Virtual(ir::VirtualCall { call }) => {
                let item = self.world.item_header(call.item)?;
                if item.trait_decl != call.trait_ref.declaration
                    || call.method_args.len() != self.world.info(item.declaration)?.own_env.arity()
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "source receiver changed canonical trait item or method arity",
                    ));
                }
                item.receiver
            }
            Callee::Untupled(call) => {
                let selected = self.world.source_trait_target(
                    call,
                    self,
                    context,
                    scratch.for_world(self.world.id),
                )?;
                if !matches!(selected, Callee::Untupled(_)) {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "source untupled receiver lost its authenticated function item",
                    ));
                }
                self.world.item_header(call.item)?.receiver
            }
        };
        let active = scratch.for_world(self.world.id);
        let subject = self
            .world
            .equality_result_subject(SignatureSubject::Call(callee), self)?;
        let (_, _, expected) = self
            .world
            .source_carrier_application(&subject, self, context, active)?;
        if !self.world.eq_signature(&expected.signature, signature)? {
            return Err(FactsError::invalid(
                self.budget(),
                "source receiver changed exact canonical signature or reference contract",
            ));
        }
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.world.validate_ty_metadata_with_facts(
                ty,
                self.env(),
                0,
                &mut active.replays,
                Some(self),
            )?;
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
        Ok(receiver)
    }
    pub(in crate::compiler) fn source_callee_entry(
        &self,
        callee: &Callee,
        nominals: &[Ty],
        scratch: &mut TraitScratch,
    ) -> Result<Callee> {
        self.world.check_facts(self, true)?;
        if self.phase() != FactsPhase::Closed {
            return Err(FactsError::invalid(
                self.budget(),
                "source callee entry requires closed caller facts",
            ));
        }
        scratch.for_facts(self);
        let validation =
            std::cell::RefCell::new(nominal_normalization::ParentLocalValidation::default());
        let transport = ParentLocalTransport::new(nominals).with_validation(&validation);
        transport.validate_with_scratch(&self.world, self, scratch)?;
        let context = ProofContext {
            transport,
            ..ProofContext::from_facts(self).for_storage(false)
        };
        let selected = match callee {
            Callee::Direct(target) => Callee::Direct(target.clone()),
            Callee::TraitItem(call)
            | Callee::Untupled(call)
            | Callee::Virtual(ir::VirtualCall { call }) => {
                let selected = self.world.source_trait_target(
                    call,
                    self,
                    context,
                    scratch.for_world(self.world.id),
                )?;
                if matches!(callee, Callee::Virtual(_)) && !matches!(selected, Callee::Virtual(_)) {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "source virtual callee changed canonical object dispatch",
                    ));
                }
                if matches!(callee, Callee::Untupled(_))
                    && (!matches!(call.trait_ref.self_ty, Ty::FnItem(_) | Ty::FnPointer(_))
                        || self
                            .world
                            .schema
                            .closure_trait_kind(call.trait_ref.declaration)?
                            .is_none())
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "source untupled callee lost its function item",
                    ));
                }
                selected
            }
        };
        if let Callee::Direct(target) = &selected {
            self.source_call_entry_with_nominals(target, nominals, scratch)?;
        }
        Ok(selected)
    }
}

impl ValidatedTraitWorld {
    fn source_trait_target(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
        caller: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Callee> {
        self.check_proof_context_caller(caller)?;
        self.validate_ref_with_facts_for_use(
            &call.trait_ref,
            caller.env(),
            None,
            &mut active.replays,
            Some(facts),
            false,
        )?;
        let item = self.item_header(call.item)?;
        if item.trait_decl != call.trait_ref.declaration
            || call.method_args.len() != self.info(item.declaration)?.own_env.arity()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source trait entry changed item owner or method argument arity",
            ));
        }
        if matches!(call.trait_ref.self_ty, Ty::FnPointer(_))
            && self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_some()
        {
            if !call.method_args.is_empty() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function pointer callable has method arguments",
                ));
            }
            let signature = self.builtin_callable_signature(&call.trait_ref.self_ty, caller)?;
            if call.trait_ref.args.len() != 1
                || !self.eq_ty(
                    &call.trait_ref.args[0],
                    &self.fn_item_arguments(&signature)?,
                    0,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function pointer callable tuple differs from exact signature",
                ));
            }
            return Ok(Callee::Untupled(call.clone()));
        }
        if let Ty::FnItem(target) = &call.trait_ref.self_ty
            && self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_some()
        {
            if !call.method_args.is_empty() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source function item entry is not a canonical callable trait",
                ));
            }
            self.ensure_provider(target.declaration)?;
            self.validate_closure_direct(target, caller)?;
            let schema = self.callable_schema(target.declaration)?;
            let instantiated = self.instantiate_scratch_for_use(
                &schema,
                &target.type_args,
                caller,
                Some(target.declaration),
                active,
                Some(facts),
                false,
            )?;
            for predicate in instantiated.predicates.iter() {
                self.require_in(predicate, caller, active)?;
            }
            if call.trait_ref.args.len() != 1
                || !self.eq_ty(
                    &call.trait_ref.args[0],
                    &self.fn_item_arguments(&instantiated.signature)?,
                    0,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source function item entry changed its exact callable tuple",
                ));
            }
            // Callable-trait operands retain their receiver and argument tuple.
            // The existing untupled adapter owns conversion to the function's
            // direct ABI. Selecting the direct item here would change only the
            // signature and leave those original operands under the wrong ABI.
            return Ok(Callee::Untupled(call.clone()));
        }
        let mut arguments = self.trait_arguments(&call.trait_ref)?;
        for argument in call.method_args.iter() {
            push(&self.budget, &mut arguments, argument.clone())?;
        }
        let raw = self.instantiate_raw_cached(
            &item.schema,
            &arguments,
            caller,
            Some(item.declaration),
            &mut active.instantiations,
        )?;
        if raw.signature.params.len() != item.schema.signature.params.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "source trait entry changed signature parameter count",
            ));
        }
        for (original, actual) in item
            .schema
            .signature
            .params
            .iter()
            .zip(&raw.signature.params)
        {
            self.validate_rhs_substitution(original, actual, &item.schema.env, &arguments, 0)?;
        }
        self.validate_rhs_substitution(
            &item.schema.signature.result,
            &raw.signature.result,
            &item.schema.env,
            &arguments,
            0,
        )?;
        let instantiated = self.instantiate_scratch_for_use(
            &item.schema,
            &arguments,
            caller,
            Some(item.declaration),
            active,
            Some(facts),
            false,
        )?;
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, caller, active)?;
        }
        let predicate = Predicate::Implements(call.trait_ref.clone());
        if matches!(call.trait_ref.self_ty, Ty::Opaque(_)) {
            let evidence = self.require_in(&predicate, caller, active)?;
            if matches!(&evidence.0.origin, ProofOrigin::Implementation { .. }) {
                if evidence.0.world != self.id
                    || evidence.0.executable_storage != caller.executable_storage
                    || !caller.matches_source(&evidence.0.source_metadata)?
                    || evidence.0.environment != caller.predicates.index
                    || evidence.0.phase != caller.phase
                    || !self.eq_predicate(&evidence.0.goal, &predicate)?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque blanket source dispatch changed exact caller membership",
                    ));
                }
                return Ok(Callee::Direct(
                    self.selected_trait_target(call, facts, caller, &evidence, false)?,
                ));
            }
            return Ok(Callee::TraitItem(call.clone()));
        }
        // A const-extent array (`[T; N]` in a const-generic body) has no
        // closed builtin provider until the instance substitutes `N`. Keep
        // the proven trait item, exactly like a rigid type parameter.
        if matches!(call.trait_ref.self_ty, Ty::DependentArray { .. }) {
            self.require_in(&predicate, caller, active)?;
            return Ok(Callee::TraitItem(call.clone()));
        }
        let closed = caller.closed_selection(self);
        let evidence = self.require_in(&predicate, closed, active)?;
        if evidence.0.executable_storage != closed.executable_storage
            || evidence.0.world != self.id
            || !closed.matches_source(&evidence.0.source_metadata)?
            || evidence.0.environment != self.empty.index
            || evidence.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&evidence.0.goal, &predicate)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source dispatch evidence changed the exact assumption-free goal",
            ));
        }
        if matches!(&evidence.0.origin, ProofOrigin::TraitObject) {
            let Ty::Dyn(object) = &call.trait_ref.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source virtual dispatch lost its object identity",
                ));
            };
            if !facts
                .object_vtable_methods(object)?
                .iter()
                .any(|(application, item)| application == &call.trait_ref && *item == call.item)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source virtual method is outside the exact object vtable",
                ));
            }
            return Ok(Callee::Virtual(ir::VirtualCall { call: call.clone() }));
        }
        Ok(Callee::Direct(self.selected_trait_target(
            call, facts, closed, &evidence, false,
        )?))
    }
}

impl ValidatedTraitWorld {
    pub(super) fn selected_trait_target(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
        closed: ProofContext<'_>,
        evidence: &ProofEvidence,
        executable_storage: bool,
    ) -> Result<CallTarget> {
        let mut target = None;
        if let ProofOrigin::Closure { kind } = &evidence.0.origin {
            let Ty::Closure(closure) = &call.trait_ref.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure proof lost its environment type",
                ));
            };
            target = Some(self.closure_target(closure, *kind)?);
        } else if let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.0.origin
        {
            let positions = self.impl_header_index.get(&self.budget, *declaration)?;
            for h in positions.iter().map(|&p| &self.headers.impls[p]) {
                if h.declaration == *declaration {
                    for (item, body) in h.items.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if *item == call.item {
                            let mut type_args = arguments.to_vec();
                            for argument in call.method_args.iter() {
                                push(&self.budget, &mut type_args, argument.clone())?;
                            }
                            target = Some(CallTarget {
                                declaration: *body,
                                type_args: type_args.into(),
                            });
                            break;
                        }
                    }
                    if target.is_none()
                        && let Some(default) = self.item_header(call.item)?.default
                    {
                        self.ensure_provider(default)?;
                        let mut type_args = self.trait_arguments(&call.trait_ref)?;
                        for argument in call.method_args.iter() {
                            push(&self.budget, &mut type_args, argument.clone())?;
                        }
                        target = Some(CallTarget {
                            declaration: default,
                            type_args: type_args.into(),
                        });
                    }
                    break;
                }
            }
        } else if let ProofOrigin::Builtin {
            item: CatalogItem::Clone,
        } = &evidence.0.origin
        {
            let marker_clone_from = call.item == self.catalog.id(CatalogItem::CloneFromItem)
                && matches!(&call.trait_ref.self_ty, Ty::PhantomData(_));
            if call.item != self.catalog.id(CatalogItem::CloneItem) && !marker_clone_from {
                if let Some(default) = self.item_header(call.item)?.default {
                    self.ensure_provider(default)?;
                }
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: builtin trait item has no executable provider",
                ));
            }
            let mut args = Vec::new();
            let descriptor = match &call.trait_ref.self_ty {
                Ty::ManuallyDrop(_) => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: ManuallyDrop Clone executable provider is not implemented",
                    ));
                }
                Ty::PhantomData(marker) => {
                    if executable_storage {
                        facts.validate_ty(&call.trait_ref.self_ty)?;
                    } else {
                        self.validate_ty_metadata_with_facts(
                            &call.trait_ref.self_ty,
                            closed.env(),
                            0,
                            &mut NominalReplays::default(),
                            Some(facts),
                        )?;
                    }
                    push(&self.budget, &mut args, marker.argument().clone())?;
                    if marker_clone_from {
                        RecipeDescriptor::CloneFromPhantomData
                    } else {
                        RecipeDescriptor::ClonePhantomData
                    }
                }
                Ty::Int(i) => RecipeDescriptor::CloneInt(*i),
                Ty::Float(float) => RecipeDescriptor::CloneFloat(*float),
                Ty::Bool => RecipeDescriptor::CloneBool,
                Ty::Char => RecipeDescriptor::CloneChar,
                Ty::Unit => RecipeDescriptor::CloneUnit,
                Ty::Never => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: CloneNever executable body",
                    ));
                }
                Ty::Tuple(parts) => {
                    for p in parts.iter() {
                        push(&self.budget, &mut args, p.clone())?;
                    }
                    RecipeDescriptor::CloneTuple {
                        arity: parts.len().try_into().map_err(|_| {
                            FactsError::resource(&self.budget, "tuple Clone arity overflow")
                        })?,
                    }
                }
                Ty::Array { element, len } => {
                    push(&self.budget, &mut args, element.as_ref().clone())?;
                    RecipeDescriptor::CloneArray {
                        len: (*len).try_into().map_err(|_| {
                            FactsError::resource(&self.budget, "array Clone length overflow")
                        })?,
                    }
                }
                Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                } => {
                    if executable_storage {
                        self.validate_executable_actual_with_facts(
                            referent,
                            closed.env(),
                            closed.metadata_facts(),
                        )?;
                    } else {
                        self.validate_ty_metadata_with_facts(
                            referent,
                            closed.env(),
                            0,
                            &mut NominalReplays::default(),
                            Some(facts),
                        )?;
                    }
                    push(&self.budget, &mut args, referent.as_ref().clone())?;
                    RecipeDescriptor::CloneSharedRef
                }
                Ty::DependentArray { .. } => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: Clone of a const-generic array field",
                    ));
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "builtin Clone evidence has invalid Self shape",
                    ));
                }
            };
            let record = self.catalog.recipe(descriptor)?;
            self.ensure_provider(record.declaration)?;
            target = Some(CallTarget {
                declaration: record.declaration,
                type_args: args.into(),
            });
        } else {
            return Err(FactsError::invalid(
                &self.budget,
                "closed dispatch cannot use assumption or noncallable builtin evidence",
            ));
        }
        let target = target.ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "selected implementation has no required item mapping",
            )
        })?;
        Ok(target)
    }
}
