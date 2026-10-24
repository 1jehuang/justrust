//! Successful raw schema substitutions reused only within one proof node.
//! This is not a proof cache: callers still normalize, validate contracts and
//! predicates, and discharge equalities after every lookup.
use super::*;
use std::collections::HashMap;

type Bucket = (
    WorldId,
    usize,
    FactsPhase,
    bool,
    bool,
    Option<DeclId>,
    usize,
);

#[derive(Default)]
pub(super) struct Substitutions {
    entries: Vec<Entry>,
    index: HashMap<Bucket, Vec<usize>>,
}

struct Entry {
    world: WorldId,
    adapter: Arc<dyn SchemaCx + Send + Sync>,
    caller: Arc<PredicateEnv>,
    phase: FactsPhase,
    ready: bool,
    finalized: bool,
    declaration: Option<DeclId>,
    schema: GenericSchema,
    arguments: Arc<[Ty]>,
    result: InstantiatedSchema,
}

impl ValidatedTraitWorld {
    pub(super) fn instantiate_raw_cached(
        &self,
        schema: &GenericSchema,
        arguments: &[Ty],
        context: ProofContext<'_>,
        declaration: Option<DeclId>,
        cache: &mut Substitutions,
    ) -> Result<InstantiatedSchema> {
        self.check_normalization_budget()?;
        self.check_context(context)?;
        charged(&self.budget, self.budget.check_depth(0))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if !self.schema.budget().same_authority(&self.budget) {
            return Err(FactsError::invalid(
                &self.budget,
                "schema substitution budget authority changed",
            ));
        }
        // The address is only a bucket hint. Entries retain this environment
        // allocation, and the complete inputs below decide every hit.
        let bucket = (
            self.id,
            context.predicates.index,
            context.phase,
            self.owner_normalization_ready,
            self.owner_normalization_finalized,
            declaration,
            Arc::as_ptr(&schema.env) as usize,
        );
        if let Some(candidates) = cache.index.get(&bucket) {
            for &index in candidates {
                charged(&self.budget, self.budget.charge_work(1))?;
                let entry = &cache.entries[index];
                if entry.world == self.id
                    && Arc::ptr_eq(&entry.adapter, &self.schema)
                    && std::ptr::eq(entry.caller.as_ref(), context.predicates)
                    && entry.phase == context.phase
                    && entry.ready == self.owner_normalization_ready
                    && entry.finalized == self.owner_normalization_finalized
                    && entry.declaration == declaration
                    && Arc::ptr_eq(&entry.schema.env, &schema.env)
                    && Arc::ptr_eq(&entry.schema.predicates, &schema.predicates)
                    && self.eq_contract(
                        &entry.schema.signature.ref_contract,
                        &schema.signature.ref_contract,
                    )?
                    && same_inputs(
                        &entry.schema.signature.params,
                        &schema.signature.params,
                        &self.budget,
                        0,
                    )?
                    && same_input(
                        &entry.schema.signature.result,
                        &schema.signature.result,
                        &self.budget,
                        0,
                    )?
                    && same_inputs(&entry.arguments, arguments, &self.budget, 0)?
                {
                    return clone_result(&entry.result, &self.budget);
                }
            }
        }
        let result =
            self.schema
                .instantiate_schema(schema, arguments, context.env(), &self.budget)?;
        // Capacity exhaustion declines caching, never a successful substitution.
        if cache.entries.len() < MAX_SCRATCH_OBLIGATIONS {
            let retained = Entry {
                world: self.id,
                adapter: self.schema.clone(),
                caller: if context.predicates.index == usize::MAX {
                    self.empty.clone()
                } else {
                    self.environments[context.predicates.index].clone()
                },
                phase: context.phase,
                ready: self.owner_normalization_ready,
                finalized: self.owner_normalization_finalized,
                declaration,
                schema: clone_schema(schema, &self.budget)?,
                arguments: {
                    retain::<Ty>(&self.budget, arguments.len())?;
                    charged(&self.budget, self.budget.charge_work(arguments.len()))?;
                    arguments.into()
                },
                result: clone_result(&result, &self.budget)?,
            };
            retain::<(Bucket, Vec<usize>)>(&self.budget, 2)?;
            cache.index.try_reserve(1).map_err(|_| {
                FactsError::resource(&self.budget, "schema substitution index allocation failed")
            })?;
            let index = cache.entries.len();
            push(&self.budget, &mut cache.entries, retained)?;
            push(&self.budget, cache.index.entry(bucket).or_default(), index)?;
        }
        Ok(result)
    }
}

