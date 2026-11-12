//! Required projected methods have two deliberately separate authorities:
//! an exact private owning-impl context, and finalized closed normalization.
//! Neither a raw projection nor an owner context is an executable carrier proof.
use super::*;

pub(super) struct OwnerNormalizationContext<'a> {
    world: WorldId,
    finalized: bool,
    implementation: &'a ImplHeader,
    predicates: Arc<PredicateEnv>,
}

#[derive(Debug)]
pub(super) struct ProjectedMethodEvidence {
    world: WorldId,
    environment: usize,
    phase: FactsPhase,
    item: DeclId,
    goal: TraitRef,
    method_args: Arc<[Ty]>,
    signature: Signature,
    source_regions: Arc<ir::SourceRegionEnv>,
    dependencies: Arc<[NormalizedAssociatedType]>,
    selection: ProofEvidence,
    universal: Option<Arc<super::universal_methods::UniversalMethodEvidence>>,
}

impl ProjectedMethodEvidence {
    pub(super) fn universal(
        world: WorldId,
        facts: &ValidatedFacts,
        authority: Arc<super::universal_methods::UniversalMethodEvidence>,
    ) -> Self {
        Self {
            world,
            environment: facts.predicates.index,
            phase: facts.phase(),
            item: authority.call.item,
            goal: authority.call.trait_ref.clone(),
            method_args: authority.call.method_args.clone(),
            signature: authority.signature.clone(),
            source_regions: authority.source_regions.clone(),
            dependencies: Arc::from([]),
            selection: authority.selection.clone(),
            universal: Some(authority),
        }
    }
    pub(super) fn goal(&self) -> &TraitRef {
        &self.goal
    }
}

pub(super) struct ProjectedApplication {
    pub(super) instantiated: InstantiatedSchema,
    pub(super) evidence: Arc<ProjectedMethodEvidence>,
}

pub(super) struct ProjectedCarrierBase {
    schema: GenericSchema,
    retained: DeclarationCarriers,
    arguments: Arc<[Ty]>,
    origin: Arc<ir::SignatureOriginSchema>,
}

impl ValidatedTraitWorld {
    pub(super) fn validate_projected_required_schema(&self, item: &TraitItemHeader) -> Result<()> {
        self.validate_declaration_schema(item.declaration, &item.schema)?;
        // Validate original source-region identity before capability rejection.
        self.validate_source_regions(item.declaration, &item.schema.signature)?;
        self.validate_predicates(&item.schema.predicates, &item.schema.env, None)?;
        if !self.signature_contains_projection(&item.schema.signature)? {
            return self.validate_schema_signature(
                item.declaration,
                &item.schema.signature,
                &item.schema.env,
                None,
            );
        }
        if item.default.is_some() {
            for ty in item
                .schema
                .signature
                .params
                .iter()
                .chain(std::iter::once(&item.schema.signature.result))
            {
                self.validate_universal_projection_type(ty, &item.schema.env, 0)?;
            }
            return self.validate_schema_signature(
                item.declaration,
                &item.schema.signature,
                &item.schema.env,
                None,
            );
        }
        self.validate_projected_value_schema(item.declaration, &item.schema, None)
    }

