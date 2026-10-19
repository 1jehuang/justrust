//! Required projected methods have two deliberately separate authorities:
//! an exact private owning-impl context, and finalized closed normalization.
//! Neither a raw projection nor an owner context is an executable carrier proof.
use super::*;

struct OwnerNormalizationContext<'a> {
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
    signature: Signature,
    source_regions: Arc<ir::SourceRegionEnv>,
    dependencies: Arc<[NormalizedAssociatedType]>,
    selection: ProofEvidence,
}

impl ProjectedMethodEvidence {
    pub(super) fn goal(&self) -> &TraitRef {
        &self.goal
    }
}

pub(super) struct ProjectedApplication {
    pub(super) instantiated: InstantiatedSchema,
    pub(super) evidence: Arc<ProjectedMethodEvidence>,
}

struct ProjectedCarrierBase {
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
        if item.default.is_some() {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: projection-bearing trait default signatures",
            ));
        }
        self.validate_projected_value_schema(&item.schema, item.receiver)
    }

    fn validate_projected_value_schema(
        &self,
        schema: &GenericSchema,
        receiver: ReceiverMode,
    ) -> Result<()> {
        for predicate in schema.predicates.iter() {
            let Predicate::Implements(goal) = predicate else {
                continue;
            };
            if self.type_contains_projection(&goal.self_ty, 0)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: projected method associated bounds",
                ));
            }
            for argument in goal.args.iter() {
                if self.type_contains_projection(argument, 0)? {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: projected method associated bounds",
                    ));
                }
            }
        }
        for (index, ty) in schema.signature.params.iter().enumerate() {
            self.validate_ty_metadata_in(ty, &schema.env, 0)?;
            self.validate_projected_parameter_references(ty, index, receiver)?;
        }
        self.validate_ty_metadata_in(&schema.signature.result, &schema.env, 0)?;
        // `&Self::Target` / `&mut Self::Output` (Deref, Index): one outer
        // reference to a bare projection, borrowed from a borrowed receiver.
        if let Ty::Ref { kind, referent } = &schema.signature.result
            && matches!(referent.as_ref(), Ty::AssociatedType(_))
            && matches!(receiver, ReceiverMode::Borrowed { kind: receiver_kind } if receiver_kind == *kind)
        {
            return Ok(());
        }
        self.reject_projected_references(&schema.signature.result, 0)
    }

    fn validate_projected_parameter_references(
        &self,
        ty: &Ty,
        index: usize,
        receiver: ReceiverMode,
    ) -> Result<()> {
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
    /// After normalizing a projected declaration (`&Self::Target`), derive the
    /// return origins the raw schema deliberately left unresolved.
    fn complete_projected_contract(&self, signature: &mut Signature) -> Result<()> {
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
            && let Some(height) = walk.lookup(key, &self.budget)?
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
                for field in value.def().fields.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
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
            walk.insert(key, height, &self.budget)?;
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
                if !self.same_parameters(&provided.schema.env, &implementation.schema_env)? {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: generic trait method implementation",
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
            for callable in self.headers.callables.iter() {
                charged(&self.budget, self.budget.charge_work(targets.len() + 1))?;
                if self.info(callable.declaration)?.owner == Some(implementation.declaration)
                    && !targets.contains(&callable.declaration)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projected owner has an unmapped implementation method",
                    ));
                }
            }
        }
        Ok(())
    }

    fn owner_context<'a>(
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
            predicates: &owner.predicates,
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
        for argument in arguments {
            self.validate_ty_metadata_in(argument, caller, 0)?;
        }
        if arguments.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "projected schema argument arity changed",
            ));
        }
        let value = self
            .schema
            .instantiate_schema(schema, arguments, caller, &self.budget)?;
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
            self.validate_ty_metadata_in(actual, caller, 0)?;
        }
        for (raw, actual) in schema.predicates.iter().zip(value.predicates.iter()) {
            self.validate_predicate_substitution(raw, actual, &schema.env, arguments)?;
        }
        Ok(value)
    }

    fn normalize_owner_type(
        &self,
        ty: &Ty,
        owner: &OwnerNormalizationContext<'_>,
        active: &mut Vec<DeclId>,
        depth: usize,
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
                    let result = self.normalize_owner_type(
                        &substituted.signature.result,
                        owner,
                        active,
                        depth + 1,
                    );
                    active.pop();
                    let normal = result?;
                    self.validate_associated_normal_form(
                        &normal,
                        &owner.implementation.schema_env,
                        ProofContext {
                            predicates: &owner.predicates,
                            phase: FactsPhase::Universal,
                        },
                        &mut Vec::new(),
                    )?;
                    Ok(normal)
                } else {
                    // No assignment authority for a different application. Only the
                    // ordinary closed selector may independently normalize it.
                    self.normalize_closed_projected_type(
                        ty,
                        ProofContext {
                            predicates: &owner.predicates,
                            phase: FactsPhase::Universal,
                        },
                        !owner.finalized,
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
                        self.normalize_owner_type(part, owner, active, depth + 1)?,
                    )?;
                }
                Ok(Ty::Tuple(normalized.into()))
            }
            Ty::Array { element, len } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Array {
                    element: Arc::new(self.normalize_owner_type(
                        element,
                        owner,
                        active,
                        depth + 1,
                    )?),
                    len: *len,
                })
            }
            Ty::Ref { kind, referent } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(self.normalize_owner_type(
                        referent,
                        owner,
                        active,
                        depth + 1,
                    )?),
                })
            }
            Ty::Slice { element } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Slice {
                    element: Arc::new(self.normalize_owner_type(
                        element,
                        owner,
                        active,
                        depth + 1,
                    )?),
                })
            }
            Ty::Struct(_) | Ty::Enum(_) => self.normalize_nominal_actuals(
                ty,
                &owner.implementation.schema_env,
                depth,
                |argument| self.normalize_owner_type(argument, owner, active, depth + 1),
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
                let normal = self.normalize_owner_type(&projection, &owner, &mut Vec::new(), 0)?;
                // The owner context contains only actual impl constraints, never
                // its own head or an equality manufactured from this assignment.
                self.require_associated_assignment_bounds(
                    value.associated,
                    &normal,
                    ProofContext {
                        predicates: &owner.predicates,
                        phase: FactsPhase::Universal,
                    },
                    &mut Vec::new(),
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
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: generic projected implementation method",
                ));
            }
            self.validate_projected_value_schema(schema, self.info(declaration)?.receiver)?;
            let mut normalized = clone_schema(schema, &self.budget)?;
            for ty in &mut normalized.signature.params {
                *ty = self.normalize_owner_type(ty, &owner, &mut Vec::new(), 0)?;
            }
            normalized.signature.result =
                self.normalize_owner_type(&schema.signature.result, &owner, &mut Vec::new(), 0)?;
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
                .normalize_owner_type(&ty, &owner, &mut Vec::new(), 0)
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
            if !self.same_parameters(&raw.env, &implementation.schema_env)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "owning callable instantiation changed implementation binders",
                ));
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
            let head = self.substitute_projected_schema(&head, &target.type_args, context.env())?;
            let goal = TraitRef {
                declaration: implementation.trait_ref.declaration,
                self_ty: head.signature.result,
                args: head.signature.params.into(),
            };
            self.validate_ref_in(&goal, context.env(), None)?;
            let closed = self.goal_is_closed(&goal, context.env())?;
            if closed {
                let selection = self.require_in(
                    &Predicate::Implements(goal),
                    ProofContext {
                        predicates: &self.empty,
                        phase: FactsPhase::Closed,
                    },
                    &mut Vec::new(),
                )?;
                if !matches!(&selection.0.origin,
                    ProofOrigin::Implementation { declaration, arguments }
                        if *declaration == owner_id
                            && self.eq_types(arguments, &target.type_args, 0)?)
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
                    &target.type_args,
                    context,
                    closed,
                )?;
                self.validate_associated_normal_form(
                    &normal,
                    context.env(),
                    context,
                    &mut Vec::new(),
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
                    &target.type_args,
                    context,
                    closed,
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

    fn instantiate_owner_projected_type(
        &self,
        ty: &Ty,
        owner: &OwnerNormalizationContext<'_>,
        arguments: &[Ty],
        context: ProofContext<'_>,
        closed: bool,
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
            env: owner.implementation.schema_env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: original,
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let value = self.substitute_projected_schema(&schema, arguments, context.env())?;
        if closed {
            self.normalize_closed_projected_type(&value.signature.result, context, false)
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
        for ty in &mut value.signature.params {
            *ty = self.normalize_owner_type(ty, &owner, &mut Vec::new(), 0)?;
        }
        value.signature.result =
            self.normalize_owner_type(&value.signature.result, &owner, &mut Vec::new(), 0)?;
        self.complete_projected_contract(&mut value.signature)?;
        // Independently establish the required declaration's contract in the real
        // implementation environment. Never copy the provided target's topology.
        self.validate_schema_signature(
            item.declaration,
            &value.signature,
            &implementation.schema_env,
            None,
        )?;
        let source = self.validate_source_regions(item.declaration, &value.signature)?;
        charged(
            &self.budget,
            ir::derive_signature_origin_schema(
                &value.signature,
                &implementation.schema_env,
                &self.budget,
            ),
        )?;
        self.derive_carriers(&value.signature, &source)?;
        Ok(value)
    }

    pub(super) fn validate_projected_carrier_refinement(
        &self,
        item: &TraitItemHeader,
        required: &Signature,
        provided: &CallableHeader,
    ) -> Result<()> {
        let source = self.validate_source_regions(item.declaration, required)?;
        let (required, _) = self.derive_carriers(required, &source)?;
        // The provided declaration was independently normalized and published
        // by build_origin_schemas. It cannot supply the required topology.
        let provided = self.declaration_carriers(provided.declaration)?;
        if required != provided.contract {
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
        context: ProofContext<'_>,
    ) -> Result<ProjectedApplication> {
        self.check_normalization_budget()?;
        self.check_context(context)?;
        self.validate_projected_required_schema(item)?;
        self.validate_ref_in(goal, context.env(), None)?;
        if goal.declaration != item.trait_decl {
            return Err(FactsError::invalid(
                &self.budget,
                "projected required application changed item owner",
            ));
        }
        if !self.goal_is_closed(goal, context.env())? {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: rigid caller projected method application",
            ));
        }
        let selection = self.require_in(
            &Predicate::Implements(goal.clone()),
            ProofContext {
                predicates: &self.empty,
                phase: FactsPhase::Closed,
            },
            &mut Vec::new(),
        )?;
        if !matches!(&selection.0.origin, ProofOrigin::Implementation { .. }) {
            return Err(FactsError::invalid(
                &self.budget,
                "projected method requires an exact implementation selection",
            ));
        }
        let arguments = self.trait_arguments(goal)?;
        let mut instantiated =
            self.substitute_projected_schema(&item.schema, &arguments, context.env())?;
        let mut dependencies = Vec::new();
        for ty in &mut instantiated.signature.params {
            let (normal, proofs) = self.normalize_closed_projected_type(ty, context, false)?;
            *ty = normal;
            for proof in proofs {
                push(&self.budget, &mut dependencies, proof)?;
            }
        }
        let (normal, proofs) =
            self.normalize_closed_projected_type(&instantiated.signature.result, context, false)?;
        instantiated.signature.result = normal;
        for proof in proofs {
            push(&self.budget, &mut dependencies, proof)?;
        }
        self.complete_projected_contract(&mut instantiated.signature)?;
        self.validate_schema_signature(
            item.declaration,
            &instantiated.signature,
            context.env(),
            None,
        )?;
        for (index, ty) in instantiated.signature.params.iter().enumerate() {
            self.validate_projected_parameter_references(ty, index, item.receiver)?;
        }
        if !matches!(&item.schema.signature.result, Ty::Ref { referent, .. } if matches!(referent.as_ref(), Ty::AssociatedType(_)))
        {
            self.reject_projected_references(&instantiated.signature.result, 0)?;
        }
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
            signature: instantiated.signature.clone(),
            source_regions,
            dependencies: dependencies.into(),
            selection,
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

    fn projected_carrier_base(
        &self,
        evidence: &ProjectedMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<ProjectedCarrierBase> {
        if evidence.selection.0.world != self.id
            || evidence.selection.0.environment != self.empty.index
            || evidence.selection.0.phase != FactsPhase::Closed
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
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.selection.0.origin
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "projected carrier lost implementation selection",
            ));
        };
        let mut implementation = None;
        for candidate in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if candidate.declaration == *declaration {
                implementation = Some(candidate);
                break;
            }
        }
        let implementation = implementation.ok_or_else(|| {
            FactsError::invalid(&self.budget, "projected carrier implementation disappeared")
        })?;
        let owner = self.predicate_env(*declaration)?;
        let required = self.instantiate_owner_item(
            self.item_header(evidence.item)?,
            implementation,
            ProofContext {
                predicates: &owner,
                phase: FactsPhase::Universal,
            },
        )?;
        let schema = GenericSchema {
            env: implementation.schema_env.clone(),
            signature: required.signature,
            predicates: required.predicates,
        };
        let instantiated = self.instantiate_in(
            &schema,
            arguments,
            ProofContext::from_facts(facts),
            Some(evidence.item),
        )?;
        if !self.eq_signature(&instantiated.signature, &evidence.signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "closed projected signature disagrees with required owner transport",
            ));
        }
        let source_regions = self.validate_source_regions(evidence.item, &schema.signature)?;
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
        )?;
        let (contract, regions) = self.derive_carriers(&schema.signature, &source_regions)?;
        Ok(ProjectedCarrierBase {
            schema,
            arguments: arguments.clone(),
            origin,
            retained: DeclarationCarriers {
                declaration: evidence.item,
                contract,
                regions,
                source_regions,
            },
        })
    }

    pub(super) fn validate_projected_carrier(
        &self,
        view: &CarrierContractView,
        evidence: &ProjectedMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<()> {
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
        let application = self.instantiate_projected_item(
            self.item_header(evidence.item)?,
            &evidence.goal,
            ProofContext::from_facts(facts),
        )?;
        match (
            &evidence.selection.0.origin,
            &application.evidence.selection.0.origin,
        ) {
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
            self.validate_projected_dependency(dependency, ProofContext::from_facts(facts))?;
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
        let expected = self.transport_carrier_parts(
            evidence.item,
            base.arguments,
            evidence.signature.clone(),
            facts,
            base.schema,
            &base.retained,
            base.origin,
            None,
            view.result_subject.clone(),
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

    fn paired() -> Arc<ValidatedTraitWorld> {
        world(
            "struct S;trait Has<A,B>{type Out;fn get(self)->Self::Out;}impl Has<i32,bool> for S{type Out=i128;fn get(self)->Self::Out{7i128}}impl Has<bool,i32> for S{type Out=bool;fn get(self)->Self::Out{true}}fn main(){}",
        )
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
                .all(|(declaration, _)| *declaration != item)
        );
        assert!(world.carriers.iter().all(|row| row.declaration != item));
        assert!(world.declaration_carriers(item).is_err());
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
        assert_eq!(
            world
                .normalize_associated_type(&projection, &facts)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Unsupported
        );
        let carrier = facts.carrier_contract(SignatureSubject::Body).unwrap();
        assert!(carrier.original_contract().results.iter().any(|result| {
            matches!(result.location.channel, ir::CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
                && !result.sources.is_empty()
        }));
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
            let proof = world.discharge_direct(&target, &facts).unwrap();
            assert!(world.eq_ty(&proof.signature.result, &actual, 0).unwrap());
        }
        for actual in [
            shared(Ty::Int(ir::IntTy::I32)),
            shared(shared(Ty::Int(ir::IntTy::I32))),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([actual]),
            };
            // The old path accepted these ordinary executable reference shapes
            // after replacing Self::Out with T. Only the owner guard rejects them.
            world
                .instantiate_in(
                    &world.callable_schema(declaration).unwrap(),
                    &target.type_args,
                    ProofContext {
                        predicates: &world.empty,
                        phase: FactsPhase::Closed,
                    },
                    Some(declaration),
                )
                .unwrap();
            assert!(matches!(world.instance_facts(&target),
                Err(error) if error.kind() == FactsErrorKind::Unsupported));
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
                        predicates: &world.empty,
                        phase: FactsPhase::Closed,
                    },
                    Some(declaration),
                )
                .unwrap();
            assert!(matches!(world.instance_facts(&target),
                Err(error) if error.kind() == FactsErrorKind::Unsupported));
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
        let mut forged = Arc::try_unwrap(first_view).unwrap();
        forged.projected = second_view.projected.clone();
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