fn clone_result(result: &InstantiatedSchema, budget: &PlanBudget) -> Result<InstantiatedSchema> {
    retain::<Ty>(budget, result.signature.params.len())?;
    charged(
        budget,
        budget.charge_work(result.signature.params.len().saturating_add(1)),
    )?;
    Ok(result.clone())
}

pub(super) fn same_inputs(
    left: &[Ty],
    right: &[Ty],
    budget: &PlanBudget,
    depth: usize,
) -> Result<bool> {
    charged(budget, budget.check_depth(depth))?;
    charged(budget, budget.charge_work(1))?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !same_input(left, right, budget, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

// Only immutable allocation identity, plus metered structural wrappers. Equal
// nominal IDs alone never match, and graph capabilities remain part of identity.
pub(super) fn same_input(left: &Ty, right: &Ty, budget: &PlanBudget, depth: usize) -> Result<bool> {
    charged(budget, budget.check_depth(depth))?;
    charged(budget, budget.charge_work(1))?;
    if left.is_nominal_back_edge() || right.is_nominal_back_edge() {
        return Ok(false);
    }
    Ok(match (left, right) {
        (Ty::Struct(a), Ty::Struct(b)) => {
            std::ptr::eq(a.def(), b.def())
                && match (a.realization(), b.realization()) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.same_fact(b),
                    _ => false,
                }
        }
        (Ty::Enum(a), Ty::Enum(b)) => {
            std::ptr::eq(a.def(), b.def())
                && match (a.realization(), b.realization()) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.same_fact(b),
                    _ => false,
                }
        }
        (Ty::Tuple(a), Ty::Tuple(b)) => Arc::ptr_eq(a, b) || same_inputs(a, b, budget, depth + 1)?,
        (
            Ty::Ref {
                kind: ak,
                referent: a,
            },
            Ty::Ref {
                kind: bk,
                referent: b,
            },
        )
        | (
            Ty::RawPtr {
                kind: ak,
                pointee: a,
            },
            Ty::RawPtr {
                kind: bk,
                pointee: b,
            },
        ) => ak == bk && (Arc::ptr_eq(a, b) || same_input(a, b, budget, depth + 1)?),
        (
            Ty::Array {
                element: a,
                len: an,
            },
            Ty::Array {
                element: b,
                len: bn,
            },
        ) => an == bn && (Arc::ptr_eq(a, b) || same_input(a, b, budget, depth + 1)?),
        (Ty::Slice { element: a }, Ty::Slice { element: b }) => {
            Arc::ptr_eq(a, b) || same_input(a, b, budget, depth + 1)?
        }
        (Ty::AssociatedType(a), Ty::AssociatedType(b)) => {
            Arc::ptr_eq(a, b)
                || (a.item == b.item
                    && a.trait_ref.declaration == b.trait_ref.declaration
                    && same_input(
                        &a.trait_ref.self_ty,
                        &b.trait_ref.self_ty,
                        budget,
                        depth + 1,
                    )?
                    && same_inputs(&a.trait_ref.args, &b.trait_ref.args, budget, depth + 1)?)
        }
        (Ty::Opaque(a), Ty::Opaque(b)) => Arc::ptr_eq(a, b),
        (Ty::Closure(a), Ty::Closure(b)) => Arc::ptr_eq(a, b),
        (Ty::FnItem(a), Ty::FnItem(b)) => Arc::ptr_eq(a, b),
        (Ty::Dyn(a), Ty::Dyn(b)) => Arc::ptr_eq(a, b),
        (Ty::Param(a), Ty::Param(b)) => a == b,
        (Ty::Int(a), Ty::Int(b)) => a == b,
        (Ty::Float(a), Ty::Float(b)) => a == b,
        (Ty::Char, Ty::Char)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Unit, Ty::Unit)
        | (Ty::Never, Ty::Never)
        | (Ty::Str, Ty::Str) => true,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn fixture() -> (Arc<ValidatedTraitWorld>, GenericSchema, Arc<ValidatedFacts>) {
        let program =
            frontend::parse("struct Wrap<T>(T);fn id<T>(x:Wrap<T>)->Wrap<T>{x}fn main(){}")
                .unwrap();
        let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let schema = world
            .callable_schema(plan.function_decl(0).unwrap())
            .unwrap();
        let facts = world
            .callable_facts(plan.function_decl(1).unwrap())
            .unwrap();
        (world, schema, facts)
    }

    #[test]
    fn raw_schema_cache_reuses_only_complete_retained_inputs() {
        let (world, schema, facts) = fixture();
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        let args = [Ty::Int(ir::IntTy::I32)];
        let first = world
            .instantiate_scratch(&schema, &args, context, None, &mut scratch)
            .unwrap();
        let count = scratch.instantiations.entries.len();
        assert_eq!(count, 1);
        let again = world
            .instantiate_scratch(&schema.clone(), &args, context, None, &mut scratch)
            .unwrap();
        assert_eq!(scratch.instantiations.entries.len(), count);
        assert!(
            same_input(
                &first.signature.result,
                &again.signature.result,
                &world.budget,
                0
            )
            .unwrap()
        );
        let fresh = world
            .instantiate_scratch(&schema, &args, context, None, &mut ProofScratch::default())
            .unwrap();
        assert!(
            !same_input(
                &first.signature.result,
                &fresh.signature.result,
                &world.budget,
                0
            )
            .unwrap()
        );
        let changed = world
            .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
            .unwrap();
        assert_eq!(scratch.instantiations.entries.len(), count + 1);
        let Ty::Struct(changed) = changed.signature.result else {
            panic!("nominal result")
        };
        assert_eq!(changed.def().type_args.as_ref(), &[Ty::Bool]);
        let mut other = schema.clone();
        other.signature.result = Ty::Bool;
        assert_eq!(
            world
                .instantiate_scratch(&other, &args, context, None, &mut scratch)
                .unwrap()
                .signature
                .result,
            Ty::Bool
        );
        let before = scratch.instantiations.entries.len();
        let phased = ProofContext {
            predicates: context.predicates,
            phase: FactsPhase::Schema,
        };
        world
            .instantiate_raw_cached(&schema, &args, phased, None, &mut scratch.instantiations)
            .unwrap();
        assert_eq!(scratch.instantiations.entries.len(), before + 1);
        world
            .instantiate_raw_cached(
                &schema,
                &args,
                context,
                Some(facts.declaration()),
                &mut scratch.instantiations,
            )
            .unwrap();
        assert_eq!(scratch.instantiations.entries.len(), before + 2);
    }

    #[test]
    fn raw_schema_cache_rejects_foreign_context_and_forged_nominal_input() {
        let (world, schema, facts) = fixture();
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        let original = world
            .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
            .unwrap();
        let (foreign_world, _, foreign) = fixture();
        assert!(
            world
                .instantiate_scratch(
                    &schema,
                    &[Ty::Bool],
                    ProofContext::from_facts(&foreign),
                    None,
                    &mut scratch
                )
                .is_err()
        );
        assert!(
            foreign_world
                .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
                .is_err()
        );
        let forged_context = PredicateEnv {
            world: context.predicates.world,
            index: context.predicates.index,
            declaration: context.predicates.declaration,
            env: context.predicates.env.clone(),
            predicates: context.predicates.predicates.clone(),
        };
        assert!(
            world
                .instantiate_scratch(
                    &schema,
                    &[Ty::Bool],
                    ProofContext {
                        predicates: &forged_context,
                        phase: context.phase
                    },
                    None,
                    &mut scratch
                )
                .is_err()
        );
        let Ty::Struct(nominal) = original.signature.result else {
            panic!("nominal result")
        };
        let valid = Ty::Struct(nominal.clone());
        world
            .instantiate_scratch(
                &schema,
                std::slice::from_ref(&valid),
                context,
                None,
                &mut scratch,
            )
            .unwrap();
        let mut definition = nominal.def().clone();
        let mut fields = definition.fields.to_vec();
        fields[0].ty = Ty::Int(ir::IntTy::I32);
        definition.fields = fields.into();
        let forged = Ty::Struct(ir::StructTy::new(definition));
        assert!(
            world
                .instantiate_scratch(&schema, &[forged], context, None, &mut scratch)
                .is_err()
        );
        assert!(
            world
                .instantiate_scratch(&schema, &[], context, None, &mut scratch)
                .is_err()
        );
    }

    #[test]
    fn raw_schema_cache_does_not_reuse_a_rebound_schema() {
        let (world, schema, facts) = fixture();
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        world
            .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
            .unwrap();
        let binder = ir::BinderId {
            declaration: facts.declaration(),
        };
        let scope = Arc::new(ir::ParamEnv::new(binder, 1).unwrap());
        let env = Arc::new(GenericEnv::new(&[scope], &world.budget).unwrap());
        let mut rebound = schema.clone();
        rebound.env = env;
        assert!(
            world
                .instantiate_raw_cached(
                    &rebound,
                    &[Ty::Bool],
                    context,
                    None,
                    &mut scratch.instantiations,
                )
                .is_err()
        );
        assert_eq!(scratch.instantiations.entries.len(), 1);
        // An equivalent retained environment allocation is still a new cache
        // input. Rebinding to a nonexistent source binder above was rejected.
        rebound.env = Arc::new(schema.env.as_ref().clone());
        let result = world
            .instantiate_raw_cached(
                &rebound,
                &[Ty::Bool],
                context,
                None,
                &mut scratch.instantiations,
            )
            .unwrap();
        let Ty::Struct(result) = result.signature.result else {
            panic!("nominal result")
        };
        assert_eq!(result.def().type_args.as_ref(), &[Ty::Bool]);
        assert_eq!(scratch.instantiations.entries.len(), 2);
    }

    #[test]
    fn projected_substitution_retains_context_validation_on_cache_hits() {
        let (world, schema, facts) = fixture();
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        for _ in 0..2 {
            world
                .substitute_projected_schema_with(&schema, &[Ty::Bool], context, &mut scratch)
                .unwrap();
        }
        assert_eq!(scratch.instantiations.entries.len(), 1);
        let (_, _, foreign) = fixture();
        assert!(
            world
                .substitute_projected_schema_with(
                    &schema,
                    &[Ty::Bool],
                    ProofContext::from_facts(&foreign),
                    &mut scratch,
                )
                .is_err()
        );
        assert!(world.budget.charge_work(usize::MAX).is_err());
        assert_eq!(
            world
                .substitute_projected_schema_with(&schema, &[Ty::Bool], context, &mut scratch)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
    }

    #[test]
    fn raw_schema_cache_keeps_resource_failure_sticky_and_comparison_bounded() {
        let (world, schema, facts) = fixture();
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        world
            .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
            .unwrap();
        assert!(world.budget.charge_work(usize::MAX).is_err());
        assert_eq!(
            world
                .instantiate_scratch(&schema, &[Ty::Bool], context, None, &mut scratch)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        let budget = PlanBudget::new(ir::PlanLimits {
            depth: 1,
            ..ir::PlanLimits::default()
        });
        let wrap = || Ty::Tuple(Arc::from([Ty::Tuple(Arc::from([Ty::Bool]))]));
        assert_eq!(
            same_input(&wrap(), &wrap(), &budget, 0).unwrap_err().kind(),
            FactsErrorKind::Resource
        );
    }
}
