//! Scoped, private authorization for equality-dependent result carriers.
//!
//! This is not a carrier-family relation and does not establish Copy. It seals
//! the obligation to reject every executable reference-bearing substitution.
use super::*;

// Charge each new allocation before reserving, then reconcile any capacity the
// allocator supplied beyond the request. The sticky ledger also retains the old
// allocation, so relocation peaks remain covered without guessing growth.
fn result_push<T>(budget: &PlanBudget, values: &mut Vec<T>, value: T) -> Result<()> {
    if budget.has_resource_failure() {
        return Err(FactsError::resource(
            budget,
            "equality result budget already exhausted",
        ));
    }
    charged(budget, budget.charge_work(1))?;
    if values.len() == values.capacity() {
        let requested = values
            .len()
            .checked_add(1)
            .ok_or_else(|| FactsError::resource(budget, "equality result capacity overflow"))?;
        retain::<T>(budget, requested)?;
        charged(budget, budget.charge_work(values.len()))?;
        values
            .try_reserve_exact(1)
            .map_err(|_| FactsError::resource(budget, "equality result allocation failed"))?;
        let excess = values.capacity().saturating_sub(requested);
        let bytes = excess
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| FactsError::resource(budget, "equality result capacity overflow"))?;
        charged(budget, budget.retain_bytes(bytes))?;
    }
    values.push(value);
    Ok(())
}

pub(super) fn result_slice<T>(budget: &PlanBudget, values: Vec<T>) -> Result<Arc<[T]>> {
    if budget.has_resource_failure() {
        return Err(FactsError::resource(
            budget,
            "equality result budget already exhausted",
        ));
    }
    // Vec-to-Arc conversion allocates a separate, exactly-sized slice while the
    // source allocation is still live.
    retain::<T>(budget, values.len())?;
    charged(budget, budget.charge_work(values.len()))?;
    Ok(values.into())
}

fn result_signature(budget: &PlanBudget, signature: &Signature) -> Result<Signature> {
    let mut params = Vec::new();
    for ty in &signature.params {
        result_push(budget, &mut params, ty.clone())?;
    }
    Ok(Signature {
        params,
        result: signature.result.clone(),
        ref_contract: signature.ref_contract.clone(),
    })
}

fn result_schema(budget: &PlanBudget, schema: &GenericSchema) -> Result<GenericSchema> {
    Ok(GenericSchema {
        env: schema.env.clone(),
        signature: result_signature(budget, &schema.signature)?,
        predicates: schema.predicates.clone(),
    })
}

#[derive(Clone, Debug)]
pub(super) struct EqualityObligationOrigin {
    declaration: DeclId,
    occurrence: usize,
    row: Predicate,
}

#[derive(Clone, Debug)]
pub(super) enum EqualityResultSubject {
    Body {
        declaration: DeclId,
        arguments: Arc<[Ty]>,
    },
    Direct(CallTarget),
    Required(TraitCall),
}

#[derive(Clone, Debug)]
struct RestrictedResultCertificate {
    origin: EqualityObligationOrigin,
    rhs_parameter: ParamId,
    substituted_row: Predicate,
    result: ir::CarrierLocation,
    dependencies: Arc<[usize]>,
}

#[derive(Clone, Debug)]
pub(super) struct EqualityResultAuthority {
    world: WorldId,
    environment: usize,
    phase: FactsPhase,
    finalized: bool,
    stage: Option<equality::DeclaredStageId>,
    subject: EqualityResultSubject,
    raw: GenericSchema,
    origins: Arc<[EqualityObligationOrigin]>,
    actual_rows: Arc<[Predicate]>,
    signature: Signature,
    source_regions: Arc<ir::SourceRegionEnv>,
    origin: Arc<ir::SignatureOriginSchema>,
    fixed: Arc<ir::RefContract>,
    regions: ir::CarrierRegionSchema,
    original_contract: ir::CarrierContract,
    contract: ir::CarrierContract,
    components: Arc<[ir::CarrierComponentMapping]>,
    nominals: Arc<[ir::NominalCarrierMapping]>,
    dependencies: Arc<[(ir::AssociatedTypeProjection, equality::ProjectionReduction)]>,
    certificates: Arc<[RestrictedResultCertificate]>,
}

