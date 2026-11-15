//! Callable signatures derived only from an exact canonical function item.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn closed_fn_item_result_signature(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        context: ProofContext<'_>,
    ) -> Result<Option<Signature>> {
        self.check_context(context)?;
        let ProofOrigin::FnItem { kind } = &selection.0.origin else {
            return Ok(None);
        };
        if self.closure_trait_kind(item.trait_decl)? != Some(*kind)
            || item.trait_decl != goal.declaration
            || selection.0.executable_storage != context.executable_storage
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
            || self.trait_header(item.trait_decl)?.items.as_ref() != [item.declaration]
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result changed exact callable selection",
            ));
        }
        let direct = self.builtin_callable_signature(&goal.self_ty, context)?;
        let arguments = self.fn_item_arguments(&direct)?;
        let receiver = match kind {
            ir::ClosureKind::FnOnce => goal.self_ty.clone(),
            ir::ClosureKind::Fn | ir::ClosureKind::FnMut => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Ref {
                    kind: if *kind == ir::ClosureKind::Fn {
                        ir::BorrowKind::Shared
                    } else {
                        ir::BorrowKind::Mut
                    },
                    referent: Arc::new(goal.self_ty.clone()),
                }
            }
        };
        if goal.args.len() != 1
            || !self.eq_ty(&goal.args[0], &arguments, 0)?
            || !self.eq_types(&signature.params, &[receiver, arguments], 0)?
            || !self.eq_ty(&signature.result, &direct.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item callable ABI differs from its exact direct signature",
            ));
        }
        let layers = charged(&self.budget, ir::root_reference_layers(&direct.result))?;
        if layers == 0 {
            return Ok(None);
        }
        if direct.ref_contract.results.len() != layers {
            return Err(FactsError::invalid(
                &self.budget,
                "function item direct result lacks source reference layers",
            ));
        }
        // Fixed root-reference rows cannot address fields of the callable's
        // argument tuple. Keep their topology but no invented fixed origin.
        // The independently replayed carrier base below carries the exact
        // direct input -> tuple field correspondence, for every component.
        let empty = |row: &ir::ResultReference| match &row.returns {
            ir::ReturnContract::SharedInputEnvelopes(inputs) => inputs.is_empty(),
            ir::ReturnContract::MutInputEnvelopes(inputs) => inputs.is_empty(),
            ir::ReturnContract::SharedInputPaths(inputs) => inputs.is_empty(),
            ir::ReturnContract::MutInputPaths(inputs) => inputs.is_empty(),
            _ => false,
        };
        if !signature.ref_contract.results.iter().all(empty) {
            return Err(FactsError::invalid(
                &self.budget,
                "function item callable has foreign fixed result origins",
            ));
        }
        let mut contract = signature.ref_contract.as_ref().clone();
        let mut regions = Vec::<(ir::RegionParamId, ir::RegionParamId)>::new();
        let mut results = Vec::new();
        for row in direct.ref_contract.results.iter() {
            let region =
                if let Some((_, region)) = regions.iter().find(|(old, _)| *old == row.region) {
                    *region
                } else {
                    if usize::from(contract.region_count) >= ir::MAX_REGION_PARAMS {
                        return Err(FactsError::resource(
                            &self.budget,
                            "function item reference region limit exceeded",
                        ));
                    }
                    let region = ir::RegionParamId(contract.region_count);
                    contract.region_count += 1;
                    if direct.ref_contract.static_regions & (1u64 << row.region.0) != 0 {
                        contract.static_regions |= 1u64 << region.0;
                    }
                    push(&self.budget, &mut regions, (row.region, region))?;
                    region
                };
            let (kind, _) = charged(&self.budget, ir::reference_at(&direct.result, row.position))?;
            push(
                &self.budget,
                &mut results,
                ir::ResultReference {
                    position: row.position,
                    region,
                    returns: match kind {
                        ir::BorrowKind::Shared => {
                            ir::ReturnContract::SharedInputEnvelopes(Arc::from([]))
                        }
                        ir::BorrowKind::Mut => ir::ReturnContract::MutInputEnvelopes(Arc::from([])),
                    },
                },
            )?;
        }
        let mut outlives = contract.outlives.to_vec();
        for (longer, shorter) in direct.ref_contract.outlives.iter() {
            let find = |region| {
                regions
                    .iter()
                    .find_map(|(old, new)| (*old == region).then_some(*new))
            };
            if let (Some(longer), Some(shorter)) = (find(*longer), find(*shorter)) {
                push(&self.budget, &mut outlives, (longer, shorter))?;
            }
        }
        contract.results = results.into();
        contract.outlives = outlives.into();
        let canonical = charged(
            &self.budget,
            ir::derive_return_contracts(
                &signature.params,
                &signature.result,
                &contract,
                &self.budget,
            ),
        )?;
        if !canonical.iter().all(empty) {
            return Err(FactsError::invalid(
                &self.budget,
                "function item tuple normalization invented a fixed result origin",
            ));
        }
        contract.results = canonical;
        retain::<ir::RefContract>(&self.budget, 1)?;
        let mut result = signature.clone();
        result.ref_contract = Arc::new(contract);
        Ok(Some(result))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn realize_source_fn_item_contract(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        declaration: Option<DeclId>,
        signature: &mut Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        let Some(declaration) = declaration else {
            return Ok(());
        };
        let Some(Ty::FnItem(_)) = arguments.first() else {
            return Ok(());
        };
        let Some(position) = self.item_header_index.get(&self.budget, declaration)? else {
            return Ok(());
        };
        let item = &self.headers.items[position];
        if self.closure_trait_kind(item.trait_decl)?.is_none() {
            return Ok(());
        }
        if context.executable_storage
            || !self.same_env(&schema.env, &item.schema.env)?
            || !self.eq_signature(&schema.signature, &item.schema.signature)?
            || schema.predicates.len() != item.schema.predicates.len()
            || arguments.len() != self.trait_header(item.trait_decl)?.env.total_arity()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source function item changed its canonical required schema",
            ));
        }
        for (actual, original) in schema.predicates.iter().zip(item.schema.predicates.iter()) {
            if !self.eq_predicate(actual, original)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source function item changed its required predicates",
                ));
            }
        }
        retain::<Ty>(&self.budget, arguments.len() - 1)?;
        let goal = TraitRef {
            declaration: item.trait_decl,
            self_ty: arguments[0].clone(),
            args: arguments[1..].into(),
        };
        let closed = context.closed_selection(self);
        let selection = self.require_in(&Predicate::Implements(goal.clone()), closed, active)?;
        if let Some(adapter) =
            self.closed_fn_item_result_signature(item, &goal, signature, &selection, closed)?
        {
            signature.ref_contract = adapter.ref_contract;
        }
        Ok(())
    }

    /// Remap all effects of the independently checked direct call. The item
    /// receiver itself can never supply an argument's returned loan.
    pub(super) fn fn_item_tuple_contract(
        &self,
        goal: &TraitRef,
        signature: &Signature,
        facts: &ValidatedFacts,
    ) -> Result<ir::CarrierContract> {
        let Ty::FnItem(item) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "function item tuple contract lost its item",
            ));
        };
        let callee = Callee::Direct(item.clone());
        let direct = self.discharge_direct(item, facts)?;
        if signature.params.len() != 2
            || !self.eq_ty(
                &signature.params[1],
                &self.fn_item_arguments(&direct.signature)?,
                0,
            )?
            || !self.eq_ty(&signature.result, &direct.signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item tuple contract changed exact direct ABI",
            ));
        }
        let checked = self.carrier_contract(SignatureSubject::Call(&callee), facts)?;
        checked.validate_for(facts)?;
        if !checked.native_heap_invariants.is_empty() {
            return Err(FactsError::unsupported(
                &self.budget,
                "function item tuple call requires native heap invariant transport",
            ));
        }
        let remap = |source: &ir::InputCarrier| -> Result<ir::InputCarrier> {
            if source.param >= direct.signature.params.len() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function item source names a foreign direct input",
                ));
            }
            let mut path = Vec::new();
            push(
                &self.budget,
                &mut path,
                ir::CarrierStep::Field(source.param),
            )?;
            for step in source.location.path.0.iter() {
                push(&self.budget, &mut path, step.clone())?;
            }
            Ok(ir::InputCarrier {
                param: 1,
                location: ir::CarrierLocation {
                    path: ir::CarrierPath(path.into()),
                    channel: source.location.channel.clone(),
                },
            })
        };
        let mut inputs = Vec::new();
        for (param, ty) in signature.params.iter().enumerate() {
            for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
                push(
                    &self.budget,
                    &mut inputs,
                    ir::InputCarrier {
                        param,
                        location: location.clone(),
                    },
                )?;
            }
        }
        for input in checked.contract().inputs.iter() {
            let mapped = remap(input)?;
            if !inputs.contains(&mapped) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function item tuple has no exact direct input component",
                ));
            }
        }
        let mut results = Vec::new();
        for returned in checked.contract().results.iter() {
            let mut sources = Vec::new();
            for source in returned.sources.iter() {
                push(&self.budget, &mut sources, remap(source)?)?;
            }
            push(
                &self.budget,
                &mut results,
                ir::CarrierReturn {
                    location: returned.location.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        let mut writes = Vec::new();
        for write in checked.contract().writes.iter() {
            let mut sources = Vec::new();
            for source in write.sources.iter() {
                push(&self.budget, &mut sources, remap(source)?)?;
            }
            push(
                &self.budget,
                &mut writes,
                ir::CarrierWriteEnvelope {
                    destination: remap(&write.destination)?,
                    sources: sources.into(),
                },
            )?;
        }
        Ok(ir::CarrierContract {
            inputs: inputs.into(),
            results: results.into(),
            writes: writes.into(),
        })
    }
}
