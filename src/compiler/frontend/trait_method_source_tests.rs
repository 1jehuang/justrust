//! Trait-member bound projections retain the member's original source recipe.
use super::*;

#[test]
fn closed_trait_method_bound_predicates_normalize_before_world_validation() {
    // No call expression or closure inference runs here. A failure identifies
    // declaration predicate construction rather than a caller's hint consumer.
    let program = parse("trait Call<A>{}struct Seed;trait Transform{type Item;fn apply<F:Call<Self::Item>>(self,f:F);}impl Transform for Seed{type Item=i32;fn apply<F:Call<i32>>(self,f:F){}}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default())
        .expect("STAGE source-schema: retain the original declaration owner");
    for item in &plan.registry.trait_items[1] {
        plan.raw_schema(*item).expect("STAGE trait raw schema");
    }
    plan.trait_world()
        .expect("STAGE world admission: normalize closed inherited bound projections");
}

fn bound_projection(head: &FunctionHead) -> &SourceProjection {
    for bound in &head.bounds {
        for path in &bound.traits {
            if let TraitPath::Source(path) = path {
                for segment in &path.segments {
                    for actual in &segment.args {
                        if let Type::Projection(projection) = actual {
                            return projection;
                        }
                    }
                }
            }
        }
    }
    panic!("fixture lacks retained bound projection");
}

#[test]
fn trait_method_bound_projection_replays_exact_member_occurrence() {
    let program = parse("trait Call<A>{}trait Transform{type Item;fn apply<F:Call<Self::Item>>(self,f:F);fn other<F:Call<Self::Item>>(self,f:F);}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let item = plan.registry.trait_items[1][0];
    let context = plan.registry.source_type_context(item).unwrap();
    let original = bound_projection(&plan.program.traits[1].items[0].signature);
    let foreign = bound_projection(&plan.program.traits[1].items[1].signature);
    plan.registry
        .validate_source_projection(context, original, &plan.budget)
        .unwrap();
    assert!(
        plan.registry
            .validate_source_projection(context, foreign, &plan.budget)
            .is_err()
    );
    let other_context = plan
        .registry
        .source_type_context(plan.registry.trait_items[1][1])
        .unwrap();
    assert!(
        plan.registry
            .validate_source_projection(other_context, original, &plan.budget)
            .is_err()
    );
}

#[test]
fn stored_closure_projection_replays_exact_parent_and_nested_source_occurrences() {
    let program = parse("trait Has{type Out;}fn first<A:Has>(){let _f=|x:A::Out|x;}fn second<A:Has>(){let _f=|x:A::Out|x;}fn nested<A:Has>(){let _f=||{let _g=|x:A::Out|x;};}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    fn annotation(plan: &PlanCtx, owner: ir::DeclId) -> &SourceProjection {
        plan.program
            .closure_sources
            .values()
            .find_map(|source| {
                let parent = plan.registry.closure_source_owner(source).unwrap();
                if plan
                    .closure_publication_owner(parent, &plan.budget)
                    .unwrap()
                    != owner
                {
                    return None;
                }
                source.stored.as_ref()?.params.iter().find_map(|(_, ty)| {
                    if let Type::Projection(projection) = ty {
                        Some(projection.as_ref())
                    } else {
                        None
                    }
                })
            })
            .expect("genuine retained closure projection annotation")
    }
    let first = plan.function_decl(0).unwrap();
    let second = plan.function_decl(1).unwrap();
    let nested = plan.function_decl(2).unwrap();
    let original = annotation(&plan, first);
    let foreign = annotation(&plan, second);
    let nested_original = annotation(&plan, nested);
    for (owner, projection) in [(first, original), (nested, nested_original)] {
        let context = plan.registry.source_type_context(owner).unwrap();
        plan.registry
            .validate_source_projection(context, projection, &budget)
            .unwrap();
        assert!(
            plan.registry
                .validate_source_projection(context, foreign, &budget)
                .is_err(),
            "same-spelled foreign closure occurrence must not gain owner authority"
        );
    }
    let second_context = plan.registry.source_type_context(second).unwrap();
    assert!(
        plan.registry
            .validate_source_projection(second_context, original, &budget)
            .is_err()
    );
}
