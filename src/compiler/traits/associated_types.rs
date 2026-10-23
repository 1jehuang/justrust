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
            Callee::TraitItem(call)
            | Callee::Virtual(ir::VirtualCall { call })
            | Callee::Untupled(call) => {
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
enum NormalizationOrigin {
    Implementation(DeclId),
    Dynamic(Arc<ir::DynTy>),
    Opaque(Arc<ir::OpaqueTy>),
}

#[derive(Clone, Debug)]
struct NormalizationEvidence {
    world: WorldId,
    key: NormalizationKey,
    membership: ProofEvidence,
    selected_goal: TraitRef,
    implementation: NormalizationOrigin,
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
    obligations: ProofScratch,
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
        self.validate_ty_metadata_with(ty, env, depth, &mut NominalReplays::default())
    }

    /// As `validate_ty_metadata_in`, skipping world replay of realized
    /// nominal facts this computation already replayed successfully.
    pub(in crate::compiler) fn validate_ty_metadata_with(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Some(height) = replays.validated_type(self, ty, env)? {
            return charged(
                &self.budget,
                self.budget.check_depth(depth.saturating_add(height)),
            );
        }
        let height =
            self.validate_ty_metadata_walk(ty, env, depth, &mut TypeWalk::default(), replays)?;
        replays.record_type(ty, env, height, &self.budget)
    }

    fn validate_ty_metadata_walk(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        walk: &mut TypeWalk,
        replays: &mut NominalReplays,
    ) -> Result<usize> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        if ty.is_nominal_back_edge() {
            return Ok(0);
        }
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
        let mut child = |value: &Ty, replays: &mut NominalReplays| -> Result<()> {
            height = height.max(
                self.validate_ty_metadata_walk(value, env, depth + 1, walk, replays)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Opaque(opaque) => {
                self.validate_opaque_metadata(opaque, env)?;
                for ty in opaque.types() {
                    child(ty, replays)?;
                }
            }
            Ty::Closure(closure) => {
                self.info(closure.owner())?;
                self.info(closure.provider())?;
                for ty in closure.types() {
                    child(ty, replays)?;
                }
            }
            Ty::FnItem(item) => {
                self.info(item.declaration)?;
                for ty in item.type_args.iter() {
                    child(ty, replays)?;
                }
            }
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
                child(&projection.trait_ref.self_ty, replays)?;
                for argument in projection.trait_ref.args.iter() {
                    child(argument, replays)?;
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
                    child(part, replays)?;
                }
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array metadata length limit exceeded",
                    ));
                }
                child(element, replays)?;
            }
            Ty::Ref { referent, .. }
            | Ty::RawPtr {
                pointee: referent, ..
            }
            | Ty::Slice { element: referent } => {
                child(referent, replays)?;
            }
            Ty::Dyn(object) => {
                let header = self.trait_header(object.principal)?;
                if object.args.len().checked_add(1) != Some(header.env.total_arity()) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "dynamic principal generic arity changed",
                    ));
                }
                for argument in object.args.iter() {
                    child(argument, replays)?;
                }
                let mut previous = None;
                for (item, value) in object.associated_types.iter() {
                    let associated = self.associated_header(*item)?;
                    if item.snapshot != self.registry
                        || previous.is_some_and(|index| index >= item.index)
                        || !self.trait_object_implements(
                            object.principal,
                            associated.trait_decl,
                            0,
                        )?
                    {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "dynamic associated equality identity or order changed",
                        ));
                    }
                    previous = Some(item.index);
                    child(value, replays)?;
                }
                let applications = self.object_applications(object, &Ty::Unit, env)?;
                for application in applications {
                    let owner = self.trait_header(application.declaration)?;
                    for item in owner.associated_types.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if !object
                            .associated_types
                            .iter()
                            .any(|(bound, _)| bound == item)
                        {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "dynamic metadata omitted a required associated equality",
                            ));
                        }
                    }
                }
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
                if let Some(capability) = value.realization() {
                    capability.replay_with(ty, self, replays).map_err(|error| {
                        FactsError::from_message(&self.budget, &error.to_string())
                    })?;
                }
                for argument in value.def().type_args.iter() {
                    child(argument, replays)?;
                }
                for field in value.def().fields.iter() {
                    child(&field.ty, replays)?;
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
                if let Some(capability) = value.realization() {
                    capability.replay_with(ty, self, replays).map_err(|error| {
                        FactsError::from_message(&self.budget, &error.to_string())
                    })?;
                }
                for argument in value.def().type_args.iter() {
                    child(argument, replays)?;
                }
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
                        child(&field.ty, replays)?;
                    }
                }
            }
            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Str | Ty::Bool | Ty::Unit | Ty::Never => {}
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
        self.check_normalization_budget()?;
        // Carrier construction and replay call this for every direct call.
        // Reuse the immutable normalized index rather than walking all source
        // callables again. A miss still consults the original schema authority,
        // including required trait items absent from executable callables.
        let mut start = 0;
        let mut end = self.normalized_callable_index.len();
        while start < end {
            charged(&self.budget, self.budget.charge_work(1))?;
            let middle = start + (end - start) / 2;
            if self.normalized_callable_index[middle].0 < declaration {
                start = middle + 1;
            } else {
                end = middle;
            }
        }
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Some((id, position)) = self.normalized_callable_index.get(start)
            && *id == declaration
        {
            return clone_schema(&self.normalized_callables[*position].schema, &self.budget);
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
        let mut stack = NormalizationStack::default();
        let normal = self.normalize_type_in(ty, context, &mut stack, &mut dependencies, 0)?;
        let mut retained = Vec::new();
        for dependency in dependencies {
            self.validate_normalization_evidence_with(
                &dependency,
                context,
                &mut stack.obligations,
            )?;
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

    pub(super) fn validate_staged_projected_dependency(
        &self,
        evidence: &NormalizedAssociatedType,
        proof: ProofContext<'_>,
        staged: bool,
    ) -> Result<()> {
        self.check_context(proof)?;
        self.validate_normalization_evidence(&evidence.0, NormalizationContext { proof, staged })
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
    pub(super) fn validate_raw_associated_ty(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
    ) -> Result<()> {
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
                    self.validate_raw_associated_ty(argument, env, depth + 1)?;
                }
                if !self.raw_associated_ty_is_closed(ty, depth + 1)?
                    && !matches!(&projection.trait_ref.self_ty, Ty::Dyn(_))
                    && !self.checked_closure_projection(projection, env)?
                {
                    self.validate_universal_projection(projection, env)?;
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
            // A raw header such as `Option<Self::Item>` (or its impl-side
            // `Option<<C as It>::Item>`) carries a projection as a nominal
            // actual. Validate the application's metadata, then each actual
            // as raw, exactly as tuples and references do above. Executable
            // types still meet the strict validator after normalization.
            Ty::Struct(_) | Ty::Enum(_) if self.type_contains_projection(ty, depth + 1)? => {
                self.validate_ty_metadata_in(ty, env, depth)?;
                let arguments = match ty {
                    Ty::Struct(value) => value.def().type_args.clone(),
                    Ty::Enum(value) => value.def().type_args.clone(),
                    _ => unreachable!(),
                };
                for argument in arguments.iter() {
                    self.validate_raw_associated_ty(argument, env, depth + 1)?;
                }
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
            Ty::Closure(closure) => {
                for value in closure.types() {
                    if !self.raw_associated_ty_is_closed(value, depth + 1)? {
                        return Ok(false);
                    }
                }
            }
            Ty::Dyn(object) => {
                for value in object.types() {
                    if !self.raw_associated_ty_is_closed(value, depth + 1)? {
                        return Ok(false);
                    }
                }
            }
            // A nominal application's fields mention only its own ordered
            // actuals, so closedness is decided by `type_arguments` below.
            // Walking fields would revisit recursive types (`Box<List>`)
            // until the depth limit.
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
            self.validate_ref_in(&implementation.trait_ref, &implementation.schema_env, None)?;
            self.validate_predicates(&implementation.predicates, &implementation.schema_env, None)?;
            self.validate_impl_parameter_constraints(implementation)?;
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
            self.associated_bound_predicates(header.declaration)?;
        }
        Ok(())
    }

    pub(super) fn require_associated_assignment_bounds(
        &self,
        associated: DeclId,
        owner: &TraitRef,
        normal: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
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
        self.discharge_associated_bound_predicates(associated, owner, normal, context, active, None)
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
                    let result =
                        self.normalize_declared_callable_schema(callable.declaration, &schema)?;
                    let position = self
                        .declared_callable_index
                        .get(&self.budget, callable.declaration)?
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "missing declared reduction destination",
                            )
                        })?;
                    schema = result.finish(&mut self.declared_callables[position], &self.budget)?;
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
                if let Some(universal) = self.normalize_universal_callable_schema(&schema)? {
                    push(
                        &self.budget,
                        &mut normalized,
                        CallableHeader {
                            declaration: callable.declaration,
                            schema: universal,
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
        let mut index = Vec::new();
        index
            .try_reserve_exact(normalized.len())
            .map_err(|_| FactsError::resource(&self.budget, "callable header index allocation"))?;
        self.budget
            .charge_work(normalized.len())
            .map_err(|message| FactsError::resource(&self.budget, &message))?;
        index.extend(
            normalized
                .iter()
                .enumerate()
                .map(|(position, header)| (header.declaration, position)),
        );
        // Stable sort keeps the first header for a declaration first, which
        // matches the previous first-match linear scan.
        index.sort_by_key(|(declaration, _)| *declaration);
        self.normalized_callable_index = index;
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
            Ty::Dyn(object) => {
                let mut arguments = Vec::new();
                for argument in object.args.iter() {
                    let normal =
                        self.normalize_type_in(argument, context, stack, dependencies, depth + 1)?;
                    push(&self.budget, &mut arguments, normal)?;
                }
                let mut associated_types = Vec::new();
                for (item, value) in object.associated_types.iter() {
                    let normal =
                        self.normalize_type_in(value, context, stack, dependencies, depth + 1)?;
                    push(&self.budget, &mut associated_types, (*item, normal))?;
                }
                retain::<ir::DynTy>(&self.budget, 1)?;
                Ok(Ty::Dyn(Arc::new(ir::DynTy {
                    principal: object.principal,
                    args: arguments.into(),
                    associated_types: associated_types.into(),
                })))
            }
            Ty::Struct(_) | Ty::Enum(_) => {
                self.normalize_nominal_actuals(ty, context.proof.env(), depth, |argument| {
                    self.normalize_type_in(argument, context, stack, dependencies, depth + 1)
                })
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
                stack.obligations.cycles += 1;
                return Err(FactsError::cycle(
                    &self.budget,
                    "associated type normalization cycle",
                ));
            }
        }
        let cached = {
            let mut cached = None;
            for entry in stack.obligations.projections.iter() {
                if self.normalization_key_equal(&entry.key, &key)? {
                    cached = Some(entry.result.clone());
                    break;
                }
            }
            cached
        };
        if let Some(cached) = cached {
            let evidence = cached?;
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            return Ok(evidence);
        }
        stack.obligations.charge_obligation(&self.budget)?;
        let cycles_before = stack.obligations.cycles;
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
        self.validate_ref_with(
            &selected_goal,
            context.proof.env(),
            None,
            &mut stack.obligations.replays,
        )?;
        if let Some(raw_rhs) = self.dynamic_associated_binding(
            &ir::AssociatedTypeProjection {
                trait_ref: selected_goal.clone(),
                item: projection.item,
            },
            context.proof.env(),
        )? {
            let Ty::Dyn(object) = &selected_goal.self_ty else {
                unreachable!()
            };
            let object = object.clone();
            let membership = match self.prove_in(
                &Predicate::Implements(selected_goal.clone()),
                context.proof,
                &mut stack.obligations,
            )? {
                ProofOutcome::Proved(evidence) => evidence,
                ProofOutcome::NotProven => {
                    return Err(FactsError::unproved(
                        &self.budget,
                        "dynamic projection lost trait membership",
                    ));
                }
            };
            push(
                &self.budget,
                &mut stack.active,
                (key.clone(), object.principal),
            )?;
            let normalized =
                self.normalize_type_in(&raw_rhs, context, stack, &mut child_dependencies, 0);
            stack.active.pop();
            let normal = normalized?;
            self.validate_ty(&normal, context.proof.env(), None, true, 0)?;
            let evidence = Arc::new(NormalizationEvidence {
                world: self.id,
                key,
                membership,
                selected_goal,
                implementation: NormalizationOrigin::Dynamic(object),
                arguments: Arc::from([]),
                raw_rhs,
                normal,
                dependencies: child_dependencies.into(),
            });
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            return Ok(evidence);
        }
        if let Some(raw_rhs) = self.opaque_associated_binding(
            &ir::AssociatedTypeProjection {
                trait_ref: selected_goal.clone(),
                item: projection.item,
            },
            context.proof.env(),
        )? {
            let Ty::Opaque(opaque) = &selected_goal.self_ty else {
                unreachable!()
            };
            let opaque = opaque.clone();
            let membership = self.require_in(
                &Predicate::Implements(selected_goal.clone()),
                context.proof,
                &mut stack.obligations,
            )?;
            push(
                &self.budget,
                &mut stack.active,
                (key.clone(), opaque.owner()),
            )?;
            let normalized =
                self.normalize_type_in(&raw_rhs, context, stack, &mut child_dependencies, 0);
            stack.active.pop();
            let normal = normalized?;
            self.validate_ty(&normal, context.proof.env(), None, true, 0)?;
            retain::<NormalizationEvidence>(&self.budget, 1)?;
            let evidence = Arc::new(NormalizationEvidence {
                world: self.id,
                key,
                membership,
                selected_goal,
                implementation: NormalizationOrigin::Opaque(opaque),
                arguments: Arc::from([]),
                raw_rhs,
                normal,
                dependencies: child_dependencies.into(),
            });
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            return Ok(evidence);
        }
        let closed_goal = self.goal_is_closed(&selected_goal, context.proof.env())?;
        let builtin_closure = self.checked_closure_projection(
            &ir::AssociatedTypeProjection {
                trait_ref: selected_goal.clone(),
                item: projection.item,
            },
            context.proof.env(),
        )?;
        if !closed_goal && !builtin_closure {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: nonclosed associated projection application",
            ));
        }
        // Concrete implementation selection remains assumption-free. A symbolic
        // builtin closure instead proves its exact owner predicates in the
        // genuine caller environment, retaining that context in its evidence.
        let selection_context = if closed_goal {
            ProofContext {
                predicates: &self.empty,
                phase: FactsPhase::Closed,
            }
        } else {
            context.proof
        };
        if let Some(ProofOutcome::Proved(membership)) = self.prove_closure(
            &Predicate::Implements(selected_goal.clone()),
            selection_context,
            &mut stack.obligations,
        )? {
            let Ty::Closure(closure) = &selected_goal.self_ty else {
                unreachable!()
            };
            let raw_rhs = closure.result().clone();
            let normal =
                self.normalize_type_in(&raw_rhs, context, stack, &mut child_dependencies, 0)?;
            retain::<NormalizationEvidence>(&self.budget, 1)?;
            let evidence = Arc::new(NormalizationEvidence {
                world: self.id,
                key,
                membership,
                selected_goal: selected_goal.clone(),
                implementation: NormalizationOrigin::Implementation(closure.provider()),
                arguments: closure.owner_args().into(),
                raw_rhs,
                normal,
                dependencies: child_dependencies.into(),
            });
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            return Ok(evidence);
        }
        if let Some(ProofOutcome::Proved(membership)) = self.prove_fn_item(
            &Predicate::Implements(selected_goal.clone()),
            selection_context,
        )? {
            let Ty::FnItem(item) = &selected_goal.self_ty else {
                unreachable!()
            };
            let raw_rhs = self.fn_item_signature(item, selection_context)?.result;
            let normal =
                self.normalize_type_in(&raw_rhs, context, stack, &mut child_dependencies, 0)?;
            retain::<NormalizationEvidence>(&self.budget, 1)?;
            let evidence = Arc::new(NormalizationEvidence {
                world: self.id,
                key,
                membership,
                selected_goal: selected_goal.clone(),
                implementation: NormalizationOrigin::Implementation(item.declaration),
                arguments: item.type_args.clone(),
                raw_rhs,
                normal,
                dependencies: child_dependencies.into(),
            });
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            return Ok(evidence);
        }
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
                stack.obligations.cycles += 1;
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
                Some(projection.item),
                &normal,
                context.proof.env(),
                selection_context,
                &mut stack.obligations,
            )?;
            self.require_associated_assignment_bounds(
                projection.item,
                &projection.trait_ref,
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
                implementation: NormalizationOrigin::Implementation(implementation),
                arguments,
                raw_rhs,
                normal,
                dependencies: dependencies.into(),
            });
            self.validate_normalization_evidence_with(&evidence, context, &mut stack.obligations)?;
            Ok(evidence)
        })();
        stack.active.pop();
        // Cycle-dependent or resource outcomes are provisional: never reuse.
        if stack.obligations.cycles != cycles_before
            || matches!(&result, Err(e) if matches!(e.kind(), FactsErrorKind::Cycle | FactsErrorKind::Resource))
        {
            return result;
        }
        stack.obligations.projections.push(NormalizationMemo {
            key,
            result: result.clone(),
        });
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
            (Ty::Opaque(left), Ty::Opaque(right)) => {
                if !left.try_eq(right, |left, right| {
                    child(left, right)?;
                    Ok::<_, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque substitution changed declaration identity",
                    ));
                }
            }
            (Ty::FnItem(left), Ty::FnItem(right))
                if left.declaration == right.declaration
                    && left.type_args.len() == right.type_args.len() =>
            {
                for (left, right) in left.type_args.iter().zip(right.type_args.iter()) {
                    child(left, right)?;
                }
            }
            (Ty::Closure(left), Ty::Closure(right)) => {
                return self.validate_closure_rhs_substitution(
                    left,
                    right,
                    env,
                    arguments,
                    depth + 1,
                );
            }
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
            (
                Ty::RawPtr {
                    kind: left_kind,
                    pointee: left,
                },
                Ty::RawPtr {
                    kind: right_kind,
                    pointee: right,
                },
            ) if left_kind == right_kind => {
                return child(left, right);
            }
            (Ty::Dyn(left), Ty::Dyn(right))
                if left.principal == right.principal
                    && left.args.len() == right.args.len()
                    && left.associated_types.len() == right.associated_types.len() =>
            {
                for (left, right) in left.args.iter().zip(right.args.iter()) {
                    child(left, right)?;
                }
                for ((left_item, left), (right_item, right)) in left
                    .associated_types
                    .iter()
                    .zip(right.associated_types.iter())
                {
                    if left_item != right_item {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "dyn substitution changed associated declaration identity",
                        ));
                    }
                    child(left, right)?;
                }
                return Ok(());
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
        item: Option<DeclId>,
        normal: &Ty,
        env: &GenericEnv,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.check_context(context)?;
        if self.type_contains_projection(normal, 0)? {
            // An owning generic impl can assign an exact rigid family from its
            // declared bounds. This is not a closed normal form or permission
            // to emit a residual projection. Keep the same immutable caller
            // environment and replay membership for every projected child.
            if context.phase != FactsPhase::Universal || !self.same_env(env, context.env())? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated type normal form retains a projection",
                ));
            }
            self.validate_universal_projection_type(normal, env, 0)?;
        }
        self.validate_actual(normal, env)?;
        charged(&self.budget, ir::reject_references(normal, &self.budget))?;
        if let Some(item) = item
            && self.associated_header(item)?.relaxed_sized
        {
            return Ok(());
        }
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
        self.validate_normalization_evidence_with(evidence, context, &mut ProofScratch::default())
    }

    fn validate_normalization_evidence_with(
        &self,
        evidence: &NormalizationEvidence,
        context: NormalizationContext<'_>,
        proofs: &mut ProofScratch,
    ) -> Result<()> {
        self.check_normalization_budget()?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let caller_membership = matches!(
            &evidence.implementation,
            NormalizationOrigin::Dynamic(_) | NormalizationOrigin::Opaque(_)
        ) || (matches!(
            &evidence.membership.0.origin,
            ProofOrigin::Closure { .. } | ProofOrigin::FnItem { .. }
        ) && !self
            .goal_is_closed(&evidence.selected_goal, context.proof.env())?);
        if evidence.world != self.id
            || evidence.key.environment != context.proof.predicates.index
            || evidence.key.phase != context.proof.phase
            || evidence.key.staged != context.staged
            || evidence.key.finalized != self.owner_normalization_finalized
            || evidence.membership.0.world != self.id
            || evidence.membership.0.environment
                != if caller_membership {
                    context.proof.predicates.index
                } else {
                    self.empty.index
                }
            || evidence.membership.0.phase
                != if caller_membership {
                    context.proof.phase
                } else {
                    FactsPhase::Closed
                }
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
        if let ProofOrigin::FnItem { kind } = &evidence.membership.0.origin {
            let Ty::FnItem(item) = &evidence.selected_goal.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function item normalization lost its item type",
                ));
            };
            let result = self.fn_item_signature(item, context.proof)?.result;
            if self
                .schema
                .closure_trait_kind(evidence.selected_goal.declaration)?
                != Some(*kind)
                || !matches!(&evidence.implementation, NormalizationOrigin::Implementation(declaration) if *declaration == item.declaration)
                || !self.eq_types(&evidence.arguments, &item.type_args, 0)?
                || !self.eq_ty(&evidence.raw_rhs, &result, 0)?
                || self
                    .associated_header(evidence.key.projection.item)?
                    .trait_decl
                    != evidence.selected_goal.declaration
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function item Output normalization differs from its signature",
                ));
            }
            return Ok(());
        }
        if let ProofOrigin::Closure { kind } = &evidence.membership.0.origin {
            let Ty::Closure(closure) = &evidence.selected_goal.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure normalization lost environment metadata",
                ));
            };
            self.validate_closure_metadata(closure, context.proof.env())?;
            if self
                .schema
                .closure_trait_kind(evidence.selected_goal.declaration)?
                != Some(*kind)
                || !closure.supports(*kind)
                || !matches!(&evidence.implementation, NormalizationOrigin::Implementation(provider) if *provider == closure.provider())
                || !self.eq_types(&evidence.arguments, closure.owner_args(), 0)?
                || !self.eq_ty(&evidence.raw_rhs, closure.result(), 0)?
                || !self.eq_ty(&evidence.normal, closure.result(), 0)?
                || self
                    .associated_header(evidence.key.projection.item)?
                    .trait_decl
                    != evidence.selected_goal.declaration
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure Output normalization differs from checked metadata",
                ));
            }
            return Ok(());
        }
        let implementation = match &evidence.implementation {
            NormalizationOrigin::Implementation(declaration) => *declaration,
            NormalizationOrigin::Opaque(opaque) => {
                if !matches!(evidence.membership.0.origin, ProofOrigin::OpaqueDeclared)
                    || !self.eq_ty(
                        &evidence.selected_goal.self_ty,
                        &Ty::Opaque(opaque.clone()),
                        0,
                    )?
                    || !evidence.arguments.is_empty()
                    || !self.same_projection(
                        &evidence.key.projection,
                        &ir::AssociatedTypeProjection {
                            trait_ref: evidence.selected_goal.clone(),
                            item: evidence.key.projection.item,
                        },
                    )?
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque normalization lost exact declared membership",
                    ));
                }
                let expected = self
                    .opaque_associated_binding(&evidence.key.projection, context.proof.env())?
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "opaque normalization lost public associated binding",
                        )
                    })?;
                if !self.eq_ty(&expected, &evidence.raw_rhs, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque normalization changed its declaration-owned RHS",
                    ));
                }
                for dependency in evidence.dependencies.iter() {
                    self.validate_normalization_evidence_with(dependency, context, proofs)?;
                }
                let mut replay_stack = NormalizationStack::default();
                push(
                    &self.budget,
                    &mut replay_stack.active,
                    (evidence.key.clone(), opaque.owner()),
                )?;
                let mut replay_dependencies = Vec::new();
                let normal = self.normalize_type_in(
                    &expected,
                    context,
                    &mut replay_stack,
                    &mut replay_dependencies,
                    0,
                )?;
                if !self.eq_ty(&normal, &evidence.normal, 0)?
                    || replay_dependencies.len() != evidence.dependencies.len()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque normalization output changed its public equality",
                    ));
                }
                self.require_associated_assignment_bounds(
                    evidence.key.projection.item,
                    &evidence.key.projection.trait_ref,
                    &evidence.normal,
                    context.proof,
                    proofs,
                )?;
                return Ok(());
            }
            NormalizationOrigin::Dynamic(object) => {
                if !matches!(evidence.membership.0.origin, ProofOrigin::TraitObject)
                    || !self.eq_ty(&evidence.selected_goal.self_ty, &Ty::Dyn(object.clone()), 0)?
                    || !evidence.arguments.is_empty()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "dynamic normalization lost immutable object membership",
                    ));
                }
                let expected = self
                    .dynamic_associated_binding(&evidence.key.projection, context.proof.env())?
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "dynamic normalization lost associated binding",
                        )
                    })?;
                if !self.eq_ty(&expected, &evidence.raw_rhs, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "dynamic normalization binding identity changed",
                    ));
                }
                for dependency in evidence.dependencies.iter() {
                    self.validate_normalization_evidence_with(dependency, context, proofs)?;
                }
                let mut replay_stack = NormalizationStack::default();
                push(
                    &self.budget,
                    &mut replay_stack.active,
                    (evidence.key.clone(), object.principal),
                )?;
                let mut replay_dependencies = Vec::new();
                let normal = self.normalize_type_in(
                    &expected,
                    context,
                    &mut replay_stack,
                    &mut replay_dependencies,
                    0,
                )?;
                if !self.eq_ty(&normal, &evidence.normal, 0)?
                    || replay_dependencies.len() != evidence.dependencies.len()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "dynamic normalization output changed immutable equality",
                    ));
                }
                self.require_associated_assignment_bounds(
                    evidence.key.projection.item,
                    &evidence.key.projection.trait_ref,
                    &evidence.normal,
                    context.proof,
                    proofs,
                )?;
                return Ok(());
            }
        };
        match &evidence.membership.0.origin {
            ProofOrigin::Implementation {
                declaration,
                arguments,
            } if *declaration == implementation
                && self.eq_types(arguments, &evidence.arguments, 0)? => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated normalization evidence lost exact impl substitution",
                ));
            }
        }
        let value = self.associated_value(implementation, evidence.key.projection.item)?;
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
            &evidence.key.projection.trait_ref,
            &evidence.normal,
            ProofContext {
                predicates: &self.empty,
                phase: FactsPhase::Closed,
            },
            proofs,
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

    #[test]
    fn generic_closure_normalization_replays_caller_authority() {
        use crate::compiler::source;
        let options = source::SourceOptions::default();
        let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
        let mut assembler = source::SourceAssembler::new(&options);
        let mut step = assembler
            .provide(Arc::new(source::prepare_text_unit(
                "generic-closure-normalization.rs",
                "fn outer<T>(value:T){let f=move||{let taken=value;7i32};}fn main(){}",
                &options,
            )))
            .unwrap();
        let prepared = loop {
            match step {
                source::AssemblyStep::Done(prepared) => break prepared,
                source::AssemblyStep::Need(need) => {
                    let unit = match need.parent.as_deref() {
                        None => source::prepare_prelude_unit(&options, &expansion),
                        Some(parent) => {
                            source::prepare_child_unit(parent, need.index, &options, &expansion)
                        }
                    };
                    step = assembler.provide(Arc::new(unit)).unwrap();
                }
            }
        };
        let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let (provider, owner) = (0..plan.callable_count())
            .find_map(|index| {
                let provider = plan.callable_decl(index).ok()?;
                plan.stored_closure_owner(provider)
                    .ok()
                    .flatten()
                    .map(|(owner, _)| (provider, owner))
            })
            .unwrap();
        let prepared = plan.prepare_body_constants(owner, &budget).unwrap();
        assert!(prepared.direct().is_empty());
        let constants = plan
            .seal_body_constants(prepared, Vec::new(), &budget)
            .unwrap();
        let (_, closures) = plan
            .publish_callable_body_with_closures(owner, constants, &budget)
            .unwrap();
        let closure = closures
            .iter()
            .find(|closure| closure.ty.provider() == provider)
            .unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = world
            .headers
            .traits
            .iter()
            .find(|header| {
                world.closure_trait_kind(header.declaration).unwrap()
                    == Some(ir::ClosureKind::FnOnce)
            })
            .unwrap()
            .declaration;
        let item = world
            .headers
            .associated_types
            .iter()
            .find(|header| header.trait_decl == declaration)
            .unwrap()
            .declaration;
        let projection = ir::AssociatedTypeProjection {
            trait_ref: ir::TraitRef {
                declaration,
                self_ty: Ty::Closure(closure.ty.clone()),
                args: Arc::from([Ty::Unit]),
            },
            item,
        };
        let facts = world.callable_facts(owner).unwrap();
        let normalized = world
            .normalize_associated_type(&projection, &facts)
            .unwrap();
        normalized.validate_for(&projection, &facts).unwrap();
        assert_eq!(
            normalized.0.membership.0.environment,
            normalized.0.key.environment
        );
        assert_eq!(normalized.0.membership.0.phase, normalized.0.key.phase);
        assert_ne!(normalized.0.key.phase, FactsPhase::Closed);
        let mut wrong_phase = (*normalized.0).clone();
        wrong_phase.key.phase = FactsPhase::Closed;
        let mut wrong_environment = (*normalized.0).clone();
        wrong_environment.membership = ProofEvidence(Arc::new({
            let mut proof = (*wrong_environment.membership.0).clone();
            proof.environment = world.empty.index;
            proof
        }));
        let mut wrong_result = (*normalized.0).clone();
        wrong_result.normal = Ty::Bool;
        let mut wrong_world = (*normalized.0).clone();
        wrong_world.world = fixture().id;
        for forged in [wrong_phase, wrong_environment, wrong_result, wrong_world] {
            assert!(
                NormalizedAssociatedType(Arc::new(forged))
                    .validate_for(&projection, &facts)
                    .is_err()
            );
        }
    }

    #[test]
    fn opaque_bound_normalization_replays_declaration_and_caller_authority() {
        let program = frontend::parse(
            "trait Has{type Output;}struct Number{value:i32}impl Has for Number{type Output=i32;}fn make<T>(arg:T)->impl Has<Output=i32>{Number{value:42}}fn other()->impl Has<Output=i32>{Number{value:42}}fn main(){}",
        ).unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let opaque = world
            .headers
            .callables
            .iter()
            .find_map(|header| match &header.schema.signature.result {
                Ty::Opaque(opaque) if !opaque.owner_args().is_empty() => Some(opaque.clone()),
                _ => None,
            })
            .unwrap();
        let foreign = world
            .headers
            .callables
            .iter()
            .find_map(|header| match &header.schema.signature.result {
                Ty::Opaque(other) if other.owner() != opaque.owner() => Some(other.clone()),
                _ => None,
            })
            .unwrap();
        let facts = world.callable_facts(opaque.owner()).unwrap();
        let bound = &opaque.bounds()[0];
        let projection = ir::AssociatedTypeProjection {
            trait_ref: bound.application(Ty::Opaque(opaque.clone())),
            item: bound.equalities()[0].0,
        };
        let normalized = world
            .normalize_associated_type(&projection, &facts)
            .unwrap();
        normalized.validate_for(&projection, &facts).unwrap();
        assert!(matches!(normalized.ty(), Ty::Int(ir::IntTy::I32)));
        assert!(matches!(
            &normalized.0.implementation,
            NormalizationOrigin::Opaque(_)
        ));
        let foreign_projection = ir::AssociatedTypeProjection {
            trait_ref: foreign.bounds()[0].application(Ty::Opaque(foreign.clone())),
            item: foreign.bounds()[0].equalities()[0].0,
        };
        assert!(
            normalized
                .validate_for(&foreign_projection, &facts)
                .is_err()
        );
        let changed_bound = ir::OpaqueBound::new(
            bound.declaration(),
            bound.args().into(),
            Arc::from([(projection.item, Ty::Bool)]),
        );
        for forged in [
            ir::OpaqueTy::new(
                opaque.owner(),
                opaque.owner_args().into(),
                opaque.site() + 1,
                opaque.bounds().into(),
                opaque.captured_regions().into(),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                opaque.owner_args().into(),
                opaque.site(),
                opaque.bounds().into(),
                Arc::from([ir::SourceRegionId {
                    declaration: opaque.owner(),
                    index: 0,
                }]),
            ),
            ir::OpaqueTy::new(
                opaque.owner(),
                opaque.owner_args().into(),
                opaque.site(),
                Arc::from([changed_bound]),
                opaque.captured_regions().into(),
            ),
        ] {
            let forged = Arc::new(forged);
            let altered = ir::AssociatedTypeProjection {
                trait_ref: forged.bounds()[0].application(Ty::Opaque(forged.clone())),
                item: projection.item,
            };
            assert!(world.normalize_associated_type(&altered, &facts).is_err());
            assert!(normalized.validate_for(&altered, &facts).is_err());
        }
        let mut wrong_owner = (*normalized.0).clone();
        wrong_owner.implementation = NormalizationOrigin::Opaque(foreign);
        let mut wrong_output = (*normalized.0).clone();
        wrong_output.raw_rhs = Ty::Bool;
        let mut wrong_phase = (*normalized.0).clone();
        wrong_phase.key.phase = FactsPhase::Closed;
        let mut wrong_environment = (*normalized.0).clone();
        wrong_environment.membership = ProofEvidence(Arc::new({
            let mut proof = (*wrong_environment.membership.0).clone();
            proof.environment = world.empty.index;
            proof
        }));
        let mut wrong_world = (*normalized.0).clone();
        wrong_world.world = fixture().id;
        for forged in [
            wrong_owner,
            wrong_output,
            wrong_phase,
            wrong_environment,
            wrong_world,
        ] {
            assert!(
                NormalizedAssociatedType(Arc::new(forged))
                    .validate_for(&projection, &facts)
                    .is_err()
            );
        }
    }

    fn fixture() -> Arc<ValidatedTraitWorld> {
        let program = frontend::parse("struct S;trait Has{type Out;type Other;}impl Has for S{type Out=i32;type Other=bool;}fn run()->i32{7}fn main(){}").unwrap();
        PlanCtx::from_program(&program, &PlanBudget::default())
            .unwrap()
            .trait_world()
            .unwrap()
    }

    #[test]
    fn normalized_schema_lookup_is_indexed_and_replays_exact_source_authority() {
        let source = (0..128)
            .map(|index| format!("fn f{index}(x:i32)->i32{{x}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let program = frontend::parse(&format!("{source}\nfn main(){{}}")).unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        // build() rejects duplicate declarations before the sole private
        // construction site enumerates and sorts this exact normalized vector.
        assert_eq!(
            world.normalized_callable_index.len(),
            world.normalized_callables.len()
        );
        assert!(
            world
                .normalized_callable_index
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        for (declaration, position) in &world.normalized_callable_index {
            assert_eq!(
                world
                    .normalized_callables
                    .get(*position)
                    .unwrap()
                    .declaration,
                *declaration
            );
        }
        for header in &world.normalized_callables {
            let before = world.budget.usage_snapshot().work;
            let schema = world
                .normalized_declaration_schema(header.declaration)
                .unwrap();
            let work = world.budget.usage_snapshot().work - before;
            // One unit for each binary-search comparison and the final probe,
            // plus the unchanged parameter-clone work. No source-count scan.
            let comparisons = usize::BITS - world.normalized_callable_index.len().leading_zeros();
            assert!(work <= comparisons as usize + 1 + header.schema.signature.params.len());
            assert!(Arc::ptr_eq(&schema.env, &header.schema.env));
            assert!(
                world
                    .eq_signature(&schema.signature, &header.schema.signature)
                    .unwrap()
            );
            assert!(Arc::ptr_eq(&schema.predicates, &header.schema.predicates));
        }
        let foreign = fixture();
        let foreign_decl = foreign.normalized_callables.last().unwrap().declaration;
        assert!(world.normalized_declaration_schema(foreign_decl).is_err());
        let missing = DeclId {
            snapshot: world.registry,
            index: u64::MAX,
        };
        assert!(world.normalized_declaration_schema(missing).is_err());
    }

    #[test]
    fn normalized_schema_index_does_not_bypass_poisoned_work_authority() {
        let world = fixture();
        let declaration = world.normalized_callables.last().unwrap().declaration;
        assert!(world.budget.charge_work(usize::MAX).is_err());
        let failure = world
            .normalized_declaration_schema(declaration)
            .unwrap_err();
        assert!(failure.is_resource());
    }

    #[test]
    fn normalized_schema_index_miss_preserves_required_item_authority() {
        let program = frontend::parse("trait Has{fn get(x:i32)->i32;}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let item = world.headers.items.last().unwrap();
        assert!(
            !world
                .normalized_callable_index
                .iter()
                .any(|(id, _)| *id == item.declaration)
        );
        let schema = world
            .normalized_declaration_schema(item.declaration)
            .unwrap();
        let authoritative = world.schema.declaration_schema(item.declaration).unwrap();
        assert!(Arc::ptr_eq(&schema.env, &authoritative.env));
        assert!(
            world
                .eq_signature(&schema.signature, &authoritative.signature)
                .unwrap()
        );
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
                    snapshot: SnapshotId::standalone().unwrap(),
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

        // Universal projection defaults now admit these exact headers. The
        // malformed callback identities below must still reject independently.
        let adapter = Arc::new(RhsAdapter {
            inner: world.schema.clone(),
            values: Vec::new(),
            bounds: Vec::new(),
            schemas: schemas.clone(),
        });
        ValidatedTraitWorld::build(headers.clone(), adapter, &world.budget).unwrap();

        for foreign in [false, true] {
            let invalid = if foreign {
                DeclId {
                    snapshot: SnapshotId::standalone().unwrap(),
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
    fn associated_bound_rejects_unproved_trait_even_with_matching_source() {
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
        assert_eq!(error.kind(), FactsErrorKind::Unproved);
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
        // Normalization scratch is node-local, so repeated calls recompute
        // rather than share a cached Arc. Both results must still validate.
        first.validate_for(&projection, &facts).unwrap();
        cached.validate_for(&projection, &facts).unwrap();
        let mut forged = (*cached.0).clone();
        forged.normal = Ty::Slice {
            element: Arc::new(Ty::Bool),
        };
        let error = NormalizedAssociatedType(Arc::new(forged))
            .validate_for(&projection, &facts)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unproved);
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
