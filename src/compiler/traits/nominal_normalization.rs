//! Normalize nominal actuals through their canonical source-application authority.
//! A field rewrite is never permission to reuse the original nominal identity.
use super::*;

/// Exact graph reads admitted by the current graph computation. This is not a
/// parent-local source-field seal and never authorizes minting an identity.
#[derive(Default)]
pub(in crate::compiler) struct GraphInvocationInputs {
    definitions: Vec<Ty>,
    engine: Option<crate::engine::EngineAuthority>,
}

impl GraphInvocationInputs {
    pub(in crate::compiler) fn new(
        definitions: Vec<Ty>,
        engine: Option<crate::engine::EngineAuthority>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Self> {
        if !definitions.is_empty() && engine.is_none() {
            return Err(FactsError::invalid(
                facts.budget(),
                "graph invocation inputs require the current engine authority",
            ));
        }
        let inputs = Self {
            definitions,
            engine,
        };
        let transport = inputs.transport();
        transport.validate_with_scratch(facts.world(), facts, scratch)?;
        Ok(inputs)
    }

    pub(in crate::compiler) fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// The exact admitted rows and engine, for retention in an immutable call
    /// receipt. Reconstruction goes through `new`, which replays every row.
    pub(in crate::compiler) fn rows(&self) -> (&[Ty], Option<crate::engine::EngineAuthority>) {
        (&self.definitions, self.engine)
    }

    #[cfg(test)]
    pub(in crate::compiler) fn select_for_tests(
        &self,
        original: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        self.transport().select(facts.world(), original, facts)
    }

    pub(super) fn transport(&self) -> ParentLocalTransport<'_> {
        ParentLocalTransport {
            definitions: &self.definitions,
            graph_engine: self.engine,
            validation: None,
        }
    }
}

/// Invocation-local input, never retained by the world. Local substitution
/// scratch may pin exact immutable rows, never this borrowed transport. A row
/// is useful only after its private source-field receipt replays under the
/// current caller. Supplying a type here does not confer construction authority.
#[derive(Clone, Copy, Default)]
pub(in crate::compiler) struct ParentLocalTransport<'a> {
    definitions: &'a [Ty],
    graph_engine: Option<crate::engine::EngineAuthority>,
    validation: Option<&'a std::cell::RefCell<ParentLocalValidation>>,
}

/// One computation retains independent metadata sessions for each exact ordered
/// transport row. Full caller context and immutable receipt identity gate reuse.
#[derive(Default)]
pub(in crate::compiler) struct ParentLocalValidation {
    invocations: Vec<ParentLocalInvocation>,
}

struct ParentLocalInvocation {
    context: body_validation::Context,
    definitions: Arc<[Ty]>,
    rows: Vec<NominalReplays>,
}

impl ParentLocalValidation {
    fn validate(
        &mut self,
        transport: ParentLocalTransport<'_>,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        world.check_normalization_budget()?;
        world.check_facts(facts, true)?;
        // Empty discovery invocations own no rows and must not evict the
        // complete sealed invocation that immediately follows them.
        if transport.definitions.is_empty() {
            return transport.validate_rows(world, facts, &mut []);
        }
        for index in 0..self.invocations.len() {
            charged(&world.budget, world.budget.charge_work(1))?;
            let previous = &mut self.invocations[index];
            if previous.context.matches(facts)?
                && schema_instantiation::same_transport(
                    &previous.definitions,
                    transport.definitions,
                    &world.budget,
                    0,
                )?
            {
                let result = transport.validate_rows(world, facts, &mut previous.rows);
                if result.is_err() {
                    // Never retain partially replayed failure state.
                    self.invocations.remove(index);
                }
                return result;
            }
        }
        let context = body_validation::Context::retain(facts)?;
        retain::<Ty>(&world.budget, transport.definitions.len())?;
        let mut invocation = ParentLocalInvocation {
            context,
            definitions: transport.definitions.into(),
            rows: Vec::new(),
        };
        for _ in transport.definitions {
            push(
                &world.budget,
                &mut invocation.rows,
                NominalReplays::default(),
            )?;
        }
        transport.validate_rows(world, facts, &mut invocation.rows)?;
        if self.invocations.len() < MAX_SCRATCH_OBLIGATIONS {
            push(&world.budget, &mut self.invocations, invocation)?;
        }
        Ok(())
    }
}

impl<'a> ParentLocalTransport<'a> {
    pub(in crate::compiler) fn new(definitions: &'a [Ty]) -> Self {
        Self {
            definitions,
            graph_engine: None,
            validation: None,
        }
    }

    pub(in crate::compiler) fn with_validation(
        self,
        validation: &'a std::cell::RefCell<ParentLocalValidation>,
    ) -> Self {
        if self.definitions.is_empty() {
            return self;
        }
        Self {
            validation: self.validation.or(Some(validation)),
            ..self
        }
    }

    pub(super) fn is_empty(self) -> bool {
        self.definitions.is_empty()
    }