impl CarrierContractView {
    /// A restricted result receipt must retain a real equality-dependent
    /// certificate, never infer authority from an empty source union.
    pub(in crate::compiler) fn validate_restricted_result(
        &self,
        facts: &ValidatedFacts,
        subject: ir::SignatureSubject<'_>,
        signature: &ir::Signature,
        location: &ir::CarrierLocation,
    ) -> Result<()> {
        self.validate_result_flow(facts, subject, signature, location)?;
        let Some(authority) = &self.equality_results else {
            return Err(FactsError::invalid(
                facts.budget(),
                "closure result lacks equality-origin authority",
            ));
        };
        if !authority
            .certificates
            .iter()
            .any(|certificate| certificate.result == *location)
            || !self
                .contract
                .results
                .iter()
                .any(|row| row.location == *location && row.sources.is_empty())
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "closure result is not an exact restricted equality-dependent result",
            ));
        }
        Ok(())
    }
    /// Consumer-side guard. Coverage is rederived from immutable declarations,
    /// never inferred from the presence of an optional certificate or an empty
    /// source union. Ordinary, non-equality result rows retain their old rules.
    pub(in crate::compiler) fn validate_result_flow(
        &self,
        facts: &ValidatedFacts,
        actual_subject: ir::SignatureSubject<'_>,
        actual_signature: &ir::Signature,
        result_location: &ir::CarrierLocation,
    ) -> Result<()> {
        self.validate_result_flow_with(
            facts,
            actual_subject,
            actual_signature,
            result_location,
            &mut false,
        )
    }

    /// As `validate_result_flow`; `view_validated` is caller-local state for
    /// one loop over this same view and facts. The complete view replay runs
    /// once per loop; subject, signature and location checks run every time.
    pub(in crate::compiler) fn validate_result_flow_with(
        &self,
        facts: &ValidatedFacts,
        actual_subject: ir::SignatureSubject<'_>,
        actual_signature: &ir::Signature,
        result_location: &ir::CarrierLocation,
        view_validated: &mut bool,
    ) -> Result<()> {
        if !*view_validated {
            self.validate_for(facts)?;
            *view_validated = true;
        }
        let world = facts.world();
        let subject = world.equality_result_subject(actual_subject, facts)?;
        if !world.same_equality_result_subject(&self.result_subject, &subject)?
            || !world.eq_signature(&self.result_signature, actual_signature)?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "result flow changed exact subject or signature",
            ));
        }
        facts.validate_signature(actual_subject, actual_signature)?;
        let locations = charged(
            facts.budget(),
            ir::carrier_locations(&actual_signature.result, facts.budget()),
        )?;
        let mut found = false;
        for location in locations.iter() {
            charged(facts.budget(), facts.budget().charge_work(1))?;
            found |= location == result_location;
        }
        if !found {
            return Err(FactsError::invalid(
                facts.budget(),
                "result flow location is outside the actual result",
            ));
        }
        // validate_for already requires exact, complete coverage, including
        // equality-dependent rows with nonempty conservative source unions.
        Ok(())
    }
}

impl ValidatedTraitWorld {
    fn clone_equality_origins(
        &self,
        origins: &[EqualityObligationOrigin],
    ) -> Result<Vec<EqualityObligationOrigin>> {
        let mut values = Vec::new();
        for origin in origins {
            result_push(&self.budget, &mut values, origin.clone())?;
        }
        Ok(values)
    }

    pub(super) fn retain_equality_origins(
        &self,
        declaration: DeclId,
        rows: &[Predicate],
        origins: &mut Vec<EqualityObligationOrigin>,
    ) -> Result<()> {
        for (occurrence, row) in rows.iter().enumerate() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if matches!(row, Predicate::ProjectionEq { .. }) {
                result_push(
                    &self.budget,
                    origins,
                    EqualityObligationOrigin {
                        declaration,
                        occurrence,
                        row: row.clone(),
                    },
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn replay_equality_rows(
        &self,
        rows: &[Predicate],
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        // Metadata errors win over domain restrictions, even for unused rows.
        for row in rows {
            self.validate_predicate_metadata(row, context.env())?;
        }
        self.replay_validated_equality_rows(rows, context, active)
    }

    /// Discharge rows whose metadata has already been checked as a complete
    /// batch. Keep this separate from standalone replay so callers cannot
    /// accidentally change metadata-error precedence by filtering first.
    pub(super) fn replay_validated_equality_rows(
        &self,
        rows: &[Predicate],
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        // Keep original occurrence rows in the carrier seal, but replay their
        // newly closed nested actuals through canonical normalization first.
        // The helper validates every normalization dependency and preserves the
        // root associated item and ordered trait application. Open families
        // are retained unchanged, never assigned by this replay operation.
        let normalized = self.normalize_instantiated_projection_predicates(rows, context)?;
        for row in normalized.iter() {
            if let Predicate::ProjectionEq { projection, ty } = row {
                self.validate_equality_domain_in(projection, ty, context, active)?;
                self.require_in(row, context, active)?;
            }
        }
        Ok(())
    }

    pub(super) fn equality_result_subject(
        &self,
        subject: SignatureSubject<'_>,
        facts: &ValidatedFacts,
    ) -> Result<EqualityResultSubject> {
        Ok(match subject {
            SignatureSubject::Body => match &facts.subject {
                FactsSubject::UniversalCallable(declaration) => {
                    let mut arguments = Vec::new();
                    for parameter in facts.env().ordered_params() {
                        result_push(&self.budget, &mut arguments, Ty::Param(parameter))?;
                    }
                    EqualityResultSubject::Body {
                        declaration: *declaration,
                        arguments: result_slice(&self.budget, arguments)?,
                    }
                }
                FactsSubject::ClosedInstance(target) => EqualityResultSubject::Body {
                    declaration: target.declaration,
                    arguments: target.type_args.clone(),
                },
                FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "schema has no result-flow body authority",
                    ));
                }
            },
            SignatureSubject::Call(Callee::Direct(target)) => {
                EqualityResultSubject::Direct(target.clone())
            }
            SignatureSubject::Call(
                Callee::TraitItem(call)
                | Callee::Virtual(ir::VirtualCall { call })
                | Callee::Untupled(call),
            ) => EqualityResultSubject::Required(call.clone()),
        })
    }

    fn same_equality_result_subject(
        &self,
        a: &EqualityResultSubject,
        b: &EqualityResultSubject,
    ) -> Result<bool> {
        Ok(match (a, b) {
            (
                EqualityResultSubject::Body {
                    declaration: a,
                    arguments: aa,
                },
                EqualityResultSubject::Body {
                    declaration: b,
                    arguments: ba,
                },
            ) => a == b && self.eq_types(aa, ba, 0)?,
            (EqualityResultSubject::Direct(a), EqualityResultSubject::Direct(b)) => {
                a.declaration == b.declaration && self.eq_types(&a.type_args, &b.type_args, 0)?
            }
            (EqualityResultSubject::Required(a), EqualityResultSubject::Required(b)) => {
                a.item == b.item
                    && self.eq_types(&a.method_args, &b.method_args, 0)?
                    && self.eq_predicate(
                        &Predicate::Implements(a.trait_ref.clone()),
                        &Predicate::Implements(b.trait_ref.clone()),
                    )?
            }
            _ => false,
        })
    }

