//! Closed associated-type normalization. Raw source terms are never carrier facts.
use super::*;

impl ValidatedFacts {
    /// Check the identity and raw type metadata of a call without executing,
    /// selecting an implementation, or admitting residual projections.
    pub fn validate_callee_metadata(&self, callee: &Callee) -> Result<()> {
        self.world.check_facts(self, false)?;
        match callee {
            Callee::Direct(target) => {
                for argument in target.type_args.iter() {
                    self.world
                        .validate_ty_metadata_in(argument, self.env(), 0)?;
                }
                let info = self.world.info(target.declaration)?;
                let header = self.world.callable_header(target.declaration)?;
                if !matches!(info.kind, DeclKind::Function | DeclKind::AssociatedFunction)
                    || target.type_args.len() != header.schema.env.total_arity()
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "direct call metadata changed declaration kind or binder arity",
                    ));
                }
            }
            Callee::TraitItem(call) => {
                self.world
                    .validate_ty_metadata_in(&call.trait_ref.self_ty, self.env(), 0)?;
                for argument in call.trait_ref.args.iter().chain(call.method_args.iter()) {
                    self.world
                        .validate_ty_metadata_in(argument, self.env(), 0)?;
                }
                let owner = self.world.trait_header(call.trait_ref.declaration)?;
                let item = self.world.item_header(call.item)?;
                let info = self.world.info(call.item)?;
                if info.kind != DeclKind::TraitItem
                    || info.owner != Some(owner.declaration)
                    || item.trait_decl != owner.declaration
                    || call.trait_ref.args.len().checked_add(1) != Some(owner.env.total_arity())
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "trait call metadata changed exact item/trait owner or argument arity",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct NormalizationKey {
    environment: usize,
    phase: FactsPhase,
    staged: bool,
    finalized: bool,
    projection: ir::AssociatedTypeProjection,
}

#[derive(Clone, Debug)]
struct NormalizationEvidence {
    world: WorldId,
    key: NormalizationKey,
    membership: ProofEvidence,
    selected_goal: TraitRef,
    implementation: DeclId,
    arguments: Arc<[Ty]>,
    raw_rhs: Ty,
    normal: Ty,
    dependencies: Arc<[Arc<NormalizationEvidence>]>,
}

/// A normalized value backed by exact, private implementation-assignment evidence.
#[derive(Clone, Debug)]
pub struct NormalizedAssociatedType(Arc<NormalizationEvidence>);
impl NormalizedAssociatedType {
    pub fn ty(&self) -> &Ty {
        &self.0.normal
    }
    pub fn validate_for(
        &self,
        projection: &ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let world = facts.world();
        world.check_normalization_budget()?;
        world.check_facts(facts, false)?;
        let context = NormalizationContext {
            proof: ProofContext::from_facts(facts),
            staged: false,
        };
        let key = NormalizationKey {
            environment: facts.predicates.index,
            phase: facts.phase(),
            staged: false,
            finalized: world.owner_normalization_finalized,
            projection: projection.clone(),
        };
        if !world.normalization_key_equal(&self.0.key, &key)? {
            return Err(FactsError::invalid(
                facts.budget(),
                "normalization proof replay changed exact projection or facts context",
            ));
        }
        world.validate_normalization_evidence(&self.0, context)
    }
}

#[derive(Clone, Debug)]
pub(super) struct NormalizationMemo {
    key: NormalizationKey,
    result: Result<Arc<NormalizationEvidence>>,
}

#[derive(Clone, Copy)]
struct NormalizationContext<'a> {
    proof: ProofContext<'a>,
    staged: bool,
}

#[derive(Default)]
struct NormalizationStack {
    active: Vec<(NormalizationKey, DeclId)>,
    obligations: Vec<GoalKey>,
}