    pub(super) fn definitions(self) -> &'a [Ty] {
        self.definitions
    }

    pub(in crate::compiler) fn validate(
        self,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        if let Some(validation) = self.validation
            && let Ok(mut validation) = validation.try_borrow_mut()
        {
            return validation.validate(self, world, facts);
        }
        // Nested proof replay can visit the same invocation. An occupied arena
        // is only a cache miss: perform every validation in fresh local rows.
        world.check_facts(facts, true)?;
        for candidate in self.definitions {
            self.validate_row(world, facts, candidate, &mut NominalReplays::default())?;
        }
        self.validate_graph_conflicts(world)
    }

    pub(super) fn validate_with_replay(
        self,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
        validation: &mut ParentLocalValidation,
    ) -> Result<()> {
        // This is an owned operation arena, not an admission shortcut. The
        // existing exact caller/ordered-seal key is checked before replaying
        // every row with its retained metadata and receipt proof scratch.
        validation.validate(self, world, facts)
    }

    pub(super) fn validate_with_scratch(
        self,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        world.check_facts(facts, true)?;
        scratch.for_facts(facts);
        self.validate_with_replay(
            world,
            facts,
            &mut scratch.for_world(world.id).transport_validation,
        )
    }

    fn validate_rows(
        self,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
        rows: &mut [NominalReplays],
    ) -> Result<()> {
        world.check_facts(facts, true)?;
        if rows.len() != self.definitions.len() {
            return Err(FactsError::invalid(
                &world.budget,
                "parent-local transport replay row count changed",
            ));
        }
        for (candidate, replays) in self.definitions.iter().zip(rows) {
            self.validate_row(world, facts, candidate, replays)?;
        }
        self.validate_graph_conflicts(world)
    }

    fn validate_graph_conflicts(self, world: &ValidatedTraitWorld) -> Result<()> {
        if self.graph_engine.is_none() {
            return Ok(());
        }
        for (index, left) in self.definitions.iter().enumerate() {
            for right in &self.definitions[index + 1..] {
                charged(&world.budget, world.budget.charge_work(1))?;
                if world.types_equal(std::slice::from_ref(left), std::slice::from_ref(right))?
                    && !schema_instantiation::same_inputs(
                        std::slice::from_ref(left),
                        std::slice::from_ref(right),
                        &world.budget,
                        0,
                    )?
                {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "graph invocation has conflicting application identities",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_row(
        self,
        world: &ValidatedTraitWorld,
        facts: &ValidatedFacts,
        candidate: &Ty,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        charged(&world.budget, world.budget.charge_work(1))?;
        if let Some(engine) = self.graph_engine {
            let read = match candidate {
                Ty::Struct(value)
                    if !value.is_back_edge()
                        && value.id().index.is_graph()
                        && value.parent_local_receipt().is_none() =>
                {
                    value.realization()
                }
                Ty::Enum(value)
                    if !value.is_back_edge()
                        && value.id().index.is_graph()
                        && value.parent_local_receipt().is_none() =>
                {
                    value.realization()
                }
                _ => None,
            }
            .ok_or_else(|| {
                FactsError::invalid(
                    &world.budget,
                    "graph invocation transport requires an exact typed graph read",
                )
            })?;
            if read.engine_authority() != engine {
                return Err(FactsError::invalid(
                    &world.budget,
                    "foreign engine graph invocation transport",
                ));
            }
            read.check_definition(candidate)
                .map_err(|error| FactsError::from_message(&world.budget, &error.to_string()))?;
            read.replay_with(candidate, world, replays)
                .map_err(|error| FactsError::from_message(&world.budget, &error.to_string()))?;
            world.validate_ty_metadata_with_facts(
                candidate,
                facts.env(),
                0,
                replays,
                Some(facts),
            )?;
            return Ok(());
        }
        let sealed = match candidate {
            Ty::Struct(value) => !value.is_back_edge() && value.parent_local_receipt().is_some(),
            Ty::Enum(value) => !value.is_back_edge() && value.parent_local_receipt().is_some(),
            _ => false,
        };
        if !sealed {
            return Err(FactsError::invalid(
                &world.budget,
                "parent-local invocation transport requires sealed nominal handles",
            ));
        }
        world.validate_ty_metadata_with_facts(candidate, facts.env(), 0, replays, Some(facts))?;
        Ok(())
    }

    pub(in crate::compiler) fn select(
        self,
        world: &ValidatedTraitWorld,
        original: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        self.select_with_replays(world, original, facts, &mut NominalReplays::default())
    }

    pub(in crate::compiler) fn select_with_replays(
        self,
        world: &ValidatedTraitWorld,
        original: &Ty,
        facts: &ValidatedFacts,
        replays: &mut NominalReplays,
    ) -> Result<Option<Ty>> {
        world.check_facts(facts, true)?;
        self.validate(world, facts)?;
        world.validate_ty_metadata_with_facts(original, facts.env(), 0, replays, Some(facts))?;
        let source = match original {
            Ty::Struct(value) if !value.is_back_edge() => value.def().source_decl,
            Ty::Enum(value) if !value.is_back_edge() => value.def().source_decl,
            _ => return Ok(None),
        };
        if self.graph_engine.is_some() {
            let parent_local = match original {
                Ty::Struct(value) => value.parent_local_receipt().is_some(),
                Ty::Enum(value) => value.parent_local_receipt().is_some(),
                _ => false,
            };
            if parent_local {
                return Err(FactsError::invalid(
                    &world.budget,
                    "graph invocation cannot replace a parent-local source seal",
                ));
            }
            let original_graph = match original {
                Ty::Struct(value) => value.id().index.is_graph(),
                Ty::Enum(value) => value.id().index.is_graph(),
                _ => false,
            };
            let mut selected: Option<Ty> = None;
            for candidate in self.definitions {
                charged(&world.budget, world.budget.charge_work(1))?;
                let same_kind = matches!(
                    (original, candidate),
                    (Ty::Struct(_), Ty::Struct(_)) | (Ty::Enum(_), Ty::Enum(_))
                );
                let declaration = match candidate {
                    Ty::Struct(value) => value.def().source_decl,
                    Ty::Enum(value) => value.def().source_decl,
                    _ => unreachable!("validated graph invocation row"),
                };
                if !same_kind
                    || declaration != source
                    || !world.types_equal(type_arguments(candidate), type_arguments(original))?
                {
                    continue;
                }
                if original_graph
                    && !schema_instantiation::same_inputs(
                        std::slice::from_ref(original),
                        std::slice::from_ref(candidate),
                        &world.budget,
                        0,
                    )?
                {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "graph invocation cannot replace an existing typed read",
                    ));
                }
                if let Some(previous) = &selected {
                    if !schema_instantiation::same_inputs(
                        std::slice::from_ref(previous),
                        std::slice::from_ref(candidate),
                        &world.budget,
                        0,
                    )? {
                        return Err(FactsError::invalid(
                            &world.budget,
                            "graph invocation has conflicting application identities",
                        ));
                    }
                } else {
                    selected = Some(candidate.clone());
                }
            }
            return Ok(selected);
        }
        let mut selected: Option<Ty> = None;
        for candidate in self.definitions {
            charged(&world.budget, world.budget.charge_work(1))?;
            let (declaration, receipt) = match candidate {
                Ty::Struct(value) => (value.def().source_decl, value.parent_local_receipt()),
                Ty::Enum(value) => (value.def().source_decl, value.parent_local_receipt()),
                _ => {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "parent-local invocation transport contains a non-nominal row",
                    ));
                }
            };
            if receipt.is_none() {
                return Err(FactsError::invalid(
                    &world.budget,
                    "parent-local invocation transport lost its source-field receipt",
                ));
            }
            // self.validate above authenticated every immutable row, including
            // nonmatching rows, under these exact current facts. Replaying the
            // same row a second time here adds no authority or mutation check.
            if declaration != source
                || !same_types(type_arguments(candidate), type_arguments(original))
            {
                continue;
            }
            if let Some(previous) = &selected {
                if !same_types(
                    std::slice::from_ref(previous),
                    std::slice::from_ref(candidate),
                ) {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "parent-local invocation transport has conflicting application identities",
                    ));
                }
            } else {
                selected = Some(candidate.clone());
            }
        }
        Ok(selected)
    }
}

