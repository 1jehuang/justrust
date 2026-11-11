//! Closed callable tuple inputs use the checked closure's exact structural ABI.
use super::*;

// Only called after exact sealed closure/adapter replay. This cannot authorize
// any fixed input origin. Actual tuple origins live in the separate receipt.
fn canonicalize_empty_projected_result(
    signature: &mut Signature,
    budget: &PlanBudget,
) -> Result<()> {
    let empty = |row: &ir::ResultReference| match &row.returns {
        ir::ReturnContract::SharedInputEnvelopes(inputs) => inputs.is_empty(),
        ir::ReturnContract::SharedInputPaths(inputs) => inputs.is_empty(),
        _ => false,
    };
    charged(
        budget,
        budget.charge_work(signature.ref_contract.results.len()),
    )?;
    if signature.ref_contract.results.len() != 1
        || !signature.ref_contract.results.iter().all(empty)
    {
        return Err(FactsError::invalid(
            budget,
            "projected closure adapter has foreign fixed result origins",
        ));
    }
    let results = charged(
        budget,
        ir::derive_return_contracts(
            &signature.params,
            &signature.result,
            &signature.ref_contract,
            budget,
        ),
    )?;
    if results.len() != signature.ref_contract.results.len()
        || results
            .iter()
            .zip(signature.ref_contract.results.iter())
            .any(|(new, old)| {
                new.position != old.position || new.region != old.region || !empty(new)
            })
    {
        return Err(FactsError::invalid(
            budget,
            "projected closure adapter normalization added result authority",
        ));
    }
    if results != signature.ref_contract.results {
        retain::<ir::RefContract>(budget, 1)?;
        signature.ref_contract = Arc::new(ir::RefContract {
            results,
            ..(*signature.ref_contract).clone()
        });
    }
    Ok(())
}

// Every result layer has a fresh region and no fixed input origin.
fn canonicalize_empty_projected_result_layers(
    signature: &mut Signature,
    budget: &PlanBudget,
) -> Result<()> {
    let results = charged(
        budget,
        ir::derive_return_contracts(
            &signature.params,
            &signature.result,
            &signature.ref_contract,
            budget,
        ),
    )?;
    charged(budget, budget.charge_work(results.len()))?;
    if results.len() != signature.ref_contract.results.len()
        || results.iter().any(|row| {
            !matches!(&row.returns,
                ir::ReturnContract::SharedInputPaths(inputs) if inputs.is_empty())
                && !matches!(&row.returns,
                    ir::ReturnContract::SharedInputEnvelopes(inputs) if inputs.is_empty())
        })
    {
        return Err(FactsError::invalid(
            budget,
            "function item result acquired a fixed input origin",
        ));
    }
    retain::<ir::RefContract>(budget, 1)?;
    signature.ref_contract = Arc::new(ir::RefContract {
        results,
        ..(*signature.ref_contract).clone()
    });
    Ok(())
}

