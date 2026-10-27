//! Borrowed carrier-boundary realization for an already replayed physical CFG.
//! This is not a declaration carrier view and cannot be passed to ordinary
//! ownership as one. The original declaration contract remains available and
//! every realized boundary retains its borrowed normalization authority.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler) struct PhysicalCarrierIdentity {
    publication: source_replay::OriginalBodyId,
    owner: ir::CallTarget,
    occurrence: Option<ir::ExprId>,
    signature: ir::Signature,
    contract: ir::CarrierContract,
}

pub(in crate::compiler) struct PhysicalCarrierBoundary<'a> {
    validation: &'a PhysicalValidation<'a>,
    function: &'a cfg::Function,
    facts: &'a traits::ValidatedFacts,
    original: Arc<traits::CarrierContractView>,
    occurrence: Option<ir::ExprId>,
    signature: ir::Signature,
    contract: ir::CarrierContract,
}

/// A local consumer's immutable result-flow authority. Construction replays
/// the borrowed physical boundary, never an ordinary declaration view.
pub(in crate::compiler) struct PhysicalCarrierFlow {
    view: Arc<traits::CarrierContractView>,
    facts: Arc<traits::ValidatedFacts>,
    target: Option<ir::Callee>,
}

impl PhysicalCarrierFlow {
    pub(in crate::compiler) fn view(&self) -> &Arc<traits::CarrierContractView> {
        &self.view
    }

    pub(in crate::compiler) fn validate_result(
        &self,
        view: &traits::CarrierContractView,
        facts: &traits::ValidatedFacts,
        subject: ir::SignatureSubject<'_>,
        signature: &ir::Signature,
        location: &ir::CarrierLocation,
    ) -> Result<(), String> {
        let budget = self.facts.budget();
        let same_subject = match (&self.target, subject) {
            (None, ir::SignatureSubject::Body) => true,
            (Some(expected), ir::SignatureSubject::Call(actual)) => match (expected, actual) {
                (ir::Callee::Direct(a), ir::Callee::Direct(b)) => {
                    a.declaration == b.declaration && Arc::ptr_eq(&a.type_args, &b.type_args)
                }
                (ir::Callee::TraitItem(a), ir::Callee::TraitItem(b))
                | (ir::Callee::Untupled(a), ir::Callee::Untupled(b)) => {
                    a.item == b.item
                        && a.trait_ref.declaration == b.trait_ref.declaration
                        && Arc::ptr_eq(&a.method_args, &b.method_args)
                        && Arc::ptr_eq(&a.trait_ref.args, &b.trait_ref.args)
                        && ir::identical_type(&a.trait_ref.self_ty, &b.trait_ref.self_ty, budget)?
                }
                (ir::Callee::Virtual(a), ir::Callee::Virtual(b)) => {
                    a.call.item == b.call.item
                        && a.call.trait_ref.declaration == b.call.trait_ref.declaration
                        && Arc::ptr_eq(&a.call.method_args, &b.call.method_args)
                        && Arc::ptr_eq(&a.call.trait_ref.args, &b.call.trait_ref.args)
                        && ir::identical_type(
                            &a.call.trait_ref.self_ty,
                            &b.call.trait_ref.self_ty,
                            budget,
                        )?
                }
                _ => false,
            },
            _ => false,
        };
        let expected = self.view.physical_source_signature();
        budget.charge_work(1)?;
        if !std::ptr::eq(view, self.view.as_ref())
            || !std::ptr::eq(facts, self.facts.as_ref())
            || !same_subject
            || signature.params.len() != expected.params.len()
            || !Arc::ptr_eq(&signature.ref_contract, &expected.ref_contract)
            || !ir::identical_type(&signature.result, &expected.result, budget)?
        {
            return Err("physical result flow changed its authenticated boundary".into());
        }
        for (actual, expected) in signature.params.iter().zip(&expected.params) {
            if !ir::identical_type(actual, expected, budget)? {
                return Err("physical result flow changed its parameter metadata".into());
            }
        }
        for candidate in ir::carrier_locations(&signature.result, budget)?.iter() {
            budget.charge_work(1)?;
            if candidate == location {
                return Ok(());
            }
        }
        Err("physical result flow location is outside the actual result".into())
    }
}