/// Immutable identity only. No world, validated facts, or planning meter is
/// retained by a parent-local source application.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct ParentLocalContext {
    world: WorldId,
    subject: FactsSubject,
    environment: Arc<GenericEnv>,
    predicates: usize,
    phase: FactsPhase,
    closure: Option<Arc<ir::ClosureTy>>,
    mode: Option<ir::ClosureKind>,
}

impl ValidatedFacts {
    /// Metadata-only simultaneous substitution for the existing parent-local
    /// issuer. This is not a callable signature proof: ordinary discharge must
    /// still check every owning predicate before admitting the resulting call.
    pub(in crate::compiler) fn parent_local_source_signature(
        &self,
        callee: &Callee,
    ) -> Result<Signature> {
        self.world.check_facts(self, true)?;
        let (schema, arguments) = match callee {
            Callee::Direct(target) => {
                let mut arguments = Vec::new();
                for argument in target.type_args.iter() {
                    push(self.budget(), &mut arguments, argument.clone())?;
                }
                (self.world.callable_schema(target.declaration)?, arguments)
            }
            Callee::TraitItem(call) => {
                let item = self.world.item_header(call.item)?;
                if item.trait_decl != call.trait_ref.declaration {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "parent-local source signature changed trait item owner",
                    ));
                }
                let mut arguments = self.world.trait_arguments(&call.trait_ref)?;
                for argument in call.method_args.iter() {
                    push(self.budget(), &mut arguments, argument.clone())?;
                }
                retain::<Ty>(self.budget(), item.schema.signature.params.len())?;
                (item.schema.clone(), arguments)
            }
            _ => {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "parent-local source signature requires an exact direct or trait item",
                ));
            }
        };
        for argument in &arguments {
            self.validate_ty_metadata(argument)?;
        }
        if arguments.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                self.budget(),
                "parent-local source signature changed ordered argument arity",
            ));
        }
        self.world
            .schema
            .instantiate_schema(&schema, &arguments, self.env(), self.budget())
            .map(|instantiated| instantiated.signature)
    }

    pub(in crate::compiler) fn parent_local_signature_inputs(
        &self,
        signature: &Signature,
    ) -> Result<Vec<Ty>> {
        self.parent_local_signature_inputs_with_scratch(signature, &mut TraitScratch::default())
    }

    pub(super) fn parent_local_signature_inputs_with_scratch(
        &self,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<Ty>> {
        self.world.check_facts(self, true)?;
        let mut result = Vec::new();
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.world.collect_parent_local_inputs(ty, &mut result, 0)?;
        }
        ParentLocalTransport::new(&result).validate_with_scratch(&self.world, self, scratch)?;
        Ok(result)
    }

    pub(in crate::compiler) fn parent_local_call_inputs(
        &self,
        callee: &Callee,
        signature: &Signature,
    ) -> Result<Vec<Ty>> {
        self.parent_local_call_inputs_with_scratch(callee, signature, &mut TraitScratch::default())
    }

    pub(in crate::compiler) fn parent_local_call_inputs_with_scratch(
        &self,
        callee: &Callee,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<Ty>> {
        let mut result = self.parent_local_signature_inputs_with_scratch(signature, scratch)?;
        for actual in callee.metadata_types() {
            self.validate_ty_metadata_scratch(actual, self.budget(), scratch)?;
            self.world
                .collect_parent_local_inputs(actual, &mut result, 0)?;
        }
        ParentLocalTransport::new(&result).validate_with_scratch(&self.world, self, scratch)?;
        Ok(result)
    }

    pub(in crate::compiler) fn parent_local_receiver_inputs(
        &self,
        receiver: &Ty,
    ) -> Result<Vec<Ty>> {
        self.parent_local_receiver_inputs_with_scratch(receiver, &mut TraitScratch::default())
    }

    pub(in crate::compiler) fn parent_local_receiver_inputs_with_scratch(
        &self,
        receiver: &Ty,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<Ty>> {
        self.world.check_facts(self, true)?;
        scratch.for_facts(self);
        self.parent_local_receiver_inputs_with_active(receiver, scratch.for_world(self.world.id))
    }

    pub(super) fn parent_local_receiver_inputs_with_active(
        &self,
        receiver: &Ty,
        active: &mut ProofScratch,
    ) -> Result<Vec<Ty>> {
        // The receiver transports existing issuer handles only. Metadata or
        // checked fields without a private source-field seal cannot mint one.
        self.world.check_facts(self, true)?;
        self.world.validate_ty_metadata_with_facts(
            receiver,
            self.env(),
            0,
            &mut active.replays,
            Some(self),
        )?;
        let mut result = Vec::new();
        self.world
            .collect_parent_local_inputs(receiver, &mut result, 0)?;
        ParentLocalTransport::new(&result).validate_with_replay(
            &self.world,
            self,
            &mut active.transport_validation,
        )?;
        Ok(result)
    }

    pub(in crate::compiler) fn validate_source_type_closing_context(
        &self,
        source: &ValidatedFacts,
        env: &GenericEnv,
        arguments: &[Ty],
    ) -> Result<bool> {
        if source.phase() != FactsPhase::Closed {
            self.validate_parent_local_closing_source(
                source,
                source.declaration(),
                env,
                arguments,
            )?;
            return Ok(false);
        }
        self.world.check_facts(self, true)?;
        self.world.check_facts(source, true)?;
        let same_target = matches!((&source.subject, &self.subject),
            (FactsSubject::ClosedInstance(original), FactsSubject::ClosedInstance(current))
            if original.declaration == current.declaration
                && schema_instantiation::same_inputs(&current.type_args, arguments, self.budget(), 0)?);
        if !Arc::ptr_eq(&self.world, &source.world)
            || !self.budget().same_authority(source.budget())
            || !std::ptr::eq(source.env().as_ref(), env)
            || !Arc::ptr_eq(source.env(), self.env())
            || !same_target
            || !body_validation::Context::retain(source)?.matches(self)?
        {
            return Err(FactsError::invalid(
                self.budget(),
                "closed source type substitution changed exact current instance authority",
            ));
        }
        Ok(true)
    }

    pub(in crate::compiler) fn validate_parent_local_closing_source(
        &self,
        source: &ValidatedFacts,
        parent: DeclId,
        env: &GenericEnv,
        arguments: &[Ty],
    ) -> Result<()> {
        self.world.check_facts(self, true)?;
        self.world.check_facts(source, true)?;
        let exact_parent = matches!((&source.subject, &self.subject),
            (FactsSubject::UniversalCallable(original), FactsSubject::ClosedInstance(target))
            if *original == parent && target.declaration == parent
                && schema_instantiation::same_inputs(&target.type_args, arguments, self.budget(), 0)?);
        if !Arc::ptr_eq(&self.world, &source.world)
            || !self.budget().same_authority(source.budget())
            || !std::ptr::eq(source.env().as_ref(), env)
            || source.phase() != FactsPhase::Universal
            || self.phase() != FactsPhase::Closed
            || !exact_parent
        {
            return Err(FactsError::invalid(
                self.budget(),
                "parent-local closing substitution changed exact source parent or instance",
            ));
        }
        Ok(())
    }

    pub(in crate::compiler) fn parent_local_context(&self) -> Result<ParentLocalContext> {
        self.world.check_facts(self, true)?;
        retain::<ParentLocalContext>(self.budget(), 1)?;
        Ok(ParentLocalContext {
            world: self.world.id,
            subject: self.subject.clone(),
            environment: self.env().clone(),
            predicates: self.predicates.index,
            phase: self.phase(),
            closure: self.closure.clone(),
            mode: self.closure_mode,
        })
    }
}

impl ValidatedTraitWorld {
    fn collect_parent_local_inputs(
        &self,
        ty: &Ty,
        result: &mut Vec<Ty>,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let back_edge = match ty {
            Ty::Struct(value) if value.is_back_edge() => Some(value.id()),
            Ty::Enum(value) if value.is_back_edge() => Some(value.id()),
            _ => None,
        };
        if let Some(identity) = back_edge {
            if matches!(identity.index, ir::NominalIndex::ParentLocal { .. }) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "parent-local invocation back edge cannot carry a source-field receipt",
                ));
            }
            return Ok(());
        }
        let nominal = match ty {
            Ty::Struct(value) => Some((
                value.parent_local_receipt().is_some()
                    || matches!(value.id().index, ir::NominalIndex::ParentLocal { .. }),
                &value.def().type_args,
            )),
            Ty::Enum(value) => Some((
                value.parent_local_receipt().is_some()
                    || matches!(value.id().index, ir::NominalIndex::ParentLocal { .. }),
                &value.def().type_args,
            )),
            _ => None,
        };
        if let Some((sealed, arguments)) = nominal {
            if sealed {
                push(&self.budget, result, ty.clone())?;
            }
            for argument in arguments.iter() {
                self.collect_parent_local_inputs(argument, result, depth + 1)?;
            }
            return Ok(());
        }
        match ty {
            Ty::Tuple(parts) => {
                for part in parts.iter() {
                    self.collect_parent_local_inputs(part, result, depth + 1)?;
                }
            }
            Ty::Ref {
                referent: child, ..
            }
            | Ty::RawPtr { pointee: child, .. }
            | Ty::Array { element: child, .. }
            | Ty::DependentArray { element: child, .. }
            | Ty::Slice { element: child } => {
                self.collect_parent_local_inputs(child, result, depth + 1)?
            }
            Ty::Cell(wrapper) => {
                self.collect_parent_local_inputs(wrapper.argument(), result, depth + 1)?
            }
            Ty::ManuallyDrop(wrapper) => {
                self.collect_parent_local_inputs(wrapper.argument(), result, depth + 1)?
            }
            Ty::PhantomData(wrapper) => {
                self.collect_parent_local_inputs(wrapper.argument(), result, depth + 1)?
            }
            Ty::FnPointer(pointer) => {
                for child in pointer
                    .inputs
                    .iter()
                    .chain(std::iter::once(&pointer.output))
                {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
            }
            Ty::Closure(closure) => {
                for child in closure.types() {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
            }
            Ty::FnItem(target) => {
                for child in target.type_args.iter() {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
            }
            Ty::Dyn(object) => {
                for child in object.types() {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
            }
            Ty::AssociatedType(projection) => {
                self.collect_parent_local_inputs(&projection.trait_ref.self_ty, result, depth + 1)?;
                for child in projection.trait_ref.args.iter() {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
            }
            Ty::Opaque(opaque) => {
                for child in opaque.owner_args() {
                    self.collect_parent_local_inputs(child, result, depth + 1)?;
                }
                for bound in opaque.bounds() {
                    for argument in bound.args() {
                        if let ir::OpaqueBoundArg::Type(child) = argument {
                            self.collect_parent_local_inputs(child, result, depth + 1)?;
                        }
                    }
                    for (_, child) in bound.equalities() {
                        self.collect_parent_local_inputs(child, result, depth + 1)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl ParentLocalContext {
    pub(in crate::compiler) fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        if !self.matches(facts)? {
            return Err(FactsError::invalid(
                facts.budget(),
                "parent-local source receipt changed exact owning facts context",
            ));
        }
        Ok(())
    }

    pub(in crate::compiler) fn matches(&self, facts: &ValidatedFacts) -> Result<bool> {
        facts.world.check_facts(facts, true)?;
        charged(facts.budget(), facts.budget().charge_work(1))?;
        Ok(!(self.world != facts.world.id
            || self.subject != facts.subject
            || !Arc::ptr_eq(&self.environment, facts.env())
            || self.predicates != facts.predicates.index
            || self.phase != facts.phase()
            || self.closure != facts.closure
            || self.mode != facts.closure_mode))
    }
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ParentLocalField {
    context: ParentLocalContext,
    variant: Option<usize>,
    field: usize,
    source: Ty,
    normalized: Ty,
    reductions: Arc<[(ir::AssociatedTypeProjection, NormalizedAssociatedType)]>,
}

enum ParentLocalReductions<'a> {
    Issue(&'a mut Vec<(ir::AssociatedTypeProjection, NormalizedAssociatedType)>),
    Replay(
        std::slice::Iter<'a, (ir::AssociatedTypeProjection, NormalizedAssociatedType)>,
        &'a mut TraitScratch,
    ),
}

impl ParentLocalReductions<'_> {
    fn normalize(
        &mut self,
        projection: ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<Ty> {
        match self {
            Self::Issue(rows) => {
                let proof = facts.world.normalize_associated_type(&projection, facts)?;
                let result = proof.ty().clone();
                push(facts.budget(), rows, (projection, proof))?;
                Ok(result)
            }
            Self::Replay(rows, scratch) => {
                let (original, proof) = rows.next().ok_or_else(|| {
                    FactsError::invalid(
                        facts.budget(),
                        "parent-local field omitted source reduction",
                    )
                })?;
                if !facts.world.same_projection(original, &projection)? {
                    return Err(FactsError::invalid(
                        facts.budget(),
                        "parent-local field changed source reduction order or projection",
                    ));
                }
                // This is the original immutable assignment evidence, not a
                // newly inferred proof from the normalized storage type.
                proof.validate_for_scratch(&projection, facts, scratch)?;
                Ok(proof.ty().clone())
            }
        }
    }
}

impl ParentLocalField {
    pub(in crate::compiler) fn ty(&self) -> &Ty {
        &self.normalized
    }

    pub(in crate::compiler) fn replay(
        &self,
        template: &Ty,
        env: &Arc<ir::ParamEnv>,
        actuals: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.context.validate_for(facts)?;
        facts.validate_ty_metadata_scratch(&self.normalized, facts.budget(), scratch)?;
        let (source_env, source) = facts.parent_local_field_source(
            template,
            env,
            actuals,
            self.variant,
            self.field,
            Some(scratch),
        )?;
        if !facts.world.eq_ty(&self.source, &source, 0)? {
            return Err(FactsError::invalid(
                facts.budget(),
                "parent-local field changed exact original source",
            ));
        }
        let mut reductions = ParentLocalReductions::Replay(self.reductions.iter(), scratch);
        let normalized =
            facts.parent_local_source_term(&source, &source_env, actuals, &mut reductions, 0)?;
        if let ParentLocalReductions::Replay(remaining, _) = &reductions
            && remaining.len() != 0
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "parent-local field added unused source reductions",
            ));
        }
        facts.validate_ty_scratch(&normalized, scratch)?;
        if !facts.world.eq_ty(&self.normalized, &normalized, 0)? {
            return Err(FactsError::invalid(
                facts.budget(),
                "parent-local field changed exact source or normalized aggregate",
            ));
        }
        Ok(())
    }
}

impl ValidatedFacts {
    pub(in crate::compiler) fn parent_local_field(
        &self,
        template: &Ty,
        env: &Arc<ir::ParamEnv>,
        actuals: &[Ty],
        variant: Option<usize>,
        field: usize,
    ) -> Result<ParentLocalField> {
        let (source_env, source) =
            self.parent_local_field_source(template, env, actuals, variant, field, None)?;
        let mut reductions = Vec::new();
        let normalized = self.parent_local_source_term(
            &source,
            &source_env,
            actuals,
            &mut ParentLocalReductions::Issue(&mut reductions),
            0,
        )?;
        self.validate_ty(&normalized)?;
        retain::<ParentLocalField>(self.budget(), 1)?;
        Ok(ParentLocalField {
            context: self.parent_local_context()?,
            variant,
            field,
            source,
            normalized,
            reductions: reductions.into(),
        })
    }

    fn parent_local_field_source(
        &self,
        template: &Ty,
        env: &Arc<ir::ParamEnv>,
        actuals: &[Ty],
        variant: Option<usize>,
        field: usize,
        mut scratch: Option<&mut TraitScratch>,
    ) -> Result<(GenericEnv, Ty)> {
        self.world.check_facts(self, true)?;
        let original_application = match template {
            Ty::Struct(value) => {
                value.parent_local_receipt().is_none() && value.id().index.source_slot().is_ok()
            }
            Ty::Enum(value) => {
                value.parent_local_receipt().is_none() && value.id().index.source_slot().is_ok()
            }
            _ => false,
        };
        if !original_application
            || type_arguments(template).len() != env.arity()
            || !type_arguments(template).iter().enumerate().all(|(index, argument)| {
                matches!(argument, Ty::Param(parameter) if parameter.owner == env.binder() && usize::from(parameter.index) == index)
            })
        {
            return Err(FactsError::invalid(self.budget(), "parent-local field requires unreceipted original symbolic source"));
        }
        let source_env = GenericEnv::new(std::slice::from_ref(env), self.budget())
            .map_err(|error| FactsError::from_message(self.budget(), &error))?;
        self.world
            .schema
            .validate_nominal(template, &source_env, self.budget())?;
        let (declaration, fields) = match (template, variant) {
            (Ty::Struct(value), None) => (value.def().source_decl, &value.def().fields),
            (Ty::Enum(value), Some(variant)) => (
                value.def().source_decl,
                &value
                    .def()
                    .variants
                    .get(variant)
                    .ok_or_else(|| {
                        FactsError::invalid(self.budget(), "parent-local source variant missing")
                    })?
                    .fields,
            ),
            _ => {
                return Err(FactsError::invalid(
                    self.budget(),
                    "parent-local source field kind changed",
                ));
            }
        };
        if declaration != env.binder().declaration || actuals.len() != env.arity() {
            return Err(FactsError::invalid(
                self.budget(),
                "parent-local source binder or arity changed",
            ));
        }
        for actual in actuals {
            if let Some(scratch) = scratch.as_deref_mut() {
                self.validate_ty_metadata_scratch(actual, self.budget(), scratch)?;
            } else {
                self.validate_ty_metadata(actual)?;
            }
        }
        let fields = nominal_fields(fields, self.budget())?;
        let source = fields
            .get(field)
            .ok_or_else(|| FactsError::invalid(self.budget(), "parent-local source field missing"))?
            .ty
            .clone();
        Ok((source_env, source))
    }

    fn parent_local_source_term(
        &self,
        source: &Ty,
        env: &GenericEnv,
        actuals: &[Ty],
        reductions: &mut ParentLocalReductions<'_>,
        depth: usize,
    ) -> Result<Ty> {
        charged(self.budget(), self.budget().check_depth(depth))?;
        charged(self.budget(), self.budget().charge_work(1))?;
        let mut child =
            |ty: &Ty| self.parent_local_source_term(ty, env, actuals, reductions, depth + 1);
        let result = match source {
            Ty::Param(parameter) => actuals
                .get(
                    env.ordered_params()
                        .position(|original| original == *parameter)
                        .ok_or_else(|| {
                            FactsError::invalid(
                                self.budget(),
                                "foreign parent-local source field binder",
                            )
                        })?,
                )
                .cloned()
                .ok_or_else(|| {
                    FactsError::invalid(self.budget(), "parent-local field actual missing")
                })?,
            Ty::AssociatedType(projection) => {
                let mut application = projection.trait_ref.clone();
                application.self_ty = child(&application.self_ty)?;
                let mut args = Vec::new();
                for argument in application.args.iter() {
                    push(self.budget(), &mut args, child(argument)?)?;
                }
                application.args = args.into();
                let projection = ir::AssociatedTypeProjection {
                    trait_ref: application,
                    item: projection.item,
                };
                reductions.normalize(projection, self)?
            }
            Ty::Struct(_) | Ty::Enum(_) => {
                let mut args = Vec::new();
                for argument in type_arguments(source).iter() {
                    push(self.budget(), &mut args, child(argument)?)?;
                }
                let reapplied =
                    self.world
                        .schema
                        .reapply_nominal(source, &args, env, self.budget())?;
                self.world.normalize_ty(&reapplied, self)?
            }
            Ty::Ref { kind, referent } => Ty::Ref {
                kind: *kind,
                referent: Arc::new(child(referent)?),
            },
            Ty::RawPtr { kind, pointee } => Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(child(pointee)?),
            },
            Ty::Array { element, len } => Ty::Array {
                element: Arc::new(child(element)?),
                len: *len,
            },
            Ty::Slice { element } => Ty::Slice {
                element: Arc::new(child(element)?),
            },
            Ty::Tuple(fields) => {
                let mut result = Vec::new();
                for field in fields.iter() {
                    push(self.budget(), &mut result, child(field)?)?;
                }
                Ty::Tuple(result.into())
            }
            Ty::Cell(value) => Ty::Cell(value.try_map(
                self.budget(),
                |error| FactsError::from_message(self.budget(), &error),
                &mut child,
            )?),
            Ty::ManuallyDrop(value) => Ty::ManuallyDrop(value.try_map(
                self.budget(),
                |error| FactsError::from_message(self.budget(), &error),
                &mut child,
            )?),
            Ty::PhantomData(value) => Ty::PhantomData(value.try_map(
                self.budget(),
                |error| FactsError::from_message(self.budget(), &error),
                &mut child,
            )?),
            Ty::FnPointer(value) => Ty::FnPointer(value.try_map(
                self.budget(),
                |error| FactsError::from_message(self.budget(), &error),
                &mut child,
            )?),
            Ty::DependentArray { .. } | Ty::Opaque(_) => {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unsupported: parent-local projected storage source form",
                ));
            }
            other => other.clone(),
        };
        Ok(result)
    }
}

/// Mutable construction state belongs to one normalization computation, not a
/// fact. Borrow it only during adapter calls, never across a recursive reducer.
#[derive(Default)]
pub(super) struct NominalOperations {
    session: std::cell::RefCell<Option<(WorldId, Box<dyn NominalValidationSession>)>>,
    selection: std::cell::RefCell<NominalReplays>,
    // Shared by all signature/predicate roots of this retained operation.
    // ParentLocalValidation still authenticates the complete caller and exact
    // ordered receipt allocations before every complete row replay.
    transport: std::cell::RefCell<ParentLocalValidation>,
}
impl NominalOperations {
    pub(super) fn select(
        &self,
        transport: ParentLocalTransport<'_>,
        world: &ValidatedTraitWorld,
        original: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        let transport = self.transport(transport);
        if let Ok(mut replays) = self.selection.try_borrow_mut() {
            transport.select_with_replays(world, original, facts, &mut replays)
        } else {
            // Reentrant selection must replay in a separate complete arena.
            transport.select(world, original, facts)
        }
    }

    pub(super) fn transport<'a>(
        &'a self,
        transport: ParentLocalTransport<'a>,
    ) -> ParentLocalTransport<'a> {
        // An outer invocation arena remains authoritative during recursive
        // reducers. Reentrant borrowing uses the existing full fresh fallback.
        transport.with_validation(&self.transport)
    }

    fn with_session<T>(
        &self,
        world: &ValidatedTraitWorld,
        operation: impl FnOnce(Option<&mut (dyn NominalValidationSession + 'static)>) -> Result<T>,
    ) -> Result<T> {
        world.check_nominal_replay_issuer()?;
        charged(&world.budget, world.budget.check_depth(0))?;
        if let Ok(mut retained) = self.session.try_borrow_mut() {
            if retained.as_ref().is_none_or(|(id, _)| *id != world.id) {
                *retained = world
                    .schema
                    .nominal_validation_session()?
                    .map(|session| (world.id, session));
            }
            operation(retained.as_mut().map(|(_, session)| session.as_mut()))
        } else {
            // A recursive adapter may reenter this operation. A fresh full
            // validation arena is always safe; no borrow panic or skipped check.
            let mut fresh = world.schema.nominal_validation_session()?;
            operation(fresh.as_deref_mut())
        }
    }
    fn validate(
        &self,
        world: &ValidatedTraitWorld,
        ty: &Ty,
        env: &GenericEnv,
        facts: Option<&ValidatedFacts>,
    ) -> Result<()> {
        if let Some(facts) = facts {
            world.check_facts(facts, true)?;
        }
        self.with_session(world, |session| {
            if let Some(session) = session
                && session.validate_full(ty, env, facts, &world.budget)?
            {
                return Ok(());
            }
            match facts {
                Some(facts) => world
                    .schema
                    .validate_nominal_with_facts(ty, facts, &world.budget),
                None => world.schema.validate_nominal(ty, env, &world.budget),
            }
        })
    }
    fn validate_node(
        &self,
        world: &ValidatedTraitWorld,
        ty: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        world.check_facts(facts, true)?;
        self.with_session(world, |session| match session {
            Some(session) => session.replay_node(ty, facts.env(), Some(facts), &world.budget),
            None => world
                .schema
                .validate_nominal_node_with_facts(ty, facts, &world.budget),
        })
    }
    fn reapply(
        &self,
        world: &ValidatedTraitWorld,
        original: &Ty,
        arguments: &[Ty],
        env: &GenericEnv,
        facts: Option<&ValidatedFacts>,
    ) -> Result<Ty> {
        if let Some(facts) = facts {
            world.check_facts(facts, true)?;
        }
        self.with_session(world, |session| {
            if let Some(session) = session
                && let Some(result) =
                    session.reapply(original, arguments, env, facts, &world.budget)?
            {
                return Ok(result);
            }
            match facts {
                Some(facts) => world.schema.reapply_nominal_with_facts(
                    original,
                    arguments,
                    facts,
                    &world.budget,
                ),
                None => world
                    .schema
                    .reapply_nominal(original, arguments, env, &world.budget),
            }
        })
    }
}

impl ValidatedTraitWorld {
    pub(super) fn normalize_nominal_actuals(
        &self,
        original: &Ty,
        env: &GenericEnv,
        depth: usize,
        normalize: impl FnMut(&Ty) -> Result<Ty>,
    ) -> Result<Ty> {
        self.normalize_nominal_actuals_with_facts(original, env, depth, None, normalize)
    }

    pub(super) fn normalize_nominal_actuals_with_facts(
        &self,
        original: &Ty,
        env: &GenericEnv,
        depth: usize,
        facts: Option<&ValidatedFacts>,
        normalize: impl FnMut(&Ty) -> Result<Ty>,
    ) -> Result<Ty> {
        self.normalize_nominal_actuals_for_use(original, env, depth, facts, true, normalize)
    }

    pub(super) fn normalize_nominal_actuals_for_use(
        &self,
        original: &Ty,
        env: &GenericEnv,
        depth: usize,
        facts: Option<&ValidatedFacts>,
        executable_storage: bool,
        normalize: impl FnMut(&Ty) -> Result<Ty>,
    ) -> Result<Ty> {
        self.normalize_nominal_actuals_in(
            original,
            env,
            depth,
            facts,
            executable_storage,
            &NominalOperations::default(),
            normalize,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn normalize_nominal_actuals_in(
        &self,
        original: &Ty,
        env: &GenericEnv,
        depth: usize,
        facts: Option<&ValidatedFacts>,
        executable_storage: bool,
        operations: &NominalOperations,
        mut normalize: impl FnMut(&Ty) -> Result<Ty>,
    ) -> Result<Ty> {
        if let Some(facts) = facts {
            self.check_facts(facts, true)?;
            // Input metadata is walked by the owning normalization context.
            // Authenticate this immutable definition under the current caller
            // even when no projection needs rewriting, without walking every
            // nested definition again for each parent in that same traversal.
            operations.validate_node(self, original, facts)?;
        }
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match original {
            Ty::Struct(_) | Ty::Enum(_) => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization requires an original nominal application",
                ));
            }
        }
        let arguments = type_arguments(original);
        // Preserve the existing no-projection fast path. Callers validate raw
        // schema metadata independently before requesting normalization.
        if !self.type_contains_projection(original, depth + 1)? {
            return Ok(original.clone());
        }
        operations.validate(self, original, env, facts)?;
        let mut normalized = Vec::new();
        for argument in arguments.iter() {
            push(&self.budget, &mut normalized, normalize(argument)?)?;
        }
        // A logical source application denotes this immutable declaration and
        // its ordered actuals, not normalized physical fields. After the full
        // current metadata replay and recursive argument normalization above,
        // an exact no-op must preserve its original sealed allocation. Calling
        // the source interner again would replace an authenticated actual with
        // a merely equivalent definition and discard receipt identity.
        if !executable_storage
            && schema_instantiation::same_inputs(arguments, &normalized, &self.budget, depth + 1)?
        {
            return Ok(original.clone());
        }
        // This callback must remint from the immutable source declaration. It
        // receives the original canonical application, not a reusable numeric
        // ID or caller-supplied replacement field metadata.
        let result = operations.reapply(self, original, &normalized, env, facts)?;
        self.validate_reapplied_nominal_in(
            original,
            &normalized,
            &result,
            env,
            depth,
            facts,
            operations,
        )?;
        // Logical trait operands identify a canonical source declaration and
        // ordered actuals, not an executable field layout. Its immutable source
        // fields were authenticated above. Only executable normalization may
        // demand or produce a storage-normalized field representation.
        if !executable_storage {
            return Ok(result);
        }
        // Reapplication authenticates source metadata, not executable storage.
        // Every source field must separately survive the owning normalization
        // context. Never overwrite a field on an existing nominal identity.
        let mut check_field = |field: &Ty| -> Result<()> {
            let normalized = normalize(field)?;
            if !self.eq_ty(field, &normalized, depth + 1)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: nominal storage projection requires canonical field reminting",
                ));
            }
            Ok(())
        };
        match &result {
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, &self.budget)?.iter() {
                    check_field(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        check_field(&field.ty)?;
                    }
                }
            }
            _ => unreachable!(),
        }
        if self.type_contains_projection(&result, depth + 1)? {
            if self.raw_associated_ty_is_closed(&result, depth + 1)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: residual projection in closed source nominal fields after actual normalization",
                ));
            }
            self.validate_universal_projection_type(&result, env, depth + 1)?;
        }
        Ok(result)
    }

    #[cfg(test)]
    fn validate_reapplied_nominal(
        &self,
        original: &Ty,
        normalized: &[Ty],
        result: &Ty,
        env: &GenericEnv,
        depth: usize,
    ) -> Result<()> {
        self.validate_reapplied_nominal_with_facts(original, normalized, result, env, depth, None)
    }

    #[cfg(test)]
    fn validate_reapplied_nominal_with_facts(
        &self,
        original: &Ty,
        normalized: &[Ty],
        result: &Ty,
        env: &GenericEnv,
        depth: usize,
        facts: Option<&ValidatedFacts>,
    ) -> Result<()> {
        self.validate_reapplied_nominal_in(
            original,
            normalized,
            result,
            env,
            depth,
            facts,
            &NominalOperations::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_reapplied_nominal_in(
        &self,
        original: &Ty,
        normalized: &[Ty],
        result: &Ty,
        env: &GenericEnv,
        depth: usize,
        facts: Option<&ValidatedFacts>,
        operations: &NominalOperations,
    ) -> Result<()> {
        operations.validate(self, original, env, facts)?;
        operations.validate(self, result, env, facts)?;
        let declaration = match original {
            Ty::Struct(value) => value.def().source_decl,
            Ty::Enum(value) => value.def().source_decl,
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal reapplication lost original application",
                ));
            }
        };
        let (result_declaration, actuals) = match (original, &result) {
            (Ty::Struct(_), Ty::Struct(value)) => (value.def().source_decl, &value.def().type_args),
            (Ty::Enum(_), Ty::Enum(value)) => (value.def().source_decl, &value.def().type_args),
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization changed the source nominal kind",
                ));
            }
        };
        if result_declaration != declaration || actuals.len() != normalized.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal normalization changed source declaration or argument arity",
            ));
        }
        for (actual, expected) in actuals.iter().zip(normalized) {
            if !self.eq_ty(actual, expected, depth + 1)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal normalization changed a normalized actual argument",
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

    fn source_applications() -> (Arc<ValidatedTraitWorld>, Vec<Ty>, Arc<GenericEnv>) {
        let program = frontend::parse(
            "struct Wrap<T>(T);struct Other<T>(T);enum Choice<T>{Value(T)}\
             fn first(x:Wrap<i32>){}fn second(x:Wrap<bool>){}\
             fn third(x:Other<bool>){}fn fourth(x:Choice<bool>){}fn main(){}",
        )
        .unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let applications = (0..4)
            .map(|index| {
                world
                    .callable_header(plan.callable_decl(index).unwrap())
                    .unwrap()
                    .schema
                    .signature
                    .params[0]
                    .clone()
            })
            .collect();
        let env = world
            .callable_header(plan.callable_decl(0).unwrap())
            .unwrap()
            .schema
            .env
            .clone();
        (world, applications, env)
    }

    #[test]
    fn reapplication_requires_exact_source_kind_and_normalized_actuals() {
        let (world, applications, env) = source_applications();
        let original = &applications[0];
        let normalized = [Ty::Bool];
        world
            .validate_reapplied_nominal(original, &normalized, &applications[1], &env, 0)
            .unwrap();
        for result in [&applications[0], &applications[2], &applications[3]] {
            let error = world
                .validate_reapplied_nominal(original, &normalized, result, &env, 0)
                .unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        }
        let error = world
            .validate_reapplied_nominal(original, &[], &applications[1], &env, 0)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
    }

    #[test]
    fn reapplication_rejects_foreign_and_rewritten_application_payloads() {
        let (world, applications, env) = source_applications();
        let (_, foreign, _) = source_applications();
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &foreign[1], &env, 0)
                .is_err()
        );
        let Ty::Struct(value) = &applications[1] else {
            panic!("source fixture is not a struct");
        };
        let Ty::Struct(other) = &applications[2] else {
            panic!("source fixture is not a struct");
        };
        let mut rebound = value.def().clone();
        rebound.source_decl = other.def().source_decl;
        let rebound = Ty::Struct(ir::StructTy::new(rebound));
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &rebound, &env, 0)
                .is_err()
        );
        let mut rewritten = value.def().clone();
        rewritten.fields = Arc::<[ir::FieldDef]>::from([ir::FieldDef {
            name: "0".into(),
            ty: Ty::Int(ir::IntTy::I32),
            visibility: nominal_fields(&rewritten.fields, &world.budget).unwrap()[0]
                .visibility
                .clone(),
            lifetime_bound: false,
        }])
        .into();
        let forged = Ty::Struct(ir::StructTy::new(rewritten));
        assert!(
            world
                .validate_reapplied_nominal(&applications[0], &[Ty::Bool], &forged, &env, 0)
                .is_err()
        );
    }

    #[test]
    fn projected_parent_local_receipt_consumes_exact_original_reductions() {
        let text = "trait Has{type A;type B;}impl Has for i32{type A=u8;type B=bool;}struct Storage<T:Has>{items:(T::A,T::B)}fn main(){}fn other(){}";
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&frontend::parse(text).unwrap(), &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let (env, template) = plan
            .nominal_schema(plan.nominal_declaration(0).unwrap(), &budget)
            .unwrap();
        let actuals = [Ty::Int(ir::IntTy::I32)];
        let original = facts
            .parent_local_field(&template, &env, &actuals, None, 0)
            .unwrap();
        assert_eq!(original.reductions.len(), 2);
        let mut scratch = TraitScratch::default();
        for _ in 0..2 {
            original
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .unwrap();
        }
        let mut changed = original.clone();
        changed.reductions = Arc::from([]);
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let mut changed = original.clone();
        let mut rows = changed.reductions.to_vec();
        rows.push(rows[0].clone());
        changed.reductions = rows.into();
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let mut changed = original.clone();
        let mut rows = changed.reductions.to_vec();
        rows.swap(0, 1);
        changed.reductions = rows.into();
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let mut changed = original.clone();
        let mut rows = changed.reductions.to_vec();
        rows[0].1 = rows[1].1.clone();
        changed.reductions = rows.into();
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let mut changed = original.clone();
        changed.source = Ty::Bool;
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let mut changed = original.clone();
        changed.normalized = Ty::Bool;
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
        let other = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        assert!(
            original
                .replay(&template, &env, &actuals, &other, &mut scratch)
                .is_err()
        );
        let foreign = PlanCtx::from_program(&frontend::parse(text).unwrap(), &budget).unwrap();
        let foreign_facts = foreign
            .callable_facts(foreign.function_decl(0).unwrap())
            .unwrap();
        let (foreign_env, foreign_template) = foreign
            .nominal_schema(foreign.nominal_declaration(0).unwrap(), &budget)
            .unwrap();
        let foreign_field = foreign_facts
            .parent_local_field(&foreign_template, &foreign_env, &actuals, None, 0)
            .unwrap();
        let mut changed = original.clone();
        changed.reductions = foreign_field.reductions;
        assert!(
            changed
                .replay(&template, &env, &actuals, &facts, &mut scratch)
                .is_err()
        );
    }

    #[test]
    fn projection_free_path_preserves_application_and_rejects_non_nominals() {
        let (world, applications, env) = source_applications();
        let result = world
            .normalize_nominal_actuals(&applications[0], &env, 0, |_| {
                panic!("projection-free application must not need reminting")
            })
            .unwrap();
        assert_eq!(result, applications[0]);
        assert!(
            world
                .normalize_nominal_actuals(&Ty::Bool, &env, 0, |_| unreachable!())
                .is_err()
        );
    }

    #[test]
    fn nominal_normalization_rejects_foreign_facts_and_planning_budget() {
        let (world, applications, env) = source_applications();
        let (foreign_world, _, foreign_env) = source_applications();
        let declaration = env.scopes().last().unwrap().binder().declaration;
        let facts = world.callable_facts(declaration).unwrap();
        let foreign_declaration = foreign_env.scopes().last().unwrap().binder().declaration;
        let foreign_facts = foreign_world.callable_facts(foreign_declaration).unwrap();
        let operations = NominalOperations::default();
        operations
            .validate(&world, &applications[0], &env, Some(&facts))
            .unwrap();
        {
            let _borrowed = operations.session.borrow_mut();
            operations
                .validate(&world, &applications[0], &env, Some(&facts))
                .unwrap();
        }
        assert!(
            operations
                .validate(&world, &applications[0], &env, Some(&foreign_facts))
                .is_err()
        );
        assert!(
            world
                .normalize_nominal_actuals_with_facts(
                    &applications[0],
                    &env,
                    0,
                    Some(&foreign_facts),
                    |argument| Ok(argument.clone()),
                )
                .is_err()
        );
        assert!(
            world
                .schema
                .reapply_nominal_with_facts(
                    &applications[0],
                    &[Ty::Bool],
                    &foreign_facts,
                    &world.budget,
                )
                .is_err()
        );
        assert!(
            world
                .normalize_ty(&applications[0], &foreign_facts)
                .is_err()
        );
        assert!(
            facts
                .validate_ty_metadata_with_budget(&applications[0], &PlanBudget::default())
                .is_err()
        );
    }
}