impl ValidatedTraitWorld {
    // A required Fn item names its result through FnOnce::Output rather than a
    // rigid result parameter. Its reference origins therefore come from the
    // selected parent-sealed closure adapter, not generic parameter expansion.
    // This supplies source topology only. Physical signature admission remains
    // independent, and the ordinary stored-contract validator still runs.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn realize_source_closure_item_contract(
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
        let Some(position) = self.item_header_index.get(&self.budget, declaration)? else {
            return Ok(());
        };
        let item = &self.headers.items[position];
        if let Some(Ty::FnItem(_)) = arguments.first() {
            return self.realize_source_fn_item_contract(
                item, schema, arguments, signature, context, active,
            );
        }
        let Some(Ty::Closure(closure)) = arguments.first() else {
            return Ok(());
        };
        if self.closure_trait_kind(item.trait_decl)?.is_none() || closure.result_receipt().is_none()
        {
            return Ok(());
        }
        if context.executable_storage
            || !self.same_env(&schema.env, &item.schema.env)?
            || !self.eq_signature(&schema.signature, &item.schema.signature)?
            || schema.predicates.len() != item.schema.predicates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source closure item changed its canonical declaration schema",
            ));
        }
        for (actual, original) in schema.predicates.iter().zip(item.schema.predicates.iter()) {
            if !self.eq_predicate(actual, original)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source closure item changed its canonical predicates",
                ));
            }
        }
        let trait_arity = self.trait_header(item.trait_decl)?.env.total_arity();
        if arguments.len() != trait_arity || trait_arity == 0 {
            return Err(FactsError::invalid(
                &self.budget,
                "source closure item changed its ordered trait arguments",
            ));
        }
        retain::<Ty>(&self.budget, arguments.len() - 1)?;
        let goal = TraitRef {
            declaration: item.trait_decl,
            self_ty: arguments[0].clone(),
            args: arguments[1..].into(),
        };
        let closed = context.closed_selection(self);
        let selection = self.require_in(&Predicate::Implements(goal.clone()), closed, active)?;
        if let Some(adapter) = self.closed_closure_argument_result_signature(
            item, &goal, signature, &selection, closed, active,
        )? {
            signature.ref_contract = adapter.ref_contract;
        }
        Ok(())
    }

    /// Source-carrier application of a builtin function item callable
    /// (`call_mut` with `Self = FnItem`) uses the same closed adapter as the
    /// executable projected application: fresh result regions, no fixed
    /// origin, receiver never an origin. Its tuple-field sources come from
    /// `fn_item_result_carriers`.
    fn realize_source_fn_item_contract(
        &self,
        item: &TraitItemHeader,
        schema: &GenericSchema,
        arguments: &[Ty],
        signature: &mut Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        if self.closure_trait_kind(item.trait_decl)?.is_none() {
            return Ok(());
        }
        if context.executable_storage
            || !self.same_env(&schema.env, &item.schema.env)?
            || !self.eq_signature(&schema.signature, &item.schema.signature)?
            || schema.predicates.len() != item.schema.predicates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source function item changed its canonical declaration schema",
            ));
        }
        for (actual, original) in schema.predicates.iter().zip(item.schema.predicates.iter()) {
            if !self.eq_predicate(actual, original)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source function item changed its canonical predicates",
                ));
            }
        }
        let trait_arity = self.trait_header(item.trait_decl)?.env.total_arity();
        if arguments.len() != trait_arity || trait_arity == 0 {
            return Err(FactsError::invalid(
                &self.budget,
                "source function item changed its ordered trait arguments",
            ));
        }
        retain::<Ty>(&self.budget, arguments.len() - 1)?;
        let goal = TraitRef {
            declaration: item.trait_decl,
            self_ty: arguments[0].clone(),
            args: arguments[1..].into(),
        };
        let closed = context.closed_selection(self);
        let selection = self.require_in(&Predicate::Implements(goal.clone()), closed, active)?;
        if let Some(adapter) = self
            .closed_fn_item_result_signature(item, &goal, signature, &selection, closed, active)?
        {
            signature.ref_contract = adapter.ref_contract;
        }
        Ok(())
    }

    // Called only after the direct producer discharged the exact canonical
    // source schema and all predicates. Recheck its sealed structural ABI
    // before applying the same origin-free representation normalization used
    // by the required callable producer.
    #[cfg(test)]
    pub(super) fn canonicalize_projected_closure_direct(
        &self,
        target: &CallTarget,
        signature: &mut Signature,
        context: ProofContext<'_>,
    ) -> Result<()> {
        self.canonicalize_projected_closure_direct_with(
            target,
            signature,
            context,
            &mut ProofScratch::default(),
        )
    }

    pub(super) fn canonicalize_projected_closure_direct_with(
        &self,
        target: &CallTarget,
        signature: &mut Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.check_context(context)?;
        if self
            .schema
            .closure_owner_site(target.declaration)?
            .is_none()
        {
            return Ok(());
        }
        let adapter = self.decode_closure_adapter(target)?;
        self.validate_closure_metadata_with(adapter.closure, context.env(), &mut active.replays)?;
        let captured = adapter
            .closure
            .result_receipt()
            .is_some_and(|receipt| receipt.capture_witness().is_some());
        if !captured
            && !adapter
                .closure
                .result_receipt()
                .is_some_and(|receipt| receipt.is_projected_input() || receipt.is_static_str())
        {
            return Ok(());
        }
        let prefix = self.closure_provider_prefix(target.declaration)?;
        let [receiver, arguments, result] = &target.type_args[prefix..] else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected direct adapter lost its canonical domain",
            ));
        };
        if !self.eq_types(&signature.params, &[receiver.clone(), arguments.clone()], 0)?
            || !self.eq_ty(&signature.result, result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected direct adapter changed its sealed ABI",
            ));
        }
        if captured {
            let facts = context.original.ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "captured direct adapter lacks original current caller facts",
                )
            })?;
            self.canonicalize_captured_result_signature(
                adapter.closure,
                adapter.mode,
                signature,
                facts,
            )
        } else {
            canonicalize_empty_projected_result(signature, &self.budget)
        }
    }

    /// The required Fn output projection may acquire reference layers only from
    /// the exact parent-sealed closure adapter selected for this application.
    pub(super) fn closed_closure_argument_result_signature(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<Signature>> {
        self.check_context(context)?;
        let ProofOrigin::Closure { kind } = &selection.0.origin else {
            return Ok(None);
        };
        let Ty::Closure(closure) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result selection lost its closure",
            ));
        };
        let Some(receipt) = closure.result_receipt() else {
            return Ok(None);
        };
        if receipt.argument_origin().is_none()
            && receipt.capture_witness().is_none()
            && !receipt.is_static_str()
        {
            return Ok(None);
        }
        if self.closure_trait_kind(item.trait_decl)? != Some(*kind)
            || item.trait_decl != goal.declaration
            || selection.0.executable_storage != context.executable_storage
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
            || !closure.supports(*kind)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result selection changed exact source authority",
            ));
        }
        if self.trait_header(item.trait_decl)?.items.as_ref() != [item.declaration]
            || self.item_header(item.declaration)?.trait_decl != item.trait_decl
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result requires the exact sole callable trait item",
            ));
        }
        self.validate_closure_metadata(closure, context.env())?;
        if *kind != ir::ClosureKind::FnOnce {
            retain::<Ty>(&self.budget, 1)?;
        }
        let target = self.closure_target(closure, *kind)?;
        let schema = self.callable_schema(target.declaration)?;
        let own = schema.env.scopes().last().ok_or_else(|| {
            FactsError::invalid(&self.budget, "closure result adapter lost its own binder")
        })?;
        let parameter = |index| {
            Ty::Param(ParamId {
                owner: own.binder(),
                index,
            })
        };
        if schema.signature.params.as_slice() != [parameter(0), parameter(1)]
            || schema.signature.result != parameter(2)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result adapter changed its canonical source signature",
            ));
        }
        // Structural replay only: the selected Closure proof already discharged
        // owner predicates. Do not enter trait normalization or call discharge
        // while deriving the declaration-owned reference topology.
        let mut provider = self.instantiate_raw_cached(
            &schema,
            &target.type_args,
            context,
            Some(target.declaration),
            &mut active.instantiations,
        )?;
        provider.signature.ref_contract = self.realize_ref_contract(
            &schema,
            &target.type_args,
            &provider.signature,
            Some(target.declaration),
        )?;
        if goal.args.len() != 1
            || !self.eq_ty(&goal.args[0], &self.closure_arguments(closure)?, 0)?
            || !self.eq_types(&signature.params, &provider.signature.params, 0)?
            || !self.eq_ty(&signature.result, &provider.signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result differs from its authenticated adapter ABI",
            ));
        }
        if receipt.capture_witness().is_some() {
            let facts = context.original.ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "captured callable result lacks original current caller facts",
                )
            })?;
            self.canonicalize_captured_result_signature(
                closure,
                *kind,
                &mut provider.signature,
                facts,
            )?;
        }
        if receipt.is_projected_input() || receipt.is_static_str() {
            canonicalize_empty_projected_result(&mut provider.signature, &self.budget)?;
        }
        Ok(Some(provider.signature))
    }

    /// Builtin `FnItem: Fn*<(A..)>` call results borrow only from the ordered
    /// argument tuple, exactly as the discharged item's own signature says.
    /// The callable receiver is a zero-sized item and is never an origin.
    /// The fixed RefContract cannot name tuple fields, so the result receives
    /// fresh regions with no fixed input origin (the closure argument-origin
    /// encoding). Exact tuple-field sources come from
    /// `fn_item_result_carriers`, which remaps the item's own carrier rows.
    pub(super) fn closed_fn_item_result_signature(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<Signature>> {
        self.check_context(context)?;
        let ProofOrigin::FnItem { kind } = &selection.0.origin else {
            return Ok(None);
        };
        let Ty::FnItem(target) = &goal.self_ty else {
            return Ok(None);
        };
        let layers = charged(&self.budget, ir::root_reference_layers(&signature.result))?;
        if layers == 0 || layers <= signature.ref_contract.results.len() {
            return Ok(None);
        }
        if self.closure_trait_kind(item.trait_decl)? != Some(*kind)
            || item.trait_decl != goal.declaration
            || selection.0.executable_storage != context.executable_storage
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result selection changed exact source authority",
            ));
        }
        if self.trait_header(item.trait_decl)?.items.as_ref() != [item.declaration]
            || self.item_header(item.declaration)?.trait_decl != item.trait_decl
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result requires the exact sole callable trait item",
            ));
        }
        let direct = self.discharge_in(target, context, active)?;
        let tuple = self.fn_item_arguments(direct.signature())?;
        if goal.args.len() != 1
            || signature.params.len() != 2
            || !self.eq_ty(&goal.args[0], &tuple, 0)?
            || !self.eq_ty(&signature.params[1], &tuple, 0)?
            || !self.eq_ty(&signature.result, &direct.signature().result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result differs from its discharged item signature",
            ));
        }
        // Only shared result layers: a mutable result would need exclusive
        // tuple-field authority this encoding does not express.
        let mut ty = &signature.result;
        while let Ty::Ref { kind, referent } = ty {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *kind != ir::BorrowKind::Shared {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: function item callable returning a mutable reference",
                ));
            }
            ty = referent.as_ref();
        }
        if !signature.ref_contract.results.is_empty() {
            return Ok(None);
        }
        let base = usize::from(signature.ref_contract.region_count);
        if base + layers > ir::MAX_REGION_PARAMS {
            return Err(FactsError::resource(
                &self.budget,
                "function item result region limit exceeded",
            ));
        }
        retain::<ir::ResultReference>(&self.budget, layers)?;
        retain::<(ir::RegionParamId, ir::RegionParamId)>(&self.budget, layers)?;
        let mut results = Vec::new();
        let mut outlives = signature.ref_contract.outlives.to_vec();
        let mut above = None;
        for depth in 0..layers {
            let region = ir::RegionParamId((base + depth) as u16);
            if let Some(above) = above {
                push(&self.budget, &mut outlives, (region, above))?;
            }
            push(
                &self.budget,
                &mut results,
                ir::ResultReference {
                    position: ir::ReferencePosition { depth: depth as u8 },
                    region,
                    returns: ir::ReturnContract::NoReference,
                },
            )?;
            above = Some(region);
        }
        let raw = ir::RefContract {
            region_count: (base + layers) as u16,
            input_regions: signature.ref_contract.input_regions.clone(),
            results: results.into(),
            outlives: outlives.into(),
            static_regions: signature.ref_contract.static_regions,
        };
        let mut adapted = Signature {
            params: signature.params.clone(),
            result: signature.result.clone(),
            ref_contract: charged(&self.budget, ir::normalize_ref_contract(&raw, &self.budget))?,
        };
        canonicalize_empty_projected_result_layers(&mut adapted, &self.budget)?;
        Ok(Some(adapted))
    }

    /// Exact tuple-field result sources for a builtin function item call:
    /// direct parameter `i` at location `L` becomes trait parameter 1 at
    /// `Field(i)` followed by `L`. Every mapped source must already be an
    /// input carrier of the tuple; the receiver is never a source.
    pub(super) fn fn_item_result_carriers(
        &self,
        goal: &TraitRef,
        contract: &ir::CarrierContract,
        facts: &ValidatedFacts,
    ) -> Result<Arc<[ir::CarrierReturn]>> {
        let Ty::FnItem(target) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result carrier lost its item",
            ));
        };
        let callee = Callee::Direct(target.as_ref().clone());
        let direct = self.carrier_contract(SignatureSubject::Call(&callee), facts)?;
        let direct = direct.contract();
        if direct.results.len() != contract.results.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "function item result carriers changed result shape",
            ));
        }
        let mut returned = Vec::new();
        for (row, expected) in direct.results.iter().zip(contract.results.iter()) {
            if !self.same_location(&row.location, &expected.location)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function item result carrier changed location",
                ));
            }
            let mut sources = Vec::new();
            for source in row.sources.iter() {
                let mut path = Vec::new();
                push(
                    &self.budget,
                    &mut path,
                    ir::CarrierStep::Field(source.param),
                )?;
                for step in source.location.path.0.iter() {
                    push(&self.budget, &mut path, step.clone())?;
                }
                let mapped = ir::InputCarrier {
                    param: 1,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(path.into()),
                        channel: source.location.channel.clone(),
                    },
                };
                charged(&self.budget, self.budget.charge_work(contract.inputs.len()))?;
                if !contract.inputs.contains(&mapped) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "function item result source is outside the argument tuple",
                    ));
                }
                if !sources.contains(&mapped) {
                    push(&self.budget, &mut sources, mapped)?;
                }
            }
            retain::<ir::InputCarrier>(&self.budget, sources.len())?;
            push(
                &self.budget,
                &mut returned,
                ir::CarrierReturn {
                    location: expected.location.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        retain::<ir::CarrierReturn>(&self.budget, returned.len())?;
        Ok(returned.into())
    }

    pub(super) fn closed_closure_tuple_input(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        index: usize,
        context: ProofContext<'_>,
    ) -> Result<bool> {
        self.check_context(context)?;
        if index != 1 {
            return Ok(false);
        }
        if let ProofOrigin::FnItem { .. } = &selection.0.origin {
            return self.closed_fn_item_tuple_input(goal, signature, index, context);
        }
        let ProofOrigin::Closure { kind } = &selection.0.origin else {
            return Ok(false);
        };
        let kind = *kind;
        let Ty::Closure(closure) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple proof has no checked closure",
            ));
        };
        if self.closure_trait_kind(item.trait_decl)? != Some(kind)
            || item.trait_decl != goal.declaration
            || selection.0.executable_storage != context.executable_storage
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple proof changed its canonical owner or phase",
            ));
        }
        self.validate_closure_metadata(closure, context.env())?;
        let target = self.closure_target(closure, kind)?;
        let mut active = ProofScratch::default();
        let provider = self.discharge_in(&target, context, &mut active)?;
        if !self.eq_types(&signature.params, &provider.signature.params, 0)?
            || !self.eq_ty(&signature.result, &provider.signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple differs from its checked adapter ABI",
            ));
        }
        let Some(tuple) = signature.params.get(index) else {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple signature has no argument position",
            ));
        };
        if goal.args.len() != 1
            || !matches!(tuple, Ty::Tuple(_) | Ty::Unit)
            || !closure.supports(kind)
            || !self.eq_ty(tuple, &goal.args[0], 0)?
            || !self.eq_ty(tuple, &self.closure_arguments(closure)?, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple differs from its checked closure arguments",
            ));
        }
        // Reference results require their own checked source-origin descriptor.
        // Input admission must not manufacture that separate capability.
        if !charged(
            &self.budget,
            ir::carrier_locations(&signature.result, &self.budget),
        )?
        .is_empty()
            && self
                .closed_closure_argument_result_signature(
                    item,
                    goal,
                    signature,
                    selection,
                    context,
                    &mut active,
                )?
                .is_none()
        {
            return Ok(false);
        }
        for location in charged(&self.budget, ir::carrier_locations(tuple, &self.budget))?.iter() {
            if !matches!(location.path.0.first(), Some(ir::CarrierStep::Field(_)))
                || !matches!(location.channel, ir::CarrierChannel::Fixed(_))
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed callable tuple has a nonstructural or unresolved carrier",
                ));
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend;

    #[test]
    fn projected_empty_topology_preserves_regions_and_rejects_new_origins() {
        let budget = PlanBudget::default();
        let reference = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
        };
        let signature = Signature {
            params: Vec::new(),
            result: reference.clone(),
            ref_contract: Arc::new(ir::RefContract {
                region_count: 1,
                results: Arc::from([ir::ResultReference {
                    position: ir::ReferencePosition { depth: 0 },
                    region: ir::RegionParamId(0),
                    returns: ir::ReturnContract::SharedInputEnvelopes(Arc::from([])),
                }]),
                ..ir::RefContract::default()
            }),
        };
        let mut canonical = signature.clone();
        canonicalize_empty_projected_result(&mut canonical, &budget).unwrap();
        assert_eq!(
            canonical.ref_contract.region_count,
            signature.ref_contract.region_count
        );
        assert!(
            matches!(&canonical.ref_contract.results[0].returns, ir::ReturnContract::SharedInputPaths(rows) if rows.is_empty())
        );
        let original = canonical.ref_contract.clone();
        canonicalize_empty_projected_result(&mut canonical, &budget).unwrap();
        assert!(Arc::ptr_eq(&original, &canonical.ref_contract));
        let input = ir::InputReference {
            param: 0,
            position: ir::ReferencePosition { depth: 0 },
        };
        let mut forged = signature.clone();
        Arc::make_mut(&mut Arc::make_mut(&mut forged.ref_contract).results)[0].returns =
            ir::ReturnContract::SharedInputEnvelopes(Arc::from([ir::InputEnvelope { input }]));
        assert!(canonicalize_empty_projected_result(&mut forged, &budget).is_err());
        let mut new_origin = signature;
        new_origin.params.push(reference);
        Arc::make_mut(&mut new_origin.ref_contract).input_regions = Arc::from([ir::InputRegion {
            input,
            region: ir::RegionParamId(0),
        }]);
        assert!(canonicalize_empty_projected_result(&mut new_origin, &budget).is_err());
    }

    #[test]
    fn projected_direct_adapter_preserves_exact_source_and_empty_origins() {
        use crate::compiler::{
            ClosureBodiesFact, CompilerProvider, Input, Snapshot, TEMPLATE_BODY, keys,
        };
        use crate::engine::Engine;
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(r#"struct Item{first:String,second:String}fn main(){let items=vec![Item{first:String::from("apple"),second:String::from("pear")}];let first:Vec<&String>=items.iter().map(|i|&i.first).collect();let second:Vec<&String>=items.iter().map(|i|&i.second).collect();}"#.into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let signatures = engine.request(keys::SignaturesDebtRoot.key());
        let body = engine.request(keys::item(TEMPLATE_BODY, 0));
        engine.run(1).unwrap();
        let plan = signatures.fact::<frontend::PlanCtx>(0).unwrap();
        let closures = body.fact::<ClosureBodiesFact>(1).unwrap();
        let closure = closures.bodies[0].ty.clone();
        let world = plan.trait_world().unwrap();
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &world.empty,
            executable_storage: true,
            phase: FactsPhase::Closed,
        };
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            let target = world.closure_target(&closure, mode).unwrap();
            let direct = world
                .discharge_in_with_facts(&target, context, &mut ProofScratch::default(), None)
                .unwrap();
            assert!(
                matches!(&direct.signature.ref_contract.results[0].returns, ir::ReturnContract::SharedInputPaths(rows) if rows.is_empty())
            );
            let facts = world.instance_facts(&target).unwrap();
            let (_, _, source) = world
                .source_carrier_application(
                    &parameter_equality::EqualityResultSubject::Direct(target.clone()),
                    &facts,
                    ProofContext::from_facts(&facts).for_storage(false),
                    &mut ProofScratch::default(),
                )
                .unwrap();
            assert!(
                world
                    .eq_signature(&source.signature, &direct.signature)
                    .unwrap()
            );
            let callee = Callee::Direct(target.clone());
            let mut source_scratch = TraitScratch::default();
            assert_eq!(
                facts
                    .source_callee_receiver_for_signature(
                        &callee,
                        &source.signature,
                        &mut source_scratch,
                    )
                    .unwrap(),
                world.closure_dispatch_receiver(&target).unwrap().unwrap(),
            );
            let mut changed_source = source.signature.clone();
            changed_source.result = Ty::Bool;
            assert!(
                facts
                    .source_callee_receiver_for_signature(
                        &callee,
                        &changed_source,
                        &mut source_scratch,
                    )
                    .is_err()
            );
            let mut same = direct.signature.clone();
            let original = same.ref_contract.clone();
            world
                .canonicalize_projected_closure_direct(&target, &mut same, context)
                .unwrap();
            assert!(Arc::ptr_eq(&original, &same.ref_contract));
            let mut active = ProofScratch::default();
            for _ in 0..3 {
                world
                    .canonicalize_projected_closure_direct_with(
                        &target,
                        &mut same,
                        context,
                        &mut active,
                    )
                    .unwrap();
                assert!(Arc::ptr_eq(&original, &same.ref_contract));
            }
            let mut changed = same.clone();
            changed.result = Ty::Bool;
            assert!(
                world
                    .canonicalize_projected_closure_direct_with(
                        &target,
                        &mut changed,
                        context,
                        &mut active
                    )
                    .is_err()
            );
            let mut changed = same.clone();
            changed.params[1] = Ty::Unit;
            assert!(
                world
                    .canonicalize_projected_closure_direct_with(
                        &target,
                        &mut changed,
                        context,
                        &mut active
                    )
                    .is_err()
            );
            let foreign = world.closure_target(&closures.bodies[1].ty, mode).unwrap();
            let mut wrong = target.clone();
            wrong.type_args = foreign.type_args;
            assert!(
                world
                    .canonicalize_projected_closure_direct_with(
                        &wrong,
                        &mut same,
                        context,
                        &mut active
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn projected_argument_result_replays_sealed_adapter_and_tuple_source() {
        use crate::compiler::{
            ClosureBodiesFact, CompilerProvider, Input, Snapshot, TEMPLATE_BODY, keys,
        };
        use crate::engine::Engine;
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(
                "struct Item{name:String}fn main(){let f=|i:&&Item|i.name.as_str();}".into(),
            ),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let signatures = engine.request(keys::SignaturesDebtRoot.key());
        let body = engine.request(keys::item(TEMPLATE_BODY, 0));
        engine.run(1).unwrap();
        let plan = signatures.fact::<frontend::PlanCtx>(0).unwrap();
        let closures = body.fact::<ClosureBodiesFact>(1).unwrap();
        let closure = closures.bodies.first().unwrap().ty.clone();
        let world = plan.trait_world().unwrap();
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            let header = world
                .headers
                .traits
                .iter()
                .find(|header| world.closure_trait_kind(header.declaration).unwrap() == Some(mode))
                .unwrap();
            let item = world.item_header(header.items[0]).unwrap();
            let goal = TraitRef {
                declaration: header.declaration,
                self_ty: Ty::Closure(closure.clone()),
                args: Arc::from([world.closure_arguments(&closure).unwrap()]),
            };
            let context = ProofContext {
                transport: ParentLocalTransport::default(),
                original: None,
                source_metadata: None,
                predicates: &world.empty,
                executable_storage: true,
                phase: FactsPhase::Closed,
            };
            let mut active = ProofScratch::default();
            let selection = world
                .require_in(&Predicate::Implements(goal.clone()), context, &mut active)
                .unwrap();
            let application = world
                .instantiate_projected_item(item, &goal, &[], context)
                .unwrap();
            let signature = &application.instantiated.signature;
            assert_eq!(signature.ref_contract.results.len(), 1);
            let source_context = context.for_storage(false);
            let arguments = world.trait_arguments(&goal).unwrap();
            let source = world
                .instantiate_scratch_for_use(
                    &item.schema,
                    &arguments,
                    source_context,
                    Some(item.declaration),
                    &mut active,
                    None,
                    false,
                )
                .unwrap();
            assert!(world.eq_signature(&source.signature, signature).unwrap());
            let mut forged_schema = item.schema.clone();
            forged_schema.signature.result = Ty::Bool;
            let mut forged_signature = signature.clone();
            assert!(
                world
                    .realize_source_closure_item_contract(
                        &forged_schema,
                        &arguments,
                        Some(item.declaration),
                        &mut forged_signature,
                        source_context,
                        &mut active,
                    )
                    .is_err()
            );
            assert!(
                world
                    .closed_closure_tuple_input(item, &goal, signature, &selection, 1, context)
                    .unwrap()
            );
            let facts = world
                .closure_callable_facts_for(closure.clone(), mode)
                .unwrap();
            let callee = Callee::TraitItem(TraitCall {
                item: item.declaration,
                trait_ref: goal.clone(),
                method_args: Arc::from([]),
            });
            let view = facts
                .carrier_contract(SignatureSubject::Call(&callee))
                .unwrap();
            view.validate_for(&facts).unwrap();
            let sources = &view.contract().results[0].sources;
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].param, 1);
            assert_eq!(
                sources[0].location.path.0.as_ref(),
                &[ir::CarrierStep::Field(0)]
            );
            assert_eq!(
                sources[0].location.channel,
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
            );
            for change in 0..3 {
                let mut record = selection.0.as_ref().clone();
                match change {
                    0 => record.environment ^= 1,
                    1 => record.phase = FactsPhase::Universal,
                    _ => {
                        record.origin = ProofOrigin::Closure {
                            kind: if mode == ir::ClosureKind::Fn {
                                ir::ClosureKind::FnOnce
                            } else {
                                ir::ClosureKind::Fn
                            },
                        }
                    }
                }
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &goal,
                            signature,
                            &ProofEvidence(Arc::new(record)),
                            context,
                            &mut active,
                        )
                        .is_err()
                );
            }
            let mut changed = signature.clone();
            changed.result = Ty::Bool;
            assert!(
                world
                    .closed_closure_argument_result_signature(
                        item,
                        &goal,
                        &changed,
                        &selection,
                        context,
                        &mut active,
                    )
                    .is_err()
            );
            let mut wrong_item = item.clone();
            wrong_item.declaration = closure.provider();
            assert!(
                world
                    .closed_closure_argument_result_signature(
                        &wrong_item,
                        &goal,
                        signature,
                        &selection,
                        context,
                        &mut active,
                    )
                    .is_err()
            );
            let mut forged = view.as_ref().clone();
            let mut results = forged.contract.results.to_vec();
            let mut sources = results[0].sources.to_vec();
            sources[0].param = 0;
            results[0].sources = sources.into();
            forged.contract.results = results.into();
            assert!(forged.validate_for(&facts).is_err());
        }
    }

    #[test]
    fn captured_result_closed_adapter_replays_source_and_current_caller() {
        use crate::compiler::{
            ClosureBodiesFact, CompilerProvider, Input, Snapshot, TEMPLATE_BODY, keys,
        };
        use crate::engine::Engine;
        let fixture = || {
            let engine = Engine::new(CompilerProvider(Arc::new(Snapshot {
                input: Input::Text(
                    "fn main(){let n=7i32;let f=||&n;let values=[2u8,3u8];let bytes:&[u8]=&values;let g=||&bytes;}"
                        .into(),
                ),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: PlanBudget::default(),
            })));
            let signatures = engine.request(keys::SignaturesDebtRoot.key());
            let body = engine.request(keys::item(TEMPLATE_BODY, 0));
            engine.run(1).unwrap();
            body.completion().unwrap();
            let plan = signatures.fact::<frontend::PlanCtx>(0).unwrap();
            let closures = body.fact::<ClosureBodiesFact>(1).unwrap();
            let world = plan.trait_world().unwrap();
            let closure = closures.bodies.first().unwrap().ty.clone();
            let facts = world
                .instance_facts(&CallTarget {
                    declaration: closure.owner(),
                    type_args: Arc::from([]),
                })
                .unwrap();
            let types = closures
                .bodies
                .iter()
                .map(|body| body.ty.clone())
                .collect::<Vec<_>>();
            (world, facts, types)
        };
        let (world, facts, closures) = fixture();
        let (foreign_world, foreign_facts, _) = fixture();
        for closure in closures {
            for mode in [
                ir::ClosureKind::Fn,
                ir::ClosureKind::FnMut,
                ir::ClosureKind::FnOnce,
            ] {
                let header = world
                    .headers
                    .traits
                    .iter()
                    .find(|header| {
                        world.closure_trait_kind(header.declaration).unwrap() == Some(mode)
                    })
                    .unwrap();
                let item = world.item_header(header.items[0]).unwrap();
                let goal = TraitRef {
                    declaration: header.declaration,
                    self_ty: Ty::Closure(closure.clone()),
                    args: Arc::from([world.closure_arguments(&closure).unwrap()]),
                };
                let context = ProofContext::from_facts(&facts);
                let mut active = ProofScratch::default();
                let selection = world
                    .require_in(&Predicate::Implements(goal.clone()), context, &mut active)
                    .unwrap();
                let application = world
                    .instantiate_projected_item(item, &goal, &[], context)
                    .unwrap();
                let signature = &application.instantiated.signature;
                assert_eq!(
                    signature.ref_contract.results.len(),
                    ir::root_reference_layers(closure.result()).unwrap(),
                );
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &goal,
                            signature,
                            &selection,
                            context,
                            &mut active,
                        )
                        .unwrap()
                        .is_some()
                );
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &goal,
                            signature,
                            &selection,
                            ProofContext {
                                original: None,
                                source_metadata: None,
                                ..context
                            },
                            &mut active,
                        )
                        .is_err()
                );
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &goal,
                            signature,
                            &selection,
                            ProofContext::from_facts(&foreign_facts),
                            &mut active,
                        )
                        .is_err()
                );
                assert!(
                    foreign_world
                        .authenticate_captured_result_signature(&closure, mode, signature, &facts,)
                        .is_err()
                );
                for change in 0..4 {
                    let mut record = selection.0.as_ref().clone();
                    match change {
                        0 => record.environment ^= 1,
                        1 => record.phase = FactsPhase::Universal,
                        2 => {
                            record.origin = ProofOrigin::Closure {
                                kind: if mode == ir::ClosureKind::Fn {
                                    ir::ClosureKind::FnOnce
                                } else {
                                    ir::ClosureKind::Fn
                                },
                            }
                        }
                        _ => record.world = foreign_world.id,
                    }
                    assert!(
                        world
                            .closed_closure_argument_result_signature(
                                item,
                                &goal,
                                signature,
                                &ProofEvidence(Arc::new(record)),
                                context,
                                &mut active,
                            )
                            .is_err()
                    );
                }
                let target = world.closure_target(&closure, mode).unwrap();
                let direct = world
                    .discharge_in_with_facts(&target, context, &mut active, Some(&facts))
                    .unwrap();
                assert_eq!(direct.signature.ref_contract, signature.ref_contract);
                let direct_callee = Callee::Direct(target.clone());
                let direct_view = facts
                    .carrier_contract(SignatureSubject::Call(&direct_callee))
                    .unwrap();
                for location in ir::carrier_locations(&signature.result, facts.budget())
                    .unwrap()
                    .iter()
                {
                    let storage = direct_view
                        .captured_result_storage(
                            &facts,
                            &direct_callee,
                            &direct.signature,
                            location,
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(storage.input().param, 0);
                    assert_eq!(storage.input().location.channel, location.channel);
                    assert!(storage.projection().is_empty());
                    assert!(
                        direct_view
                            .captured_result_storage(
                                &foreign_facts,
                                &direct_callee,
                                &direct.signature,
                                location,
                            )
                            .is_err()
                    );
                    for mutation in 0..3 {
                        let mut forged = direct_view.as_ref().clone();
                        let mut rows = forged.contract.results.to_vec();
                        let row = rows
                            .iter_mut()
                            .find(|row| row.location == *location)
                            .unwrap();
                        let mut sources = row.sources.to_vec();
                        match mutation {
                            0 => sources[0].param = 1,
                            1 => {
                                sources[0].location.channel =
                                    ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                        depth: location.channel.prefix() + 1,
                                    })
                            }
                            _ => {
                                sources[0].location.path =
                                    ir::CarrierPath(Arc::from([ir::CarrierStep::Field(99)]))
                            }
                        }
                        row.sources = sources.into();
                        forged.contract.results = rows.into();
                        assert!(
                            forged
                                .captured_result_storage(
                                    &facts,
                                    &direct_callee,
                                    &direct.signature,
                                    location,
                                )
                                .is_err()
                        );
                    }
                    let receipt = closure.result_receipt().unwrap();
                    for mutation in 0..7 {
                        let mut witness = receipt.capture_witness().unwrap().clone();
                        let mut nodes = witness.nodes.to_vec();
                        let node = nodes
                            .iter_mut()
                            .find(|node| node.expression == receipt.tail())
                            .unwrap();
                        let ir::CaptureResultKind::Borrow {
                            mut field,
                            mut source,
                            mut original,
                            mut prefix,
                        } = node.kind.clone()
                        else {
                            panic!("original borrowed source");
                        };
                        match mutation {
                            0 => {
                                node.kind = ir::CaptureResultKind::Read {
                                    field,
                                    source,
                                    original,
                                    prefix,
                                };
                            }
                            1 => {
                                node.kind = ir::CaptureResultKind::Call {
                                    target: direct_callee.clone(),
                                    arguments: Arc::from([]),
                                    source_ty: Ty::Unit,
                                };
                            }
                            _ => {
                                match mutation {
                                    2 => field += 1,
                                    3 => prefix = Arc::from([ir::Projection::Deref]),
                                    4 => original.projection.push(ir::Projection::Field(0)),
                                    5 => source.kind = ir::CaptureKind::Mut,
                                    _ => source.place.local.0 += 1,
                                }
                                node.kind = ir::CaptureResultKind::Borrow {
                                    field,
                                    source,
                                    original,
                                    prefix,
                                };
                            }
                        }
                        witness.nodes = nodes.into();
                        let receipt = Arc::new(
                            ir::ClosureResultReceipt::captured(
                                receipt.owner(),
                                receipt.provider(),
                                receipt.site(),
                                receipt.tail(),
                                witness,
                                facts.budget(),
                            )
                            .unwrap(),
                        );
                        let forged_closure =
                            Arc::new(closure.as_ref().clone().with_result_receipt(Some(receipt)));
                        let mut forged_view = direct_view.as_ref().clone();
                        forged_view.closure_result = Some((forged_closure, mode));
                        assert!(
                            !matches!(
                                forged_view.captured_result_storage(
                                    &facts,
                                    &direct_callee,
                                    &direct.signature,
                                    location,
                                ),
                                Ok(Some(_))
                            ),
                            "storage source mutation={mutation}"
                        );
                        assert!(forged_view.validate_for(&facts).is_err());
                    }
                }
                let mut direct_signature = direct.signature.clone();
                let direct_contract = direct_signature.ref_contract.clone();
                world
                    .canonicalize_projected_closure_direct(&target, &mut direct_signature, context)
                    .unwrap();
                assert!(Arc::ptr_eq(
                    &direct_contract,
                    &direct_signature.ref_contract
                ));
                assert!(
                    world
                        .canonicalize_projected_closure_direct(
                            &target,
                            &mut direct_signature,
                            ProofContext {
                                original: None,
                                source_metadata: None,
                                ..context
                            },
                        )
                        .is_err()
                );
                direct_signature.result = Ty::Bool;
                assert!(
                    world
                        .canonicalize_projected_closure_direct(
                            &target,
                            &mut direct_signature,
                            context
                        )
                        .is_err()
                );
                let mut same = signature.clone();
                let unchanged = same.ref_contract.clone();
                world
                    .canonicalize_captured_result_signature(&closure, mode, &mut same, &facts)
                    .unwrap();
                assert!(Arc::ptr_eq(&unchanged, &same.ref_contract));
                for returns in [
                    ir::ReturnContract::NoReference,
                    ir::ReturnContract::MutInputPaths(Arc::from([])),
                    ir::ReturnContract::SharedInputEnvelopes(Arc::from([ir::InputEnvelope {
                        input: ir::InputReference {
                            param: 0,
                            position: ir::ReferencePosition { depth: 0 },
                        },
                    }])),
                ] {
                    let mut forged = signature.clone();
                    let mut contract = forged.ref_contract.as_ref().clone();
                    let mut rows = contract.results.to_vec();
                    rows[0].returns = returns;
                    contract.results = rows.into();
                    forged.ref_contract = Arc::new(contract);
                    assert!(
                        world
                            .canonicalize_captured_result_signature(
                                &closure,
                                mode,
                                &mut forged,
                                &facts
                            )
                            .is_err()
                    );
                }
                let mut changed = signature.clone();
                let mut contract = changed.ref_contract.as_ref().clone();
                let mut rows = contract.results.to_vec();
                rows.last_mut().unwrap().position.depth += 1;
                contract.results = rows.into();
                changed.ref_contract = Arc::new(contract);
                assert!(
                    world
                        .authenticate_captured_result_signature(&closure, mode, &changed, &facts,)
                        .is_err()
                );
                let missing = Arc::new(
                    ir::ClosureTy::new(
                        closure.owner(),
                        closure.owner_args().into(),
                        closure.site(),
                        closure.provider(),
                        closure.params().into(),
                        closure.result().clone(),
                        closure.captures().into(),
                        closure.kind(),
                    )
                    .with_sources(closure.sources().into()),
                );
                let mut missing_goal = goal.clone();
                missing_goal.self_ty = Ty::Closure(missing);
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &missing_goal,
                            signature,
                            &selection,
                            context,
                            &mut active,
                        )
                        .unwrap()
                        .is_none()
                );
                assert!(
                    world
                        .require_in(&Predicate::Implements(missing_goal), context, &mut active,)
                        .is_err()
                );
            }
        }
    }

    fn fixture() -> (Arc<ValidatedTraitWorld>, Arc<ir::ClosureTy>) {
        let mut program = frontend::parse("mod ops {pub trait FnOnce<A>{type Output;fn call_once(self,args:A)->Self::Output;}pub trait Fn<A>:FnOnce<A>{fn call(&self,args:A)->Self::Output;}}use ops::{Fn,FnOnce};fn main(){let f=|x:&i32,flag:bool|if flag{*x}else{0};}").unwrap();
        program.prelude_module = program.modules.parent(program.traits[0].module).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let index = program
            .functions
            .iter()
            .position(|function| {
                program
                    .names
                    .spelling(*function.name.segments().last().unwrap())
                    .unwrap()
                    == "main"
            })
            .unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected checked parent source");
        };
        let closure = publication
            .source()
            .exprs
            .iter()
            .find_map(|expression| match &expression.ty {
                Ty::Closure(closure) => Some(closure.clone()),
                _ => None,
            })
            .unwrap();
        (plan.trait_world().unwrap(), closure)
    }

    #[test]
    fn closed_callable_tuple_replays_exact_proof_and_adapter_identity() {
        let (world, closure) = fixture();
        let header = world
            .headers
            .traits
            .iter()
            .find(|header| {
                world.closure_trait_kind(header.declaration).unwrap() == Some(ir::ClosureKind::Fn)
            })
            .unwrap();
        let item = world.item_header(header.items[0]).unwrap();
        let goal = TraitRef {
            declaration: header.declaration,
            self_ty: Ty::Closure(closure.clone()),
            args: Arc::from([world.closure_arguments(&closure).unwrap()]),
        };
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &world.empty,
            executable_storage: true,
            phase: FactsPhase::Closed,
        };
        let selection = world
            .require_in(
                &Predicate::Implements(goal.clone()),
                context,
                &mut ProofScratch::default(),
            )
            .unwrap();
        let application = world
            .instantiate_projected_item(item, &goal, &[], context)
            .unwrap();
        let signature = application.instantiated.signature;
        assert!(
            world
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 1, context)
                .unwrap()
        );
        assert!(
            !world
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 0, context)
                .unwrap()
        );

        let reject = |goal: &TraitRef, signature: &Signature, selection: &ProofEvidence| {
            assert!(
                matches!(world.closed_closure_tuple_input(item, goal, signature, selection, 1, context), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        };
        let mut changed = signature.clone();
        changed.params.swap(0, 1);
        reject(&goal, &changed, &selection);
        let mut changed = signature.clone();
        let Ty::Tuple(fields) = &signature.params[1] else {
            panic!("checked callable has no tuple");
        };
        let mut reversed = fields.to_vec();
        reversed.reverse();
        changed.params[1] = Ty::Tuple(reversed.into());
        reject(&goal, &changed, &selection);
        let mut strengthened = fields.to_vec();
        let Ty::Ref { referent, .. } = &strengthened[0] else {
            panic!("expected reference input");
        };
        strengthened[0] = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: referent.clone(),
        };
        changed.params[1] = Ty::Tuple(strengthened.into());
        reject(&goal, &changed, &selection);
        let mut changed = signature.clone();
        changed.result = Ty::Bool;
        reject(&goal, &changed, &selection);
        for phase in [FactsPhase::Universal, FactsPhase::Closed] {
            let mut record = selection.0.as_ref().clone();
            record.phase = phase;
            record.environment ^= 1;
            reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        }
        let mut record = selection.0.as_ref().clone();
        record.phase = FactsPhase::Universal;
        reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        let mut record = selection.0.as_ref().clone();
        record.origin = ProofOrigin::Closure {
            kind: ir::ClosureKind::FnOnce,
        };
        reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        let forged = Arc::new(
            ir::ClosureTy::new(
                closure.owner(),
                closure.owner_args().into(),
                closure.site() + 1,
                closure.provider(),
                closure.params().into(),
                closure.result().clone(),
                closure.captures().into(),
                closure.kind(),
            )
            .with_sources(closure.sources().into()),
        );
        let mut changed_goal = goal.clone();
        changed_goal.self_ty = Ty::Closure(forged);
        let mut record = selection.0.as_ref().clone();
        record.goal = Predicate::Implements(changed_goal.clone());
        reject(&changed_goal, &signature, &ProofEvidence(Arc::new(record)));
        let (other, other_closure) = fixture();
        let foreign_context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &other.empty,
            executable_storage: true,
            phase: FactsPhase::Closed,
        };
        assert!(
            other
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 1, foreign_context)
                .is_err()
        );
        let mut changed_goal = goal.clone();
        changed_goal.self_ty = Ty::Closure(other_closure);
        let mut record = selection.0.as_ref().clone();
        record.goal = Predicate::Implements(changed_goal.clone());
        reject(&changed_goal, &signature, &ProofEvidence(Arc::new(record)));
    }
}
