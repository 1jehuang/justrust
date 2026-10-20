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
