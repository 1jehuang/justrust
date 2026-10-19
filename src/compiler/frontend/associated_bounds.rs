//! Private source/registry controls for written associated declaration bounds.
use super::*;

const SOURCE: &str = "trait Family{type Out:Copy+Sized+Copy;fn value()->u64{7}}struct S;impl Family for S{type Out=u64;}fn main(){}";

#[test]
fn associated_bounds_keep_order_duplicates_and_original_catalog_occurrences() {
    let program = parse(SOURCE).unwrap();
    let bounds = &program.traits[0].associated_types[0].bounds;
    assert_eq!(bounds.len(), 3);
    assert!(
        bounds
            .iter()
            .all(|path| matches!(path, TraitPath::Source(_)))
    );
    let (TraitPath::Source(first), TraitPath::Source(last)) = (&bounds[0], &bounds[2]) else {
        unreachable!()
    };
    assert!(
        !exact_source_generic_path(&program.names, first, last, &ir::PlanBudget::default(), 0)
            .unwrap()
    );
    program
        .validate_associated_bound_sources(&ir::PlanBudget::default())
        .unwrap();
}

#[test]
fn associated_bounds_resolve_catalog_imports_without_losing_source_paths() {
    let program = parse("use core::marker::Copy as Bits;trait Family{type A:Bits;type B:core::marker::Sized;}fn main(){}").unwrap();
    assert!(matches!(
        program.associated_bound_sources[0].bounds[0].1,
        TraitPath::Builtin(BuiltinTrait::Copy)
    ));
    assert!(matches!(
        program.associated_bound_sources[1].bounds[0].1,
        TraitPath::Builtin(BuiltinTrait::Sized)
    ));
    for member in &program.traits[0].associated_types {
        let TraitPath::Source(path) = &member.bounds[0] else {
            panic!("erased source bound")
        };
        assert!(path.resolved_use.is_some());
    }
    let user = parse("trait Copy{}trait Family{type Out:Copy;}fn main(){}").unwrap();
    assert!(matches!(
        user.associated_bound_sources[0].bounds[0].1,
        TraitPath::Source(_)
    ));
    user.validate_associated_bound_sources(&ir::PlanBudget::default())
        .unwrap();
    let clone = parse("trait Family{type Out:Clone;}fn main(){}").unwrap();
    assert!(matches!(
        clone.associated_bound_sources[0].bounds[0].1,
        TraitPath::Builtin(BuiltinTrait::Clone)
    ));
}

#[test]
fn associated_bounds_reject_transplants_reorder_removal_and_bare_builtin() {
    let program = parse(SOURCE).unwrap();
    for mutation in 0..5 {
        let mut candidate = program.clone();
        let bounds = &mut candidate.traits[0].associated_types[0].bounds;
        match mutation {
            0 => bounds.swap(0, 2),
            1 => {
                bounds.remove(0);
            }
            2 => bounds[0] = TraitPath::Builtin(BuiltinTrait::Copy),
            3 => bounds.push(bounds[0].clone()),
            _ => bounds.clear(),
        }
        assert!(
            candidate
                .validate_associated_bound_sources(&ir::PlanBudget::default())
                .is_err()
        );
    }
    let program = parse("trait A{type Out:Copy;}trait B{type Out:Copy;}fn main(){}").unwrap();
    let mut candidate = program.clone();
    candidate.traits[0].associated_types[0].bounds[0] =
        candidate.traits[1].associated_types[0].bounds[0].clone();
    assert!(
        candidate
            .validate_associated_bound_sources(&ir::PlanBudget::default())
            .is_err()
    );
}

#[test]
fn associated_bounds_admit_all_handles_before_user_trait_boundary() {
    let mut parsed =
        parse_unresolved("trait User{}trait Family{type Out:User+Copy;}fn main(){}").unwrap();
    let foreign = parse("trait Foreign{}fn main(){}").unwrap();
    let foreign_name = foreign.traits[0].name.segments()[0];
    let TraitPath::Source(path) = &mut parsed.program.traits[1].associated_types[0].bounds[1]
    else {
        unreachable!()
    };
    path.segments[0].name = foreign_name;
    let error = root_resolution::resolve(&parsed).unwrap_err();
    assert!(!error.starts_with("unsupported:"), "{error}");
}

#[test]
fn associated_bounds_registry_callback_is_frozen_ordered_and_lock_free() {
    use tr::SchemaCx;
    let program = parse(SOURCE).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        state: plan.state.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: plan.budget.clone(),
    };
    let member = plan.registry.trait_associated_types[0][0];
    // Holding this guard would deadlock a callback that reentered PlanState.
    let _guard = plan.state.lock().unwrap();
    let ids = adapter.associated_type_bounds(member).unwrap();
    assert_eq!(
        &*ids,
        &[
            plan.catalog.id(tr::CatalogItem::Copy),
            plan.catalog.id(tr::CatalogItem::Sized),
            plan.catalog.id(tr::CatalogItem::Copy)
        ]
    );
    assert!(
        adapter
            .associated_type_bounds(plan.registry.trait_ids[0])
            .is_err()
    );
    let foreign = parse("trait Other{type Out:Copy;}fn main(){}").unwrap();
    let foreign = PlanCtx::from_program(&foreign, &ir::PlanBudget::default()).unwrap();
    assert!(
        adapter
            .associated_type_bounds(foreign.registry.trait_associated_types[0][0])
            .is_err()
    );
}

#[test]
fn associated_bounds_owning_callable_replay_checks_exact_source() {
    let program = parse(SOURCE).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let default = plan
        .registry
        .callable_sources
        .iter()
        .enumerate()
        .find_map(|(index, source)| {
            matches!(source, CallableSource::TraitDefault { .. })
                .then_some(plan.registry.function_ids[index])
        })
        .unwrap();
    plan.validate_callable_associated_bounds(&program, default, &plan.budget)
        .unwrap();
    let mut candidate = program.clone();
    candidate.traits[0].associated_types[0].bounds.swap(0, 2);
    assert!(
        plan.validate_callable_associated_bounds(&candidate, default, &plan.budget)
            .is_err()
    );
    let mut candidate = program.clone();
    candidate.traits[0].associated_types[0].bounds.clear();
    assert!(
        plan.validate_callable_associated_bounds(&candidate, default, &plan.budget)
            .is_err()
    );
}

#[test]
fn associated_bounds_nested_unused_payload_clone_is_metered() {
    let plain = parse_unresolved("trait User<T>{}trait Family{type Out;}fn main(){}").unwrap();
    let nested =
        parse_unresolved("trait User<T>{}trait Family{type Out:User<(bool,[u64;3])>;}fn main(){}")
            .unwrap();
    let a = ir::PlanBudget::default();
    let b = ir::PlanBudget::default();
    source_clone::preflight_program_clone(&plain.program, &a).unwrap();
    source_clone::preflight_program_clone(&nested.program, &b).unwrap();
    assert!(b.usage_snapshot().bytes > a.usage_snapshot().bytes);
    let denied = ir::PlanBudget::new(ir::PlanLimits {
        bytes: a.usage_snapshot().bytes,
        ..Default::default()
    });
    assert!(source_clone::preflight_program_clone(&nested.program, &denied).is_err());
    assert!(denied.first_resource_failure().is_some());
}