impl ValidatedTraitWorld {
    pub(super) fn check_normalization_budget(&self) -> Result<()> {
        if self.budget.has_resource_failure() {
            return Err(FactsError::resource(
                &self.budget,
                "associated normalization planning ledger is poisoned",
            ));
        }
        Ok(())
    }
    pub(super) fn validate_header_type_metadata(&self) -> Result<()> {
        for header in self.headers.traits.iter() {
            self.info(header.declaration)?;
            for goal in header.supertraits.iter() {
                self.validate_ty_metadata_in(&goal.self_ty, &header.env, 0)?;
                for argument in goal.args.iter() {
                    self.validate_ty_metadata_in(argument, &header.env, 0)?;
                }
            }
        }
        for header in self.headers.associated_types.iter() {
            self.info(header.declaration)?;
            let declared = self.schema.associated_type_bounds(header.declaration)?;
            if header.bounds.len() > MAX_PREDICATES || declared.len() > MAX_PREDICATES {
                return Err(FactsError::resource(
                    &self.budget,
                    "associated declaration bound limit exceeded",
                ));
            }
            // Admit callback-only identities before signature capability checks.
            // Ordered agreement remains in validate_associated_headers.
            for bound in header.bounds.iter().chain(declared.iter()) {
                charged(&self.budget, self.budget.charge_work(1))?;
                if self.info(*bound)?.kind != DeclKind::Trait {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated bound does not name a trait",
                    ));
                }
            }
        }
        for header in self
            .headers
            .items
            .iter()
            .map(|item| (item.declaration, &item.schema))
            .chain(
                self.headers
                    .callables
                    .iter()
                    .map(|item| (item.declaration, &item.schema)),
            )
        {
            self.info(header.0)?;
            for ty in header
                .1
                .signature
                .params
                .iter()
                .chain(std::iter::once(&header.1.signature.result))
            {
                self.validate_ty_metadata_in(ty, &header.1.env, 0)?;
            }
            for predicate in header.1.predicates.iter() {
                self.validate_predicate_metadata(predicate, &header.1.env)?;
            }
        }
        for header in self.headers.impls.iter() {
            self.info(header.declaration)?;
            self.validate_ty_metadata_in(&header.trait_ref.self_ty, &header.schema_env, 0)?;
            for argument in header.trait_ref.args.iter() {
                self.validate_ty_metadata_in(argument, &header.schema_env, 0)?;
            }
            for predicate in header.predicates.iter() {
                self.validate_predicate_metadata(predicate, &header.schema_env)?;
            }
            for value in header.associated_types.iter() {
                self.validate_ty_metadata_in(&value.rhs, &header.schema_env, 0)?;
            }
        }
        Ok(())
    }

    /// Formation/identity only. In particular this never grants a projection a
    /// layout, Copy, Sized or reference-origin contract.
    pub(super) fn validate_ty_metadata_in(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
    ) -> Result<()> {
        self.validate_ty_metadata_walk(ty, env, depth, &mut TypeWalk::default())
            .map(|_| ())
    }

    fn validate_ty_metadata_walk(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        walk: &mut TypeWalk,
    ) -> Result<usize> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        let key = TypeWalk::key(ty).map(|node| (node, TypeUse::Root));
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
        let mut child = |value: &Ty| -> Result<()> {
            height = height.max(
                self.validate_ty_metadata_walk(value, env, depth + 1, walk)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Param(parameter) => {
                if !env.contains(*parameter) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "type parameter outside authoritative metadata environment",
                    ));
                }
                self.info(parameter.owner.declaration)?;
            }
            Ty::AssociatedType(projection) => {
                child(&projection.trait_ref.self_ty)?;
                for argument in projection.trait_ref.args.iter() {
                    child(argument)?;
                }
                let owner = self.trait_header(projection.trait_ref.declaration)?;
                let item = self.associated_header(projection.item)?;
                let info = self.info(projection.item)?;
                if item.trait_decl != owner.declaration
                    || info.kind != DeclKind::AssociatedType
                    || info.owner != Some(owner.declaration)
                    || projection.trait_ref.args.len().checked_add(1)
                        != Some(owner.env.total_arity())
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projection metadata changed trait/item ownership or arity",
                    ));
                }
            }
            Ty::Tuple(parts) => {
                if parts.is_empty() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "empty tuple metadata must be Unit",
                    ));
                }
                for part in parts.iter() {
                    child(part)?;
                }
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array metadata length limit exceeded",
                    ));
                }
                child(element)?;
            }
            Ty::Ref { referent, .. } | Ty::Slice { element: referent } => {
                child(referent)?;
            }
            Ty::Struct(value) => {
                if value.def().id.snapshot != self.registry
                    || value.def().source_decl.snapshot != self.registry
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "foreign nominal metadata identity",
                    ));
                }
                validate_nominal_modules(ty, self.catalog.modules(), &self.budget)?;
                self.schema.validate_nominal(ty, env, &self.budget)?;
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for field in value.def().fields.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                if value.def().id.snapshot != self.registry
                    || value.def().source_decl.snapshot != self.registry
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "foreign nominal metadata identity",
                    ));
                }
                validate_nominal_modules(ty, self.catalog.modules(), &self.budget)?;
                self.schema.validate_nominal(ty, env, &self.budget)?;
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Unit | Ty::Never => {}
        }
        if let Some(key) = key {
            walk.insert(key, height, &self.budget)?;
        }
        Ok(height)
    }

    pub fn normalized_callable_schema(&self, declaration: DeclId) -> Result<GenericSchema> {
        clone_schema(&self.callable_header(declaration)?.schema, &self.budget)
    }

    pub(super) fn normalized_declaration_schema(
        &self,
        declaration: DeclId,
    ) -> Result<GenericSchema> {
        for callable in self.normalized_callables.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if callable.declaration == declaration {
                return clone_schema(&callable.schema, &self.budget);
            }
        }
        self.schema.declaration_schema(declaration)
    }

    pub fn normalize_associated_type(
        &self,
        projection: &ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<NormalizedAssociatedType> {
        self.check_normalization_budget()?;
        self.check_facts(facts, false)?;
        let context = NormalizationContext {
            proof: ProofContext::from_facts(facts),
            staged: false,
        };
        let evidence =
            self.normalize_projection_in(projection, context, &mut NormalizationStack::default())?;
        self.validate_normalization_evidence(&evidence, context)?;
        Ok(NormalizedAssociatedType(evidence))
    }

    /// Normalize raw local annotations after the caller releases source-state locks.
    pub fn normalize_ty(&self, ty: &Ty, facts: &ValidatedFacts) -> Result<Ty> {
        self.check_normalization_budget()?;
        self.check_facts(facts, false)?;
        self.validate_ty_metadata_in(ty, facts.env(), 0)?;
        if facts
            .predicates
            .predicates
            .iter()
            .any(|p| matches!(p, Predicate::ProjectionEq { .. }))
        {
            let normal = self.normalize_equality_type(ty, facts)?;
            self.validate_ty(&normal, facts.env(), None, true, 0)?;
            return Ok(normal);
        }
        if self.type_contains_projection(ty, 0)?
            && let Some(normal) = self.normalize_owner_annotation(ty, facts)?
        {
            return Ok(normal);
        }
        let context = NormalizationContext {
            proof: ProofContext::from_facts(facts),
            staged: false,
        };
        let normal = self.normalize_type_in(
            ty,
            context,
            &mut NormalizationStack::default(),
            &mut Vec::new(),
            0,
        )?;
        self.validate_ty(&normal, facts.env(), None, true, 0)?;
        Ok(normal)
    }

    pub(super) fn normalize_closed_projected_type(
        &self,
        ty: &Ty,
        proof: ProofContext<'_>,
        staged: bool,
    ) -> Result<(Ty, Vec<NormalizedAssociatedType>)> {
        self.check_normalization_budget()?;
        self.check_context(proof)?;
        let context = NormalizationContext { proof, staged };
        let mut dependencies = Vec::new();
        let normal = self.normalize_type_in(
            ty,
            context,
            &mut NormalizationStack::default(),
            &mut dependencies,
            0,
        )?;
        let mut retained = Vec::new();
        for dependency in dependencies {
            self.validate_normalization_evidence(&dependency, context)?;
            push(
                &self.budget,
                &mut retained,
                NormalizedAssociatedType(dependency),
            )?;
        }
        Ok((normal, retained))
    }

    pub(super) fn normalize_closed_equality_projection(
        &self,
        projection: &ir::AssociatedTypeProjection,
        proof: ProofContext<'_>,
        active: &[GoalKey],
    ) -> Result<NormalizedAssociatedType> {
        let mut stack = NormalizationStack::default();
        for goal in active {
            push(&self.budget, &mut stack.obligations, goal.clone())?;
        }
        let context = NormalizationContext {
            proof,
            staged: false,
        };
        let evidence = self.normalize_projection_in(projection, context, &mut stack)?;
        self.validate_normalization_evidence(&evidence, context)?;
        Ok(NormalizedAssociatedType(evidence))
    }

    pub(super) fn validate_projected_dependency(
        &self,
        evidence: &NormalizedAssociatedType,
        proof: ProofContext<'_>,
    ) -> Result<()> {
        self.check_context(proof)?;
        self.validate_normalization_evidence(
            &evidence.0,
            NormalizationContext {
                proof,
                staged: false,
            },
        )
    }

    pub(super) fn same_projected_dependency(
        &self,
        left: &NormalizedAssociatedType,
        right: &NormalizedAssociatedType,
    ) -> Result<bool> {
        self.normalization_key_equal(&left.0.key, &right.0.key)
    }

    pub(super) fn associated_header(&self, declaration: DeclId) -> Result<&AssociatedTypeHeader> {
        for header in self.headers.associated_types.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if header.declaration == declaration {
                return Ok(header);
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "unknown associated type declaration",
        ))
    }

    pub(super) fn associated_value(
        &self,
        implementation: DeclId,
        associated: DeclId,
    ) -> Result<&AssociatedTypeValue> {
        for header in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if header.declaration == implementation {
                for value in header.associated_types.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if value.associated == associated && value.implementation == implementation {
                        return Ok(value);
                    }
                }
                break;
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "missing associated type assignment",
        ))
    }

    pub(super) fn schema_contains_projection(&self, schema: &GenericSchema) -> Result<bool> {
        for ty in schema
            .signature
            .params
            .iter()
            .chain(std::iter::once(&schema.signature.result))
        {
            if self.type_contains_projection(ty, 0)? {
                return Ok(true);
            }
        }
        for predicate in schema.predicates.iter() {
            // Equality rows are retained raw requirements, not signature occurrences.
            if let Predicate::Implements(_) = predicate {
                for argument in predicate.metadata_types() {
                    if self.type_contains_projection(argument, 0)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    pub(super) fn type_contains_projection(&self, ty: &Ty, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(
            &self.budget,
            ir::contains_associated_types(ty, &self.budget),
        )
    }

    /// Validate identities and binder scopes without treating a raw projection as a carrier.
    fn validate_raw_associated_ty(&self, ty: &Ty, env: &GenericEnv, depth: usize) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        match ty {
            Ty::AssociatedType(projection) => {
                let item = self.associated_header(projection.item)?;
                if item.trait_decl != projection.trait_ref.declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated projection item belongs to another trait",
                    ));
                }
                self.validate_ty_metadata_in(ty, env, depth)?;
                self.validate_raw_associated_ty(&projection.trait_ref.self_ty, env, depth + 1)?;
                for argument in projection.trait_ref.args.iter() {
                    if self.type_contains_projection(argument, depth + 1)? {
                        return Err(FactsError::unsupported(
                            &self.budget,
                            "unsupported: projection-bearing trait actual",
                        ));
                    }
                    self.validate_ty(argument, env, None, true, depth + 1)?;
                }
                if !self.raw_associated_ty_is_closed(ty, depth + 1)? {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: rigid generic associated projection",
                    ));
                }
            }
            Ty::Tuple(parts) => {
                if parts.is_empty() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "empty raw tuple must be Unit",
                    ));
                }
                for part in parts.iter() {
                    self.validate_raw_associated_ty(part, env, depth + 1)?;
                }
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array length limit exceeded",
                    ));
                }
                self.validate_raw_associated_ty(element, env, depth + 1)?;
            }
            Ty::Ref { referent, .. } | Ty::Slice { element: referent } => {
                self.validate_raw_associated_ty(referent, env, depth + 1)?;
            }
            _ => self.validate_ty(ty, env, None, true, depth)?,
        }
        Ok(())
    }

    pub(super) fn raw_associated_ty_is_closed(&self, ty: &Ty, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Param(_) => return Ok(false),
            Ty::Struct(value) => {
                for field in value.def().fields.iter() {
                    if !self.raw_associated_ty_is_closed(&field.ty, depth + 1)? {
                        return Ok(false);
                    }
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
                        if !self.raw_associated_ty_is_closed(&field.ty, depth + 1)? {
                            return Ok(false);
                        }
                    }
                }
            }
            Ty::AssociatedType(projection) => {
                if !self.raw_associated_ty_is_closed(&projection.trait_ref.self_ty, depth + 1)? {
                    return Ok(false);
                }
                for argument in projection.trait_ref.args.iter() {
                    if !self.raw_associated_ty_is_closed(argument, depth + 1)? {
                        return Ok(false);
                    }
                }
            }
            Ty::Ref { referent, .. }
            | Ty::Array {
                element: referent, ..
            }
            | Ty::Slice { element: referent } => {
                return self.raw_associated_ty_is_closed(referent, depth + 1);
            }
            _ => {}
        }
        for child in type_arguments(ty) {
            if !self.raw_associated_ty_is_closed(child, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn validate_associated_headers(&mut self) -> Result<()> {
        for header in self.headers.associated_types.iter() {
            self.has_associated_bounds |= !header.bounds.is_empty();
            let info = self.info(header.declaration)?;
            let owner = self.trait_header(header.trait_decl)?;
            if info.kind != DeclKind::AssociatedType
                || info.owner != Some(header.trait_decl)
                || info.receiver != ReceiverMode::None
                || !self.same_env(&info.env, &header.env)?
                || !self.same_env(&header.env, &owner.env)?
                || info.own_env.arity() != 0
                || !owner.associated_types.contains(&header.declaration)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated type header has foreign owner or environment",
                ));
            }
            let declared = self.schema.associated_type_bounds(header.declaration)?;
            if header.bounds.len() > MAX_PREDICATES || declared.len() > MAX_PREDICATES {
                return Err(FactsError::resource(
                    &self.budget,
                    "associated declaration bound limit exceeded",
                ));
            }
            // Inspect every callback row before comparing or rejecting capabilities.
            for bound in declared.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if self.info(*bound)?.kind != DeclKind::Trait {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated source bound does not name a trait",
                    ));
                }
            }
            charged(&self.budget, self.budget.charge_work(declared.len()))?;
            if header.bounds.as_ref() != declared.as_ref() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated bounds disagree with immutable source metadata",
                ));
            }
        }
        for owner in self.headers.traits.iter() {
            let mut seen = Vec::new();
            if self.catalog.declaration(owner.declaration).is_some()
                && !owner.associated_types.is_empty()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "canonical trait associated types were replaced",
                ));
            }
            for item in owner.associated_types.iter() {
                charged(&self.budget, self.budget.charge_work(seen.len() + 1))?;
                if seen.contains(item)
                    || self.associated_header(*item)?.trait_decl != owner.declaration
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "duplicate or foreign associated type member",
                    ));
                }
                push(&self.budget, &mut seen, *item)?;
            }
        }
        // This stratum does not validate any method signature or use callable facts.
        for implementation in self.headers.impls.iter() {
            let info = self.info(implementation.declaration)?;
            if info.kind != DeclKind::TraitImpl
                || !self.same_env(&info.env, &implementation.schema_env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated assignment implementation environment mismatch",
                ));
            }
            if !matches!(
                implementation.trait_ref.self_ty,
                Ty::Struct(_) | Ty::Enum(_)
            ) {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: nonnominal trait implementation head",
                ));
            }
            self.validate_ref_in(&implementation.trait_ref, &implementation.schema_env, None)?;
            self.validate_predicates(&implementation.predicates, &implementation.schema_env, None)?;
            for parameter in implementation.schema_env.ordered_params() {
                let mut constrained =
                    self.contains_param(&implementation.trait_ref.self_ty, parameter, 0)?;
                for argument in implementation.trait_ref.args.iter() {
                    constrained |= self.contains_param(argument, parameter, 0)?;
                }
                if !constrained {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "unconstrained trait impl parameter",
                    ));
                }
            }
            let owner = self.trait_header(implementation.trait_ref.declaration)?;
            let mut seen = Vec::new();
            for value in implementation.associated_types.iter() {
                charged(
                    &self.budget,
                    self.budget
                        .charge_work(seen.len() + owner.associated_types.len() + 1),
                )?;
                if value.implementation != implementation.declaration
                    || seen.contains(&value.associated)
                    || !owner.associated_types.contains(&value.associated)
                    || self.associated_header(value.associated)?.trait_decl != owner.declaration
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "duplicate or foreign associated type assignment pair",
                    ));
                }
                let authoritative = self
                    .schema
                    .associated_type_rhs(value.implementation, value.associated)?;
                if !self.eq_ty(&authoritative, &value.rhs, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated type RHS disagrees with immutable source metadata",
                    ));
                }
                // Formation is checked here. Exact owner reduction runs only
                // after every assignment map and the coherence graph is checked.
                self.validate_ty_metadata_in(&value.rhs, &implementation.schema_env, 0)?;
                push(&self.budget, &mut seen, value.associated)?;
            }
            for item in owner.associated_types.iter() {
                charged(&self.budget, self.budget.charge_work(seen.len() + 1))?;
                if !seen.contains(item) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "missing required associated type assignment",
                    ));
                }
            }
        }
        // All headers, callback identities and assignment metadata precede the
        // capability boundary, including a bad later member after a user trait.
        for header in self.headers.associated_types.iter() {
            for bound in header.bounds.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if *bound != self.catalog.id(CatalogItem::Copy)
                    && *bound != self.catalog.id(CatalogItem::Sized)
                {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: associated bounds require canonical Copy or Sized",
                    ));
                }
                if self.trait_header(*bound)?.env.total_arity() != 1 {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "canonical associated bound has unexpected arguments",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn require_associated_assignment_bounds(
        &self,
        associated: DeclId,
        normal: &Ty,
        context: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<()> {
        if !self.has_associated_bounds {
            return Ok(());
        }
        let header = self.associated_header(associated)?;
        if header.bounds.is_empty() {
            return Ok(());
        }
        let declared = self.schema.associated_type_bounds(associated)?;
        charged(&self.budget, self.budget.charge_work(declared.len()))?;
        if declared.as_ref() != header.bounds.as_ref() {
            return Err(FactsError::invalid(
                &self.budget,
                "associated bounds changed after header validation",
            ));
        }
        for bound in header.bounds.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            self.require_in(
                &Predicate::Implements(TraitRef {
                    declaration: *bound,
                    self_ty: normal.clone(),
                    args: Arc::from([]),
                }),
                context,
                active,
            )?;
        }
        Ok(())
    }

    pub(super) fn stage_associated_types(&mut self) -> Result<()> {
        // Only build() can reach this after complete head/coherence/assignment checks.
        // The stage is separate from public facts and from public projection memo keys.
        let context = NormalizationContext {
            proof: ProofContext {
                predicates: &self.empty,
                phase: FactsPhase::Schema,
            },
            staged: true,
        };
        self.stage_owner_assignments()?;
        let mut normalized = Vec::new();
        for callable in self.headers.callables.iter() {
            let mut schema = clone_schema(&callable.schema, &self.budget)?;
            if self.schema_contains_projection(&schema)? {
                if self.has_declared_callable_equalities(callable.declaration)? {
                    schema =
                        self.normalize_declared_callable_schema(callable.declaration, &schema)?;
                    push(
                        &self.budget,
                        &mut normalized,
                        CallableHeader {
                            declaration: callable.declaration,
                            schema,
                        },
                    )?;
                    continue;
                }
                if let Some(owned) =
                    self.normalize_owner_callable_schema(callable.declaration, &schema)?
                {
                    push(
                        &self.budget,
                        &mut normalized,
                        CallableHeader {
                            declaration: callable.declaration,
                            schema: owned,
                        },
                    )?;
                    continue;
                }
                for ty in schema
                    .signature
                    .params
                    .iter()
                    .chain(std::iter::once(&schema.signature.result))
                {
                    self.validate_raw_associated_ty(ty, &schema.env, 0)?;
                }
                let mut stack = NormalizationStack::default();
                let mut dependencies = Vec::new();
                let mut params = Vec::new();
                for ty in &schema.signature.params {
                    push(
                        &self.budget,
                        &mut params,
                        self.normalize_type_in(ty, context, &mut stack, &mut dependencies, 0)?,
                    )?;
                }
                schema.signature.params = params;
                schema.signature.result = self.normalize_type_in(
                    &schema.signature.result,
                    context,
                    &mut stack,
                    &mut dependencies,
                    0,
                )?;
                self.validate_schema_signature(
                    callable.declaration,
                    &schema.signature,
                    &schema.env,
                    None,
                )?;
            }
            push(
                &self.budget,
                &mut normalized,
                CallableHeader {
                    declaration: callable.declaration,
                    schema,
                },
            )?;
        }
        self.normalized_callables = normalized;
        Ok(())
    }

    fn normalize_type_in(
        &self,
        ty: &Ty,
        context: NormalizationContext<'_>,
        stack: &mut NormalizationStack,
        dependencies: &mut Vec<Arc<NormalizationEvidence>>,
        depth: usize,
    ) -> Result<Ty> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        match ty {
            Ty::AssociatedType(projection) => {
                let evidence = self.normalize_projection_in(projection, context, stack)?;
                let normal = evidence.normal.clone();
                push(&self.budget, dependencies, evidence)?;
                Ok(normal)
            }
            Ty::Tuple(parts) => {
                let mut normal = Vec::new();
                for part in parts.iter() {
                    let part =
                        self.normalize_type_in(part, context, stack, dependencies, depth + 1)?;
                    push(&self.budget, &mut normal, part)?;
                }
                Ok(Ty::Tuple(normal.into()))
            }
            Ty::Array { element, len } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Array {
                    element: Arc::new(self.normalize_type_in(
                        element,
                        context,
                        stack,
                        dependencies,
                        depth + 1,
                    )?),
                    len: *len,
                })
            }
            Ty::Ref { kind, referent } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(self.normalize_type_in(
                        referent,
                        context,
                        stack,
                        dependencies,
                        depth + 1,
                    )?),
                })
            }
            Ty::Slice { element } => {
                retain::<Ty>(&self.budget, 1)?;
                Ok(Ty::Slice {
                    element: Arc::new(self.normalize_type_in(
                        element,
                        context,
                        stack,
                        dependencies,
                        depth + 1,
                    )?),
                })
            }
            Ty::Struct(_) | Ty::Enum(_) => {
                // Canonical nominal identities cannot be rebuilt by rewriting their fields.
                if self.type_contains_projection(ty, depth + 1)? {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: projection in nominal field or actual metadata",
                    ));
                }
                Ok(ty.clone())
            }
            _ => Ok(ty.clone()),
        }
    }

    fn normalization_key_equal(
        &self,
        left: &NormalizationKey,
        right: &NormalizationKey,
    ) -> Result<bool> {
        Ok(left.environment == right.environment
            && left.phase == right.phase
            && left.staged == right.staged
            && left.finalized == right.finalized
            && left.projection.item == right.projection.item
            && self.eq_predicate(
                &Predicate::Implements(left.projection.trait_ref.clone()),
                &Predicate::Implements(right.projection.trait_ref.clone()),
            )?)
    }

    fn normalize_projection_in(
        &self,
        projection: &ir::AssociatedTypeProjection,
        context: NormalizationContext<'_>,
        stack: &mut NormalizationStack,
    ) -> Result<Arc<NormalizationEvidence>> {
        self.check_normalization_budget()?;
        self.check_context(context.proof)?;
        charged(&self.budget, self.budget.check_depth(stack.active.len()))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
        self.validate_raw_associated_ty(
            &Ty::AssociatedType(Arc::new(projection.clone())),
            context.proof.env(),
            0,
        )?;
        let key = NormalizationKey {
            environment: context.proof.predicates.index,
            phase: context.proof.phase,
            staged: context.staged,
            finalized: self.owner_normalization_finalized,
            projection: projection.clone(),
        };
        for (old, _) in &stack.active {
            if self.normalization_key_equal(old, &key)? {
                return Err(FactsError::cycle(
                    &self.budget,
                    "associated type normalization cycle",
                ));
            }
        }
        let cached = {
            let memo = self.projection_memo.lock().map_err(|_| {
                FactsError::resource(&self.budget, "associated normalization cache poisoned")
            })?;
            let mut cached = None;
            for entry in memo.iter() {
                if self.normalization_key_equal(&entry.key, &key)? {
                    cached = Some(entry.result.clone());
                    break;
                }
            }
            cached
        };
        if let Some(cached) = cached {
            let evidence = cached?;
            self.validate_normalization_evidence(&evidence, context)?;
            return Ok(evidence);
        }
        charged(&self.budget, self.budget.charge_obligation())?;
        let mut child_dependencies = Vec::new();
        let selected_goal = TraitRef {
            declaration: projection.trait_ref.declaration,
            self_ty: self.normalize_type_in(
                &projection.trait_ref.self_ty,
                context,
                stack,
                &mut child_dependencies,
                0,
            )?,
            args: projection.trait_ref.args.clone(),
        };
        self.validate_ref_in(&selected_goal, context.proof.env(), None)?;
        if !self.goal_is_closed(&selected_goal, context.proof.env())? {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: nonclosed associated projection application",
            ));
        }
        // Unlike prove_in, this bypasses assumption-only membership and demands an impl witness.
        // This selector validates logical head predicates/WF, never method signatures or bodies.
        let selection_context = ProofContext {
            predicates: &self.empty,
            phase: FactsPhase::Closed,
        };
        let membership = match self.prove_implementation_in(
            &Predicate::Implements(selected_goal.clone()),
            selection_context,
            &mut stack.obligations,
        )? {
            ProofOutcome::Proved(evidence) => evidence,
            ProofOutcome::NotProven => {
                return Err(FactsError::unproved(
                    &self.budget,
                    "associated projection has no applicable implementation",
                ));
            }
        };
        let (implementation, arguments) = match &membership.0.origin {
            ProofOrigin::Implementation {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            _ => {
                return Err(FactsError::unproved(
                    &self.budget,
                    "associated projection requires an implementation assignment witness",
                ));
            }
        };
        // A repeated assignment pair with expanding arguments cannot terminate in this
        // closed-only RHS language. Catch it before spending the depth/resource allowance.
        for (old, owner) in &stack.active {
            if *owner == implementation && old.projection.item == projection.item {
                return Err(FactsError::cycle(
                    &self.budget,
                    "growing associated type assignment cycle",
                ));
            }
        }
        push(
            &self.budget,
            &mut stack.active,
            (key.clone(), implementation),
        )?;
        let result = (|| {
            let value = self.associated_value(implementation, projection.item)?;
            let header = self
                .headers
                .impls
                .iter()
                .find(|header| header.declaration == implementation)
                .ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "selected associated implementation disappeared",
                    )
                })?;
            let raw_rhs = self
                .schema
                .associated_type_rhs(implementation, projection.item)?;
            if !self.eq_ty(&raw_rhs, &value.rhs, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated RHS callback changed after head validation",
                ));
            }
            retain::<ir::RefContract>(&self.budget, 1)?;
            let schema = GenericSchema {
                env: header.schema_env.clone(),
                signature: Signature {
                    params: Vec::new(),
                    result: raw_rhs.clone(),
                    ref_contract: Arc::new(ir::RefContract::default()),
                },
                predicates: Arc::from([]),
            };
            let instantiated = self.schema.instantiate_schema(
                &schema,
                &arguments,
                context.proof.env(),
                &self.budget,
            )?;
            self.validate_rhs_substitution(
                &raw_rhs,
                &instantiated.signature.result,
                &header.schema_env,
                &arguments,
                0,
            )?;
            if !instantiated.signature.params.is_empty()
                || !instantiated.predicates.is_empty()
                || !self.eq_contract(
                    &schema.signature.ref_contract,
                    &instantiated.signature.ref_contract,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated RHS substitution changed structural schema authority",
                ));
            }
            let mut dependencies = child_dependencies;
            let normal = self.normalize_type_in(
                &instantiated.signature.result,
                context,
                stack,
                &mut dependencies,
                0,
            )?;
            self.validate_associated_normal_form(
                &normal,
                context.proof.env(),
                selection_context,
                &mut stack.obligations,
            )?;
            self.require_associated_assignment_bounds(
                projection.item,
                &normal,
                selection_context,
                &mut stack.obligations,
            )?;
            retain::<NormalizationEvidence>(&self.budget, 1)?;
            let evidence = Arc::new(NormalizationEvidence {
                world: self.id,
                key: key.clone(),
                membership,
                selected_goal,
                implementation,
                arguments,
                raw_rhs,
                normal,
                dependencies: dependencies.into(),
            });
            self.validate_normalization_evidence(&evidence, context)?;
            Ok(evidence)
        })();
        stack.active.pop();
        let mut memo = self.projection_memo.lock().map_err(|_| {
            FactsError::resource(&self.budget, "associated normalization cache poisoned")
        })?;
        push(
            &self.budget,
            &mut memo,
            NormalizationMemo {
                key,
                result: result.clone(),
            },
        )?;
        result
    }

    pub(super) fn validate_rhs_substitution(
        &self,
        raw: &Ty,
        actual: &Ty,
        env: &GenericEnv,
        arguments: &[Ty],
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Ty::Param(parameter) = raw {
            for (index, original) in env.ordered_params().enumerate() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if original == *parameter {
                    let expected = arguments.get(index).ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "associated RHS substitution omitted an impl argument",
                        )
                    })?;
                    if self.eq_ty(expected, actual, depth + 1)? {
                        return Ok(());
                    }
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated RHS substitution changed a bound parameter",
                    ));
                }
            }
            return Err(FactsError::invalid(
                &self.budget,
                "associated RHS parameter escaped its impl binder",
            ));
        }
        let child = |left: &Ty, right: &Ty| {
            self.validate_rhs_substitution(left, right, env, arguments, depth + 1)
        };
        match (raw, actual) {
            (Ty::AssociatedType(left), Ty::AssociatedType(right))
                if left.item == right.item
                    && left.trait_ref.declaration == right.trait_ref.declaration
                    && left.trait_ref.args.len() == right.trait_ref.args.len() =>
            {
                child(&left.trait_ref.self_ty, &right.trait_ref.self_ty)?;
                for (left, right) in left.trait_ref.args.iter().zip(right.trait_ref.args.iter()) {
                    child(left, right)?;
                }
                return Ok(());
            }
            (
                Ty::Ref {
                    kind: left_kind,
                    referent: left,
                },
                Ty::Ref {
                    kind: right_kind,
                    referent: right,
                },
            ) if left_kind == right_kind => {
                return child(left, right);
            }
            (
                Ty::Array {
                    element: left,
                    len: left_len,
                },
                Ty::Array {
                    element: right,
                    len: right_len,
                },
            ) if left_len == right_len => {
                return child(left, right);
            }
            (Ty::Slice { element: left }, Ty::Slice { element: right }) => {
                return child(left, right);
            }
            (Ty::Tuple(_), Ty::Tuple(_)) => {}
            (Ty::Struct(left), Ty::Struct(right))
                if left.def().source_decl == right.def().source_decl => {}
            (Ty::Enum(left), Ty::Enum(right))
                if left.def().source_decl == right.def().source_decl => {}
            _ => {
                if self.eq_ty(raw, actual, depth + 1)? {
                    return Ok(());
                }
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated RHS structural substitution changed its source type",
                ));
            }
        }
        let left = type_arguments(raw);
        let right = type_arguments(actual);
        if left.len() != right.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "associated RHS substitution changed aggregate arity",
            ));
        }
        for (left, right) in left.iter().zip(right.iter()) {
            child(left, right)?;
        }
        Ok(())
    }

    pub(super) fn validate_associated_normal_form(
        &self,
        normal: &Ty,
        env: &GenericEnv,
        context: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<()> {
        if self.type_contains_projection(normal, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "associated type normal form retains a projection",
            ));
        }
        self.validate_actual(normal, env)?;
        charged(&self.budget, ir::reject_references(normal, &self.budget))?;
        self.require_in(
            &Predicate::Implements(TraitRef {
                declaration: self.catalog.id(CatalogItem::Sized),
                self_ty: normal.clone(),
                args: Arc::from([]),
            }),
            context,
            active,
        )?;
        Ok(())
    }

    fn validate_normalization_evidence(
        &self,
        evidence: &NormalizationEvidence,
        context: NormalizationContext<'_>,
    ) -> Result<()> {
        self.check_normalization_budget()?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if evidence.world != self.id
            || evidence.key.environment != context.proof.predicates.index
            || evidence.key.phase != context.proof.phase
            || evidence.key.staged != context.staged
            || evidence.key.finalized != self.owner_normalization_finalized
            || evidence.membership.0.world != self.id
            || evidence.membership.0.environment != self.empty.index
            || evidence.membership.0.phase != FactsPhase::Closed
            || !self.eq_predicate(
                &evidence.membership.0.goal,
                &Predicate::Implements(evidence.selected_goal.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "associated normalization evidence lost world/environment/phase authority",
            ));
        }
        match &evidence.membership.0.origin {
            ProofOrigin::Implementation {
                declaration,
                arguments,
            } if *declaration == evidence.implementation
                && self.eq_types(arguments, &evidence.arguments, 0)? => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated normalization evidence lost exact impl substitution",
                ));
            }
        }
        let value = self.associated_value(evidence.implementation, evidence.key.projection.item)?;
        if !self.eq_ty(&evidence.raw_rhs, &value.rhs, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "associated normalization evidence lost assignment identity",
            ));
        }
        // Dependency records are sealed by this module and bound to the same authority.
        for dependency in evidence.dependencies.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if dependency.world != evidence.world
                || dependency.key.environment != evidence.key.environment
                || dependency.key.phase != evidence.key.phase
                || dependency.key.staged != evidence.key.staged
                || dependency.key.finalized != evidence.key.finalized
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated normalization dependency authority mismatch",
                ));
            }
        }
        // Replay the actual selected RHS requirement, including on a cache hit.
        // Caller assumptions must not certify a closed implementation's bound.
        self.require_associated_assignment_bounds(
            evidence.key.projection.item,
            &evidence.normal,
            ProofContext {
                predicates: &self.empty,
                phase: FactsPhase::Closed,
            },
            &mut Vec::new(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    struct RhsAdapter {
        inner: Arc<dyn SchemaCx + Send + Sync>,
        values: Vec<AssociatedTypeValue>,
        bounds: Vec<(DeclId, Arc<[DeclId]>)>,
        schemas: Vec<(DeclId, GenericSchema)>,
    }
    impl SchemaCx for RhsAdapter {
        fn budget(&self) -> &PlanBudget {
            self.inner.budget()
        }
        fn registry_snapshot(&self) -> SnapshotId {
            self.inner.registry_snapshot()
        }
        fn core_catalog(&self) -> &Arc<CoreCatalog> {
            self.inner.core_catalog()
        }
        fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl> {
            self.inner.declaration_info(id)
        }
        fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema> {
            if let Some((_, schema)) = self.schemas.iter().find(|(decl, _)| *decl == id) {
                return Ok(schema.clone());
            }
            self.inner.declaration_schema(id)
        }
        fn associated_type_bounds(&self, associated: DeclId) -> Result<Arc<[DeclId]>> {
            for (declaration, bounds) in &self.bounds {
                if *declaration == associated {
                    return Ok(bounds.clone());
                }
            }
            self.inner.associated_type_bounds(associated)
        }
        fn associated_type_rhs(&self, implementation: DeclId, associated: DeclId) -> Result<Ty> {
            for value in &self.values {
                if value.implementation == implementation && value.associated == associated {
                    return Ok(value.rhs.clone());
                }
            }
            self.inner.associated_type_rhs(implementation, associated)
        }
        fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>> {
            self.inner.source_region_env(id)
        }
        fn instantiate_schema(
            &self,
            schema: &GenericSchema,
            arguments: &[Ty],
            caller: &GenericEnv,
            budget: &PlanBudget,
        ) -> Result<InstantiatedSchema> {
            self.inner
                .instantiate_schema(schema, arguments, caller, budget)
        }
        fn validate_nominal(&self, ty: &Ty, env: &GenericEnv, budget: &PlanBudget) -> Result<()> {
            self.inner.validate_nominal(ty, env, budget)
        }
    }

    fn fixture() -> Arc<ValidatedTraitWorld> {
        let program = frontend::parse("struct S;trait Has{type Out;type Other;}impl Has for S{type Out=i32;type Other=bool;}fn run()->i32{7}fn main(){}").unwrap();
        PlanCtx::from_program(&program, &PlanBudget::default())
            .unwrap()
            .trait_world()
            .unwrap()
    }

    fn assignment(world: &ValidatedTraitWorld) -> (usize, AssociatedTypeValue) {
        world
            .headers
            .impls
            .iter()
            .enumerate()
            .find_map(|(index, header)| {
                header
                    .associated_types
                    .first()
                    .map(|value| (index, value.clone()))
            })
            .unwrap()
    }

    #[test]
    fn associated_raw_rhs_callback_must_agree_with_frozen_header() {
        let world = fixture();
        let (_, mut value) = assignment(&world);
        value.rhs = Ty::Bool;
        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            schemas: Vec::new(),
            values: vec![value],
            bounds: Vec::new(),
        });
        let error =
            ValidatedTraitWorld::build(world.headers.clone(), adapter, &world.budget).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn associated_assignment_pair_cannot_change_impl_owner_or_duplicate() {
        let world = fixture();
        let (index, mut value) = assignment(&world);
        let mut headers = world.headers.clone();
        value.implementation = value.associated;
        Arc::make_mut(&mut headers.impls)[index].associated_types = Arc::from([value]);
        assert_eq!(
            ValidatedTraitWorld::build(headers, world.schema.clone(), &world.budget)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
        let (_, value) = assignment(&world);
        let mut headers = world.headers.clone();
        Arc::make_mut(&mut headers.impls)[index].associated_types =
            Arc::from([value.clone(), value]);
        assert_eq!(
            ValidatedTraitWorld::build(headers, world.schema.clone(), &world.budget)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn associated_header_environment_is_authoritative() {
        let world = fixture();
        let mut headers = world.headers.clone();
        Arc::make_mut(&mut headers.associated_types)[0].env = world.empty.env.clone();
        assert_eq!(
            ValidatedTraitWorld::build(headers, world.schema.clone(), &world.budget)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn associated_unused_direct_and_mutual_cycles_are_typed() {
        for mutual in [false, true] {
            let world = fixture();
            let (index, _) = assignment(&world);
            let implementation = &world.headers.impls[index];
            let mut values = implementation.associated_types.to_vec();
            let first = values[0].associated;
            let second = if mutual { values[1].associated } else { first };
            values[0].rhs = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                trait_ref: implementation.trait_ref.clone(),
                item: second,
            }));
            if mutual {
                values[1].rhs = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: implementation.trait_ref.clone(),
                    item: first,
                }));
            }
            let adapter = Arc::new(RhsAdapter {
                inner: world.schema.clone(),
                schemas: Vec::new(),
                values: values.clone(),
                bounds: Vec::new(),
            });
            let mut headers = world.headers.clone();
            Arc::make_mut(&mut headers.impls)[index].associated_types = values.into();
            assert_eq!(
                ValidatedTraitWorld::build(headers, adapter, &world.budget)
                    .unwrap_err()
                    .kind(),
                FactsErrorKind::Cycle
            );
        }
    }

    #[test]
    fn associated_bound_header_cannot_add_or_erase_source_requirements() {
        let world = fixture();
        let associated = world.headers.associated_types[0].declaration;
        let copy = world.catalog.id(CatalogItem::Copy);
        let mut headers = world.headers.clone();
        Arc::make_mut(&mut headers.associated_types)[0].bounds = Arc::from([copy]);
        let error =
            ValidatedTraitWorld::build(headers, world.schema.clone(), &world.budget).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);

        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            schemas: Vec::new(),
            values: Vec::new(),
            bounds: vec![(associated, Arc::from([copy]))],
        });
        let error =
            ValidatedTraitWorld::build(world.headers.clone(), adapter, &world.budget).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn associated_bound_checks_later_foreign_and_wrong_kind_before_capability() {
        for foreign in [false, true] {
            let world = fixture();
            let header = &world.headers.associated_types[0];
            let later = if foreign {
                DeclId {
                    snapshot: SnapshotId::allocate().unwrap(),
                    index: header.declaration.index,
                }
            } else {
                header.declaration
            };
            // Has is a valid source trait but outside the bounded capability.
            // Its unsupported status must not hide the invalid later identity.
            let bounds: Arc<[DeclId]> = Arc::from([header.trait_decl, later]);
            let mut headers = world.headers.clone();
            Arc::make_mut(&mut headers.associated_types)[0].bounds = bounds.clone();
            let adapter = Arc::new(RhsAdapter {
                inner: world.schema.clone(),
                schemas: Vec::new(),
                values: Vec::new(),
                bounds: vec![(header.declaration, bounds)],
            });
            let error = ValidatedTraitWorld::build(headers, adapter, &world.budget).unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn callback_bound_identity_precedes_projected_default_capability() {
        let program = frontend::parse(
            "struct S;trait Has{type Out;fn value()->i32{7}}\
             impl Has for S{type Out=i32;}fn main(){}",
        )
        .unwrap();
        let world = PlanCtx::from_program(&program, &PlanBudget::default())
            .unwrap()
            .trait_world()
            .unwrap();
        let associated = &world.headers.associated_types[0];
        let owner = world.trait_header(associated.trait_decl).unwrap();
        let mut headers = world.headers.clone();
        let item = Arc::make_mut(&mut headers.items)
            .iter_mut()
            .find(|item| item.trait_decl == owner.declaration)
            .unwrap();
        let default = item.default.unwrap();
        item.schema.signature.result = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: owner.declaration,
                self_ty: Ty::Param(owner.self_param),
                args: Arc::from([]),
            },
            item: associated.declaration,
        }));
        let mut schemas = vec![(item.declaration, item.schema.clone())];
        let callable = Arc::make_mut(&mut headers.callables)
            .iter_mut()
            .find(|callable| callable.declaration == default)
            .unwrap();
        callable.schema.signature.result = schemas[0].1.signature.result.clone();
        schemas.push((default, callable.schema.clone()));

        // First prove that the same headers reach an actual capability rejection.
        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            values: Vec::new(),
            bounds: Vec::new(),
            schemas: schemas.clone(),
        });
        let error =
            ValidatedTraitWorld::build(headers.clone(), adapter, &world.budget).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
        assert!(
            error
                .to_string()
                .contains("projection-bearing trait default signatures"),
            "{error}"
        );

        for foreign in [false, true] {
            let invalid = if foreign {
                DeclId {
                    snapshot: SnapshotId::allocate().unwrap(),
                    index: associated.declaration.index,
                }
            } else {
                associated.declaration
            };
            // Header bounds stay empty: only the authoritative callback is invalid.
            let adapter = Arc::new(RhsAdapter {
                inner: world.schema.clone(),
                values: Vec::new(),
                bounds: vec![(associated.declaration, Arc::from([invalid]))],
                schemas: schemas.clone(),
            });
            let error =
                ValidatedTraitWorld::build(headers.clone(), adapter, &world.budget).unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        }
    }

    #[test]
    fn associated_bound_rejects_noncanonical_trait_even_with_matching_source() {
        let world = fixture();
        let header = &world.headers.associated_types[0];
        let bounds: Arc<[DeclId]> = Arc::from([header.trait_decl]);
        let mut headers = world.headers.clone();
        Arc::make_mut(&mut headers.associated_types)[0].bounds = bounds.clone();
        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            schemas: Vec::new(),
            values: Vec::new(),
            bounds: vec![(header.declaration, bounds)],
        });
        let error = ValidatedTraitWorld::build(headers, adapter, &world.budget).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
    }

    #[test]
    fn associated_bound_preserves_duplicate_canonical_rows() {
        let world = fixture();
        let header = &world.headers.associated_types[0];
        let copy = world.catalog.id(CatalogItem::Copy);
        let sized = world.catalog.id(CatalogItem::Sized);
        let bounds: Arc<[DeclId]> = Arc::from([copy, sized, copy]);
        let mut headers = world.headers.clone();
        Arc::make_mut(&mut headers.associated_types)[0].bounds = bounds.clone();
        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            schemas: Vec::new(),
            values: Vec::new(),
            bounds: vec![(header.declaration, bounds.clone())],
        });
        let rebuilt = ValidatedTraitWorld::build(headers, adapter, &world.budget).unwrap();
        assert_eq!(
            rebuilt
                .associated_header(header.declaration)
                .unwrap()
                .bounds,
            bounds
        );
    }

    #[test]
    fn associated_source_unused_generic_assignment_requires_actual_copy_bound() {
        for copy in [false, true] {
            let bound = if copy { ":Copy" } else { "" };
            let source = frontend::parse(&format!(
                "trait Has{{type Out:Copy;}}struct Wrap<T>{{value:T}}\
                 impl<T{bound}> Has for Wrap<T>{{type Out=T;}}fn main(){{}}"
            ))
            .unwrap();
            let result = PlanCtx::from_program(&source, &PlanBudget::default());
            if copy {
                assert!(result.is_ok());
            } else {
                let error = match result {
                    Err(error) => error,
                    Ok(_) => panic!("unused non-Copy assignment accepted"),
                };
                assert!(error.contains("trait obligation"), "{error}");
            }
        }
    }

    #[test]
    fn associated_closed_bound_survives_cached_normalization_and_replay() {
        let source = frontend::parse(
            "struct S;trait Has{type Out:Copy;}impl Has for S{type Out=i32;}\
             fn run()->i32{7}fn main(){}",
        )
        .unwrap();
        let plan = PlanCtx::from_program(&source, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let implementation = world
            .headers
            .impls
            .iter()
            .find(|header| !header.associated_types.is_empty())
            .unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: implementation.trait_ref.clone(),
            item: implementation.associated_types[0].associated,
        };
        let declaration = world
            .headers
            .callables
            .iter()
            .find(|header| header.schema.env.total_arity() == 0)
            .unwrap()
            .declaration;
        let facts = world.callable_facts(declaration).unwrap();
        let first = world
            .normalize_associated_type(&projection, &facts)
            .unwrap();
        let cached = world
            .normalize_associated_type(&projection, &facts)
            .unwrap();
        assert!(Arc::ptr_eq(&first.0, &cached.0));
        cached.validate_for(&projection, &facts).unwrap();
        let mut forged = (*cached.0).clone();
        forged.normal = Ty::Slice {
            element: Arc::new(Ty::Bool),
        };
        let error = NormalizedAssociatedType(Arc::new(forged))
            .validate_for(&projection, &facts)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
    }

    #[test]
    fn associated_source_copy_bound_does_not_elaborate_equality_parameter() {
        for copy in [false, true] {
            let bound = if copy { ":Copy" } else { "" };
            let source = frontend::parse(&format!(
                "trait Source:Sized{{type Out:Copy;fn get(self)->Self::Out;}}\
                 fn produce<T:Source<Out=U>,U{bound}>(x:T)->U{{x.get()}}fn main(){{}}"
            ))
            .unwrap();
            let plan = PlanCtx::from_program(&source, &PlanBudget::default()).unwrap();
            let world = plan.trait_world().unwrap();
            let callable = world
                .headers
                .callables
                .iter()
                .find(|header| header.schema.env.total_arity() == 2)
                .unwrap();
            let rhs = callable.schema.env.ordered_params().nth(1).unwrap();
            let facts = world.callable_facts(callable.declaration).unwrap();
            assert_eq!(facts.must_copy(&Ty::Param(rhs)).unwrap(), copy);
            // The existing result certificate remains usable and does not
            // change the independent Copy decision above.
            facts.carrier_contract(SignatureSubject::Body).unwrap();
        }
    }
}