impl PhysicalValidation<'_> {
    #[cfg(test)]
    pub(in crate::compiler) fn validate_carrier_view(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        occurrence: Option<ir::ExprId>,
        view: &traits::CarrierContractView,
    ) -> Result<(), String> {
        let expected = if let Some(site) = occurrence {
            let index = function
                .blocks
                .iter()
                .position(|block| block.terminator.origin == Some(site))
                .ok_or("physical carrier view lost its original call occurrence")?;
            self.call_carriers(function, facts, cfg::BlockId(index))?
                .realize()?
        } else {
            self.body_carriers(function, facts)?.realize()?
        };
        view.matches_physical_boundary(&expected, facts)
    }

    pub(in crate::compiler) fn original_call_evidence(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<Arc<ir::ValidatedCallEvidence>, String> {
        self.replay_call_arguments(function, facts, block)?;
        let site = function.blocks[block.0]
            .terminator
            .origin
            .ok_or("physical call lost source occurrence")?;
        self.original
            .call_evidence
            .iter()
            .find(|evidence| evidence.call() == site)
            .cloned()
            .ok_or_else(|| "physical call lacks canonical original argument evidence".into())
    }

    pub(in crate::compiler) fn body_carriers<'a>(
        &'a self,
        function: &'a cfg::Function,
        facts: &'a traits::ValidatedFacts,
    ) -> Result<PhysicalCarrierBoundary<'a>, String> {
        self.validate_cfg(function, facts)?;
        let signature = ir::Signature {
            params: function
                .params
                .iter()
                .map(|id| function.locals[id.0].ty.clone())
                .collect(),
            result: function.result.clone(),
            ref_contract: function.ref_contract.clone(),
        };
        let original = facts
            .carrier_contract(ir::SignatureSubject::Body)
            .map_err(|error| {
                error
                    .into_diagnostic(self.budget)
                    .unwrap_or_else(|error| error.to_string())
            })?;
        original.validate_for(facts).map_err(|error| {
            error
                .into_diagnostic(self.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        let contract = preserved_contract(
            &original,
            &signature,
            facts,
            self.representations,
            self.budget,
        )?;
        Ok(PhysicalCarrierBoundary {
            validation: self,
            function,
            facts,
            original,
            occurrence: None,
            signature,
            contract,
        })
    }

    pub(in crate::compiler) fn call_carriers<'a>(
        &'a self,
        function: &'a cfg::Function,
        facts: &'a traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<PhysicalCarrierBoundary<'a>, String> {
        self.validate_cfg(function, facts)?;
        let block = function
            .blocks
            .get(block.0)
            .ok_or("physical carrier call block is absent")?;
        let cfg::Terminator::Call {
            target, signature, ..
        } = &block.terminator.value
        else {
            return Err("physical carrier boundary does not name a call".into());
        };
        self.validate_call(block.terminator.origin, target, signature)?;
        let original = facts
            .carrier_contract(ir::SignatureSubject::Call(target))
            .map_err(|error| {
                error
                    .into_diagnostic(self.budget)
                    .unwrap_or_else(|error| error.to_string())
            })?;
        original.validate_for(facts).map_err(|error| {
            error
                .into_diagnostic(self.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        let site = block
            .terminator
            .origin
            .ok_or("physical carrier call has no source occurrence")?;
        // Direct and selected-trait boundaries both retain their genuine
        // declaration-owned rows. Only authenticated empty opaque tails vanish.
        let contract = preserved_contract(
            &original,
            signature,
            facts,
            self.representations,
            self.budget,
        )?;
        Ok(PhysicalCarrierBoundary {
            validation: self,
            function,
            facts,
            original,
            occurrence: Some(site),
            signature: signature.clone(),
            contract,
        })
    }

    /// Recover exact original argument evidence only after the full CFG and
    /// occurrence-specific physical callee mapping have replayed. This never
    /// relabels the original universal capsule as closed evidence.
    pub(in crate::compiler) fn replay_call_arguments(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<(), String> {
        self.validate_cfg(function, facts)?;
        let block = function
            .blocks
            .get(block.0)
            .ok_or("physical evidence call block is absent")?;
        let cfg::Terminator::Call {
            target,
            signature,
            args,
            ..
        } = &block.terminator.value
        else {
            return Err("physical evidence does not name a call".into());
        };
        self.validate_call(block.terminator.origin, target, signature)?;
        let site = block
            .terminator
            .origin
            .ok_or("physical evidence lost source occurrence")?;
        let evidence = self
            .original
            .call_evidence
            .iter()
            .find(|evidence| evidence.call() == site)
            .ok_or("physical call lacks original validated argument evidence")?;
        if !Arc::ptr_eq(evidence.facts(), self.publication.facts()) {
            return Err("physical call argument evidence belongs to another facts context".into());
        }
        let expression = self
            .publication
            .source()
            .exprs
            .get(site.0)
            .ok_or("physical call source occurrence is absent")?;
        let ir::ExprKind::Call {
            args: original_args,
            ..
        } = &expression.kind
        else {
            return Err("physical argument evidence source is not a call".into());
        };
        if evidence.trace().arguments.len() != args.len()
            || original_args.len() != args.len()
            || evidence
                .trace()
                .arguments
                .iter()
                .enumerate()
                .any(|(index, row)| row.argument != index || row.value != original_args[index])
        {
            return Err("physical call original argument trace changed".into());
        }
        Ok(())
    }
}

impl PhysicalCarrierBoundary<'_> {
    pub(in crate::compiler) fn realize_flow(
        &self,
        facts: &Arc<traits::ValidatedFacts>,
    ) -> Result<PhysicalCarrierFlow, String> {
        if !std::ptr::eq(facts.as_ref(), self.facts) {
            return Err("physical result flow changed its facts authority".into());
        }
        let view = self.realize()?;
        let target = match self.occurrence {
            None => None,
            Some(site) => {
                self.validation
                    .budget
                    .charge_work(self.function.blocks.len())?;
                let block = self
                    .function
                    .blocks
                    .iter()
                    .find(|block| block.terminator.origin == Some(site))
                    .ok_or("physical result flow lost its call occurrence")?;
                let cfg::Terminator::Call { target, .. } = &block.terminator.value else {
                    return Err("physical result flow changed its call kind".into());
                };
                Some(target.clone())
            }
        };
        self.validation
            .budget
            .retain_bytes(std::mem::size_of::<PhysicalCarrierFlow>())?;
        Ok(PhysicalCarrierFlow {
            view,
            facts: facts.clone(),
            target,
        })
    }

    pub(in crate::compiler) fn realize(&self) -> Result<Arc<traits::CarrierContractView>, String> {
        self.validate()?;
        if (self.original.source_region_env().region_count != 0
            || !self.original.region_schema().constraints.is_empty())
            && !identity_region_transport(
                &self.original,
                &self.signature,
                &self.contract,
                self.facts,
                self.validation.representations,
            )?
        {
            return Err(
                "unsupported: physical carrier realization has declared lifetime constraints"
                    .into(),
            );
        }
        let identity = PhysicalCarrierIdentity {
            publication: self.validation.publication.identity().clone(),
            owner: self.validation.proof.target.clone(),
            occurrence: self.occurrence,
            signature: self.signature.clone(),
            contract: self.contract.clone(),
        };
        Ok(Arc::new(
            traits::CarrierContractView::from_physical_boundary(
                &self.original,
                identity,
                self.signature.clone(),
                self.contract.clone(),
            ),
        ))
    }
    pub(in crate::compiler) fn validate(&self) -> Result<(), String> {
        self.validation.validate_cfg(self.function, self.facts)?;
        self.original.validate_for(self.facts).map_err(|error| {
            error
                .into_diagnostic(self.validation.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        if let Some(site) = self.occurrence {
            let block = self
                .function
                .blocks
                .iter()
                .find(|block| block.terminator.origin == Some(site))
                .ok_or("physical carrier boundary occurrence disappeared")?;
            let cfg::Terminator::Call {
                target, signature, ..
            } = &block.terminator.value
            else {
                return Err("physical carrier boundary occurrence changed kind".into());
            };
            self.validation
                .validate_call(Some(site), target, signature)?;
            if signature != &self.signature {
                return Err("physical carrier boundary signature changed".into());
            }
        }
        let expected_contract = preserved_contract(
            &self.original,
            &self.signature,
            self.facts,
            self.validation.representations,
            self.validation.budget,
        )?;
        if self.contract != expected_contract {
            return Err("physical carrier boundary changed original carrier origins".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::compiler) fn contract(&self) -> &ir::CarrierContract {
        &self.contract
    }
    #[cfg(test)]
    pub(in crate::compiler) fn original(&self) -> &traits::CarrierContractView {
        &self.original
    }
}

/// Transport named regions only across an identical boundary. For synthesized
/// (unnamed) regions, fixed rows are unchanged and a family-to-fixed obligation
/// may additionally survive when the exact instantiated family is proven empty.
/// Such an obligation has no reference inhabitants and grants no new origin.
fn identity_region_transport(
    original: &traits::CarrierContractView,
    signature: &ir::Signature,
    contract: &ir::CarrierContract,
    facts: &traits::ValidatedFacts,
    representations: &opaque_instances::Representations,
) -> Result<bool, String> {
    facts.budget().charge_work(
        contract
            .inputs
            .len()
            .saturating_add(contract.results.len())
            .saturating_add(contract.writes.len())
            .saturating_add(1),
    )?;
    if signature.ref_contract != *original.ref_contract() {
        return Ok(false);
    }
    if contract == original.contract()
        && facts
            .world()
            .signatures_equal(signature, original.physical_source_signature())
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?
    {
        return Ok(true);
    }
    let source = original.source_region_env();
    if source.region_count != 0 || !source.outlives.is_empty() || !source.normalized.is_empty() {
        return Ok(false);
    }
    for row in source.carriers.iter() {
        facts
            .budget()
            .charge_work(row.location.path.0.len().saturating_add(1))?;
        if !matches!(row.region, ir::CarrierRegionTerm::Fixed(_)) {
            return Ok(false);
        }
        let ty = match row.parameter {
            Some(index) => signature.params.get(index),
            None => Some(&signature.result),
        };
        let Some(ty) = ty else { return Ok(false) };
        if !contains_location(
            &ir::carrier_locations(ty, facts.budget())?,
            &row.location,
            facts.budget(),
        )? {
            return Ok(false);
        }
    }
    for constraint in original.region_schema().constraints.iter() {
        facts.budget().charge_work(1)?;
        let ir::CarrierRegionTerm::Fixed(shorter) = constraint.shorter else {
            return Ok(false);
        };
        match constraint.longer {
            ir::CarrierRegionTerm::Fixed(longer) => {
                facts
                    .budget()
                    .charge_work(signature.ref_contract.outlives.len())?;
                if !signature.ref_contract.outlives.contains(&(longer, shorter)) {
                    return Ok(false);
                }
            }
            ir::CarrierRegionTerm::Family { parameter } => {
                let Some((_, actual)) = original
                    .parameter_actual(parameter)
                    .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?
                else {
                    return Ok(false);
                };
                let normalized = representations
                    .normalize(actual, facts.budget())
                    .map_err(|error| error.to_string())?;
                if !ir::carrier_locations(&normalized, facts.budget())?.is_empty() {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Preserve every concrete location in declaration order. The sole erasure is
/// an opaque family independently proven empty by its graph-read representation.
/// In particular an outer reference to an empty family is never erased.
pub(in crate::compiler) fn require_preserved_boundary(
    public: &ir::Signature,
    physical: &ir::Signature,
    world: &traits::ValidatedTraitWorld,
    representations: &opaque_instances::Representations,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for contract in [&public.ref_contract, &physical.ref_contract] {
        budget.charge_work(
            contract
                .input_regions
                .len()
                .saturating_add(contract.results.len())
                .saturating_add(contract.outlives.len())
                .saturating_add(1),
        )?;
    }
    if public.params.len() != physical.params.len() || public.ref_contract != physical.ref_contract
    {
        return Err("physical boundary changed parameter count or reference origins".into());
    }
    for (before, after) in public
        .params
        .iter()
        .chain(std::iter::once(&public.result))
        .zip(
            physical
                .params
                .iter()
                .chain(std::iter::once(&physical.result)),
        )
    {
        let normalized = representations
            .normalize(before, budget)
            .map_err(|e| e.to_string())?;
        if !world
            .types_equal(
                std::slice::from_ref(&normalized),
                std::slice::from_ref(after),
            )
            .map_err(|error| error.into_legacy_diagnostic(budget))?
        {
            return Err("physical boundary differs from authenticated normalization".into());
        }
        let physical_locations = ir::carrier_locations(after, budget)?;
        let mut next = 0;
        for location in ir::carrier_locations(before, budget)?.iter() {
            budget.charge_work(1)?;
            if let ir::CarrierChannel::OpaqueTail { opaque, .. } = &location.channel {
                let hidden = representations
                    .normalize(&ir::Ty::Opaque(opaque.clone()), budget)
                    .map_err(|e| e.to_string())?;
                if ir::carrier_locations(&hidden, budget)?.is_empty() {
                    continue;
                }
            }
            budget.charge_work(location.path.0.len().saturating_add(1))?;
            if physical_locations.get(next) != Some(location) {
                return Err(
                    "unsupported: physical boundary changed reference carrier topology".into(),
                );
            }
            next += 1;
        }
        if next != physical_locations.len() {
            return Err("unsupported: physical boundary changed reference carrier topology".into());
        }
    }
    ir::validate_stored_ref_contract(
        &physical.params,
        &physical.result,
        &physical.ref_contract,
        budget,
    )
}

fn preserved_contract(
    original: &traits::CarrierContractView,
    signature: &ir::Signature,
    facts: &traits::ValidatedFacts,
    representations: &opaque_instances::Representations,
    budget: &ir::PlanBudget,
) -> Result<ir::CarrierContract, String> {
    require_preserved_boundary(
        original.physical_source_signature(),
        signature,
        facts.world(),
        representations,
        budget,
    )?;
    transport_contract(original.contract(), signature, budget)
}

/// Remove only rows for erased families. A surviving row must retain *all* of
/// its original sources, in their original order. Empty family erasure cannot
/// become permission to invent or discard a concrete reference origin.
fn transport_contract(
    original: &ir::CarrierContract,
    signature: &ir::Signature,
    budget: &ir::PlanBudget,
) -> Result<ir::CarrierContract, String> {
    let mut inputs = carrier_buffer(signature.params.len(), budget)?;
    for ty in &signature.params {
        inputs.push(ir::carrier_locations(ty, budget)?);
    }
    let results = ir::carrier_locations(&signature.result, budget)?;
    let has_input = |input: &ir::InputCarrier| -> Result<bool, String> {
        budget.charge_work(1)?;
        match inputs.get(input.param) {
            Some(locations) => contains_location(locations, &input.location, budget),
            None => Ok(false),
        }
    };
    let mut retained_inputs = carrier_buffer(original.inputs.len(), budget)?;
    for row in original.inputs.iter() {
        if has_input(row)? {
            retained_inputs.push(row.clone());
        }
    }
    let mut retained_results = carrier_buffer(original.results.len(), budget)?;
    for row in original.results.iter() {
        budget.charge_work(1)?;
        if contains_location(&results, &row.location, budget)? {
            for source in row.sources.iter() {
                if !has_input(source)? {
                    return Err("physical carrier erasure changed a surviving result origin".into());
                }
            }
            retained_results.push(row.clone());
        }
    }
    let mut writes = carrier_buffer(original.writes.len(), budget)?;
    for row in original.writes.iter() {
        if has_input(&row.destination)? {
            for source in row.sources.iter() {
                if !has_input(source)? {
                    return Err("physical carrier erasure changed a surviving write origin".into());
                }
            }
            writes.push(row.clone());
        }
    }
    Ok(ir::CarrierContract {
        inputs: retained_inputs.into(),
        results: retained_results.into(),
        writes: writes.into(),
    })
}

fn carrier_buffer<T>(count: usize, budget: &ir::PlanBudget) -> Result<Vec<T>, String> {
    budget.charge_work(count)?;
    // Include the temporary Vec and its final Arc slice, including Arc header.
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
        .ok_or("physical carrier allocation overflow")?;
    budget.retain_bytes(bytes)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(count)
        .map_err(|_| "physical carrier allocation failed")?;
    Ok(rows)
}

fn contains_location(
    locations: &[ir::CarrierLocation],
    expected: &ir::CarrierLocation,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    for location in locations {
        budget.charge_work(
            location
                .path
                .0
                .len()
                .saturating_add(expected.path.0.len())
                .saturating_add(1),
        )?;
        if location == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutable_callable_instantiation_preserves_writable_carrier_envelope() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }
            fn start(&self, cx: Context, key: Key) -> TaskFuture {
                if key.kind != 9018 {
                    return self.0.start(cx, key);
                }
                let provider = self.0.clone();
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let template: Arc<TemplateBodyFact> = facts
                        .get(crate::compiler::keys::item(TEMPLATE_BODY, 1))
                        .await?;
                    let source = provider.validated_body(&types, 1, &template)?;
                    let declaration = types.callable_decl(0).map_err(Error::new)?;
                    let target = source
                        .exprs
                        .iter()
                        .find_map(|expression| match &expression.kind {
                            ir::ExprKind::Call {
                                target: ir::Callee::Direct(target),
                                ..
                            } if target.declaration == declaration => Some(target.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| Error::new("missing mutable generic call"))?;
                    let closed = types.instance_facts(&target).map_err(Error::new)?;
                    let view = closed
                        .carrier_contract(ir::SignatureSubject::Body)
                        .map_err(|error| source_fact_error(error, &provider.0.budget))?;
                    assert!(
                        !view.contract().writes.is_empty(),
                        "mutable generic instance lost writable carrier envelope: target={target:?}; signature={:?}; contract={:?}; components={:?}",
                        view.physical_source_signature(),
                        view.contract(),
                        view.component_mappings()
                    );
                    assert_eq!(view.contract().writes.len(), 1);
                    assert_eq!(view.contract().writes[0].destination.param, 0);
                    assert_eq!(
                        view.contract().writes[0]
                            .destination
                            .location
                            .path
                            .0
                            .as_ref(),
                        &[ir::CarrierStep::Referent, ir::CarrierStep::Field(0)]
                    );
                    let template: Arc<TemplateBodyFact> = facts
                        .get(crate::compiler::keys::item(TEMPLATE_BODY, 0))
                        .await?;
                    let publication = template.publication.as_ref().unwrap();
                    let budget = &provider.0.budget;
                    let representations =
                        opaque_instances::read(&facts, &target.type_args, budget).await?;
                    let lowered = cfg::lower_source_template(publication, budget)
                        .map_err(|error| Error::new(error.to_string()))?;
                    let mut physical = normalize(
                        publication,
                        &target,
                        lowered.function(),
                        &representations,
                        &[],
                        &types,
                        budget,
                    )?;
                    let specialization = opaque_specialization::Specialization::new(
                        publication,
                        &target,
                        &types,
                        budget,
                        representations.engine(),
                    )?;
                    specialization.authenticate_context(publication, &types, budget)?;
                    let foreign_budget = ir::PlanBudget::default();
                    assert!(
                        specialization
                            .authenticate_context(publication, &types, &foreign_budget)
                            .is_err()
                    );
                    physical
                        .seal(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            &closed,
                            budget,
                        )
                        .map_err(Error::new)?;
                    let validation = physical
                        .validation(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            budget,
                        )
                        .map_err(Error::new)?;
                    let mut boundary = validation
                        .body_carriers(physical.function(), &closed)
                        .map_err(Error::new)?;
                    let realized = boundary.realize().map_err(Error::new)?;
                    validation
                        .validate_carrier_view(physical.function(), &closed, None, &realized)
                        .map_err(Error::new)?;
                    let genuine = boundary.contract.clone();
                    boundary.contract.writes = Arc::from([]);
                    assert!(
                        boundary.validate().is_err(),
                        "deleted mutable write envelope accepted"
                    );
                    boundary.contract = genuine.clone();
                    let mut writes = boundary.contract.writes.to_vec();
                    writes[0].destination = boundary.contract.inputs[0].clone();
                    boundary.contract.writes = writes.into();
                    assert!(
                        boundary.validate().is_err(),
                        "write to outer owned reference accepted"
                    );
                    boundary.contract = genuine;
                    let mut writes = boundary.contract.writes.to_vec();
                    writes[0].sources = Arc::from([ir::InputCarrier {
                        param: 99,
                        location: writes[0].destination.location.clone(),
                    }]);
                    boundary.contract.writes = writes.into();
                    assert!(
                        boundary.validate().is_err(),
                        "forged mutable write origin accepted"
                    );
                    cx.publish_fact(0, ())
                })
            }
        }
        for workers in [1, 8] {
            // The header, not an incidental body write, owns the admissible envelope.
            let snapshot = Arc::new(Snapshot { input: Input::Text("fn mutable<F:FnMut(i32)->i32>(mut f:F)->i32{0} fn main(){let mut n=5;let mut a=|x:i32|{n+=x;n};let first=&mut a;println!(\"{}\",mutable(first));}".into()), source_options: source::SourceOptions::default(), budget: ir::PlanBudget::default() });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(crate::compiler::keys::test_key(9018, vec![1]));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }

    #[test]
    fn physical_carrier_transport_storage_and_scan_are_metered() {
        let bytes = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..ir::PlanLimits::default()
        });
        assert!(carrier_buffer::<ir::InputCarrier>(1, &bytes).is_err());
        let work = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..ir::PlanLimits::default()
        });
        let location = ir::CarrierLocation {
            path: ir::CarrierPath::default(),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        assert!(contains_location(std::slice::from_ref(&location), &location, &work).is_err());
    }

    #[test]
    fn physical_borrowed_opaque_apply_matches_stock_workers1_8() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;
        let source = "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)} fn make_adder(n:i32)->impl Fn(i32)->i32{move |x|x+n} fn main(){let a=make_adder(5);println!(\"{}\",apply(&a,10));}";
        let directory = std::env::temp_dir().join(format!(
            "physical-borrowed-opaque-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let input = directory.join("main.rs");
        let stock = directory.join("stock");
        std::fs::write(&input, source).unwrap();
        let result = Command::new(crate::compiler::stock_rustc::stock_rustc())
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "--crate-name=physical_oracle",
            ])
            .arg(&input)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let expected = Command::new(&stock).output().unwrap();
        assert!(expected.status.success());
        assert_eq!(expected.stdout, b"15\n");
        for workers in [1, 8] {
            let checked = compile(source.into(), workers, true)
                .unwrap_or_else(|error| panic!("CHECK workers={workers}: {error}"));
            assert!(checked.executable.is_none());
            let built = compile(source.into(), workers, false)
                .unwrap_or_else(|error| panic!("BUILD workers={workers}: {error}"));
            let path = directory.join(format!("native-{workers}"));
            std::fs::write(&path, built.executable.unwrap()).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let actual = Command::new(path).output().unwrap();
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "workers={workers}: {actual:?}"
            );
            assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
            assert_eq!(actual.stderr, expected.stderr, "workers={workers}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn physical_boundary_replays_reference_contract_and_rejects_forged_rows() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }
            fn start(&self, cx: Context, key: Key) -> TaskFuture {
                if key.kind != 9017 {
                    return self.0.start(cx, key);
                }
                let provider = self.0.clone();
                let expect_realization = key.params[0] == 1;
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let template: Arc<TemplateBodyFact> = facts
                        .get(crate::compiler::keys::item(TEMPLATE_BODY, 0))
                        .await?;
                    let target = ir::CallTarget {
                        declaration: types.callable_decl(0).map_err(Error::new)?,
                        type_args: Arc::from([ir::Ty::Bool]),
                    };
                    let publication = template.publication.as_ref().unwrap();
                    let budget = &provider.0.budget;
                    let lowered = cfg::lower_source_template(publication, budget)
                        .map_err(|error| Error::new(error.to_string()))?;
                    let closed = types.instance_facts(&target).map_err(Error::new)?;
                    let declared = closed
                        .carrier_contract(ir::SignatureSubject::Body)
                        .map_err(|error| source_fact_error(error, budget))?;
                    let representations = opaque_instances::read(
                        &facts,
                        std::slice::from_ref(&declared.physical_source_signature().result),
                        budget,
                    )
                    .await?;
                    let mut physical = normalize(
                        publication,
                        &target,
                        lowered.function(),
                        &representations,
                        &[],
                        &types,
                        budget,
                    )?;
                    physical
                        .seal(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            &closed,
                            budget,
                        )
                        .map_err(Error::new)?;
                    let validation = physical
                        .validation(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            budget,
                        )
                        .map_err(Error::new)?;
                    let mut boundary = validation
                        .body_carriers(physical.function(), &closed)
                        .map_err(Error::new)?;
                    assert_eq!(
                        &boundary.signature.ref_contract,
                        boundary.original.ref_contract()
                    );
                    if !expect_realization {
                        assert_ne!(boundary.original.source_region_env().region_count, 0);
                        assert!(
                            boundary.realize().is_err(),
                            "declared lifetime transport was invented"
                        );
                        return cx.publish_fact(0, ());
                    }
                    assert!(!boundary.contract.inputs.is_empty());
                    assert_eq!(boundary.contract, *boundary.original.contract());
                    assert!(
                        identity_region_transport(
                            &boundary.original,
                            &boundary.signature,
                            &boundary.contract,
                            &closed,
                            &representations,
                        )
                        .map_err(Error::new)?
                    );
                    let view = boundary.realize().map_err(Error::new)?;
                    validation
                        .validate_carrier_view(physical.function(), &closed, None, &view)
                        .map_err(Error::new)?;
                    let flow = boundary.realize_flow(&closed).map_err(Error::new)?;
                    for location in ir::carrier_locations(&boundary.signature.result, budget)
                        .map_err(Error::new)?
                        .iter()
                    {
                        flow.validate_result(
                            flow.view(),
                            &closed,
                            ir::SignatureSubject::Body,
                            &boundary.signature,
                            location,
                        )
                        .map_err(Error::new)?;
                        let cloned_view = flow.view().as_ref().clone();
                        assert!(
                            flow.validate_result(
                                &cloned_view,
                                &closed,
                                ir::SignatureSubject::Body,
                                &boundary.signature,
                                location
                            )
                            .is_err()
                        );
                        let cloned_facts = closed.as_ref().clone();
                        assert!(
                            flow.validate_result(
                                flow.view(),
                                &cloned_facts,
                                ir::SignatureSubject::Body,
                                &boundary.signature,
                                location
                            )
                            .is_err()
                        );
                        let other_subject = ir::Callee::Direct(target.clone());
                        assert!(
                            flow.validate_result(
                                flow.view(),
                                &closed,
                                ir::SignatureSubject::Call(&other_subject),
                                &boundary.signature,
                                location
                            )
                            .is_err()
                        );
                        let mut changed_signature = boundary.signature.clone();
                        changed_signature.result = ir::Ty::Bool;
                        assert!(
                            flow.validate_result(
                                flow.view(),
                                &closed,
                                ir::SignatureSubject::Body,
                                &changed_signature,
                                location
                            )
                            .is_err()
                        );
                        let mut changed_location = location.clone();
                        changed_location.path =
                            ir::CarrierPath(Arc::from([ir::CarrierStep::Field(usize::MAX)]));
                        assert!(
                            flow.validate_result(
                                flow.view(),
                                &closed,
                                ir::SignatureSubject::Body,
                                &boundary.signature,
                                &changed_location
                            )
                            .is_err()
                        );
                    }
                    let original_contract = boundary.contract.clone();
                    boundary.contract = ir::CarrierContract::default();
                    assert!(
                        boundary.validate().is_err(),
                        "deleted reference row accepted"
                    );
                    boundary.contract = original_contract.clone();
                    let mut rows = boundary.contract.inputs.to_vec();
                    rows.push(rows[0].clone());
                    boundary.contract.inputs = rows.into();
                    assert!(boundary.validate().is_err(), "added reference row accepted");
                    boundary.contract = original_contract.clone();
                    let mut rows = boundary.contract.inputs.to_vec();
                    rows[0].param = 99;
                    boundary.contract.inputs = rows.into();
                    assert!(
                        boundary.validate().is_err(),
                        "changed reference origin accepted"
                    );
                    boundary.contract = original_contract;
                    let signature = boundary.signature.clone();
                    boundary.signature.params.push(ir::Ty::Bool);
                    assert!(boundary.validate().is_err(), "added parameter accepted");
                    boundary.signature = signature.clone();
                    boundary.signature.params.remove(0);
                    assert!(boundary.validate().is_err(), "deleted parameter accepted");
                    boundary.signature = signature.clone();
                    boundary.signature.params.swap(0, 1);
                    assert!(
                        boundary.validate().is_err(),
                        "reordered parameters accepted"
                    );
                    boundary.signature = signature.clone();
                    boundary.signature.result = ir::Ty::Bool;
                    assert!(
                        boundary.validate().is_err(),
                        "deleted result carrier accepted"
                    );
                    boundary.signature = signature;
                    let mut wrong_contract = boundary.signature.ref_contract.as_ref().clone();
                    wrong_contract.region_count += 1;
                    boundary.signature.ref_contract = Arc::new(wrong_contract.clone());
                    assert!(
                        boundary.validate().is_err(),
                        "changed reference contract accepted"
                    );
                    let mut forged = physical.function().clone();
                    forged.ref_contract = Arc::new(wrong_contract);
                    assert!(validation.validate_cfg(&forged, &closed).is_err());
                    cx.publish_fact(0, ())
                })
            }
        }
        for (source, workers, expect_realization) in [
            ("fn sink<T>(x:&i32,y:T)->i32{*x} fn main(){}", 1, true),
            ("fn sink<T>(x:&i32,y:T)->i32{*x} fn main(){}", 8, true),
            ("fn relay<T>(x:&i32,y:T)->&i32{x} fn main(){}", 1, true),
            ("fn relay<T>(x:&i32,y:T)->&i32{x} fn main(){}", 8, true),
            (
                "fn relay<'a,T>(x:&'a i32,y:T)->&'a i32{x} fn main(){}",
                1,
                true,
            ),
            (
                "fn relay<'a,T>(x:&'a i32,y:T)->&'a i32{x} fn main(){}",
                8,
                true,
            ),
            ("fn make<'a,T>(x:T)->impl Sized{7i32} fn main(){}", 1, false),
            ("fn make<'a,T>(x:T)->impl Sized{7i32} fn main(){}", 8, false),
        ] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(source.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(crate::compiler::keys::test_key(
                9017,
                vec![u64::from(expect_realization)],
            ));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }

    #[test]
    fn physical_transport_keeps_exact_result_and_write_sources() {
        let budget = ir::PlanBudget::default();
        let reference = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        };
        let location = ir::carrier_locations(&reference, &budget).unwrap()[0].clone();
        let source = ir::InputCarrier {
            param: 0,
            location: location.clone(),
        };
        let signature = ir::Signature {
            params: vec![reference.clone()],
            result: reference,
            ref_contract: Arc::new(ir::RefContract::default()),
        };
        let contract = ir::CarrierContract {
            inputs: Arc::from([source.clone()]),
            results: Arc::from([ir::CarrierReturn {
                location,
                sources: Arc::from([source.clone()]),
            }]),
            writes: Arc::from([ir::CarrierWriteEnvelope {
                destination: source.clone(),
                sources: Arc::from([source]),
            }]),
        };
        assert_eq!(
            transport_contract(&contract, &signature, &budget).unwrap(),
            contract
        );
        for result in [true, false] {
            let mut forged = contract.clone();
            let mut bad_source = forged.inputs[0].clone();
            bad_source.param = 1;
            if result {
                let mut rows = forged.results.to_vec();
                rows[0].sources = Arc::from([bad_source]);
                forged.results = rows.into();
            } else {
                let mut rows = forged.writes.to_vec();
                rows[0].sources = Arc::from([bad_source]);
                forged.writes = rows.into();
            }
            assert!(transport_contract(&forged, &signature, &budget).is_err());
        }
    }
}
