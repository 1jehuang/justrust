//! Full type validation successes retained only by one checked-body scratch.
//! A hit reuses the exact immutable type under the complete original facts
//! context. It is never evidence for another phase, caller, or usage position.
use super::*;

#[derive(Default)]
pub(super) struct Validated {
    context: Option<Context>,
    entries: Vec<(TypeNode, Ty)>,
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
    pub(super) fn check(&mut self, facts: &ValidatedFacts, ty: &Ty) -> Result<()> {
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
        if !self
            .context
            .as_ref()
            .map(|value| value.matches(facts))
            .transpose()?
            .unwrap_or(false)
        {
            self.entries.clear();
            self.context = Some(Context::retain(facts)?);
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
        facts.validate_ty(ty)?;
        if let Some(key) = key
            && self.entries.len() < MAX_SCRATCH_OBLIGATIONS
        {
            push(facts.budget(), &mut self.entries, (key, ty.clone()))?;
        }
        Ok(())
    }
}
