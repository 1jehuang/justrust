//! Full type validation successes retained only by one checked-body scratch.
//! A hit reuses the exact immutable type under the complete original facts
//! context. It is never evidence for another phase, caller, or usage position.
use super::*;

/// Complete raw type-formation successes. These records never derive from
/// metadata-only validation and never authorize a caller-specific operation.
#[derive(Default)]
pub(super) struct RawFormations {
    entries: Vec<RawFormation>,
}

struct RawFormation {
    world: WorldId,
    registry: SnapshotId,
    ready: bool,
    finalized: bool,
    limits: [usize; 6],
    env: GenericEnv,
    usage: TypeUse,
    maybe_unsized: Option<ParamId>,
    key: TypeNode,
    ty: Ty,
    height: usize,
}

impl RawFormations {
    #[cfg(test)]
    pub(super) fn retained_roots(&self) -> usize {
        self.entries.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check(
        &mut self,
        world: &ValidatedTraitWorld,
        ty: &Ty,
        env: &GenericEnv,
        usage: TypeUse,
        maybe_unsized: Option<ParamId>,
        depth: usize,
    ) -> Result<()> {
        world.check_normalization_budget()?;
        charged(&world.budget, world.budget.check_depth(depth))?;
        charged(&world.budget, world.budget.charge_work(1))?;
        checked_module(
            &world.budget,
            world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        let key = if ty.is_nominal_back_edge() {
            None
        } else {
            TypeWalk::key(ty)
        };
        if let Some(key) = key {
            for previous in &self.entries {
                charged(&world.budget, world.budget.charge_work(1))?;
                if previous.world == world.id
                    && previous.registry == world.registry
                    && previous.ready == world.owner_normalization_ready
                    && previous.finalized == world.owner_normalization_finalized
                    && previous.limits == limits(&world.budget)
                    && previous.usage == usage
                    && previous.maybe_unsized == maybe_unsized
                    && previous.key == key
                    && world.same_env(&previous.env, env)?
                    && same_type_allocation(&previous.ty, ty)
                {
                    let complete_depth = depth.checked_add(previous.height).ok_or_else(|| {
                        FactsError::resource(&world.budget, "raw formation depth overflow")
                    })?;
                    charged(&world.budget, world.budget.check_depth(complete_depth))?;
                    return charged(&world.budget, world.budget.charge_nodes(1));
                }
            }
        }
        let mut walk = TypeWalk {
            maybe_unsized,
            ..TypeWalk::default()
        };
        let height = world.validate_ty_inner(ty, env, depth, usage, &mut walk)?;
        if let Some(key) = key
            && self.entries.len() < MAX_SCRATCH_OBLIGATIONS
        {
            push(
                &world.budget,
                &mut self.entries,
                RawFormation {
                    world: world.id,
                    registry: world.registry,
                    ready: world.owner_normalization_ready,
                    finalized: world.owner_normalization_finalized,
                    limits: limits(&world.budget),
                    env: env.clone(),
                    usage,
                    maybe_unsized,
                    key,
                    ty: ty.clone(),
                    height,
                },
            )?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Validated {
    context: Option<Context>,
    // Signature roots always use maybe_unsized=None, unlike body roots.
    signature_roots: bool,
    entries: Vec<(TypeNode, Ty)>,
    // Completed subtrees retain their exact inputs inside TypeWalk. This arena
    // is reused only for signature roots in the same fully authenticated context.
    signature_walk: TypeWalk,
}

struct Context {
    world: WorldId,
    registry: SnapshotId,
    predicates: Arc<PredicateEnv>,
    subject: FactsSubject,
    requirements: Arc<[ProofEvidence]>,
    closure: Option<Arc<ir::ClosureTy>>,
    mode: Option<ir::ClosureKind>,
    ready: bool,
    finalized: bool,
    limits: [usize; 6],
}

fn limits(budget: &PlanBudget) -> [usize; 6] {
    let value = budget.limits();
    [
        value.params,
        value.applications,
        value.depth,
        value.nodes,
        value.work,
        value.bytes,
    ]
}

impl Context {
    fn matches(&self, facts: &ValidatedFacts) -> Result<bool> {
        let world = &facts.world;
        charged(&world.budget, world.budget.charge_work(1))?;
        if self.world != world.id
            || self.registry != world.registry
            || !Arc::ptr_eq(&self.predicates, &facts.predicates)
            || !Arc::ptr_eq(&self.requirements, &facts.requirements)
            || self.mode != facts.closure_mode
            || self.ready != world.owner_normalization_ready
            || self.finalized != world.owner_normalization_finalized
            || self.limits != limits(&world.budget)
            || match (&self.closure, &facts.closure) {
                (None, None) => false,
                (Some(left), Some(right)) => !Arc::ptr_eq(left, right),
                _ => true,
            }
        {
            return Ok(false);
        }
        Ok(match (&self.subject, &facts.subject) {
            (FactsSubject::AssociatedConstant(a), FactsSubject::AssociatedConstant(b))
            | (FactsSubject::SchemaItem(a), FactsSubject::SchemaItem(b))
            | (FactsSubject::UniversalCallable(a), FactsSubject::UniversalCallable(b)) => a == b,
            (FactsSubject::ClosedInstance(a), FactsSubject::ClosedInstance(b)) => {
                a.declaration == b.declaration
                    && schema_instantiation::same_inputs(
                        &a.type_args,
                        &b.type_args,
                        &world.budget,
                        0,
                    )?
            }
            _ => false,
        })
    }

    fn retain(facts: &ValidatedFacts) -> Result<Self> {
        retain::<Self>(facts.budget(), 1)?;
        Ok(Self {
            world: facts.world.id,
            registry: facts.world.registry,
            predicates: facts.predicates.clone(),
            subject: facts.subject.clone(),
            requirements: facts.requirements.clone(),
            closure: facts.closure.clone(),
            mode: facts.closure_mode,
            ready: facts.world.owner_normalization_ready,
            finalized: facts.world.owner_normalization_finalized,
            limits: limits(facts.budget()),
        })
    }
}

impl Validated {
    #[cfg(test)]
    pub(super) fn retained_roots(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn retained_subtrees(&self) -> usize {
        self.signature_walk.complete.len()
    }

    pub(super) fn check(&mut self, facts: &ValidatedFacts, ty: &Ty) -> Result<()> {
        self.check_with(facts, ty, false)
    }

    pub(super) fn check_signature_root(&mut self, facts: &ValidatedFacts, ty: &Ty) -> Result<()> {
        self.check_with(facts, ty, true)
    }

    fn check_with(&mut self, facts: &ValidatedFacts, ty: &Ty, signature_roots: bool) -> Result<()> {
        let world = &facts.world;
        // A previous success cannot hide a later latched planning or source
        // authority failure. These checks run even for an identical root.
        world.check_normalization_budget()?;
        charged(facts.budget(), facts.budget().check_depth(0))?;
        world.check_facts(facts, false)?;
        world.check_context(ProofContext::from_facts(facts))?;
        checked_module(
            facts.budget(),
            world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        if self.signature_roots != signature_roots
            || !self
                .context
                .as_ref()
                .map(|value| value.matches(facts))
                .transpose()?
                .unwrap_or(false)
        {
            self.entries.clear();
            self.context = Some(Context::retain(facts)?);
            self.signature_roots = signature_roots;
            self.signature_walk = TypeWalk::default();
        }
        let key = if ty.is_nominal_back_edge() {
            None
        } else {
            TypeWalk::key(ty)
        };
        if let Some(key) = key {
            for (old_key, old) in &self.entries {
                charged(facts.budget(), facts.budget().charge_work(1))?;
                if *old_key == key && schema_instantiation::same_input(old, ty, facts.budget(), 0)?
                {
                    // Every call validates a root at depth zero. The retained
                    // root passed the same immutable depth limit on insertion.
                    charged(facts.budget(), facts.budget().charge_nodes(1))?;
                    return Ok(());
                }
            }
        }
        if signature_roots {
            if self.signature_walk.complete.len() >= MAX_SCRATCH_OBLIGATIONS {
                self.signature_walk = TypeWalk::default();
            }
            let result = world.validate_ty_inner_with_facts(
                ty,
                facts.env(),
                0,
                TypeUse::Root,
                &mut self.signature_walk,
                Some(facts),
            );
            if let Err(error) = result {
                // Do not retain partial computations across failed roots.
                self.signature_walk = TypeWalk::default();
                return Err(error);
            }
        } else {
            facts.validate_ty(ty)?;
        }
        if let Some(key) = key
            && self.entries.len() < MAX_SCRATCH_OBLIGATIONS
        {
            push(facts.budget(), &mut self.entries, (key, ty.clone()))?;
        }
        Ok(())
    }
}