    fn validate_projected_value_schema(
        &self,
        declaration: DeclId,
        schema: &GenericSchema,
        owner: Option<&OwnerNormalizationContext<'_>>,
    ) -> Result<()> {
        self.projected_marker_predicates(schema)?;
        // Required declarations can still contain a deferred NoReference row
        // for `&Self::Target`. Authenticate the exact raw schema here, not an
        // executable stored contract before its projection is normalized.
        self.validate_declaration_schema(declaration, schema)?;
        let source = self.validate_source_regions(declaration, &schema.signature)?;
        // Explicit reference layers, including layers inside an enum or tuple,
        // need a declaration-owned lifetime row. Projection tails retain their
        // separate origin authority and are never guessed from a normalized type.
        for (parameter, ty) in schema
            .signature
            .params
            .iter()
            .enumerate()
            .map(|(index, ty)| (Some(index), ty))
            .chain(std::iter::once((None, &schema.signature.result)))
        {
            // The exact implementation owns closed `Self::Item` assignments.
            // They must reduce before rigid-family admission, never bypass its
            // closed-projection guard. Source lifetime/carrier rows below still
            // authenticate the original unnormalized declaration.
            if let Some(owner) = owner {
                let normal =
                    self.normalize_owner_type_for_use(ty, owner, &mut Vec::new(), 0, false)?;
                self.validate_universal_projection_type(&normal, &schema.env, 0)?;
            } else {
                self.validate_universal_projection_type(ty, &schema.env, 0)?;
            }
            for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
                if !matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                    continue;
                }
                let mut retained = false;
                for row in source.carriers.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.parameter == parameter && self.same_location(&row.location, location)? {
                        retained = true;
                        break;
                    }
                }
                if !retained {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: projected reference carrier has no declaration-owned lifetime",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Admit only source-authenticated marker obligations on one exact projection.
    /// This is a capability check, not evidence that its RHS is Copy or Sized.
    /// Substituted applications must normalize and prove these predicates below.
    fn projected_marker_predicates(&self, schema: &GenericSchema) -> Result<bool> {
        // Inspect all canonical metadata before any capability rejection.
        // Callers authenticate this schema against the original declaration.
        for predicate in schema.predicates.iter() {
            self.validate_predicate_metadata(predicate, &schema.env)?;
        }
        let mut projected = false;
        for predicate in schema.predicates.iter() {
            let Predicate::Implements(goal) = predicate else {
                continue;
            };
            for argument in goal.args.iter() {
                if self.type_contains_projection(argument, 0)? {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: projected method associated bounds",
                    ));
                }
            }
            if !self.type_contains_projection(&goal.self_ty, 0)? {
                continue;
            }
            if !goal.args.is_empty()
                || !matches!(&goal.self_ty, Ty::AssociatedType(_))
                || (goal.declaration != self.catalog.id(CatalogItem::Copy)
                    && goal.declaration != self.catalog.id(CatalogItem::Sized))
            {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: projected method associated bounds",
                ));
            }
            projected = true;
        }
        Ok(projected)
    }

    #[cfg(test)]
    fn validate_projected_parameter_references(
        &self,
        ty: &Ty,
        index: usize,
        receiver: ReceiverMode,
    ) -> Result<()> {
        // A fixed shared str input has no projected referent or hidden type
        // arguments. Its loan is represented by the original source contract,
        // independently of an associated result (for example FromStr::Err).
        // This admits no new result origins or projection-bearing inputs.
        if matches!(ty, Ty::Ref { kind: ir::BorrowKind::Shared, referent }
            if matches!(referent.as_ref(), Ty::Str))
        {
            return Ok(());
        }
        // Source-region and declaration-schema validation authenticate receiver
        // identity before this capability check. Only an ordinary projection-
        // free borrowed receiver uses the existing reference-input machinery.
        // Projected references and all reference-bearing results remain outside
        // this value-result stage.
        if index == 0
            && let ReceiverMode::Borrowed { kind } = receiver
            && let Ty::Ref {
                kind: actual,
                referent,
            } = ty
            && *actual == kind
            && !self.type_contains_projection(referent, 1)?
        {
            return Ok(());
        }
        self.reject_projected_references(ty, 0)
    }

    // This is a value-oriented stage, including references hidden in nominals.
    // Do not ask the IR carrier walker to interpret an unreduced projection.
    /// A universal caller's projection (`&C::Item` with `C: Container`) is
    /// rigid: it never normalizes in that body. Its deferred `NoReference`
    /// rows take the opaque input envelopes the canonical derivation assigns
    /// to an uninterpreted referent (every eligible input loan, the most
    /// conservative origin set). Any other derived shape fails closed.
    pub(super) fn complete_rigid_projected_contract(
        &self,
        signature: &mut Signature,
    ) -> Result<()> {
        if !signature
            .ref_contract
            .results
            .iter()
            .any(|r| matches!(r.returns, ir::ReturnContract::NoReference))
            || !charged(
                &self.budget,
                ir::contains_associated_types(&signature.result, &self.budget),
            )?
        {
            return Ok(());
        }
        let derived = charged(
            &self.budget,
            ir::derive_return_contracts(
                &signature.params,
                &signature.result,
                &signature.ref_contract,
                &self.budget,
            ),
        )?;
        if derived.len() != signature.ref_contract.results.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "rigid projected result changed its reference positions",
            ));
        }
        retain::<ir::ResultReference>(&self.budget, derived.len())?;
        let mut results = Vec::new();
        results
            .try_reserve_exact(derived.len())
            .map_err(|_| FactsError::resource(&self.budget, "rigid projected result rows"))?;
        for (raw, derived) in signature.ref_contract.results.iter().zip(derived.iter()) {
            charged(&self.budget, self.budget.charge_work(1))?;
            if raw.position != derived.position || raw.region != derived.region {
                return Err(FactsError::invalid(
                    &self.budget,
                    "rigid projected result changed its reference positions",
                ));
            }
            match (&raw.returns, &derived.returns) {
                (
                    ir::ReturnContract::NoReference,
                    ir::ReturnContract::SharedInputEnvelopes(_)
                    | ir::ReturnContract::MutInputEnvelopes(_),
                ) => results.push(derived.clone()),
                (ir::ReturnContract::NoReference, _) => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: rigid projected reference result without opaque input envelopes",
                    ));
                }
                _ => results.push(raw.clone()),
            }
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        signature.ref_contract = Arc::new(ir::RefContract {
            results: results.into(),
            ..(*signature.ref_contract).clone()
        });
        Ok(())
    }

    /// After normalizing a projected declaration (`&Self::Target`), derive the
    /// return origins the raw schema deliberately left unresolved.
    pub(super) fn complete_projected_contract(&self, signature: &mut Signature) -> Result<()> {
        let layers = charged(&self.budget, ir::root_reference_layers(&signature.result))?;
        if !charged(
            &self.budget,
            ir::contains_associated_types(&signature.result, &self.budget),
        )? && layers > signature.ref_contract.results.len()
        {
            // A normalized projection substituted reference layers below the
            // declared outer reference (`&Self::Output` with `Output = &T`).
            // Each substituted layer is a value stored inside the borrowed
            // input: give it a fresh region that outlives the layer above,
            // exactly the well-formedness bound (`&'a &'b T` implies 'b: 'a)
            // that the implementation's own signature derives.
            let Some(outer) = signature
                .ref_contract
                .results
                .iter()
                .find(|row| row.position.depth == 0)
                .cloned()
            else {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: projected result introduces reference layers without source lifetime authority",
                ));
            };
            let base = usize::from(signature.ref_contract.region_count);
            if signature.ref_contract.results.len() != 1 || base + layers > 64 {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: projected result introduces reference layers without source lifetime authority",
                ));
            }
            retain::<ir::ResultReference>(&self.budget, layers)?;
            retain::<(ir::RegionParamId, ir::RegionParamId)>(&self.budget, layers)?;
            let mut results = Vec::new();
            let mut outlives = signature.ref_contract.outlives.to_vec();
            results.try_reserve_exact(layers).map_err(|_| {
                FactsError::resource(&self.budget, "projected result layer allocation")
            })?;
            results.push(outer.clone());
            let mut above = outer.region;
            for depth in 1..layers {
                let region = ir::RegionParamId((base + depth - 1) as u16);
                outlives.push((region, above));
                results.push(ir::ResultReference {
                    position: ir::ReferencePosition { depth: depth as u8 },
                    region,
                    returns: ir::ReturnContract::NoReference,
                });
                above = region;
            }
            let raw = ir::RefContract {
                region_count: (base + layers - 1) as u16,
                input_regions: signature.ref_contract.input_regions.clone(),
                results: results.into(),
                outlives: outlives.into(),
                static_regions: signature.ref_contract.static_regions,
            };
            signature.ref_contract =
                charged(&self.budget, ir::normalize_ref_contract(&raw, &self.budget))?;
            let results = charged(
                &self.budget,
                ir::derive_return_contracts(
                    &signature.params,
                    &signature.result,
                    &signature.ref_contract,
                    &self.budget,
                ),
            )?;
            retain::<ir::RefContract>(&self.budget, 1)?;
            signature.ref_contract = Arc::new(ir::RefContract {
                results,
                ..(*signature.ref_contract).clone()
            });
        }
        if !signature
            .ref_contract
            .results
            .iter()
            .any(|r| matches!(r.returns, ir::ReturnContract::NoReference))
        {
            return Ok(());
        }
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            if charged(
                &self.budget,
                ir::contains_associated_types(ty, &self.budget),
            )? {
                return Ok(());
            }
        }
        let results = charged(
            &self.budget,
            ir::derive_return_contracts(
                &signature.params,
                &signature.result,
                &signature.ref_contract,
                &self.budget,
            ),
        )?;
        retain::<ir::RefContract>(&self.budget, 1)?;
        signature.ref_contract = Arc::new(ir::RefContract {
            results,
            ..(*signature.ref_contract).clone()
        });
        Ok(())
    }

    pub(super) fn reject_projected_references(&self, ty: &Ty, depth: usize) -> Result<()> {
        self.projected_reference_walk(ty, depth, &mut TypeWalk::default())
            .map(|_| ())
    }

    fn projected_reference_walk(
        &self,
        ty: &Ty,
        depth: usize,
        walk: &mut TypeWalk,
    ) -> Result<usize> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let key = TypeWalk::key(ty).map(|key| (key, TypeUse::Root));
        if let Some(key) = key
            && let Some(height) = walk.lookup(key, ty, &self.budget)?
        {
            charged(
                &self.budget,
                self.budget.check_depth(depth.saturating_add(height)),
            )?;
            return Ok(height);
        }
        let mut height = 0usize;
        let mut child = |ty: &Ty| -> Result<()> {
            height = height.max(
                self.projected_reference_walk(ty, depth + 1, walk)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Ref { .. } | Ty::Slice { .. } => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: reference-bearing projected method signature",
                ));
            }
            Ty::AssociatedType(_) => return Ok(0),
            Ty::Array { element, .. } => child(element)?,
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, &self.budget)?.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            _ => {}
        }
        for argument in type_arguments(ty) {
            child(argument)?;
        }
        if let Some(key) = key {
            walk.insert(key, height, ty, &self.budget)?;
        }
        Ok(height)
    }

    /// Complete mapping validation precedes issuance of any private owner context.
    pub(super) fn validate_projected_owner_metadata(&self) -> Result<()> {
        for implementation in self.headers.impls.iter() {
            let owner_info = self.info(implementation.declaration)?;
            let owner = self.trait_header(implementation.trait_ref.declaration)?;
            let mut items = Vec::new();
            let mut targets = Vec::new();
            for (item, target) in implementation.items.iter() {
                charged(
                    &self.budget,
                    self.budget.charge_work(items.len() + targets.len() + 1),
                )?;
                let required = self.item_header(*item)?;
                let provided = self.callable_header(*target)?;
                let info = self.info(*target)?;
                if items.contains(item)
                    || targets.contains(target)
                    || required.trait_decl != owner.declaration
                    || info.owner != Some(implementation.declaration)
                    || info.receiver != required.receiver
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected owner has duplicate, foreign or rebound method mapping",
                    ));
                }
                if self.info(required.declaration)?.own_env.arity() != info.own_env.arity()
                    || !self.method_environment_matches(
                        &provided.schema.env,
                        &implementation.schema_env,
                        &info.own_env,
                    )?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "trait implementation method binder mismatch",
                    ));
                }
                let private = match info.visibility {
                    ir::Visibility::Public => false,
                    ir::Visibility::Restricted(scope) => {
                        checked_module(&self.budget, self.catalog.modules().check(scope))?;
                        scope == owner_info.module
                    }
                };
                if !private {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected owner method visibility is not its implementation scope",
                    ));
                }
                self.validate_declaration_schema(*target, &provided.schema)?;
                push(&self.budget, &mut items, *item)?;
                push(&self.budget, &mut targets, *target)?;
            }
            for item in owner.items.iter() {
                charged(&self.budget, self.budget.charge_work(items.len() + 1))?;
                if self.item_header(*item)?.default.is_none() && !items.contains(item) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected owner is missing a required method",
                    ));
                }
            }
            for owned in self.owned_callables(implementation.declaration) {
                charged(&self.budget, self.budget.charge_work(targets.len() + 1))?;
                if !targets.contains(owned) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected owner has an unmapped implementation method",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn owner_context<'a>(
        &self,
        implementation: &'a ImplHeader,
    ) -> Result<OwnerNormalizationContext<'a>> {
        self.check_normalization_budget()?;
        charged(
            &self.budget,
            self.budget.charge_work(self.headers.impls.len()),
        )?;
        if !self.owner_normalization_ready
            || !self
                .headers
                .impls
                .iter()
                .any(|header| std::ptr::eq(header, implementation))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "owning-impl normalization was requested before validated staging",
            ));
        }
        let predicates = self.predicate_env(implementation.declaration)?;
        if !self.same_env(&predicates.env, &implementation.schema_env)? {
            return Err(FactsError::invalid(
                &self.budget,
                "owning-impl binders changed",
            ));
        }
        Ok(OwnerNormalizationContext {
            world: self.id,
            finalized: self.owner_normalization_finalized,
            implementation,
            predicates,
        })
    }

    fn check_owner_context(&self, owner: &OwnerNormalizationContext<'_>) -> Result<()> {
        self.check_normalization_budget()?;
        if owner.world != self.id
            || !self.owner_normalization_ready
            || owner.finalized != self.owner_normalization_finalized
        {
            return Err(FactsError::invalid(
                &self.budget,
                "foreign owning-impl normalization context",
            ));
        }
        self.check_context(ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &owner.predicates,
            executable_storage: true,
            phase: FactsPhase::Universal,
        })
    }

    /// Structural substitution must preserve every original occurrence and binder.
    pub(super) fn substitute_projected_schema(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        caller: &GenericEnv,
    ) -> Result<InstantiatedSchema> {
        self.substitute_projected_schema_inner(
            schema,
            arguments,
            caller,
            None,
            &mut ProofScratch::default(),
        )
    }

    pub(super) fn substitute_projected_schema_raw_with(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        caller: &GenericEnv,
        active: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        self.substitute_projected_schema_inner(schema, arguments, caller, None, active)
    }

    pub(super) fn substitute_projected_schema_with(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        self.check_context(context)?;
        self.substitute_projected_schema_inner(
            schema,
            arguments,
            context.env(),
            Some(context),
            active,
        )
    }

    fn substitute_projected_schema_inner(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        caller: &GenericEnv,
        context: Option<ProofContext<'_>>,
        active: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        let original = context.and_then(ProofContext::metadata_facts);
        for argument in arguments {
            self.validate_ty_metadata_with_facts(
                argument,
                caller,
                0,
                &mut active.replays,
                original,
            )?;
        }
        if arguments.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "projected schema argument arity changed",
            ));
        }
        let value = match context {
            Some(context) => self.instantiate_raw_cached(
                schema,
                arguments,
                context,
                None,
                &mut active.instantiations,
            )?,
            None => self
                .schema
                .instantiate_schema(schema, arguments, caller, &self.budget)?,
        };
        if value.signature.params.len() != schema.signature.params.len()
            || value.predicates.len() != schema.predicates.len()
            || !self.eq_contract(
                &value.signature.ref_contract,
                &schema.signature.ref_contract,
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected substitution changed original schema shape",
            ));
        }
        for (raw, actual) in schema
            .signature
            .params
            .iter()
            .zip(value.signature.params.iter())
            .chain(std::iter::once((
                &schema.signature.result,
                &value.signature.result,
            )))
        {
            self.validate_rhs_substitution(raw, actual, &schema.env, arguments, 0)?;
            self.validate_ty_metadata_with_facts(actual, caller, 0, &mut active.replays, original)?;
        }
        for (raw, actual) in schema.predicates.iter().zip(value.predicates.iter()) {
            self.validate_predicate_substitution(raw, actual, &schema.env, arguments)?;
        }
        Ok(value)
    }

    pub(super) fn normalize_owner_type(
        &self,
        ty: &Ty,
        owner: &OwnerNormalizationContext<'_>,
        active: &mut Vec<DeclId>,
        depth: usize,
    ) -> Result<Ty> {
        self.normalize_owner_type_for_use(ty, owner, active, depth, true)
    }

    /// Declaration schemas retain canonical source applications. Only the
    /// executable path may demand storage, which graph realization remints.
    pub(super) fn normalize_owner_type_for_use(
        &self,
        ty: &Ty,
        owner: &OwnerNormalizationContext<'_>,
        active: &mut Vec<DeclId>,
        depth: usize,
        executable_storage: bool,
    ) -> Result<Ty> {
        self.check_owner_context(owner)?;
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::AssociatedType(projection) => {
                self.validate_ty_metadata_in(ty, &owner.implementation.schema_env, depth)?;
                if self.associated_header(projection.item)?.trait_decl
                    == owner.implementation.trait_ref.declaration
                    && self.eq_predicate(
                        &Predicate::Implements(projection.trait_ref.clone()),
                        &Predicate::Implements(owner.implementation.trait_ref.clone()),
                    )?
                {
                    charged(&self.budget, self.budget.charge_work(active.len()))?;
                    if active.contains(&projection.item) {
                        return Err(FactsError::cycle(
                            &self.budget,
                            "owning-impl associated assignment cycle",
                        ));
                    }
                    let value =
                        self.associated_value(owner.implementation.declaration, projection.item)?;
                    let rhs = self
                        .schema
                        .associated_type_rhs(owner.implementation.declaration, projection.item)?;
                    if !self.eq_ty(&rhs, &value.rhs, 0)? {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "owning-impl RHS callback changed",
                        ));
                    }
                    let mut arguments = Vec::new();
                    for parameter in owner.implementation.schema_env.ordered_params() {
                        push(&self.budget, &mut arguments, Ty::Param(parameter))?;
                    }
                    retain::<ir::RefContract>(&self.budget, 1)?;
                    let schema = GenericSchema {
                        env: owner.implementation.schema_env.clone(),
                        signature: Signature {
                            params: Vec::new(),
                            result: rhs,
                            ref_contract: Arc::new(ir::RefContract::default()),
                        },
                        predicates: Arc::from([]),
                    };
                    let substituted = self.substitute_projected_schema(
                        &schema,
                        &arguments,
                        &owner.implementation.schema_env,
                    )?;
                    push(&self.budget, active, projection.item)?;
                    let result = self.normalize_owner_type_for_use(
                        &substituted.signature.result,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    );
                    active.pop();
                    let normal = result?;
                    self.validate_associated_normal_form_for_use(
                        Some(projection.item),
                        &normal,
                        &owner.implementation.schema_env,
                        ProofContext {
                            transport: ParentLocalTransport::default(),
                            original: None,
                            source_metadata: None,
                            predicates: &owner.predicates,
                            executable_storage,
                            phase: FactsPhase::Universal,
                        },
                        &mut ProofScratch::default(),
                        executable_storage,
                    )?;
                    Ok(normal)
                } else if self.eq_ty(
                    &projection.trait_ref.self_ty,
                    &owner.implementation.trait_ref.self_ty,
                    0,
                )? && self.associated_header(projection.item)?.trait_decl
                    != owner.implementation.trait_ref.declaration
                {
                    let context = ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &owner.predicates,
                        executable_storage,
                        phase: FactsPhase::Universal,
                    };
                    // A supertrait's assignment is not owned by this impl.
                    // Demand an independent concrete implementation witness,
                    // even when its arguments retain this impl's rigid binders.
                    let selection = self.prove_implementation_in(
                        &Predicate::Implements(projection.trait_ref.clone()),
                        context,
                        &mut ProofScratch::default(),
                    )?;
                    let ProofOutcome::Proved(selection) = selection else {
                        // Prefer a proven generic implementation's assignment
                        // (for example Index<usize>::Output = T on Vec<T>).
                        // Only an ordinary unproved implementation lookup may
                        // retain the exact rigid projection from a caller bound.
                        if let Some(symbolic) =
                            self.retain_universal_owner_projection(projection, context)?
                        {
                            return Ok(symbolic);
                        }
                        return Err(FactsError::unproved(
                            &self.budget,
                            "inherited associated type has no implementation witness",
                        ));
                    };
                    let ProofOrigin::Implementation {
                        declaration,
                        arguments,
                    } = &selection.0.origin
                    else {
                        return Err(FactsError::unproved(
                            &self.budget,
                            "inherited associated type needs an assignment",
                        ));
                    };
                    let header = self
                        .headers
                        .impls
                        .iter()
                        .find(|h| h.declaration == *declaration)
                        .ok_or_else(|| {
                            FactsError::invalid(&self.budget, "missing inherited implementation")
                        })?;
                    let rhs = self
                        .schema
                        .associated_type_rhs(*declaration, projection.item)?;
                    let retained = self.associated_value(*declaration, projection.item)?;
                    if !self.eq_ty(&rhs, &retained.rhs, 0)? {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "inherited associated RHS changed",
                        ));
                    }
                    if active.contains(&projection.item) {
                        return Err(FactsError::cycle(
                            &self.budget,
                            "inherited associated assignment cycle",
                        ));
                    }
                    let schema = GenericSchema {
                        env: header.schema_env.clone(),
                        signature: Signature {
                            params: Vec::new(),
                            result: rhs,
                            ref_contract: Arc::new(ir::RefContract::default()),
                        },
                        predicates: Arc::from([]),
                    };
                    let substituted = self.substitute_projected_schema(
                        &schema,
                        arguments,
                        &owner.implementation.schema_env,
                    )?;
                    push(&self.budget, active, projection.item)?;
                    let result = self.normalize_owner_type_for_use(
                        &substituted.signature.result,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    );
                    active.pop();
                    result
                } else {
                    // No assignment authority for a different application. A rigid
                    // application retains its identity only when the owner's own
                    // predicates prove membership (e.g. `I::Item` under `I: Iterator`).
                    // Otherwise only the ordinary closed selector may normalize it.
                    let context = ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &owner.predicates,
                        executable_storage,
                        phase: FactsPhase::Universal,
                    };
                    if matches!(projection.trait_ref.self_ty, Ty::Param(_))
                        && let Some(symbolic) =
                            self.retain_universal_owner_projection(projection, context)?
                    {
                        return Ok(symbolic);
                    }
                    self.normalize_closed_projected_type_for_use(
                        ty,
                        ProofContext {
                            transport: ParentLocalTransport::default(),
                            original: None,
                            source_metadata: None,
                            predicates: &owner.predicates,
                            executable_storage,
                            phase: FactsPhase::Universal,
                        },
                        !owner.finalized,
                        &mut ProofScratch::default(),
                        executable_storage,
                    )
                    .map(|(normal, _)| normal)
                }
            }
            Ty::Tuple(parts) => {
                let mut normalized = Vec::new();
                for part in parts.iter() {
                    push(
                        &self.budget,
                        &mut normalized,
                        self.normalize_owner_type_for_use(
                            part,
                            owner,
                            active,
                            depth + 1,
                            executable_storage,
                        )?,
                    )?;
                }
                Ok(Ty::Tuple(normalized.into()))
            }
            Ty::Array { element, len } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Array {
                    element: Arc::new(self.normalize_owner_type_for_use(
                        element,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    )?),
                    len: *len,
                })
            }
            Ty::RawPtr { kind, pointee } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(self.normalize_owner_type_for_use(
                        pointee,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    )?),
                })
            }
            Ty::Ref { kind, referent } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(self.normalize_owner_type_for_use(
                        referent,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    )?),
                })
            }
            Ty::Slice { element } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Slice {
                    element: Arc::new(self.normalize_owner_type_for_use(
                        element,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    )?),
                })
            }
            Ty::Struct(_) | Ty::Enum(_) => self.normalize_nominal_actuals_for_use(
                ty,
                &owner.implementation.schema_env,
                depth,
                None,
                executable_storage,
                |argument| {
                    self.normalize_owner_type_for_use(
                        argument,
                        owner,
                        active,
                        depth + 1,
                        executable_storage,
                    )
                },
            ),
            _ if self.type_contains_projection(ty, depth)? => Err(FactsError::unsupported(
                &self.budget,
                "unsupported: projected nominal metadata in owning implementation",
            )),
            _ => Ok(ty.clone()),
        }
    }

    pub(super) fn stage_owner_assignments(&self) -> Result<()> {
        for implementation in self.headers.impls.iter() {
            let owner = self.owner_context(implementation)?;
            for value in implementation.associated_types.iter() {
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                let projection = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: implementation.trait_ref.clone(),
                    item: value.associated,
                }));
                let normal = self.normalize_owner_type_for_use(
                    &projection,
                    &owner,
                    &mut Vec::new(),
                    0,
                    false,
                )?;
                // The owner context contains only actual impl constraints, never
                // its own head or an equality manufactured from this assignment.
                self.discharge_associated_bound_predicates(
                    value.associated,
                    &implementation.trait_ref,
                    &normal,
                    ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &owner.predicates,
                        executable_storage: true,
                        phase: FactsPhase::Universal,
                    },
                    &mut ProofScratch::default(),
                    Some(&owner),
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn normalize_owner_callable_schema(
        &self,
        declaration: DeclId,
        schema: &GenericSchema,
    ) -> Result<Option<GenericSchema>> {
        let Some(owner_id) = self.info(declaration)?.owner else {
            return Ok(None);
        };
        for implementation in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if implementation.declaration != owner_id {
                continue;
            }
            let owner = self.owner_context(implementation)?;
            self.validate_declaration_schema(declaration, schema)?;
            if !self.same_parameters(&schema.env, &implementation.schema_env)? {
                // With no owned assignments there is nothing for the impl-only
                // reducer to substitute. Method-local projections retain the
                // full callable environment and its exact declared bounds.
                if implementation.associated_types.is_empty()
                    && let Some(normalized) = self.normalize_universal_callable_schema(schema)?
                {
                    self.validate_schema_signature(
                        declaration,
                        &normalized.signature,
                        &normalized.env,
                        None,
                    )?;
                    self.validate_source_regions(declaration, &normalized.signature)?;
                    return Ok(Some(normalized));
                }
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: generic projected implementation method",
                ));
            }
            self.validate_projected_value_schema(declaration, schema, Some(&owner))?;
            let mut normalized = clone_schema(schema, &self.budget)?;
            for ty in &mut normalized.signature.params {
                *ty = self.normalize_owner_type_for_use(ty, &owner, &mut Vec::new(), 0, false)?;
            }
            normalized.signature.result = self.normalize_owner_type_for_use(
                &schema.signature.result,
                &owner,
                &mut Vec::new(),
                0,
                false,
            )?;
            self.complete_projected_contract(&mut normalized.signature)?;
            self.validate_schema_signature(
                declaration,
                &normalized.signature,
                &normalized.env,
                None,
            )?;
            self.validate_source_regions(declaration, &normalized.signature)?;
            return Ok(Some(normalized));
        }
        Ok(None)
    }

    /// Method alpha-renaming preserves raw projections for substitution
    /// validation. Only after that check may the exact implementation owner
    /// supply its associated assignments for signature conformance.
    pub(super) fn normalize_required_owner_method_signature(
        &self,
        declaration: DeclId,
        signature: &mut Signature,
    ) -> Result<()> {
        let env = &self.info(declaration)?.env;
        let mut reduce = |projection: &ir::AssociatedTypeProjection| match self
            .normalize_exact_owner_projection(declaration, projection)?
        {
            Some(normal) => Ok(normal),
            None => {
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                Ok(Ty::AssociatedType(Arc::new(projection.clone())))
            }
        };
        for parameter in &mut signature.params {
            *parameter = self.map_equality_type(parameter, env, 0, &mut reduce)?;
        }
        signature.result = self.map_equality_type(&signature.result, env, 0, &mut reduce)?;
        self.complete_projected_contract(signature)
    }

    pub(super) fn normalize_exact_owner_projection(
        &self,
        declaration: DeclId,
        projection: &ir::AssociatedTypeProjection,
    ) -> Result<Option<Ty>> {
        let Some(owner_id) = self.info(declaration)?.owner else {
            return Ok(None);
        };
        for implementation in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if implementation.declaration != owner_id
                || !self.eq_predicate(
                    &Predicate::Implements(projection.trait_ref.clone()),
                    &Predicate::Implements(implementation.trait_ref.clone()),
                )?
            {
                continue;
            }
            let owner = self.owner_context(implementation)?;
            retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
            let ty = Ty::AssociatedType(Arc::new(projection.clone()));
            return self
                .normalize_owner_type_for_use(&ty, &owner, &mut Vec::new(), 0, false)
                .map(Some);
        }
        Ok(None)
    }

    pub(super) fn normalize_owner_annotation(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        let FactsSubject::UniversalCallable(declaration) = &facts.subject else {
            return Ok(None);
        };
        let Some(owner_id) = self.info(*declaration)?.owner else {
            return Ok(None);
        };
        for implementation in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if implementation.declaration != owner_id {
                continue;
            }
            let owner = self.owner_context(implementation)?;
            self.validate_ty_metadata_in(ty, facts.env(), 0)?;
            if !self.same_parameters(facts.env(), &implementation.schema_env)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "owner annotation escaped actual implementation binders",
                ));
            }
            let normal = self.normalize_owner_type(ty, &owner, &mut Vec::new(), 0)?;
            self.validate_ty(&normal, facts.env(), None, true, 0)?;
            return Ok(Some(normal));
        }
        Ok(None)
    }

    /// Owner normalization is universal, not permission to instantiate an
    /// associated output with a reference later. Revisit the original schemas
    /// even when only a body annotation used the owner's assignments.
    pub(super) fn validate_owner_callable_instantiation(
        &self,
        target: &CallTarget,
        signature: &Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        let Some(owner_id) = self.info(target.declaration)?.owner else {
            return Ok(());
        };
        for implementation in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if implementation.declaration != owner_id {
                continue;
            }
            if implementation.associated_types.is_empty() {
                return Ok(());
            }
            self.check_context(context)?;
            let owner = self.owner_context(implementation)?;
            let mut raw = None;
            for callable in self.headers.callables.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if callable.declaration == target.declaration {
                    raw = Some(&callable.schema);
                    break;
                }
            }
            let raw = raw.ok_or_else(|| {
                FactsError::invalid(&self.budget, "owning callable lost its original schema")
            })?;
            self.validate_declaration_schema(target.declaration, raw)?;
            let callable_info = self.info(target.declaration)?;
            if target.type_args.len() != raw.env.total_arity()
                || (!self.same_parameters(&raw.env, &implementation.schema_env)?
                    && !self.method_environment_matches(
                        &raw.env,
                        &implementation.schema_env,
                        &callable_info.own_env,
                    )?)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "owning callable instantiation changed implementation binders",
                ));
            }
            // The source-authorized environment above establishes the owner
            // prefix. Replay each ParamId, not merely its width: a method's
            // own actuals are not implementation selection/RHS arguments.
            let mut ordered = raw.env.ordered_params().zip(target.type_args.iter());
            let mut owner_arguments = Vec::new();
            for parameter in implementation.schema_env.ordered_params() {
                charged(&self.budget, self.budget.charge_work(1))?;
                let Some((actual_parameter, argument)) = ordered.next() else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owning callable omitted an implementation parameter",
                    ));
                };
                if actual_parameter != parameter {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owning callable reordered implementation parameters",
                    ));
                }
                push(&self.budget, &mut owner_arguments, argument.clone())?;
            }
            let mut arguments = Vec::new();
            for argument in implementation.trait_ref.args.iter() {
                push(&self.budget, &mut arguments, argument.clone())?;
            }
            retain::<ir::RefContract>(&self.budget, 1)?;
            let head = GenericSchema {
                env: implementation.schema_env.clone(),
                signature: Signature {
                    params: arguments,
                    result: implementation.trait_ref.self_ty.clone(),
                    ref_contract: Arc::new(ir::RefContract::default()),
                },
                predicates: Arc::from([]),
            };
            let mut head =
                self.substitute_projected_schema_with(&head, &owner_arguments, context, active)?;
            self.normalize_instantiated_projection_signature(&mut head.signature, context, active)?;
            let goal = TraitRef {
                declaration: implementation.trait_ref.declaration,
                self_ty: head.signature.result,
                args: head.signature.params.into(),
            };
            self.validate_ref_with_facts(
                &goal,
                context.env(),
                None,
                &mut active.replays,
                context.metadata_facts(),
            )?;
            let closed = match context.metadata_facts() {
                Some(facts) => self.goal_is_closed_with_facts_storage(
                    &goal,
                    facts,
                    &mut active.closedness,
                    &mut active.closedness_storage,
                )?,
                None => self.goal_is_closed_with(&goal, context.env(), active)?,
            };
            if closed {
                let selection_context = if context.phase == FactsPhase::Closed
                    && std::ptr::eq(context.predicates, self.empty.as_ref())
                {
                    context
                } else {
                    ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &self.empty,
                        executable_storage: true,
                        phase: FactsPhase::Closed,
                    }
                };
                let selection =
                    self.require_in(&Predicate::Implements(goal), selection_context, active)?;
                if !matches!(&selection.0.origin,
                    ProofOrigin::Implementation { declaration, arguments }
                        if *declaration == owner_id
                            && self.eq_types(arguments, &owner_arguments, 0)?)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owning callable changed exact implementation selection or arguments",
                    ));
                }
            }
            for value in implementation.associated_types.iter() {
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                let projection = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: implementation.trait_ref.clone(),
                    item: value.associated,
                }));
                let normal = self.instantiate_owner_projected_type(
                    &projection,
                    &owner,
                    &implementation.schema_env,
                    &owner_arguments,
                    context,
                    closed,
                    active,
                )?;
                self.validate_associated_normal_form(
                    Some(value.associated),
                    &normal,
                    context.env(),
                    context,
                    active,
                )?;
            }
            if raw.signature.params.len() != signature.params.len() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "owning callable instantiation changed signature arity",
                ));
            }
            for (original, actual) in raw
                .signature
                .params
                .iter()
                .zip(signature.params.iter())
                .chain(std::iter::once((&raw.signature.result, &signature.result)))
            {
                let normal = self.instantiate_owner_projected_type(
                    original,
                    &owner,
                    &raw.env,
                    &target.type_args,
                    context,
                    closed,
                    active,
                )?;
                if !self.eq_ty(&normal, actual, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owning callable disagrees with instantiated original projection",
                    ));
                }
            }
            return Ok(());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn instantiate_owner_projected_type(
        &self,
        ty: &Ty,
        owner: &OwnerNormalizationContext<'_>,
        source_env: &Arc<GenericEnv>,
        arguments: &[Ty],
        context: ProofContext<'_>,
        closed: bool,
        active: &mut ProofScratch,
    ) -> Result<Ty> {
        // Rigid callers retain private owning-impl authority. Concrete callers
        // must instead use finalized closed selection and its RHS/domain checks.
        let original = if closed {
            ty.clone()
        } else {
            self.normalize_owner_type(ty, owner, &mut Vec::new(), 0)?
        };
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: source_env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: original,
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let value = self.substitute_projected_schema_with(&schema, arguments, context, active)?;
        if closed {
            self.normalize_closed_projected_type_with(
                &value.signature.result,
                context,
                false,
                active,
            )
            .map(|(normal, _)| normal)
        } else {
            Ok(value.signature.result)
        }
    }

    pub(super) fn instantiate_owner_item(
        &self,
        item: &TraitItemHeader,
        implementation: &ImplHeader,
        context: ProofContext<'_>,
    ) -> Result<InstantiatedSchema> {
        self.validate_projected_required_schema(item)?;
        let owner = self.owner_context(implementation)?;
        self.check_context(context)?;
        if item.trait_decl != implementation.trait_ref.declaration
            || !self.same_env(context.env(), &implementation.schema_env)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "required projected signature changed exact owner",
            ));
        }
        let arguments = self.trait_arguments(&implementation.trait_ref)?;
        let mut value =
            self.substitute_projected_schema(&item.schema, &arguments, &implementation.schema_env)?;
        // A reference Self actual (`impl Tr for &'a Vec<T>`) expands the raw
        // declaration's Self tail into exact input regions, as ordinary
        // instantiation does. Topology derives from the original schema only.
        value.signature.ref_contract = self.realize_ref_contract(
            &item.schema,
            &arguments,
            &value.signature,
            Some(item.declaration),
        )?;
        for ty in &mut value.signature.params {
            *ty = self.normalize_owner_type_for_use(ty, &owner, &mut Vec::new(), 0, false)?;
        }
        value.signature.result = self.normalize_owner_type_for_use(
            &value.signature.result,
            &owner,
            &mut Vec::new(),
            0,
            false,
        )?;
        self.complete_projected_contract(&mut value.signature)?;
        // Independently establish the required declaration's contract in the real
        // implementation environment. Never copy the provided target's topology.
        self.validate_schema_signature(
            item.declaration,
            &value.signature,
            &implementation.schema_env,
            None,
        )?;
        let source = self.owner_applied_source_regions(item, implementation, &value.signature)?;
        charged(
            &self.budget,
            ir::derive_signature_origin_schema(
                &value.signature,
                &implementation.schema_env,
                &self.budget,
            ),
        )?;
        self.derive_carriers(&value.signature, &source, &implementation.schema_env)?;
        Ok(value)
    }

    pub(super) fn validate_projected_carrier_refinement(
        &self,
        item: &TraitItemHeader,
        implementation: &ImplHeader,
        required: &Signature,
        provided: &CallableHeader,
    ) -> Result<()> {
        let source = self.owner_applied_source_regions(item, implementation, required)?;
        let (mut required, _) = self.derive_carriers(required, &source, &provided.schema.env)?;
        // The required declaration is evaluated in the real implementation
        // environment, whose where-clauses (`I: Iterator<Item = T>`) are
        // exactly the provided callable's declared rows. Those rows admit the
        // same producer-family write edges for both sides.
        let mut writes = Vec::new();
        for write in required.writes.iter() {
            let mut sources = write.sources.to_vec();
            for input in required.inputs.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if !sources.contains(input)
                    && self.projection_produced_write(
                        provided.declaration,
                        &write.destination.location.channel,
                        &input.location.channel,
                    )?
                {
                    push(&self.budget, &mut sources, input.clone())?;
                }
            }
            push(
                &self.budget,
                &mut writes,
                ir::CarrierWriteEnvelope {
                    destination: write.destination.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        required.writes = writes.into();
        // Re-run derive_carriers' ENTRY closure over the widened writes.
        let mut results = required.results.to_vec();
        let mut changed = true;
        while changed {
            changed = false;
            for result in &mut results {
                let mut sources = result.sources.to_vec();
                for write in required.writes.iter() {
                    charged(&self.budget, self.budget.charge_work(sources.len() + 1))?;
                    if !sources.contains(&write.destination) {
                        continue;
                    }
                    for input in write.sources.iter() {
                        if !sources.contains(input) {
                            push(&self.budget, &mut sources, input.clone())?;
                            changed = true;
                        }
                    }
                }
                result.sources = sources.into();
            }
        }
        required.results = results.into();
        // The provided declaration was independently normalized and published
        // by build_origin_schemas. It cannot supply the required topology.
        let provided = self.declaration_carriers(provided.declaration)?;
        if !self.carrier_contract_refines(&required, &provided.contract)? {
            return Err(FactsError::invalid(
                &self.budget,
                "trait implementation carrier contract does not match normalized required declaration",
            ));
        }
        Ok(())
    }

    pub(super) fn instantiate_projected_item(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        method_args: &[Ty],
        context: ProofContext<'_>,
    ) -> Result<ProjectedApplication> {
        self.instantiate_projected_item_with_scratch(
            item,
            goal,
            method_args,
            context,
            &mut ProofScratch::default(),
        )
    }
    pub(super) fn instantiate_projected_item_with_scratch(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        method_args: &[Ty],
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<ProjectedApplication> {
        self.check_normalization_budget()?;
        self.check_proof_context_caller(context)?;
        self.validate_projected_required_schema(item)?;
        self.validate_ref_with_facts(
            goal,
            context.env(),
            None,
            &mut active.replays,
            context.metadata_facts(),
        )?;
        if goal.declaration != item.trait_decl {
            return Err(FactsError::invalid(
                &self.budget,
                "projected required application changed item owner",
            ));
        }
        let opaque_reference = matches!(&goal.self_ty, Ty::Ref { referent, .. } if matches!(referent.as_ref(), Ty::Opaque(_)));
        let closed = self.goal_is_closed_in(goal, context)?;
        let builtin_pointer = matches!(goal.self_ty, Ty::FnPointer(_))
            && self.schema.closure_trait_kind(goal.declaration)?.is_some();
        // A rigid nominal reference (`&'a Vec<K>`) selects its implementation by
        // head constructor; its generic actuals are proved in the caller env.
        let nominal_reference = !closed
            && matches!(&goal.self_ty, Ty::Ref { referent, .. } if matches!(referent.as_ref(), Ty::Struct(_) | Ty::Enum(_)));
        if !closed
            && !matches!(&goal.self_ty, Ty::Dyn(_) | Ty::Opaque(_))
            && !opaque_reference
            && !nominal_reference
            && !builtin_pointer
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: rigid caller projected method application",
            ));
        }
        // Preserve a checked caller only when selection has not changed its
        // closed predicate environment. Other callers still enter the raw
        // empty context, without transporting source-owned nominal authority.
        let same_closed_context = context.metadata_facts().is_some()
            && context.phase == FactsPhase::Closed
            && std::ptr::eq(context.predicates, self.empty.as_ref());
        let selection = self.require_in(
            &Predicate::Implements(goal.clone()),
            if matches!(&goal.self_ty, Ty::Dyn(_) | Ty::Opaque(_))
                || opaque_reference
                || nominal_reference
                || builtin_pointer
                || same_closed_context
            {
                context
            } else {
                ProofContext {
                    transport: ParentLocalTransport::default(),
                    original: None,
                    source_metadata: None,
                    predicates: &self.empty,
                    executable_storage: true,
                    phase: FactsPhase::Closed,
                }
            },
            active,
        )?;
        if !matches!(
            &selection.0.origin,
            ProofOrigin::Implementation { .. }
                | ProofOrigin::Closure { .. }
                | ProofOrigin::FnItem { .. }
                | ProofOrigin::TraitObject
                | ProofOrigin::OpaqueDeclared
        ) {
            return Err(FactsError::invalid(
                &self.budget,
                "projected method requires an exact implementation selection",
            ));
        }
        let mut arguments = self.trait_arguments(goal)?;
        for argument in method_args {
            push(&self.budget, &mut arguments, argument.clone())?;
        }
        let mut instantiated =
            self.substitute_projected_schema_with(&item.schema, &arguments, context, active)?;
        // As in `instantiate_owner_item`: a reference Self actual
        // (`IntoIterator for &'a Vec<T>`) expands the raw declaration's Self
        // tail into exact input regions. Topology derives from the original
        // schema only, never from the substituted result.
        instantiated.signature.ref_contract = self.realize_ref_contract(
            &item.schema,
            &arguments,
            &instantiated.signature,
            Some(item.declaration),
        )?;
        let mut dependencies = Vec::new();
        for ty in &mut instantiated.signature.params {
            let (normal, proofs) =
                self.normalize_closed_projected_type_with(ty, context, false, active)?;
            *ty = normal;
            for proof in proofs {
                push(&self.budget, &mut dependencies, proof)?;
            }
        }
        let (normal, proofs) = self.normalize_closed_projected_type_with(
            &instantiated.signature.result,
            context,
            false,
            active,
        )?;
        instantiated.signature.result = normal;
        for proof in proofs {
            push(&self.budget, &mut dependencies, proof)?;
        }
        // A provided method (`Iterator::max where Self::Item: Ord`) keeps its
        // declared where-clauses; the closed application must prove each
        // exactly as substituted, whatever trait bounds the projection.
        let provided_bounds = item.default.is_some() && !item.schema.predicates.is_empty();
        if !method_args.is_empty()
            || provided_bounds
            || self.projected_marker_predicates(&item.schema)?
        {
            // Even a nongeneric required method may carry an original
            // `Self::Out: Copy`/`Sized` obligation. Normalize it only after exact
            // substitution and prove its actual RHS in this caller context.
            // Carrier replay re-instantiates the same original predicates.
            instantiated.predicates = self.normalize_instantiated_projection_predicates(
                &instantiated.predicates,
                context,
                active,
            )?;
            for predicate in instantiated.predicates.iter() {
                self.require_in(predicate, context, active)?;
            }
        }
        if let Some(adapter) = self.closed_closure_argument_result_signature(
            item,
            goal,
            &instantiated.signature,
            &selection,
            context,
            active,
        )? {
            if !method_args.is_empty() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure argument result has method type arguments",
                ));
            }
            instantiated.signature.ref_contract = adapter.ref_contract;
        }
        if closed
            && method_args.is_empty()
            && let Some(adapter) = self.closed_fn_item_result_signature(
                item,
                goal,
                &instantiated.signature,
                &selection,
                context,
                active,
            )?
        {
            instantiated.signature.ref_contract = adapter.ref_contract;
        }
        if closed
            && method_args.is_empty()
            && matches!(&selection.0.origin, ProofOrigin::Implementation { .. })
            && charged(
                &self.budget,
                ir::root_reference_layers(&instantiated.signature.result),
            )? > instantiated.signature.ref_contract.results.len()
            && let Some(contract) = self.projected_implementation_result_contract(
                item,
                goal,
                &selection,
                &instantiated.signature,
                context,
            )?
        {
            instantiated.signature.ref_contract = contract;
        }
        self.complete_projected_contract(&mut instantiated.signature)?;
        self.validate_schema_signature_with_scratch(
            item.declaration,
            &instantiated.signature,
            context.env(),
            None,
            context.metadata_facts(),
            Some(&mut active.signature_formations),
        )?;
        for (index, ty) in instantiated.signature.params.iter().enumerate() {
            if !self.closed_closure_tuple_input(
                item,
                goal,
                &instantiated.signature,
                &selection,
                index,
                context,
            )? && charged(
                &self.budget,
                ir::contains_associated_types(ty, &self.budget),
            )? {
                // A closed, normalized parameter (for example the by-value
                // `Iter<'a, T>` receiver of `into_iter`) carries its loans
                // through the ordinary contract derived from the closed
                // signature above, exactly like a non-projected callable.
                // The original signature's fixed layers were authenticated
                // against source lifetime rows above. Retained projections use
                // their canonical universal declaration, not the old value-only
                // reference capability gate.
                self.validate_universal_projection_type(ty, context.env(), 0)?;
            }
        }
        // A projection normalized to a reference-bearing value
        // (`Option<Self::Item>` with `Item = &'a T`) is governed by the
        // contract derived from the normalized signature above.
        let _ = &item.schema.signature.result;
        let source_regions =
            self.validate_source_regions(item.declaration, &instantiated.signature)?;
        retain::<ProjectedMethodEvidence>(&self.budget, 1)?;
        retain::<Ty>(&self.budget, instantiated.signature.params.len())?;
        let evidence = Arc::new(ProjectedMethodEvidence {
            world: self.id,
            environment: context.predicates.index,
            phase: context.phase,
            item: item.declaration,
            goal: goal.clone(),
            method_args: {
                let mut retained = Vec::new();
                for argument in method_args {
                    push(&self.budget, &mut retained, argument.clone())?;
                }
                retained.into()
            },
            signature: instantiated.signature.clone(),
            source_regions,
            dependencies: dependencies.into(),
            selection,
            universal: None,
        });
        Ok(ProjectedApplication {
            instantiated,
            evidence,
        })
    }

    pub(super) fn projected_carrier_contract(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<Arc<CarrierContractView>> {
        // Selection is normal closed dispatch; signature authority remains required.
        let selected = self.resolve_trait_call(call, facts)?;
        let application = self.instantiate_projected_item(
            self.item_header(call.item)?,
            &call.trait_ref,
            &call.method_args,
            ProofContext::from_facts(facts),
        )?;
        let signature = application.instantiated.signature;
        if !self.eq_types(&signature.params, &selected.signature.params, 0)?
            || !self.eq_ty(&signature.result, &selected.signature.result, 0)?
            || !self.return_refines(&signature.ref_contract, &selected.signature.ref_contract)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier and selected call disagree",
            ));
        }
        let base = self.projected_carrier_base(&application.evidence, facts)?;
        self.transport_carrier_contract(
            call.item,
            base.arguments,
            signature,
            facts,
            base.schema,
            &base.retained,
            base.origin,
            Some(application.evidence),
            super::parameter_equality::EqualityResultSubject::Required(call.clone()),
        )
    }

    // Reconstruct the required declaration in the selected implementation's
    // original binder. The provided method body/signature supplies no authority.
    fn projected_implementation_base(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        selection: &ProofEvidence,
    ) -> Result<ProjectedCarrierBase> {
        self.validate_projected_required_schema(item)?;
        if selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || item.trait_decl != goal.declaration
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected owner changed exact selection context",
            ));
        }
        self.selected_implementation_base(item, goal, selection)
    }

    fn selected_implementation_base(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        selection: &ProofEvidence,
    ) -> Result<ProjectedCarrierBase> {
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &selection.0.origin
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected owner lost implementation selection",
            ));
        };
        let implementation = self
            .impl_header_index
            .get(&self.budget, *declaration)?
            .and_then(|position| self.headers.impls.get(position))
            .filter(|implementation| implementation.declaration == *declaration)
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "projected owner implementation disappeared")
            })?;
        if arguments.len() != implementation.schema_env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "projected owner changed ordered actual count",
            ));
        }
        self.validate_predicate_substitution(
            &Predicate::Implements(implementation.trait_ref.clone()),
            &Predicate::Implements(goal.clone()),
            &implementation.schema_env,
            arguments,
        )?;
        let owner = self.predicate_env(*declaration)?;
        let required = self.instantiate_owner_item(
            item,
            implementation,
            ProofContext {
                transport: ParentLocalTransport::default(),
                original: None,
                source_metadata: None,
                predicates: &owner,
                executable_storage: true,
                phase: FactsPhase::Universal,
            },
        )?;
        let schema = GenericSchema {
            env: implementation.schema_env.clone(),
            signature: required.signature,
            predicates: required.predicates,
        };
        let source_regions = self.validate_source_regions(item.declaration, &schema.signature)?;
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
        )?;
        let (contract, regions) =
            self.derive_carriers(&schema.signature, &source_regions, &schema.env)?;
        Ok(ProjectedCarrierBase {
            schema,
            arguments: arguments.clone(),
            origin,
            retained: DeclarationCarriers {
                direct_return: None,
                declaration: item.declaration,
                contract,
                regions,
                source_regions,
            },
        })
    }

    pub(super) fn specialize_owner_callable_result(
        &self,
        target: &CallTarget,
        schema: &GenericSchema,
        signature: &Signature,
        context: ProofContext<'_>,
    ) -> Result<Option<Signature>> {
        let info = self.info(target.declaration)?;
        if info.own_env.arity() != 0
            || !matches!(schema.signature.result, Ty::Param(_))
            || charged(&self.budget, ir::root_reference_layers(&signature.result))? == 0
            || self.signature_contains_projection(&schema.signature)?
        {
            return Ok(None);
        }
        let Some(owner) = info.owner else {
            return Ok(None);
        };
        let Some(position) = self.impl_header_index.get(&self.budget, owner)? else {
            return Ok(None);
        };
        let implementation = &self.headers.impls[position];
        if implementation.associated_types.is_empty() {
            return Ok(None);
        }
        // This producer uses the PROVIDED declaration's own source receipt.
        // It cannot borrow the required item's family or a selected signature.
        let canonical = self.callable_schema(target.declaration)?;
        if !self.same_env(&schema.env, &canonical.env)?
            || !self.eq_signature(&schema.signature, &canonical.signature)?
            || schema.predicates.len() != canonical.predicates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "direct result changed canonical callable requirements schema",
            ));
        }
        for (given, required) in schema.predicates.iter().zip(canonical.predicates.iter()) {
            if !self.eq_predicate(given, required)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "direct result changed ordered callable requirements",
                ));
            }
        }
        let original = self
            .callable_header_index
            .get(&self.budget, target.declaration)?
            .and_then(|position| self.headers.callables.get(position))
            .filter(|header| header.declaration == target.declaration)
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "direct result lost original callable header")
            })?;
        self.validate_declaration_schema(target.declaration, &original.schema)?;
        // Owner normalization may turn an original associated projection into
        // a parameter. That is not this original bare-parameter receipt. Keep
        // those calls on their established projection-authorized path.
        if !matches!(original.schema.signature.result, Ty::Param(_))
            || self.signature_contains_projection(&original.schema.signature)?
        {
            return Ok(None);
        }
        if !self.same_env(&original.schema.env, &schema.env)?
            || !self.eq_signature(&original.schema.signature, &schema.signature)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "direct result changed original provided signature adapter",
            ));
        }
        let schema = &original.schema;
        if implementation.declaration != owner
            || !self.method_environment_matches(
                &schema.env,
                &implementation.schema_env,
                &info.own_env,
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "direct result changed exact implementation owner binder",
            ));
        }
        let retained = self.declaration_carriers(target.declaration)?;
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
        )?;
        let base = ProjectedCarrierBase {
            schema: clone_schema(schema, &self.budget)?,
            arguments: target.type_args.clone(),
            origin,
            retained: DeclarationCarriers {
                direct_return: None,
                declaration: retained.declaration,
                contract: retained.contract.clone(),
                regions: retained.regions.clone(),
                source_regions: retained.source_regions.clone(),
            },
        };
        let original = self.specialize_projected_owner_base(&base, context)?;
        if !self.eq_types(&original.params, &signature.params, 0)?
            || !self.eq_ty(&original.result, &signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "direct result differs from its original provided source signature",
            ));
        }
        Ok(Some(original))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn specialize_required_owner_result(
        &self,
        declaration: DeclId,
        schema: &GenericSchema,
        arguments: &[Ty],
        signature: &Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<Arc<ir::RefContract>>> {
        if charged(&self.budget, ir::root_reference_layers(&signature.result))?
            <= signature.ref_contract.results.len()
            || context.phase != FactsPhase::Closed
            || !std::ptr::eq(context.predicates, self.empty.as_ref())
        {
            return Ok(None);
        }
        let Some(position) = self.item_header_index.get(&self.budget, declaration)? else {
            return Ok(None);
        };
        let item = &self.headers.items[position];
        let owner = self.trait_header(item.trait_decl)?;
        if item.default.is_some()
            || arguments.len() != owner.env.total_arity()
            || self.info(declaration)?.own_env.arity() != 0
            || !matches!(item.schema.signature.result, Ty::AssociatedType(_))
        {
            return Ok(None);
        }
        self.validate_declaration_schema(declaration, schema)?;
        let self_ty = arguments.first().ok_or_else(|| {
            FactsError::invalid(&self.budget, "required source family lost Self actual")
        })?;
        let goal = TraitRef {
            declaration: item.trait_decl,
            self_ty: self_ty.clone(),
            args: arguments[1..].into(),
        };
        if !self.goal_is_closed_in_with(&goal, context, active)? {
            return Ok(None);
        }
        let selection = self.require_in(&Predicate::Implements(goal.clone()), context, active)?;
        if !matches!(selection.0.origin, ProofOrigin::Implementation { .. }) {
            return Ok(None);
        }
        self.projected_implementation_result_contract(item, &goal, &selection, signature, context)
    }

    pub(super) fn specialize_universal_required_source_result(
        &self,
        declaration: DeclId,
        schema: &GenericSchema,
        arguments: &[Ty],
        signature: &Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<Arc<ir::RefContract>>> {
        if context.phase != FactsPhase::Universal
            || context.executable_storage
            || context.metadata_facts().is_none()
            || charged(&self.budget, ir::root_reference_layers(&signature.result))?
                <= signature.ref_contract.results.len()
        {
            return Ok(None);
        }
        self.check_proof_context_caller(context)?;
        let Some(position) = self.item_header_index.get(&self.budget, declaration)? else {
            return Ok(None);
        };
        let item = &self.headers.items[position];
        let owner = self.trait_header(item.trait_decl)?;
        if item.default.is_some()
            || arguments.len() != owner.env.total_arity()
            || self.info(declaration)?.own_env.arity() != 0
            || !matches!(item.schema.signature.result, Ty::AssociatedType(_))
        {
            return Ok(None);
        }
        self.validate_declaration_schema(declaration, schema)?;
        let self_ty = arguments.first().ok_or_else(|| {
            FactsError::invalid(&self.budget, "universal source family lost Self actual")
        })?;
        let goal = TraitRef {
            declaration: item.trait_decl,
            self_ty: self_ty.clone(),
            args: arguments[1..].into(),
        };
        let selection = self.require_in(&Predicate::Implements(goal.clone()), context, active)?;
        if !matches!(selection.0.origin, ProofOrigin::Implementation { .. }) {
            return Ok(None);
        }
        self.universal_implementation_result_contract(item, &goal, &selection, signature, context)
    }

    fn specialize_projected_owner_base(
        &self,
        base: &ProjectedCarrierBase,
        context: ProofContext<'_>,
    ) -> Result<Signature> {
        let instantiated = self.instantiate_scratch_for_use_with_owner_family(
            &base.schema,
            &base.arguments,
            context,
            Some(base.retained.declaration),
            &mut ProofScratch::default(),
            context.original,
            context.executable_storage,
            Some(base),
        )?;
        Ok(instantiated.signature)
    }

    pub(super) fn complete_projected_owner_family(
        &self,
        base: &ProjectedCarrierBase,
        signature: &mut Signature,
    ) -> Result<()> {
        let Ty::Param(parameter) = &base.schema.signature.result else {
            return Ok(());
        };
        if charged(&self.budget, ir::root_reference_layers(&signature.result))? == 0 {
            return Ok(());
        }
        // The root row is only region topology. The nonempty return authority
        // is the exact original parameter family, including its owned field
        // path, transported later by ordinary carrier substitution. Neither an
        // empty envelope nor coincident concrete reference types issue it.
        let source =
            self.validate_source_regions(base.retained.declaration, &base.schema.signature)?;
        let (contract, regions) =
            self.derive_carriers(&base.schema.signature, &source, &base.schema.env)?;
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(
                &base.schema.signature,
                &base.schema.env,
                &self.budget,
            ),
        )?;
        if !base.schema.env.contains(*parameter)
            || base.retained.source_regions != source
            || base.retained.contract != contract
            || base.retained.regions != regions
            || base.origin != origin
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected owner changed original family receipt",
            ));
        }
        let mut issued = false;
        for result in contract.results.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if !result.location.path.0.is_empty()
                || !matches!(result.location.channel, ir::CarrierChannel::ParamTail { parameter: p, prefix: 0 } if p == *parameter)
                || result.sources.is_empty()
            {
                continue;
            }
            for input in result.sources.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if !matches!(input.location.channel, ir::CarrierChannel::ParamTail { parameter: p, prefix: 0 } if p == *parameter)
                    || charged(
                        &self.budget,
                        ir::carrier_at(
                            base.schema
                                .signature
                                .params
                                .get(input.param)
                                .ok_or_else(|| {
                                    FactsError::invalid(
                                        &self.budget,
                                        "projected family source outside original parameters",
                                    )
                                })?,
                            &input.location,
                            &self.budget,
                        ),
                    )? != Ty::Param(*parameter)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected result family crossed original binder or source path",
                    ));
                }
            }
            issued = true;
        }
        if !issued {
            return Err(FactsError::unsupported(
                &self.budget,
                "projected result has no original input family authority",
            ));
        }
        // A family stored in an owned input field has no positioned input
        // reference. Consequently ordinary topology substitution cannot issue
        // its result rows. The authenticated carrier above, not a provided
        // method or a matching concrete input type, owns those hidden loans.
        // Materialize only the positions of this exact parameter actual here.
        // Carrier substitution retains the nonempty field-path sources.
        let layers = charged(&self.budget, ir::root_reference_layers(&signature.result))?;
        let fixed = &signature.ref_contract;
        let mut positioned = Vec::new();
        let mut outlives = Vec::new();
        for row in fixed.results.iter() {
            push(&self.budget, &mut positioned, row.clone())?;
        }
        for edge in fixed.outlives.iter() {
            push(&self.budget, &mut outlives, *edge)?;
        }
        let mut next = usize::from(fixed.region_count);
        for depth in 0..layers {
            charged(
                &self.budget,
                self.budget
                    .charge_work(positioned.len().saturating_mul(2) + 1),
            )?;
            let position = ir::ReferencePosition { depth: depth as u8 };
            if positioned.iter().any(|row| row.position == position) {
                continue;
            }
            if next >= ir::MAX_REGION_PARAMS {
                return Err(FactsError::resource(
                    &self.budget,
                    "projected family topology region limit exceeded",
                ));
            }
            let region = ir::RegionParamId(next as u16);
            next += 1;
            if depth > 0 {
                let above = positioned
                    .iter()
                    .find(|row| usize::from(row.position.depth) == depth - 1)
                    .ok_or_else(|| {
                        FactsError::invalid(&self.budget, "projected family lost parent layer")
                    })?;
                push(&self.budget, &mut outlives, (region, above.region))?;
            }
            push(
                &self.budget,
                &mut positioned,
                ir::ResultReference {
                    position,
                    region,
                    returns: ir::ReturnContract::NoReference,
                },
            )?;
        }
        charged(
            &self.budget,
            self.budget
                .charge_work(positioned.len().saturating_mul(positioned.len())),
        )?;
        positioned.sort_by_key(|row| row.position);
        let raw = ir::RefContract {
            region_count: next as u16,
            input_regions: fixed.input_regions.clone(),
            results: positioned.into(),
            outlives: outlives.into(),
            static_regions: fixed.static_regions,
        };
        signature.ref_contract =
            charged(&self.budget, ir::normalize_ref_contract(&raw, &self.budget))?;
        let results = charged(
            &self.budget,
            ir::derive_return_contracts(
                &signature.params,
                &signature.result,
                &signature.ref_contract,
                &self.budget,
            ),
        )?;
        retain::<ir::RefContract>(&self.budget, 1)?;
        signature.ref_contract = Arc::new(ir::RefContract {
            results,
            ..(*signature.ref_contract).clone()
        });
        Ok(())
    }

    /// A universal caller may select a concrete implementation head while its
    /// arguments remain rigid. Only that exact implementation source can expose
    /// reference layers hidden behind the required item's associated result.
    pub(super) fn universal_implementation_result_contract(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        selection: &ProofEvidence,
        signature: &Signature,
        context: ProofContext<'_>,
    ) -> Result<Option<Arc<ir::RefContract>>> {
        self.check_proof_context_caller(context)?;
        self.validate_projected_required_schema(item)?;
        if context.phase != FactsPhase::Universal
            || context.metadata_facts().is_none()
            || selection.0.world != self.id
            || selection.0.environment != context.predicates.index
            || selection.0.phase != context.phase
            || selection.0.executable_storage != context.executable_storage
            || item.trait_decl != goal.declaration
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal result changed exact caller implementation selection",
            ));
        }
        if !matches!(&selection.0.origin, ProofOrigin::Implementation { .. }) {
            return Ok(None);
        }
        let base = self.selected_implementation_base(item, goal, selection)?;
        if !matches!(base.schema.signature.result, Ty::Param(_))
            || self.signature_contains_projection(&base.schema.signature)?
        {
            return Ok(None);
        }
        let original = self.specialize_projected_owner_base(&base, context)?;
        if !self.eq_types(&original.params, &signature.params, 0)?
            || !self.eq_ty(&original.result, &signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal result differs from exact owner source signature",
            ));
        }
        Ok(Some(original.ref_contract))
    }

    fn projected_implementation_result_contract(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        selection: &ProofEvidence,
        signature: &Signature,
        context: ProofContext<'_>,
    ) -> Result<Option<Arc<ir::RefContract>>> {
        let base = self.projected_implementation_base(item, goal, selection)?;
        // Only an exact owner-normalized original schema can expose a latent
        // result parameter. An unresolved associated family grants no layers.
        if !matches!(base.schema.signature.result, Ty::Param(_))
            || self.signature_contains_projection(&base.schema.signature)?
        {
            // Existing outer references retain their source-issued root row.
            // complete_projected_contract expands only their inner layers.
            return Ok(None);
        }
        let original = self.specialize_projected_owner_base(&base, context)?;
        if !self.eq_types(&original.params, &signature.params, 0)?
            || !self.eq_ty(&original.result, &signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected result differs from exact owner source signature",
            ));
        }
        Ok(Some(original.ref_contract))
    }

    fn projected_carrier_base(
        &self,
        evidence: &ProjectedMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<ProjectedCarrierBase> {
        let builtin_pointer = matches!(evidence.goal.self_ty, Ty::FnPointer(_))
            && matches!(&evidence.selection.0.origin, ProofOrigin::FnItem { kind }
                if self.schema.closure_trait_kind(evidence.goal.declaration)? == Some(*kind));
        let caller_owned = builtin_pointer
            || matches!(
                &evidence.selection.0.origin,
                ProofOrigin::TraitObject | ProofOrigin::OpaqueDeclared
            );
        if evidence.selection.0.world != self.id
            || !evidence.selection.0.executable_storage
            || evidence.selection.0.environment
                != if caller_owned {
                    facts.predicates.index
                } else {
                    self.empty.index
                }
            || evidence.selection.0.phase
                != if caller_owned {
                    facts.phase()
                } else {
                    FactsPhase::Closed
                }
            || !self.eq_predicate(
                &evidence.selection.0.goal,
                &Predicate::Implements(evidence.goal.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier selection changed exact goal authority",
            ));
        }
        if !evidence.method_args.is_empty() {
            let schema = self.normalized_declaration_schema(evidence.item)?;
            let retained = self.declaration_carriers(evidence.item)?;
            let source_regions = retained.source_regions.clone();
            let (contract, regions) =
                self.derive_carriers(&schema.signature, &source_regions, &schema.env)?;
            let mut arguments = self.trait_arguments(&evidence.goal)?;
            for argument in evidence.method_args.iter() {
                push(&self.budget, &mut arguments, argument.clone())?;
            }
            let origin = charged(
                &self.budget,
                ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
            )?;
            return Ok(ProjectedCarrierBase {
                schema: clone_schema(&schema, &self.budget)?,
                arguments: arguments.into(),
                origin,
                retained: DeclarationCarriers {
                    direct_return: None,
                    declaration: retained.declaration,
                    contract,
                    regions,
                    source_regions,
                },
            });
        }
        if let ProofOrigin::Closure { kind } = &evidence.selection.0.origin {
            let mut active = ProofScratch::default();
            if let Some(adapter) = self.closed_closure_argument_result_signature(
                self.item_header(evidence.item)?,
                &evidence.goal,
                &evidence.signature,
                &evidence.selection,
                ProofContext::from_facts(facts),
                &mut active,
            )? {
                let Ty::Closure(closure) = &evidence.goal.self_ty else {
                    unreachable!()
                };
                if *kind != ir::ClosureKind::FnOnce {
                    retain::<Ty>(&self.budget, 1)?;
                }
                let target = self.closure_target(closure, *kind)?;
                let callee = Callee::Direct(target);
                let checked = self.closure_result_carrier_parts(
                    closure.clone(),
                    *kind,
                    adapter,
                    SignatureSubject::Call(&callee),
                    facts,
                )?;
                charged(
                    &self.budget,
                    self.budget.retain_bytes(4 * std::mem::size_of::<usize>()),
                )?;
                retain::<Ty>(&self.budget, evidence.signature.params.len())?;
                let schema = GenericSchema {
                    env: self.empty.env.clone(),
                    signature: evidence.signature.clone(),
                    predicates: Arc::from([]),
                };
                let source_regions =
                    self.validate_source_regions(evidence.item, &schema.signature)?;
                let origin = charged(
                    &self.budget,
                    ir::derive_signature_origin_schema(
                        &schema.signature,
                        &schema.env,
                        &self.budget,
                    ),
                )?;
                return Ok(ProjectedCarrierBase {
                    schema,
                    arguments: Arc::from([]),
                    origin,
                    retained: DeclarationCarriers {
                        direct_return: None,
                        declaration: evidence.item,
                        contract: checked.contract,
                        regions: checked.regions,
                        source_regions,
                    },
                });
            }
        }
        if matches!(
            &evidence.selection.0.origin,
            ProofOrigin::Closure { .. }
                | ProofOrigin::FnItem { .. }
                | ProofOrigin::TraitObject
                | ProofOrigin::OpaqueDeclared
        ) {
            let env = if caller_owned {
                facts.env().clone()
            } else {
                self.empty.env.clone()
            };
            let mut arguments = Vec::new();
            for parameter in env.ordered_params() {
                push(&self.budget, &mut arguments, Ty::Param(parameter))?;
            }
            let schema = GenericSchema {
                env,
                signature: evidence.signature.clone(),
                predicates: Arc::from([]),
            };
            let source_regions = self.validate_source_regions(evidence.item, &schema.signature)?;
            let origin = charged(
                &self.budget,
                ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
            )?;
            let (mut contract, regions) =
                self.derive_carriers(&schema.signature, &source_regions, &schema.env)?;
            if !caller_owned
                && matches!(&evidence.selection.0.origin, ProofOrigin::FnItem { .. })
                && matches!(evidence.goal.self_ty, Ty::FnItem(_))
                && !charged(
                    &self.budget,
                    ir::carrier_locations(&schema.signature.result, &self.budget),
                )?
                .is_empty()
            {
                contract.results =
                    self.fn_item_result_carriers(&evidence.goal, &contract, facts)?;
            }
            return Ok(ProjectedCarrierBase {
                schema,
                arguments: arguments.into(),
                origin,
                retained: DeclarationCarriers {
                    direct_return: None,
                    declaration: evidence.item,
                    contract,
                    regions,
                    source_regions,
                },
            });
        }
        let base = self.projected_implementation_base(
            self.item_header(evidence.item)?,
            &evidence.goal,
            &evidence.selection,
        )?;
        let signature =
            self.specialize_projected_owner_base(&base, ProofContext::from_facts(facts))?;
        if !self.eq_signature(&signature, &evidence.signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "closed projected signature disagrees with required owner transport",
            ));
        }
        Ok(base)
    }

    pub(super) fn validate_projected_carrier(
        &self,
        view: &CarrierContractView,
        evidence: &ProjectedMethodEvidence,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.check_facts(facts, true)?;
        scratch.for_facts(facts);
        self.check_normalization_budget()?;
        if evidence.world != self.id
            || evidence.environment != facts.predicates.index
            || evidence.phase != facts.phase()
            || evidence.item != view.declaration
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier changed exact application or caller",
            ));
        }
        if let super::parameter_equality::EqualityResultSubject::Required(call) =
            &view.result_subject
        {
            if call.item != evidence.item
                || !self.eq_types(&call.method_args, &evidence.method_args, 0)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "projected carrier changed exact method arguments",
                ));
            }
        } else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier lost required call identity",
            ));
        }
        let Some(subject) = &view.projected_subject else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier lost its exact trait application",
            ));
        };
        if !self.eq_predicate(
            &Predicate::Implements(subject.clone()),
            &Predicate::Implements(evidence.goal.clone()),
        )? {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier changed its full trait application",
            ));
        }
        if let Some(authority) = &evidence.universal {
            return self.validate_universal_required_carrier(view, authority, facts);
        }
        let application = self.instantiate_projected_item_with_scratch(
            self.item_header(evidence.item)?,
            &evidence.goal,
            &evidence.method_args,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        match (
            &evidence.selection.0.origin,
            &application.evidence.selection.0.origin,
        ) {
            (ProofOrigin::Closure { kind: left }, ProofOrigin::Closure { kind: right })
            | (ProofOrigin::FnItem { kind: left }, ProofOrigin::FnItem { kind: right })
                if left == right => {}
            (ProofOrigin::TraitObject, ProofOrigin::TraitObject)
            | (ProofOrigin::OpaqueDeclared, ProofOrigin::OpaqueDeclared) => {}
            (
                ProofOrigin::Implementation {
                    declaration: left,
                    arguments: left_args,
                },
                ProofOrigin::Implementation {
                    declaration: right,
                    arguments: right_args,
                },
            ) if left == right && self.eq_types(left_args, right_args, 0)? => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "projected carrier changed closed implementation selection",
                ));
            }
        }
        if !self.eq_signature(&evidence.signature, &application.instantiated.signature)?
            || evidence.source_regions != application.evidence.source_regions
            || !self.eq_contract(&view.fixed, &evidence.signature.ref_contract)?
            || evidence.dependencies.len() != application.evidence.dependencies.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier changed required signature or source regions",
            ));
        }
        for (dependency, expected) in evidence
            .dependencies
            .iter()
            .zip(application.evidence.dependencies.iter())
        {
            self.validate_projected_dependency_with(
                dependency,
                ProofContext::from_facts(facts),
                scratch.for_world(self.id),
            )?;
            if !self.same_projected_dependency(dependency, expected)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "projected carrier dependency changed original required projection",
                ));
            }
        }
        let base = self.projected_carrier_base(evidence, facts)?;
        if !self.same_env(&view.original_env, &base.schema.env)?
            || !self.eq_types(&view.arguments, &base.arguments, 0)?
            || view.original_contract != base.retained.contract
            || view.origin != base.origin
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier changed owner schema or substitution",
            ));
        }
        // Reuse ordinary transport, with no projected proof, to independently
        // check the final carrier shape without recursive evidence validation.
        retain::<Ty>(&self.budget, evidence.signature.params.len())?;
        let expected = self.transport_carrier_parts_with_scratch(
            evidence.item,
            base.arguments,
            evidence.signature.clone(),
            facts,
            base.schema,
            &base.retained,
            base.origin,
            None,
            view.result_subject.clone(),
            scratch,
        )?;
        if view.contract != expected.contract
            || view.regions != expected.regions
            || view.source_regions != expected.source_regions
            || view.components != expected.components
            || view.nominals != expected.nominals
        {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier transport changed normalized required contract",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn world(source: &str) -> Arc<ValidatedTraitWorld> {
        let source = frontend::parse(source).unwrap();
        PlanCtx::from_program(&source, &PlanBudget::default())
            .unwrap()
            .trait_world()
            .unwrap()
    }

    #[test]
    fn projected_borrow_origin_composition_preserves_source_authority() {
        for actual in ["i32", "&'a i32"] {
            let source = format!(
                "struct Payload<T,U>{{value:T,other:U}}struct Owner<T,U>{{payload:Payload<T,U>}}struct Impostor<T,U>{{payload:Payload<T,U>}}trait Project{{type Out;fn project(&self)->&Self::Out;}}impl<T,U> Project for Owner<T,U>{{type Out=Payload<T,U>;fn project(&self)->&Payload<T,U>{{&self.payload}}}}fn scoped<'a>(owner:Owner<{actual},bool>){{}}fn other<'a>(owner:Impostor<{actual},bool>){{}}fn main(){{}}"
            );
            let program = frontend::parse(&source).unwrap();
            let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
            let world = plan.trait_world().unwrap();
            let declaration = plan.function_decl(0).unwrap();
            let facts = world.callable_facts(declaration).unwrap();
            let self_ty = plan.signature(declaration).unwrap().signature.params[0].clone();
            let owner = owners(&world)[0];
            let call = TraitCall {
                trait_ref: TraitRef {
                    declaration: owner.trait_ref.declaration,
                    self_ty: self_ty.clone(),
                    args: Arc::from([]),
                },
                item: owner.items[0].0,
                method_args: Arc::from([]),
            };
            let callee = Callee::TraitItem(call.clone());
            let required = world.instantiate_trait_signature(&call, &facts).unwrap();
            facts
                .validate_signature(SignatureSubject::Call(&callee), &required)
                .unwrap();
            let selected = world.resolve_trait_call(&call, &facts).unwrap();
            facts
                .validate_signature(
                    SignatureSubject::Call(&selected.callee),
                    &selected.signature,
                )
                .unwrap();
            assert!(!required.ref_contract.results.is_empty());
            for change in 0..3 {
                let mut forged = required.clone();
                let contract = Arc::make_mut(&mut forged.ref_contract);
                let rows = Arc::make_mut(&mut contract.results);
                match change {
                    0 => rows[0].returns = ir::ReturnContract::SharedInputPaths(Arc::from([])),
                    1 => rows[0].region = ir::RegionParamId(contract.region_count),
                    _ => contract.input_regions = Arc::from([]),
                }
                assert!(
                    matches!(facts.validate_signature(SignatureSubject::Call(&callee), &forged),
                        Err(error) if error.kind() == FactsErrorKind::InvalidMetadata),
                    "forged origin {change}, actual {actual}"
                );
            }
            let Ty::Struct(nominal) = &self_ty else {
                panic!("expected owner")
            };
            for change in 0..3 {
                let mut forged = nominal.def().clone();
                match change {
                    0 => {
                        let mut fields = forged.fields.read(&world.budget).unwrap().to_vec();
                        fields[0].ty = Ty::Bool;
                        forged.fields = fields.into();
                    }
                    1 => Arc::make_mut(&mut forged.type_args).swap(0, 1),
                    _ => {
                        let other = plan.signature(plan.function_decl(1).unwrap()).unwrap();
                        let Ty::Struct(other) = &other.signature.params[0] else {
                            panic!("expected impostor")
                        };
                        let name = forged.name;
                        forged = other.def().clone();
                        forged.name = name;
                    }
                }
                let mut wrong = call.clone();
                wrong.trait_ref.self_ty = Ty::Struct(ir::StructTy::new(forged));
                assert!(
                    world.resolve_trait_call(&wrong, &facts).is_err(),
                    "forged nominal {change}, actual {actual}"
                );
            }
            let foreign_plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
            let foreign_world = foreign_plan.trait_world().unwrap();
            let foreign_facts = foreign_world
                .callable_facts(foreign_plan.function_decl(0).unwrap())
                .unwrap();
            assert!(world.resolve_trait_call(&call, &foreign_facts).is_err());
            let mut foreign_call = call.clone();
            foreign_call.trait_ref.self_ty = foreign_plan
                .signature(foreign_plan.function_decl(0).unwrap())
                .unwrap()
                .signature
                .params[0]
                .clone();
            assert!(world.resolve_trait_call(&foreign_call, &facts).is_err());
        }
    }

    fn paired() -> Arc<ValidatedTraitWorld> {
        world(
            "struct S;trait Has<A,B>{type Out;fn get(self)->Self::Out;}impl Has<i32,bool> for S{type Out=i128;fn get(self)->Self::Out{7i128}}impl Has<bool,i32> for S{type Out=bool;fn get(self)->Self::Out{true}}fn main(){}",
        )
    }

    fn projected_marker_paired() -> Arc<ValidatedTraitWorld> {
        world(
            "struct S;trait Has<A,B> where Self::Out:Copy+Sized{type Out;fn get(self)->Self::Out;}impl Has<i32,bool> for S{type Out=i128;fn get(self)->i128{7i128}}impl Has<bool,i32> for S{type Out=bool;fn get(self)->bool{true}}fn main(){}",
        )
    }

    #[test]
    fn projected_marker_bounds_normalize_actuals_and_replay_exact_applications() {
        let world = projected_marker_paired();
        let implementations = owners(&world);
        let facts = world.callable_facts(implementations[0].items[0].1).unwrap();
        let mut views = Vec::new();
        for owner in &implementations {
            let call = closed_call(owner);
            let application = world
                .instantiate_projected_item(
                    world.item_header(call.item).unwrap(),
                    &call.trait_ref,
                    &call.method_args,
                    ProofContext::from_facts(&facts),
                )
                .unwrap();
            assert!(
                world
                    .projected_marker_predicates(&world.item_header(call.item).unwrap().schema,)
                    .unwrap()
            );
            for predicate in application.instantiated.predicates.iter() {
                assert!(
                    !world
                        .type_contains_projection(&predicate.trait_ref().self_ty, 0,)
                        .unwrap()
                );
                world
                    .require_in(
                        predicate,
                        ProofContext::from_facts(&facts),
                        &mut ProofScratch::default(),
                    )
                    .unwrap();
            }
            assert!(
                world
                    .eq_ty(
                        &application.instantiated.signature.result,
                        &owner.associated_types[0].rhs,
                        0,
                    )
                    .unwrap()
            );
            let callee = Callee::TraitItem(call);
            let view = facts
                .carrier_contract(SignatureSubject::Call(&callee))
                .unwrap();
            view.validate_for(&facts).unwrap();
            views.push(view);
        }
        // Same Self and same associated declaration do not merge ordered args.
        let mut wrong_arguments = views[0].as_ref().clone();
        wrong_arguments.projected = views[1].projected.clone();
        assert_eq!(
            wrong_arguments.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata,
        );
        let foreign = projected_marker_paired();
        let foreign_facts = foreign
            .callable_facts(owners(&foreign)[0].items[0].1)
            .unwrap();
        assert_eq!(
            views[0].validate_for(&foreign_facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata,
        );
    }

    #[test]
    fn projected_marker_bounds_do_not_grant_copy_to_noncopy_actuals() {
        for (bound, succeeds) in [("Copy", false), ("Sized", true)] {
            let source = format!(
                "struct S;struct NoCopy;trait Has{{type Out;fn get(self)->Self::Out where Self::Out:{bound};}}impl Has for S{{type Out=NoCopy;fn get(self)->NoCopy{{NoCopy}}}}fn main(){{}}"
            );
            let world = world(&source);
            let owner = owners(&world)[0];
            let facts = world.callable_facts(owner.items[0].1).unwrap();
            let call = closed_call(owner);
            let application = world.instantiate_projected_item(
                world.item_header(call.item).unwrap(),
                &call.trait_ref,
                &call.method_args,
                ProofContext::from_facts(&facts),
            );
            if succeeds {
                let application = application.unwrap();
                assert!(
                    !facts
                        .must_copy(&application.instantiated.signature.result)
                        .unwrap()
                );
            } else {
                assert!(
                    matches!(application, Err(error) if error.kind() == FactsErrorKind::Unproved)
                );
            }
        }
    }

    #[test]
    fn projected_marker_bounds_reject_forged_original_item_arguments_and_identity() {
        let world = projected_marker_paired();
        let item = world.item_header(owners(&world)[0].items[0].0).unwrap();
        for mutation in 0..3 {
            let mut forged = item.clone();
            let row = Arc::make_mut(&mut forged.schema.predicates)
                .iter_mut()
                .find_map(|predicate| match predicate {
                    Predicate::Implements(goal)
                        if matches!(&goal.self_ty, Ty::AssociatedType(_)) =>
                    {
                        Some(goal)
                    }
                    _ => None,
                })
                .unwrap();
            if mutation == 2 {
                row.declaration = world.catalog.id(CatalogItem::Clone);
            } else {
                let Ty::AssociatedType(projection) = &mut row.self_ty else {
                    unreachable!()
                };
                let projection = Arc::make_mut(projection);
                if mutation == 0 {
                    projection.item.index = u64::MAX;
                } else {
                    Arc::make_mut(&mut projection.trait_ref.args).swap(0, 1);
                }
            }
            assert!(matches!(
                world.validate_projected_required_schema(&forged),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
            ));
        }
    }

    #[test]
    fn projected_marker_bounds_preserve_user_trait_capability_boundary() {
        let source = frontend::parse(
            "trait Copy{}trait Has where Self::Out:Copy{type Out;fn get(self)->Self::Out;}fn main(){}",
        ).unwrap();
        let result = PlanCtx::from_program(&source, &PlanBudget::default())
            .and_then(|plan| plan.trait_world());
        assert!(matches!(
            result,
            Err(error) if error.contains("unsupported: projected method associated bounds")
        ));
    }

    #[test]
    fn projected_marker_impl_wf_rejects_missing_actual_copy_even_unused() {
        for source in [
            "struct S;struct NoCopy;trait Has where Self::Out:Copy{type Out;fn get(self)->Self::Out;}impl Has for S{type Out=NoCopy;fn get(self)->NoCopy{NoCopy}}fn main(){}",
            "struct Wrap<T>(T);trait Has where Self::Out:Copy{type Out;fn get(self)->Self::Out;}impl<T> Has for Wrap<T>{type Out=T;fn get(self)->T{self.0}}fn main(){}",
        ] {
            let program = frontend::parse(source).unwrap();
            let result = PlanCtx::from_program(&program, &PlanBudget::default())
                .and_then(|plan| plan.trait_world());
            assert!(matches!(
                result,
                Err(error) if error.contains("trait obligation is not proven by caller bounds or a conditional implementation")
            ));
        }
    }

    #[test]
    fn projected_marker_impl_wf_rejects_copied_owner_and_changed_ordered_arguments() {
        let world = projected_marker_paired();
        let implementations = owners(&world);
        let first = implementations[0];
        let facts = world.callable_facts(first.items[0].1).unwrap();
        let copied = (*first).clone();
        assert!(matches!(
            world.require_impl_wf(
                &copied,
                &first.trait_ref,
                &[],
                ProofContext::from_facts(&facts),
                &mut ProofScratch::default(),
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        assert!(matches!(
            world.require_impl_wf(
                first,
                &implementations[1].trait_ref,
                &[],
                ProofContext::from_facts(&facts),
                &mut ProofScratch::default(),
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
    }

    #[test]
    fn projected_marker_normalization_preserves_active_rhs_obligation_ancestry() {
        let world = projected_marker_paired();
        let owner = owners(&world)[0];
        let mut active = ProofScratch::default();
        // Closed implementation selection uses the assumption-free world
        // environment, not the callable environment of the original item.
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &world.empty,
            executable_storage: true,
            phase: FactsPhase::Closed,
        };
        let subject = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: owner.trait_ref.clone(),
            item: owner.associated_types[0].associated,
        }));
        let goal = Predicate::Implements(TraitRef {
            declaration: world.catalog.id(CatalogItem::Copy),
            self_ty: subject,
            args: Arc::from([]),
        });
        active
            .with_caller_arena(world.id, context.metadata_facts(), |arena| {
                arena.active.push(GoalKey {
                    executable_storage: context.executable_storage,
                    environment: context.predicates.index,
                    phase: context.phase,
                    source_metadata: context.source_key()?,
                    finalized: world.owner_normalization_finalized,
                    goal: Predicate::Implements(TraitRef {
                        declaration: world.catalog.id(CatalogItem::Copy),
                        self_ty: owner.associated_types[0].rhs.clone(),
                        args: Arc::from([]),
                    }),
                });
                // The owning implementation must prove its actual RHS Copy
                // obligation. Preserve that ancestry through normalization.
                // Implementation selection itself is not a prove_in entry.
                let result = world.prove_projected_subject(&goal, context, arena);
                match result {
                    Err(error) => assert_eq!(error.kind(), FactsErrorKind::Cycle),
                    Ok(Some(ProofOutcome::Proved(_))) => {
                        panic!("normalization unexpectedly proved an active RHS obligation")
                    }
                    Ok(Some(ProofOutcome::NotProven)) => {
                        panic!("normalization returned NotProven instead of Cycle")
                    }
                    Ok(None) => panic!("projected subject was not consumed"),
                }
                assert_eq!(arena.active.len(), 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn borrowed_receiver_gate_rejects_other_positions_kinds_and_projected_referents() {
        let world = paired();
        let shared = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
        };
        let receiver = ReceiverMode::Borrowed {
            kind: ir::BorrowKind::Shared,
        };
        world
            .validate_projected_parameter_references(&shared, 0, receiver)
            .unwrap();
        for (index, mode) in [
            (1, receiver),
            (0, ReceiverMode::None),
            (0, ReceiverMode::Value),
            (
                0,
                ReceiverMode::Borrowed {
                    kind: ir::BorrowKind::Mut,
                },
            ),
        ] {
            assert!(
                world
                    .validate_projected_parameter_references(&shared, index, mode)
                    .is_err()
            );
        }
        let projection = world
            .headers
            .items
            .iter()
            .find_map(|item| {
                matches!(&item.schema.signature.result, Ty::AssociatedType(_))
                    .then(|| item.schema.signature.result.clone())
            })
            .unwrap();
        let projected = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(projection),
        };
        assert!(
            world
                .validate_projected_parameter_references(&projected, 0, receiver)
                .is_err()
        );
        assert!(world.reject_projected_references(&shared, 0).is_err());
    }

    #[test]
    fn projected_fixed_string_input_does_not_authorize_results_or_mutable_inputs() {
        let world = paired();
        let shared = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Str),
        };
        for index in [0, 1] {
            world
                .validate_projected_parameter_references(&shared, index, ReceiverMode::None)
                .unwrap();
        }
        assert!(world.reject_projected_references(&shared, 0).is_err());
        let mutable = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(Ty::Str),
        };
        assert!(
            world
                .validate_projected_parameter_references(&mutable, 0, ReceiverMode::None)
                .is_err()
        );
        let nested = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(shared),
        };
        assert!(
            world
                .validate_projected_parameter_references(&nested, 0, ReceiverMode::None)
                .is_err()
        );
    }

    fn owners(world: &ValidatedTraitWorld) -> Vec<&ImplHeader> {
        world
            .headers
            .impls
            .iter()
            .filter(|h| !h.associated_types.is_empty())
            .collect()
    }

    fn closed_call(owner: &ImplHeader) -> TraitCall {
        TraitCall {
            trait_ref: owner.trait_ref.clone(),
            item: owner.items[0].0,
            method_args: Arc::from([]),
        }
    }

    #[test]
    fn projected_required_schema_never_publishes_global_empty_carriers() {
        let world = paired();
        let item = owners(&world)[0].items[0].0;
        assert!(
            world
                .schema_contains_projection(&world.item_header(item).unwrap().schema)
                .unwrap()
        );
        assert!(
            world
                .origins
                .iter()
                .any(|(declaration, _)| *declaration == item)
        );
        let carriers = world.declaration_carriers(item).unwrap();
        assert!(carriers.contract.results.iter().any(|result| matches!(
            result.location.channel,
            ir::CarrierChannel::ProjectionTail { .. }
        )));
        let facts = world.callable_facts(owners(&world)[0].items[0].1).unwrap();
        assert_eq!(facts.source_region_env(item).unwrap().declaration, item);
    }

    #[test]
    fn projected_owner_substitution_keeps_real_generic_parameter_and_is_not_closed_evidence() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn get(self)->Self::Out;}impl<T> Has for Wrap<T>{type Out=T;fn get(self)->Self::Out{let value:Self::Out=self.value;value}}fn main(){}",
        );
        let owner = owners(&world)[0];
        let parameter = owner.schema_env.ordered_params().next().unwrap();
        let target = owner.items[0].1;
        let facts = world.callable_facts(target).unwrap();
        let raw = world.schema.declaration_schema(target).unwrap();
        assert!(
            world
                .type_contains_projection(&raw.signature.result, 0)
                .unwrap()
        );
        let normalized = world.normalized_callable_schema(target).unwrap();
        assert!(matches!(normalized.signature.result, Ty::Param(p) if p == parameter));
        let projection = ir::AssociatedTypeProjection {
            trait_ref: owner.trait_ref.clone(),
            item: owner.associated_types[0].associated,
        };
        let ty = Ty::AssociatedType(Arc::new(projection.clone()));
        assert!(matches!(world.normalize_ty(&ty, &facts).unwrap(), Ty::Param(p) if p == parameter));
        let normalized = world
            .normalize_associated_type(&projection, &facts)
            .unwrap();
        normalized.validate_for(&projection, &facts).unwrap();
        assert!(matches!(normalized.ty(), Ty::Param(p) if *p == parameter));
        assert!(
            !world
                .goal_is_closed(&projection.trait_ref, facts.env())
                .unwrap()
        );
        let carrier = facts.carrier_contract(SignatureSubject::Body).unwrap();
        assert!(carrier.original_contract().results.iter().any(|result| {
            matches!(result.location.channel, ir::CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
                && !result.sources.is_empty()
        }));

        // Required-item source identity remains the trait declaration, but
        // its instantiated signature is scoped by the implementation binder.
        // Associated producer bounds must never be read in the trait's env.
        let world = self::world(
            "trait Make{type Out;fn get(&mut self)->Self::Out;}trait Source{type Item;type Producer:Make<Out=Self::Item>;}struct Wrap<A,B,S:Source>{a:A,b:B,producer:S::Producer}trait Next{type Item;fn next(&mut self)->Self::Item;}impl<A,B,S:Source> Next for Wrap<A,B,S>{type Item=S::Item;fn next(&mut self)->Self::Item{self.producer.get()}}fn main(){}",
        );
        let owner = owners(&world)[0];
        let target = owner.items[0].1;
        let schema = world.normalized_callable_schema(target).unwrap();
        let source = world
            .validate_source_regions(target, &schema.signature)
            .unwrap();
        let (cold, _) = world
            .derive_carriers(&schema.signature, &source, &schema.env)
            .unwrap();
        let (warm, _) = world
            .derive_carriers(&schema.signature, &source, &schema.env)
            .unwrap();
        assert!(!cold.results[0].sources.is_empty());
        assert_eq!(cold.results, warm.results);
        let trait_env = &world.info(owner.trait_ref.declaration).unwrap().env;
        assert!(matches!(
            world.derive_carriers(&schema.signature, &source, trait_env),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
    }

    fn shared(ty: Ty) -> Ty {
        Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ty),
        }
    }

    #[test]
    fn projected_owner_direct_instance_rechecks_associated_output_after_substitution() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn get(value:Self::Out)->Self::Out;}impl<T> Has for Wrap<T>{type Out=T;fn get(value:Self::Out)->Self::Out{value}}fn main(){}",
        );
        let declaration = owners(&world)[0].items[0].1;
        let mut scratch = TraitScratch::default();
        for actual in [
            Ty::Int(ir::IntTy::I32),
            Ty::Int(ir::IntTy::I128),
            Ty::Tuple(Arc::from([Ty::Bool, Ty::Int(ir::IntTy::I32)])),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([actual.clone()]),
            };
            let facts = world.instance_facts(&target).unwrap();
            let proof = world
                .discharge_direct_with(&target, &facts, &mut scratch)
                .unwrap();
            assert!(world.eq_ty(&proof.signature.result, &actual, 0).unwrap());
            for _ in 0..3 {
                let replay = world
                    .discharge_direct_with(&target, &facts, &mut scratch)
                    .unwrap();
                assert!(
                    world
                        .eq_signature(&replay.signature, &proof.signature)
                        .unwrap()
                );
            }
            let callee = Callee::Direct(target.clone());
            let mut forged = proof.signature.clone();
            forged.result = Ty::Never;
            assert!(matches!(
                facts.validate_signature_with(SignatureSubject::Call(&callee), &forged, &mut scratch),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
            ));
            let universal = world.callable_facts(declaration).unwrap();
            let replay = world
                .discharge_direct_with(&target, &universal, &mut scratch)
                .unwrap();
            assert!(
                world
                    .eq_signature(&replay.signature, &proof.signature)
                    .unwrap()
            );
        }
        for actual in [
            shared(Ty::Int(ir::IntTy::I32)),
            shared(shared(Ty::Int(ir::IntTy::I32))),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([actual.clone()]),
            };
            // Root-reference actuals are admitted by the common carrier path.
            // The direct proof must still preserve the exact associated output.
            world
                .instantiate_in(
                    &world.callable_schema(declaration).unwrap(),
                    &target.type_args,
                    ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &world.empty,
                        executable_storage: true,
                        phase: FactsPhase::Closed,
                    },
                    Some(declaration),
                )
                .unwrap();
            let facts = world.instance_facts(&target).unwrap();
            let proof = world.discharge_direct(&target, &facts).unwrap();
            assert!(world.eq_ty(&proof.signature.result, &actual, 0).unwrap());
        }
    }

    #[test]
    fn projected_owner_direct_instance_rechecks_body_local_only_assignments() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn consume(self)->i32;}impl<T> Has for Wrap<T>{type Out=T;fn consume(self)->i32{let value:Self::Out=self.value;0i32}}fn main(){}",
        );
        let declaration = owners(&world)[0].items[0].1;
        assert!(
            !world
                .schema_contains_projection(&world.schema.declaration_schema(declaration).unwrap())
                .unwrap()
        );
        for actual in [Ty::Bool, Ty::Int(ir::IntTy::I128)] {
            world
                .instance_facts(&CallTarget {
                    declaration,
                    type_args: Arc::from([actual]),
                })
                .unwrap();
        }
        for actual in [
            shared(Ty::Int(ir::IntTy::I32)),
            shared(shared(Ty::Int(ir::IntTy::I32))),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([actual]),
            };
            world
                .instantiate_in(
                    &world.callable_schema(declaration).unwrap(),
                    &target.type_args,
                    ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &world.empty,
                        executable_storage: true,
                        phase: FactsPhase::Closed,
                    },
                    Some(declaration),
                )
                .unwrap();
            let facts = world.instance_facts(&target).unwrap();
            let proof = world.discharge_direct(&target, &facts).unwrap();
            assert_eq!(proof.signature.result, Ty::Int(ir::IntTy::I32));
        }
    }

    #[test]
    fn projected_owner_direct_instance_allows_fixed_output_and_unrelated_reference_actual() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn get(self)->Self::Out;}impl<T> Has for Wrap<T>{type Out=i32;fn get(self)->Self::Out{7i32}}fn main(){}",
        );
        let declaration = owners(&world)[0].items[0].1;
        for actual in [
            shared(Ty::Int(ir::IntTy::I32)),
            shared(shared(Ty::Int(ir::IntTy::I32))),
        ] {
            world
                .instance_facts(&CallTarget {
                    declaration,
                    type_args: Arc::from([actual]),
                })
                .unwrap();
        }
    }

    #[test]
    fn projected_owner_direct_generic_call_retains_real_rigid_parameter() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn get(value:Self::Out)->Self::Out;}impl<T> Has for Wrap<T>{type Out=T;fn get(value:Self::Out)->Self::Out{value}}fn main(){}",
        );
        let owner = owners(&world)[0];
        let parameter = owner.schema_env.ordered_params().next().unwrap();
        let declaration = owner.items[0].1;
        let facts = world.callable_facts(declaration).unwrap();
        let proof = world
            .discharge_direct(
                &CallTarget {
                    declaration,
                    type_args: Arc::from([Ty::Param(parameter)]),
                },
                &facts,
            )
            .unwrap();
        assert!(matches!(proof.signature.result, Ty::Param(p) if p == parameter));
    }

    #[test]
    fn ordinary_identity_direct_instance_still_accepts_reference_actuals() {
        let world = world("fn identity<T>(value:T)->T{value}fn main(){}");
        // This fixture has exactly one generic source function. Catalog
        // callables can have the same arity but are not the identity control.
        let mut identities = world.headers.callables.iter().filter(|callable| {
            world.catalog.declaration(callable.declaration).is_none()
                && callable.schema.env.total_arity() == 1
        });
        let declaration = identities.next().unwrap().declaration;
        assert!(identities.next().is_none());
        for actual in [
            Ty::Int(ir::IntTy::I32),
            shared(Ty::Int(ir::IntTy::I32)),
            shared(shared(Ty::Int(ir::IntTy::I32))),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([actual.clone()]),
            };
            let facts = world.instance_facts(&target).unwrap();
            let proof = world.discharge_direct(&target, &facts).unwrap();
            assert!(world.eq_ty(&proof.signature.result, &actual, 0).unwrap());
        }
    }

    #[test]
    fn closed_owner_nested_item_normalization_preserves_authored_schema() {
        let world = world(
            "enum Maybe<T>{None,Some(T)}trait It{type Item;fn next(&mut self)->Maybe<Self::Item>;}struct Left;struct Right;impl It for Left{type Item=i32;fn next(&mut self)->Maybe<Self::Item>{Maybe::None}}impl It for Right{type Item=bool;fn next(&mut self)->Maybe<Self::Item>{Maybe::None}}fn main(){}",
        );
        let implementations = owners(&world);
        assert_eq!(implementations.len(), 2);
        let mut declarations = Vec::new();
        for implementation in &implementations {
            let callable = world
                .headers
                .callables
                .iter()
                .find(|callable| {
                    world.info(callable.declaration).unwrap().owner
                        == Some(implementation.declaration)
                })
                .unwrap();
            let raw = &callable.schema;
            assert!(
                world
                    .type_contains_projection(&raw.signature.result, 0)
                    .unwrap()
            );
            let normalized = world
                .normalize_owner_callable_schema(callable.declaration, raw)
                .unwrap()
                .unwrap();
            assert!(
                !world
                    .type_contains_projection(&normalized.signature.result, 0)
                    .unwrap()
            );
            // A normalized descriptor is not permission to replace the authored
            // projection occurrence in the owning declaration.
            let mut forged = clone_schema(raw, &world.budget).unwrap();
            forged.signature.result = normalized.signature.result.clone();
            assert!(
                world
                    .normalize_owner_callable_schema(callable.declaration, &forged)
                    .is_err()
            );
            declarations.push((callable.declaration, raw));
        }
        assert!(
            world
                .normalize_owner_callable_schema(declarations[0].0, declarations[1].1)
                .is_err()
        );
        assert!(
            world
                .normalize_owner_callable_schema(declarations[1].0, declarations[0].1)
                .is_err()
        );
    }

    #[test]
    fn projected_owner_source_schema_does_not_claim_nominal_storage() {
        let world = world(
            "trait It{type Item;}struct Cursor;impl It for Cursor{type Item=i32;}enum Slot<T>{Empty,Value(T)}struct Buffer<I:It>{value:Slot<I::Item>}trait Make{type Out;}struct C;impl Make for C{type Out=Buffer<Cursor>;}fn main(){}",
        );
        let owner = owners(&world)
            .into_iter()
            .find(|owner| {
                owner.associated_types.iter().any(|value| {
                    matches!(&value.rhs, Ty::Struct(_))
                        && world.type_contains_projection(&value.rhs, 0).unwrap()
                })
            })
            .unwrap();
        let context = world.owner_context(owner).unwrap();
        let value = &owner.associated_types[0];
        let projection = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: owner.trait_ref.clone(),
            item: value.associated,
        }));
        let source = world
            .normalize_owner_type_for_use(&projection, &context, &mut Vec::new(), 0, false)
            .unwrap();
        assert!(world.eq_ty(&source, &value.rhs, 0).unwrap());
        assert!(world.type_contains_projection(&source, 0).unwrap());
        // Source replay is not a storage receipt. The ordinary executable
        // entry point must still require independent canonical field reminting.
        let error = world
            .normalize_owner_type(&projection, &context, &mut Vec::new(), 0)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
    }

    #[test]
    fn projected_owner_context_rejects_copied_and_foreign_headers() {
        let world = paired();
        let owner = owners(&world)[0];
        assert!(world.owner_context(owner).is_ok());
        let copied = (*owner).clone();
        assert!(
            matches!(world.owner_context(&copied), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
        let foreign = paired();
        let context = world.owner_context(owner).unwrap();
        assert_eq!(
            foreign.check_owner_context(&context).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut wrong_stage = world.owner_context(owner).unwrap();
        wrong_stage.finalized = !wrong_stage.finalized;
        assert_eq!(
            world.check_owner_context(&wrong_stage).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn projected_owner_does_not_equate_reordered_complete_trait_arguments() {
        let world = paired();
        let owners = owners(&world);
        let context = world.owner_context(owners[0]).unwrap();
        let other = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: owners[1].trait_ref.clone(),
            item: owners[1].associated_types[0].associated,
        }));
        // The other closed application is resolved by its own selected assignment,
        // never by this owner's RHS despite identical Self and associated item.
        let actual = world
            .normalize_owner_type(&other, &context, &mut Vec::new(), 0)
            .unwrap();
        assert!(
            world
                .eq_ty(&actual, &owners[1].associated_types[0].rhs, 0)
                .unwrap()
        );
        assert!(
            !world
                .eq_ty(&actual, &owners[0].associated_types[0].rhs, 0)
                .unwrap()
        );
    }

    #[test]
    fn projected_pointer_owner_preserves_kind_and_exact_assignment() {
        let world = paired();
        let owners = owners(&world);
        let context = world.owner_context(owners[0]).unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: owners[1].trait_ref.clone(),
            item: owners[1].associated_types[0].associated,
        };
        for kind in [ir::BorrowKind::Shared, ir::BorrowKind::Mut] {
            let pointer = Ty::RawPtr {
                kind,
                pointee: Arc::new(Ty::AssociatedType(Arc::new(projection.clone()))),
            };
            let actual = world
                .normalize_owner_type(&pointer, &context, &mut Vec::new(), 0)
                .unwrap();
            let expected = Ty::RawPtr {
                kind,
                pointee: Arc::new(owners[1].associated_types[0].rhs.clone()),
            };
            assert!(world.eq_ty(&actual, &expected, 0).unwrap());
            let wrong = Ty::RawPtr {
                kind,
                pointee: Arc::new(owners[0].associated_types[0].rhs.clone()),
            };
            assert!(!world.eq_ty(&actual, &wrong, 0).unwrap());
            let mut forged = projection.clone();
            forged.item.index = u64::MAX;
            let forged = Ty::RawPtr {
                kind,
                pointee: Arc::new(Ty::AssociatedType(Arc::new(forged))),
            };
            assert!(
                world
                    .normalize_owner_type(&forged, &context, &mut Vec::new(), 0)
                    .is_err()
            );
            let mut wrong_owner = projection.clone();
            wrong_owner.trait_ref.declaration = owners[0].declaration;
            let wrong_owner = Ty::RawPtr {
                kind,
                pointee: Arc::new(Ty::AssociatedType(Arc::new(wrong_owner))),
            };
            assert!(
                world
                    .normalize_owner_type(&wrong_owner, &context, &mut Vec::new(), 0)
                    .is_err()
            );
        }
    }

    #[test]
    fn projected_required_carriers_bind_exact_item_application_and_caller() {
        let world = paired();
        let owners = owners(&world);
        let facts = world.callable_facts(owners[0].items[0].1).unwrap();
        let first = closed_call(owners[0]);
        let second = closed_call(owners[1]);
        let first_callee = Callee::TraitItem(first.clone());
        let second_callee = Callee::TraitItem(second.clone());
        let first_view = facts
            .carrier_contract(SignatureSubject::Call(&first_callee))
            .unwrap();
        first_view.validate_for(&facts).unwrap();
        let second_view = facts
            .carrier_contract(SignatureSubject::Call(&second_callee))
            .unwrap();
        assert!(
            world
                .eq_ty(
                    &first_view.projected.as_ref().unwrap().signature.result,
                    &owners[0].associated_types[0].rhs,
                    0
                )
                .unwrap()
        );
        assert!(
            world
                .eq_ty(
                    &second_view.projected.as_ref().unwrap().signature.result,
                    &owners[1].associated_types[0].rhs,
                    0
                )
                .unwrap()
        );
        let foreign_facts = world.callable_facts(owners[1].items[0].1).unwrap();
        assert_eq!(
            first_view.validate_for(&foreign_facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut wrong_method_arguments = first_view.as_ref().clone();
        if let super::super::parameter_equality::EqualityResultSubject::Required(call) =
            &mut wrong_method_arguments.result_subject
        {
            call.method_args = Arc::from([Ty::Int(ir::IntTy::I32)]);
        } else {
            panic!("expected required result subject");
        }
        assert_eq!(
            wrong_method_arguments
                .validate_for(&facts)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut forged = Arc::try_unwrap(first_view).unwrap();
        forged.projected = second_view.projected.clone();
        assert_eq!(
            forged.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn projected_generic_method_replays_full_substitution() {
        let world = world(
            "struct S;trait Has{type Out;fn get<T:Copy>(self,value:T)->Self::Out;}impl Has for S{type Out=i32;fn get<T:Copy>(self,value:T)->i32{7}}fn main(){}",
        );
        let owner = owners(&world)[0];
        let facts = world.callable_facts(owner.items[0].1).unwrap();
        let mut call = closed_call(owner);
        call.method_args = Arc::from([Ty::Int(ir::IntTy::I32)]);
        let callee = Callee::TraitItem(call);
        let view = facts
            .carrier_contract(SignatureSubject::Call(&callee))
            .unwrap();
        view.validate_for(&facts).unwrap();
        let mut forged = view.as_ref().clone();
        if let super::super::parameter_equality::EqualityResultSubject::Required(call) =
            &mut forged.result_subject
        {
            call.method_args = Arc::from([Ty::Bool]);
        }
        assert_eq!(
            forged.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn projected_required_closed_view_keeps_owner_parameter_carriers() {
        let world = world(
            "struct Wrap<T>{value:T}trait Has{type Out;fn get(self)->Self::Out;}impl<T> Has for Wrap<T>{type Out=T;fn get(self)->Self::Out{self.value}}fn closed(value:Wrap<i32>){}fn main(){}",
        );
        let owner = owners(&world)[0];
        let target = world
            .headers
            .callables
            .iter()
            .find(|c| {
                c.schema.env.total_arity() == 0
                    && c.schema.signature.params.len() == 1
                    && matches!(c.schema.signature.params[0], Ty::Struct(_))
            })
            .unwrap();
        let facts = world.callable_facts(target.declaration).unwrap();
        let call = Callee::TraitItem(TraitCall {
            trait_ref: TraitRef {
                declaration: owner.trait_ref.declaration,
                self_ty: target.schema.signature.params[0].clone(),
                args: Arc::from([]),
            },
            item: owner.items[0].0,
            method_args: Arc::from([]),
        });
        let view = facts
            .carrier_contract(SignatureSubject::Call(&call))
            .unwrap();
        let parameter = owner.schema_env.ordered_params().next().unwrap();
        assert_eq!(view.original_env.ordered_params().next(), Some(parameter));
        assert!(matches!(view.arguments.as_ref(), [Ty::Int(ir::IntTy::I32)]));
        assert!(view.original_contract.results.iter().any(|r| {
            matches!(r.location.channel, ir::CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
                && !r.sources.is_empty()
        }));
        assert!(view.contract.results.is_empty());
        let mut forged = Arc::try_unwrap(view).unwrap();
        forged.original_contract = ir::CarrierContract::default();
        assert_eq!(
            forged.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn projected_owner_reference_result_replays_exact_source_origin() {
        let world = world(
            "struct Wrap<T,U>{value:T,other:U}struct Other<T,U>{value:T,other:U}trait Has{type Out:Copy;fn get(self)->Self::Out;}impl<T:Copy,U> Has for Wrap<T,U>{type Out=T;fn get(self)->T{self.value}}impl<T:Copy,U> Has for Other<T,U>{type Out=T;fn get(self)->T{self.value}}fn closed(value:Wrap<&i32,bool>){}fn main(){}",
        );
        let implementations = owners(&world);
        let owner = implementations[0];
        let target = world
            .headers
            .callables
            .iter()
            .find(|header| {
                header.schema.env.total_arity() == 0
                    && header.schema.signature.params.len() == 1
                    && matches!(header.schema.signature.params[0], Ty::Struct(_))
            })
            .unwrap();
        let facts = world.callable_facts(target.declaration).unwrap();
        let call = TraitCall {
            trait_ref: TraitRef {
                declaration: owner.trait_ref.declaration,
                self_ty: target.schema.signature.params[0].clone(),
                args: Arc::from([]),
            },
            item: owner.items[0].0,
            method_args: Arc::from([]),
        };
        let callee = Callee::TraitItem(call.clone());
        let view = facts
            .carrier_contract(SignatureSubject::Call(&callee))
            .unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(view.fixed.results.len(), 1);
        assert!(view.fixed.input_regions.is_empty());
        assert!(
            view.fixed
                .results
                .iter()
                .all(|row| !matches!(row.returns, ir::ReturnContract::NoReference))
        );
        let selected = world.resolve_trait_call(&call, &facts).unwrap();
        let Callee::Direct(provider) = &selected.callee else {
            panic!("expected exact provided method")
        };
        let direct = world.discharge_direct(provider, &facts).unwrap();
        assert!(
            world
                .eq_signature(&selected.signature, &direct.signature)
                .unwrap()
        );
        assert!(
            world
                .eq_contract(&view.fixed, &direct.signature.ref_contract)
                .unwrap()
        );
        let direct_callee = Callee::Direct(provider.clone());
        facts
            .validate_signature(SignatureSubject::Call(&direct_callee), &direct.signature)
            .unwrap();
        let provider_schema = world.callable_schema(provider.declaration).unwrap();
        let source_schema = world
            .schema
            .declaration_schema(provider.declaration)
            .unwrap();
        assert!(!provider_schema.predicates.is_empty());
        assert!(
            matches!(world.specialize_owner_callable_result(provider, &source_schema,
            &direct.signature, ProofContext::from_facts(&facts)),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        for change in 0..3 {
            let mut forged = clone_schema(&provider_schema, &world.budget).unwrap();
            match change {
                0 => forged.predicates = Arc::from([]),
                1 => {
                    let mut rows = forged.predicates.to_vec();
                    rows.push(rows[0].clone());
                    forged.predicates = rows.into();
                }
                _ => {
                    Arc::make_mut(&mut forged.predicates)[0] = Predicate::Implements(TraitRef {
                        declaration: world.catalog.id(CatalogItem::Copy),
                        self_ty: Ty::Bool,
                        args: Arc::from([]),
                    });
                }
            }
            assert!(
                matches!(world.specialize_owner_callable_result(provider, &forged,
                &direct.signature, ProofContext::from_facts(&facts)),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        }
        let raw = world
            .substitute_projected_schema_with(
                &provider_schema,
                &provider.type_args,
                ProofContext::from_facts(&facts),
                &mut ProofScratch::default(),
            )
            .unwrap();
        assert!(
            !world
                .eq_contract(&raw.signature.ref_contract, &direct.signature.ref_contract)
                .unwrap()
        );
        assert!(
            matches!(facts.validate_signature(SignatureSubject::Call(&direct_callee), &raw.signature),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        let mut wrong_actuals = provider.clone();
        Arc::make_mut(&mut wrong_actuals.type_args).swap(0, 1);
        assert!(
            matches!(world.specialize_owner_callable_result(&wrong_actuals, &provider_schema,
            &direct.signature, ProofContext::from_facts(&facts)),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        assert!(
            view.original_contract
                .results
                .iter()
                .any(|result| !result.sources.is_empty())
        );
        assert!(
            view.contract
                .results
                .iter()
                .any(|result| !result.sources.is_empty())
        );
        let evidence = view.projected.as_ref().unwrap();
        let item = world.item_header(call.item).unwrap();
        for change in 0..5 {
            let mut record = (*evidence.selection.0).clone();
            match change {
                0 => {
                    record.environment = record.environment.wrapping_add(1);
                    assert_ne!(record.environment, evidence.selection.0.environment);
                }
                1 => record.phase = FactsPhase::Universal,
                _ => {
                    let ProofOrigin::Implementation {
                        declaration,
                        arguments,
                    } = &mut record.origin
                    else {
                        unreachable!()
                    };
                    match change {
                        2 => *declaration = implementations[1].declaration,
                        3 => Arc::make_mut(arguments).swap(0, 1),
                        _ => *arguments = Arc::from([]),
                    }
                }
            }
            match world.projected_implementation_base(
                item,
                &call.trait_ref,
                &ProofEvidence(Arc::new(record)),
            ) {
                Err(error) => assert_eq!(
                    error.kind(),
                    FactsErrorKind::InvalidMetadata,
                    "selection mutation {change}"
                ),
                Ok(_) => panic!("accepted forged selection mutation {change}"),
            }
        }
        for change in 0..6 {
            let mut base = world
                .projected_implementation_base(item, &call.trait_ref, &evidence.selection)
                .unwrap();
            match change {
                0..=2 => {
                    let result = Arc::make_mut(&mut base.retained.contract.results)
                        .iter_mut()
                        .find(|result| !result.sources.is_empty())
                        .unwrap();
                    match change {
                        0 => result.sources = Arc::from([]),
                        1 => {
                            let foreign = implementations[1]
                                .schema_env
                                .ordered_params()
                                .next()
                                .unwrap();
                            Arc::make_mut(&mut result.sources)[0].location.channel =
                                ir::CarrierChannel::ParamTail {
                                    parameter: foreign,
                                    prefix: 0,
                                };
                        }
                        _ => {
                            let source = &mut Arc::make_mut(&mut result.sources)[0];
                            assert!(!source.location.path.0.is_empty());
                            source.location.path = ir::CarrierPath(Arc::from([]));
                        }
                    }
                }
                3 => base.retained.contract.inputs = Arc::from([]),
                4 => {
                    Arc::make_mut(&mut base.retained.source_regions).declaration = owner.items[0].1
                }
                _ => Arc::make_mut(&mut base.origin).result_tails = Arc::from([]),
            }
            assert!(
                matches!(world.specialize_projected_owner_base(&base, ProofContext::from_facts(&facts)),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        }
        let mut wrong_item = item.clone();
        wrong_item.declaration = owner.items[0].1;
        assert!(
            matches!(world.projected_implementation_base(&wrong_item, &call.trait_ref,
            &evidence.selection), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        for change in 0..4 {
            let mut forged = view.as_ref().clone();
            match change {
                0 => forged.fixed = Arc::new(ir::RefContract::default()),
                1 => forged.original_contract = ir::CarrierContract::default(),
                2 => Arc::make_mut(&mut forged.source_regions).declaration = owner.items[0].1,
                _ => Arc::make_mut(&mut forged.origin).result_tails = Arc::from([]),
            }
            assert!(
                matches!(forged.validate_for(&facts), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        }
        for ty in [
            Ty::Bool,
            Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: Arc::new(Ty::Int(ir::IntTy::I32)),
            },
        ] {
            let mut signature = evidence.signature.clone();
            signature.result = ty;
            assert!(
                matches!(world.projected_implementation_result_contract(item, &call.trait_ref,
                &evidence.selection, &signature, ProofContext::from_facts(&facts)),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        }
    }

    #[test]
    fn projected_owner_annotation_uses_sticky_planning_budget() {
        let world = paired();
        let owner = owners(&world)[0];
        let facts = world.callable_facts(owner.items[0].1).unwrap();
        let ty = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: owner.trait_ref.clone(),
            item: owner.associated_types[0].associated,
        }));
        world.normalize_ty(&ty, &facts).unwrap();
        assert!(world.budget.charge_work(usize::MAX).is_err());
        for _ in 0..2 {
            assert_eq!(
                world.normalize_ty(&ty, &facts).unwrap_err().kind(),
                FactsErrorKind::Resource
            );
        }
    }

    #[test]
    fn projected_carrier_cannot_drop_required_normalization_dependencies() {
        let world = paired();
        let owner = owners(&world)[0];
        let facts = world.callable_facts(owner.items[0].1).unwrap();
        let call = Callee::TraitItem(closed_call(owner));
        let view = facts
            .carrier_contract(SignatureSubject::Call(&call))
            .unwrap();
        let mut view = Arc::try_unwrap(view).unwrap();
        let evidence = Arc::get_mut(view.projected.as_mut().unwrap()).unwrap();
        assert!(!evidence.dependencies.is_empty());
        evidence.dependencies = Arc::from([]);
        assert_eq!(
            view.validate_for(&facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
    }
}
