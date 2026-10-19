use super::*;
use crate::compiler::frontend::{self, PlanCtx};

fn fixture() -> (ValidatedTraitWorld, DeclId, PlanBudget) {
    let source = frontend::parse(
        "trait Other:Sized{type X;fn x(self)->Self::X;}\
         trait Source<A>{type Out;fn get(self,value:A)->Self::Out;}\
         struct Wrap<T>{value:T}\
         impl<T:Other<X=i32>> Source<i32> for Wrap<T>{\
           type Out=i32;\
           fn get(self,value:<T as Other>::X)->Self::Out{value}\
         }fn main(){}",
    )
    .unwrap();
    let budget = PlanBudget::default();
    let plan = PlanCtx::from_program(&source, &budget).unwrap();
    let world = plan.trait_world().unwrap();
    let declaration = world
        .headers
        .impls
        .iter()
        .find(|h| !h.items.is_empty())
        .unwrap()
        .items[0]
        .1;
    drop(plan);
    let world = Arc::try_unwrap(world)
        .unwrap_or_else(|_| panic!("fixture world has no hidden retained owner"));
    (world, declaration, budget)
}

#[test]
fn declared_reductions_final_replay_rejects_foreign_stale_owner_rhs_and_rows() {
    for variant in 0..6 {
        let (mut world, declaration, _) = fixture();
        let position = world
            .declared_callable_index
            .get(&world.budget, declaration)
            .unwrap()
            .unwrap();
        let foreign = WorldId(SnapshotId::allocate().unwrap());
        let context = &mut world.declared_callables[position];
        assert!(!context.reductions.values.is_empty());
        match variant {
            0 => context.reductions.values[0].world = foreign,
            1 => context.reductions.values[0].stage = DeclaredStageId(foreign),
            2 => {
                context.reductions.values[0].declaration = DeclId {
                    snapshot: declaration.snapshot,
                    index: u32::MAX,
                }
            }
            3 => context.reductions.values[0].rhs = Ty::Bool,
            4 => context.predicates = Arc::from([]),
            5 => context.env = Arc::new(GenericEnv::new(&[], &world.budget).unwrap()),
            _ => unreachable!(),
        }
        if variant == 5 {
            // Binder provenance is checked by final evidence replay, not the
            // construction dependency-row comparison.
            let world = Arc::new(world);
            let facts = world.callable_facts(declaration).unwrap();
            assert!(world.validate_final_equality_context(&facts).is_err());
        } else {
            assert!(
                world.validate_final_declared_contexts().is_err(),
                "variant {variant}"
            );
        }
    }
}

#[test]
fn declared_reductions_result_admission_is_atomic_for_foreign_stage_owner_and_payer() {
    for variant in 0..6 {
        let (mut world, declaration, budget) = fixture();
        let position = world
            .declared_callable_index
            .get(&budget, declaration)
            .unwrap()
            .unwrap();
        let context = &mut world.declared_callables[position];
        let mut reductions = ReductionRows::default();
        reductions
            .push(context.reductions.values[0].clone(), &budget)
            .unwrap();
        let mut result = DeclaredCallableNormalization {
            schema: clone_schema(&context.raw, &budget).unwrap(),
            world: context.world,
            stage: context.stage,
            declaration,
            reductions,
        };
        let foreign = WorldId(SnapshotId::allocate().unwrap());
        match variant {
            0 => result.world = foreign,
            1 => result.stage = DeclaredStageId(foreign),
            2 => {
                result.declaration = DeclId {
                    snapshot: declaration.snapshot,
                    index: u32::MAX,
                }
            }
            3 => result.reductions.values[0].world = foreign,
            4 => result.reductions.values[0].stage = DeclaredStageId(foreign),
            5 => {}
            _ => unreachable!(),
        }
        let pointer = context.reductions.values.as_ptr();
        let count = context.reductions.values.len();
        let other = PlanBudget::new(budget.limits());
        let payer = if variant == 5 { &other } else { &budget };
        assert!(result.finish(context, payer).is_err(), "variant {variant}");
        assert_eq!(context.reductions.values.as_ptr(), pointer);
        assert_eq!(context.reductions.values.len(), count);
        world.validate_final_declared_contexts().unwrap();
    }
}

#[test]
fn declared_reductions_normalization_failure_and_finalized_stage_never_mutate_rows() {
    let (mut world, declaration, budget) = fixture();
    let raw = world.schema.declaration_schema(declaration).unwrap();
    let position = world
        .declared_callable_index
        .get(&budget, declaration)
        .unwrap()
        .unwrap();
    let pointer = world.declared_callables[position]
        .reductions
        .values
        .as_ptr();
    let count = world.declared_callables[position].reductions.values.len();
    assert!(
        world
            .normalize_declared_callable_schema(declaration, &raw)
            .is_err(),
        "published world cannot reopen construction stage"
    );
    world.owner_normalization_finalized = false;
    let mut wrong = clone_schema(&raw, &budget).unwrap();
    wrong.signature.params.push(Ty::Bool);
    assert!(
        world
            .normalize_declared_callable_schema(declaration, &wrong)
            .is_err()
    );
    assert_eq!(
        world.declared_callables[position]
            .reductions
            .values
            .as_ptr(),
        pointer
    );
    assert_eq!(
        world.declared_callables[position].reductions.values.len(),
        count
    );
    world.owner_normalization_finalized = true;
    world.validate_final_declared_contexts().unwrap();
}

#[test]
fn declared_reductions_published_schema_reads_keep_frozen_rows_and_receipt() {
    let (world, declaration, budget) = fixture();
    let context = world.declared_callable_context(declaration).unwrap();
    let pointer = context.reductions.values.as_ptr();
    let count = context.reductions.values.len();
    for _ in 0..3 {
        world.normalized_callable_schema(declaration).unwrap();
        world.validate_final_declared_contexts().unwrap();
    }
    assert_eq!(context.reductions.values.as_ptr(), pointer);
    assert_eq!(context.reductions.values.len(), count);
    context.reductions.check(&budget).unwrap();
}