    pub(super) fn seal_equality_results(
        &self,
        view: &mut CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        view.equality_results = self.derive_equality_results(view, facts)?;
        Ok(())
    }

    /// Recompute the obligation set even if the caller removed the optional
    /// authority. No client-supplied flags decide whether sealing is required.
    pub(super) fn validate_equality_results(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let expected = self.derive_equality_results(view, facts)?;
        match (&view.equality_results, expected) {
            (None, None) => Ok(()),
            (Some(stored), Some(expected)) => {
                self.compare_equality_results(stored, &expected, view, facts)
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "missing or surplus restricted result authority",
            )),
        }
    }

    fn derive_equality_results(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<Option<Arc<EqualityResultAuthority>>> {
        self.check_facts(facts, true)?;
        if !self.owner_normalization_finalized {
            return Err(FactsError::invalid(
                &self.budget,
                "private-stage result authority cannot escape",
            ));
        }
        let (raw, arguments, mut origins, stage) = match &view.result_subject {
            EqualityResultSubject::Body {
                declaration,
                arguments,
            } => {
                let actual = self.equality_result_subject(SignatureSubject::Body, facts)?;
                if !self.same_equality_result_subject(&view.result_subject, &actual)?
                    || *declaration != view.declaration
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "body result authority changed declaration",
                    ));
                }
                let context = self.declared_callable_context(*declaration)?;
                // The immutable context includes owner predicates as well as
                // callable predicates. With no origins there is no obligation
                // to instantiate or result proof to retain on this path.
                if context.origins.is_empty() {
                    return Ok(None);
                }
                (
                    result_schema(&self.budget, &context.raw)?,
                    arguments.clone(),
                    self.clone_equality_origins(&context.origins)?,
                    Some(context.stage),
                )
            }
            EqualityResultSubject::Direct(target) => {
                if target.declaration != view.declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "direct result authority changed declaration",
                    ));
                }
                let context = self.declared_callable_context(target.declaration)?;
                if context.origins.is_empty() {
                    return Ok(None);
                }
                (
                    result_schema(&self.budget, &context.raw)?,
                    target.type_args.clone(),
                    self.clone_equality_origins(&context.origins)?,
                    Some(context.stage),
                )
            }
            EqualityResultSubject::Required(call) => {
                if call.item != view.declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "required result authority changed declaration",
                    ));
                }
                let schema = &self.item_header(call.item)?.schema;
                // A required projection can acquire caller-origin obligations
                // even when the item's own predicate rows contain no equality.
                // Only a projection-free, equality-free item is an empty path.
                if !schema
                    .predicates
                    .iter()
                    .any(|row| matches!(row, Predicate::ProjectionEq { .. }))
                    && !self.schema_contains_projection(schema)?
                {
                    return Ok(None);
                }
                let raw = result_schema(&self.budget, schema)?;
                let mut origins = Vec::new();
                self.retain_equality_origins(call.item, &raw.predicates, &mut origins)?;
                let mut arguments = Vec::new();
                for argument in std::iter::once(&call.trait_ref.self_ty)
                    .chain(call.trait_ref.args.iter())
                    .chain(call.method_args.iter())
                {
                    result_push(&self.budget, &mut arguments, argument.clone())?;
                }
                (raw, result_slice(&self.budget, arguments)?, origins, None)
            }
        };
        self.validate_declaration_schema(view.declaration, &raw)?;
        if let EqualityResultSubject::Required(call) = &view.result_subject
            && self.schema_contains_projection(&raw)?
        {
            let closed = self.goal_is_closed(&call.trait_ref, facts.env())?
                || matches!(&call.trait_ref.self_ty, Ty::Dyn(_));
            if (closed && view.projected.is_none())
                || (!closed && view.equality_required.is_none() && view.projected.is_none())
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required result lost its checked projection authority",
                ));
            }
        }
        let mut dependencies = Vec::new();
        let mut caller_origins = Vec::new();
        let mut caller_stage = stage;
        // Required projections have caller equality dependencies. Their original
        // source regions and raw signature are always the required item's, not
        // a selected implementation's topology.
        if let EqualityResultSubject::Required(call) = &view.result_subject
            && self.schema_contains_projection(&raw)?
            && !self.goal_is_closed(&call.trait_ref, facts.env())?
            && !matches!(&call.trait_ref.self_ty, Ty::Dyn(_))
        {
            let (application, authority) = self.instantiate_universal_required_item(call, facts)?;
            if !self.eq_signature(&application.signature, &view.result_signature)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required result signature changed",
                ));
            }
            let declaration = facts.predicates.declaration.ok_or_else(|| {
                FactsError::invalid(&self.budget, "required equality lost caller declaration")
            })?;
            let context = self.declared_callable_context(declaration)?;
            caller_stage = Some(context.stage);
            for origin in context.origins.iter() {
                let Predicate::ProjectionEq { projection, ty } = &origin.row else {
                    unreachable!()
                };
                for (index, (actual, proof)) in authority.dependencies.iter().enumerate() {
                    if let equality::ProjectionReduction::Equality(proof) = proof
                        && self.same_projection(projection, actual)?
                        && self.eq_ty(ty, proof.ty(), 0)?
                    {
                        result_push(&self.budget, &mut caller_origins, (origin.clone(), index))?;
                    }
                }
            }
            for dependency in authority.dependencies.iter() {
                result_push(&self.budget, &mut dependencies, dependency.clone())?;
            }
            // Discharge all raw caller occurrences, not just those visible in
            // the signature. The dependency selection above only scopes result
            // coverage; it is never an obligation filter.
            let mut rows = Vec::new();
            for origin in context.origins.iter() {
                result_push(&self.budget, &mut rows, origin.row.clone())?;
            }
            self.replay_equality_rows(
                &rows,
                ProofContext::from_facts(facts),
                &mut ProofScratch::default(),
            )?;
        }
        let mut raw_rows = Vec::new();
        for origin in &origins {
            result_push(&self.budget, &mut raw_rows, origin.row.clone())?;
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: raw.env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: Ty::Unit,
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: result_slice(&self.budget, raw_rows)?,
        };
        let instantiated = self.substitute_projected_schema(&schema, &arguments, facts.env())?;
        self.replay_equality_rows(
            &instantiated.predicates,
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        let mut actual_rows = Vec::new();
        let mut certificates = Vec::new();
        for (origin, actual) in origins.iter().zip(instantiated.predicates.iter()) {
            result_push(&self.budget, &mut actual_rows, actual.clone())?;
            self.certify_equality_results(view, origin, actual, &[], &mut certificates, facts)?;
        }
        for (origin, dependency) in caller_origins {
            self.certify_equality_results(
                view,
                &origin,
                &origin.row,
                &[dependency],
                &mut certificates,
                facts,
            )?;
            result_push(&self.budget, &mut actual_rows, origin.row.clone())?;
            result_push(&self.budget, &mut origins, origin)?;
        }
        if origins.is_empty() {
            return Ok(None);
        }
        self.validate_result_source_transport(view, &raw, facts)?;
        // Source transport above reconstructs the canonical body or direct
        // signature, including direct requirements, before comparing it.
        // Keep metadata validation here without discharging the same target
        // again. Closure bodies and required calls retain their distinct
        // subject-specific validation below.
        match &view.result_subject {
            EqualityResultSubject::Body { .. } if facts.closure.is_some() => {
                facts.validate_signature(SignatureSubject::Body, &view.result_signature)?
            }
            EqualityResultSubject::Body { .. } => {
                self.validate_signature_shape(&view.result_signature, facts.env(), None)?;
            }
            // Source transport above already discharges this exact direct
            // target, checks every predicate, and compares its canonical
            // signature against result_signature. Repeating validate_signature
            // here rebuilds the same nested projection proofs without adding
            // an independent check.
            EqualityResultSubject::Direct(_) => {
                self.validate_signature_shape(&view.result_signature, facts.env(), None)?;
            }
            EqualityResultSubject::Required(call) => facts.validate_signature(
                SignatureSubject::Call(&Callee::TraitItem(call.clone())),
                &view.result_signature,
            )?,
        }
        retain::<EqualityResultAuthority>(&self.budget, 1)?;
        Ok(Some(Arc::new(EqualityResultAuthority {
            world: self.id,
            environment: facts.predicates.index,
            phase: facts.phase(),
            finalized: self.owner_normalization_finalized,
            stage: caller_stage,
            subject: view.result_subject.clone(),
            raw,
            origins: result_slice(&self.budget, origins)?,
            actual_rows: result_slice(&self.budget, actual_rows)?,
            signature: result_signature(&self.budget, &view.result_signature)?,
            source_regions: view.source_regions.clone(),
            origin: view.origin.clone(),
            fixed: view.fixed.clone(),
            regions: view.regions.clone(),
            original_contract: view.original_contract.clone(),
            contract: view.contract.clone(),
            components: view.components.clone(),
            nominals: view.nominals.clone(),
            dependencies: result_slice(&self.budget, dependencies)?,
            certificates: result_slice(&self.budget, certificates)?,
        })))
    }

    fn validate_result_source_transport(
        &self,
        view: &CarrierContractView,
        raw: &GenericSchema,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        if let EqualityResultSubject::Required(call) = &view.result_subject
            && self.schema_contains_projection(raw)?
        {
            if self.goal_is_closed(&call.trait_ref, facts.env())? {
                // validate_for replayed the mandatory selected required view.
                return Ok(());
            }
            let source = self.validate_source_regions(call.item, &raw.signature)?;
            let (application, _) = self.instantiate_universal_required_item(call, facts)?;
            let signature = &application.signature;
            let (contract, regions) = self.derive_carriers(signature, &source)?;
            let origin = charged(
                &self.budget,
                ir::derive_signature_origin_schema(signature, facts.env(), &self.budget),
            )?;
            if source != view.source_regions
                || contract != view.contract
                || contract != view.original_contract
                || regions != view.regions
                || origin != view.origin
                || view.fixed != signature.ref_contract
                || !self.same_env(&view.original_env, facts.env())?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required result changed original source topology",
                ));
            }
            return Ok(());
        }
        let canonical = self.normalized_declaration_schema(view.declaration)?;
        let retained = self.declaration_carriers(view.declaration)?;
        let source = self.validate_source_regions(view.declaration, &canonical.signature)?;
        if source != retained.source_regions
            || view.original_contract != retained.contract
            || !self.same_env(&view.original_env, &canonical.env)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed immutable declaration topology",
            ));
        }
        let expected_arguments = match &view.result_subject {
            EqualityResultSubject::Body { arguments, .. } => arguments.clone(),
            EqualityResultSubject::Direct(target) => target.type_args.clone(),
            EqualityResultSubject::Required(call) => {
                let mut arguments = Vec::new();
                for argument in std::iter::once(&call.trait_ref.self_ty)
                    .chain(call.trait_ref.args.iter())
                    .chain(call.method_args.iter())
                {
                    result_push(&self.budget, &mut arguments, argument.clone())?;
                }
                result_slice(&self.budget, arguments)?
            }
        };
        if !self.eq_types(&expected_arguments, &view.arguments, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed full substitution",
            ));
        }
        // Reconstruct from declaration authority, not from the candidate's
        // origin, region schema, or nominal/component mappings. In particular,
        // a concrete target signature cannot replace the original ParamTails.
        let signature = match &view.result_subject {
            EqualityResultSubject::Body { .. }
                if matches!(facts.subject, FactsSubject::UniversalCallable(_)) =>
            {
                result_signature(&self.budget, &canonical.signature)?
            }
            EqualityResultSubject::Body {
                declaration,
                arguments,
            } => {
                self.discharge_direct(
                    &CallTarget {
                        declaration: *declaration,
                        type_args: arguments.clone(),
                    },
                    facts,
                )?
                .signature
            }
            EqualityResultSubject::Direct(target) => {
                self.discharge_direct(target, facts)?.signature
            }
            EqualityResultSubject::Required(call) => {
                self.instantiate_trait_signature(call, facts)?
            }
        };
        if !self.eq_signature(&signature, &view.result_signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed canonical signature",
            ));
        }
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&canonical.signature, &canonical.env, &self.budget),
        )?;
        let expected = self.transport_carrier_parts(
            view.declaration,
            expected_arguments,
            signature,
            facts,
            result_schema(&self.budget, &canonical)?,
            retained,
            origin,
            None,
            view.result_subject.clone(),
        )?;
        if view.origin != expected.origin
            || view.fixed != expected.fixed
            || view.regions != expected.regions
            || view.contract != expected.contract
            || view.components != expected.components
            || view.nominals != expected.nominals
            || view.source_regions != expected.source_regions
        {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed canonical origin, fixed or region topology",
            ));
        }
        let mut normalized = Vec::new();
        for original in source.normalized.iter() {
            let mapped = original
                .map(|id| {
                    self.transport_fixed_region(
                        id,
                        &canonical.signature.ref_contract,
                        &view.result_signature.ref_contract,
                    )
                })
                .transpose()?;
            result_push(&self.budget, &mut normalized, mapped)?;
        }
        if source.declaration != view.source_regions.declaration
            || source.region_count != view.source_regions.region_count
            || source.outlives != view.source_regions.outlives
            || normalized.as_slice() != view.source_regions.normalized.as_ref()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed original source regions",
            ));
        }
        let mut expected_results = Vec::new();
        for result in retained.contract.results.iter() {
            for output in self.expand_carrier(
                &result.location,
                &canonical.env,
                &view.arguments,
                &view.nominals,
                facts,
            )? {
                let sources = self.expand_carrier_sources(
                    &result.location.channel,
                    &result.sources,
                    output.component.as_ref(),
                    &canonical.env,
                    &view.arguments,
                    &view.nominals,
                    facts,
                )?;
                result_push(
                    &self.budget,
                    &mut expected_results,
                    ir::CarrierReturn {
                        location: output.location,
                        sources,
                    },
                )?;
            }
        }
        if expected_results.as_slice() != view.contract.results.as_ref() {
            return Err(FactsError::invalid(
                &self.budget,
                "result transport changed exact carrier locations or sources",
            ));
        }
        Ok(())
    }

    fn certify_equality_results(
        &self,
        view: &CarrierContractView,
        origin: &EqualityObligationOrigin,
        actual: &Predicate,
        dependencies: &[usize],
        certificates: &mut Vec<RestrictedResultCertificate>,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let Predicate::ProjectionEq {
            ty: Ty::Param(rhs_parameter),
            ..
        } = &origin.row
        else {
            return Ok(());
        };
        let Predicate::ProjectionEq { ty, .. } = actual else {
            return Err(FactsError::invalid(
                &self.budget,
                "result obligation lost equality kind",
            ));
        };
        // Concrete substitutions are still checked above even if their tail
        // disappears. Only exact original ParamTail coordinates yield coverage.
        for result in view.original_contract.results.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if !matches!(result.location.channel, ir::CarrierChannel::ParamTail { parameter, .. } if parameter == *rhs_parameter)
            {
                continue;
            }
            let expanded = if dependencies.is_empty() {
                self.expand_carrier(
                    &result.location,
                    &view.original_env,
                    &view.arguments,
                    &view.nominals,
                    facts,
                )?
            } else {
                let mut expanded = Vec::new();
                result_push(
                    &self.budget,
                    &mut expanded,
                    ExpandedCarrier {
                        location: result.location.clone(),
                        component: None,
                    },
                )?;
                expanded
            };
            for output in expanded {
                if let ir::CarrierChannel::ParamTail { parameter, .. } = output.location.channel
                    && matches!(ty, Ty::Param(actual) if *actual == parameter)
                {
                    retain::<usize>(&self.budget, dependencies.len())?;
                    result_push(
                        &self.budget,
                        certificates,
                        RestrictedResultCertificate {
                            origin: origin.clone(),
                            rhs_parameter: *rhs_parameter,
                            substituted_row: actual.clone(),
                            result: output.location,
                            dependencies: dependencies.into(),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn compare_equality_results(
        &self,
        stored: &EqualityResultAuthority,
        expected: &EqualityResultAuthority,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        if stored.world != self.id
            || stored.environment != facts.predicates.index
            || stored.phase != facts.phase()
            || stored.finalized != self.owner_normalization_finalized
            || stored.stage != expected.stage
            || !self.same_equality_result_subject(&stored.subject, &expected.subject)?
            || !self.same_env(&stored.raw.env, &expected.raw.env)?
            || !self.eq_signature(&stored.raw.signature, &expected.raw.signature)?
            || !self.eq_signature(&stored.signature, &expected.signature)?
            || stored.origin != expected.origin
            || stored.fixed != expected.fixed
            || stored.regions != expected.regions
            || stored.source_regions != view.source_regions
            || stored.original_contract != view.original_contract
            || stored.contract != view.contract
            || stored.components != view.components
            || stored.nominals != view.nominals
            || stored.origins.len() != expected.origins.len()
            || stored.actual_rows.len() != expected.actual_rows.len()
            || stored.dependencies.len() != expected.dependencies.len()
            || stored.certificates.len() != expected.certificates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "restricted result authority changed context, epoch, source or coverage",
            ));
        }
        self.compare_result_rows(&stored.raw.predicates, &expected.raw.predicates)?;
        self.compare_result_rows(&stored.actual_rows, &expected.actual_rows)?;
        for (a, b) in stored.origins.iter().zip(expected.origins.iter()) {
            self.compare_result_origin(a, b)?;
        }
        for ((a, proof), (b, _)) in stored.dependencies.iter().zip(expected.dependencies.iter()) {
            if !self.same_projection(a, b)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "result dependency changed projection",
                ));
            }
            match proof {
                equality::ProjectionReduction::Equality(proof) => proof.validate_for(a, facts)?,
                equality::ProjectionReduction::Implementation(proof) => {
                    proof.validate_for(a, facts)?
                }
            }
        }
        for (a, b) in stored.certificates.iter().zip(expected.certificates.iter()) {
            charged(&self.budget, self.budget.charge_work(1))?;
            self.compare_result_origin(&a.origin, &b.origin)?;
            if a.rhs_parameter != b.rhs_parameter
                || a.result != b.result
                || a.dependencies != b.dependencies
                || !self.eq_predicate(&a.substituted_row, &b.substituted_row)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "restricted result certificate changed row, parameter, location or dependencies",
                ));
            }
        }
        Ok(())
    }

    fn compare_result_origin(
        &self,
        a: &EqualityObligationOrigin,
        b: &EqualityObligationOrigin,
    ) -> Result<()> {
        if a.declaration != b.declaration
            || a.occurrence != b.occurrence
            || !self.eq_predicate(&a.row, &b.row)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "equality result lost exact raw occurrence origin",
            ));
        }
        Ok(())
    }

    fn compare_result_rows(&self, a: &[Predicate], b: &[Predicate]) -> Result<()> {
        if a.len() != b.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "equality result row count changed",
            ));
        }
        for (a, b) in a.iter().zip(b) {
            if !self.eq_predicate(a, b)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "equality result row changed",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn source_world(copy: bool) -> (Arc<ValidatedTraitWorld>, Arc<ValidatedFacts>) {
        let bound = if copy { ":Copy" } else { "" };
        let source = frontend::parse(&format!(
            "trait Source:Sized{{type Out;fn get(self)->Self::Out;}}\
             fn produce<T:Source<Out=U>,U{bound}>(x:T)->U{{x.get()}}\
             fn main(){{}}"
        ))
        .unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = world
            .headers
            .callables
            .iter()
            .find(|header| header.schema.env.total_arity() == 2)
            .unwrap()
            .declaration;
        let facts = world.callable_facts(declaration).unwrap();
        (world, facts)
    }

    fn body_view(facts: &ValidatedFacts) -> CarrierContractView {
        Arc::try_unwrap(facts.carrier_contract(SignatureSubject::Body).unwrap()).unwrap()
    }

    fn reject_mutation(facts: &ValidatedFacts, change: impl FnOnce(&mut EqualityResultAuthority)) {
        let mut view = body_view(facts);
        change(Arc::make_mut(view.equality_results.as_mut().unwrap()));
        assert_eq!(
            view.validate_for(facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn closed_projected_callable_rows_replay_exact_owner_arguments_and_occurrences() {
        fn fixture() -> (
            Arc<ValidatedTraitWorld>,
            Arc<ValidatedFacts>,
            Arc<ValidatedFacts>,
        ) {
            let source = frontend::parse("trait Has{type Out;} trait Call<A,B>{type Output;} struct S; impl Has for S{type Out=i32;} struct C; impl Call<(i32,),bool> for C{type Output=i32;} fn keep<I:Has,F:Call<(I::Out,),bool,Output=i32>>(f:F,x:I::Out)->i32{0} fn concrete(s:S,c:C){} fn main(){}").unwrap();
            let plan = PlanCtx::from_program(&source, &PlanBudget::default()).unwrap();
            let world = plan.trait_world().unwrap();
            let keep = plan.function_decl(0).unwrap();
            let concrete = plan.function_decl(1).unwrap();
            let arguments = world
                .callable_header(concrete)
                .unwrap()
                .schema
                .signature
                .params
                .clone();
            let closed = world
                .instance_facts(&CallTarget {
                    declaration: keep,
                    type_args: arguments.into(),
                })
                .unwrap();
            let universal = world.callable_facts(keep).unwrap();
            (world, closed, universal)
        }
        let (_world, facts, universal) = fixture();
        let view = body_view(&facts);
        assert!(view.equality_results.is_some());
        view.validate_for(&facts).unwrap();
        assert!(view.validate_for(&universal).is_err());
        let (_, foreign, _) = fixture();
        assert!(view.validate_for(&foreign).is_err());
        reject_mutation(&facts, |authority| authority.actual_rows = Arc::from([]));
        reject_mutation(&facts, |authority| authority.origins = Arc::from([]));
        reject_mutation(&facts, |authority| {
            let rows = Arc::make_mut(&mut authority.actual_rows);
            let Predicate::ProjectionEq { projection, .. } = &mut rows[0] else {
                panic!("missing original equality occurrence")
            };
            let args = Arc::make_mut(&mut projection.trait_ref.args);
            args.swap(0, 1);
        });
        reject_mutation(&facts, |authority| {
            let rows = Arc::make_mut(&mut authority.actual_rows);
            let Predicate::ProjectionEq { projection, .. } = &mut rows[0] else {
                panic!("missing original equality occurrence")
            };
            projection.item.index = projection.item.index.wrapping_add(1);
        });
        reject_mutation(&facts, |authority| {
            let origins = Arc::make_mut(&mut authority.origins);
            origins[0].occurrence = origins[0].occurrence.wrapping_add(1);
        });
    }

    #[test]
    fn ordinary_result_replay_does_not_retain_equality_proof_storage() {
        let source = frontend::parse("fn plain(x:i32)->i32{x} fn main(){}").unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = world
            .headers
            .callables
            .iter()
            .find(|header| header.schema.signature.params.len() == 1)
            .unwrap()
            .declaration;
        let facts = world.callable_facts(declaration).unwrap();
        let callee = Callee::Direct(CallTarget {
            declaration,
            type_args: Arc::from([]),
        });
        let view = facts
            .carrier_contract(SignatureSubject::Call(&callee))
            .unwrap();
        assert!(view.equality_results.is_none());
        let before = budget.usage_snapshot().bytes;
        for _ in 0..16 {
            assert!(
                world
                    .derive_equality_results(&view, &facts)
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(budget.usage_snapshot().bytes, before);
    }

    #[test]
    fn parameter_result_storage_charges_capacity_and_arc_relocation() {
        let budget = PlanBudget::default();
        let mut values = Vec::new();
        for value in 0..16_u64 {
            let before = budget.usage_snapshot().bytes;
            let previous_capacity = values.capacity();
            result_push(&budget, &mut values, value).unwrap();
            let charged = budget.usage_snapshot().bytes - before;
            if values.capacity() != previous_capacity {
                assert!(
                    charged
                        >= values.capacity() * std::mem::size_of::<u64>()
                            + 2 * std::mem::size_of::<usize>()
                );
            } else {
                assert_eq!(charged, 0);
            }
        }
        let before = budget.usage_snapshot().bytes;
        let slice = result_slice(&budget, values).unwrap();
        assert_eq!(
            budget.usage_snapshot().bytes - before,
            slice.len() * std::mem::size_of::<u64>() + 2 * std::mem::size_of::<usize>()
        );
        let tiny = PlanBudget::new(ir::PlanLimits {
            bytes: 1,
            ..ir::PlanLimits::default()
        });
        assert_eq!(
            result_push(&tiny, &mut Vec::new(), 0_u64)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        assert!(result_push(&tiny, &mut Vec::new(), ()).is_err());
    }

    #[test]
    fn parameter_result_body_and_direct_reject_topology_and_resealed_topology() {
        let (_, facts) = source_world(false);
        let callee = Callee::Direct(CallTarget {
            declaration: facts.declaration(),
            type_args: facts
                .env()
                .ordered_params()
                .map(Ty::Param)
                .collect::<Vec<_>>()
                .into(),
        });
        for direct in [false, true] {
            for mutation in 0..3 {
                for reseal in [false, true] {
                    let mut view = if direct {
                        Arc::try_unwrap(
                            facts
                                .carrier_contract(SignatureSubject::Call(&callee))
                                .unwrap(),
                        )
                        .unwrap()
                    } else {
                        body_view(&facts)
                    };
                    match mutation {
                        0 => {
                            assert!(!view.origin.result_tails.is_empty());
                            Arc::make_mut(&mut view.origin).result_tails = Arc::from([]);
                        }
                        1 => Arc::make_mut(&mut view.fixed).region_count += 1,
                        _ => {
                            let parameter = facts.env().ordered_params().next().unwrap();
                            view.regions.constraints = Arc::from([ir::CarrierRegionConstraint {
                                longer: ir::CarrierRegionTerm::Family { parameter },
                                shorter: ir::CarrierRegionTerm::Family { parameter },
                            }]);
                        }
                    }
                    if reseal {
                        // Alter the private snapshot as well: only independent
                        // declaration reconstruction can reject this forgery.
                        let authority = Arc::make_mut(view.equality_results.as_mut().unwrap());
                        authority.origin = view.origin.clone();
                        authority.fixed = view.fixed.clone();
                        authority.regions = view.regions.clone();
                    }
                    assert_eq!(
                        view.validate_for(&facts).unwrap_err().kind(),
                        FactsErrorKind::InvalidMetadata
                    );
                }
            }
        }
        reject_mutation(&facts, |authority| {
            Arc::make_mut(&mut authority.origin).result_tails = Arc::from([]);
        });
        reject_mutation(&facts, |authority| {
            Arc::make_mut(&mut authority.fixed).region_count += 1;
        });
        reject_mutation(&facts, |authority| {
            let parameter = facts.env().ordered_params().next().unwrap();
            authority.regions.constraints = Arc::from([ir::CarrierRegionConstraint {
                longer: ir::CarrierRegionTerm::Family { parameter },
                shorter: ir::CarrierRegionTerm::Family { parameter },
            }]);
        });
    }

    #[test]
    fn parameter_result_body_retains_tail_and_requires_private_certificate() {
        let (_, facts) = source_world(false);
        let mut view = body_view(&facts);
        assert_eq!(view.contract.results.len(), 1);
        assert!(view.contract.results[0].sources.is_empty());
        assert!(matches!(
            view.contract.results[0].location.channel,
            ir::CarrierChannel::ParamTail { .. }
        ));
        assert_eq!(
            view.equality_results.as_ref().unwrap().certificates.len(),
            1
        );
        view.validate_result_flow(
            &facts,
            SignatureSubject::Body,
            &view.result_signature,
            &view.contract.results[0].location,
        )
        .unwrap();
        view.equality_results = None;
        assert_eq!(
            view.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn parameter_result_certificate_rejects_missing_foreign_and_stale_authority() {
        let (_, facts) = source_world(false);
        reject_mutation(&facts, |authority| authority.certificates = Arc::from([]));
        reject_mutation(&facts, |authority| authority.environment = usize::MAX);
        reject_mutation(&facts, |authority| {
            authority.world = WorldId(SnapshotId::standalone().unwrap())
        });
        reject_mutation(&facts, |authority| authority.phase = FactsPhase::Closed);
        reject_mutation(&facts, |authority| authority.finalized = false);
        reject_mutation(&facts, |authority| authority.stage = None);
        reject_mutation(&facts, |authority| {
            let certificate = &mut Arc::make_mut(&mut authority.certificates)[0];
            certificate.rhs_parameter.index = 0;
        });
        reject_mutation(&facts, |authority| {
            let certificate = &mut Arc::make_mut(&mut authority.certificates)[0];
            certificate.result.channel =
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 });
        });
        reject_mutation(&facts, |authority| {
            let certificate = &mut Arc::make_mut(&mut authority.certificates)[0];
            certificate.dependencies = Arc::from([0]);
        });
        reject_mutation(&facts, |authority| {
            Arc::make_mut(&mut authority.origins)[0].occurrence = usize::MAX;
        });
        reject_mutation(&facts, |authority| {
            if let Predicate::ProjectionEq { ty, .. } =
                &mut Arc::make_mut(&mut authority.actual_rows)[0]
            {
                *ty = Ty::Bool;
            }
        });
    }

    #[test]
    fn parameter_result_certificate_never_manufactures_copy() {
        let (_, facts) = source_world(false);
        let view = body_view(&facts);
        assert!(!facts.must_copy(&view.result_signature.result).unwrap());
        let (_, copy_facts) = source_world(true);
        let view = body_view(&copy_facts);
        assert!(copy_facts.must_copy(&view.result_signature.result).unwrap());
    }

    #[test]
    fn parameter_result_direct_signature_without_projection_has_mandatory_authority() {
        let (_, facts) = source_world(false);
        let target = CallTarget {
            declaration: facts.declaration(),
            type_args: facts
                .env()
                .ordered_params()
                .map(Ty::Param)
                .collect::<Vec<_>>()
                .into(),
        };
        let callee = Callee::Direct(target);
        let mut view = Arc::try_unwrap(
            facts
                .carrier_contract(SignatureSubject::Call(&callee))
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(view.result_signature.result, Ty::Param(_)));
        assert!(view.projected.is_none());
        assert!(view.equality_required.is_none());
        assert_eq!(
            view.equality_results.as_ref().unwrap().certificates.len(),
            1
        );
        view.validate_result_flow(
            &facts,
            SignatureSubject::Call(&callee),
            &view.result_signature,
            &view.contract.results[0].location,
        )
        .unwrap();
        assert_eq!(
            view.validate_result_flow(
                &facts,
                SignatureSubject::Body,
                &view.result_signature,
                &view.contract.results[0].location
            )
            .unwrap_err()
            .kind(),
            FactsErrorKind::InvalidMetadata
        );
        view.equality_results = None;
        assert_eq!(
            view.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn parameter_equality_actual_domain_walks_zero_arrays_and_nested_reference_children() {
        let (world, facts) = source_world(false);
        let context = world
            .declared_callable_context(facts.declaration())
            .unwrap();
        let Predicate::ProjectionEq { projection, .. } = &context.origins[0].row else {
            panic!("expected equality")
        };
        let reference = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
        };
        for rhs in [
            reference.clone(),
            Ty::Array {
                element: Arc::new(reference.clone()),
                len: 0,
            },
            Ty::Tuple(Arc::from([Ty::Tuple(Arc::from([reference]))])),
        ] {
            // Stock accepts shared-reference RHS types, including nested
            // in zero-length arrays and tuples; equality rows are type data.
            world
                .validate_equality_domain(projection, &rhs, facts.env())
                .unwrap();
        }
    }

    #[test]
    fn closed_lhs_cannot_infer_rigid_rhs_and_closed_instances_replay_unused_rows() {
        let source = frontend::parse(
            "trait Source:Sized{type Out;fn get(self)->Self::Out;}\
             struct Record{value:i32}\
             impl Source for Record{type Out=i32;fn get(self)->i32{self.value}}\
             fn produce<T:Source<Out=U>,U>(x:T)->U{x.get()}\
             fn unused<T:Source<Out=U>,U>(x:T){}\
             fn main(){}",
        )
        .unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let implementation = world
            .headers
            .impls
            .iter()
            .find(|header| !header.associated_types.is_empty())
            .unwrap();
        let receiver = implementation.trait_ref.self_ty.clone();
        for header in world
            .headers
            .callables
            .iter()
            .filter(|header| header.schema.env.total_arity() == 2)
        {
            let good = CallTarget {
                declaration: header.declaration,
                type_args: Arc::from([receiver.clone(), Ty::Int(ir::IntTy::I32)]),
            };
            let facts = world.instance_facts(&good).unwrap();
            let view = facts.carrier_contract(SignatureSubject::Body).unwrap();
            assert!(view.contract.results.is_empty());
            assert!(view.equality_results.is_some());
            let bad = CallTarget {
                declaration: header.declaration,
                type_args: Arc::from([receiver.clone(), Ty::Bool]),
            };
            assert!(world.instance_facts(&bad).is_err());
            let universal = world.callable_facts(header.declaration).unwrap();
            let parameter = universal.env().ordered_params().nth(1).unwrap();
            let projection = ir::AssociatedTypeProjection {
                item: implementation.associated_types[0].associated,
                trait_ref: implementation.trait_ref.clone(),
            };
            assert_eq!(
                world
                    .validate_equality_domain(&projection, &Ty::Param(parameter), universal.env())
                    .unwrap_err()
                    .kind(),
                FactsErrorKind::Unsupported
            );
        }
    }
}
