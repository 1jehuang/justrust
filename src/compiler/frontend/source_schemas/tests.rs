use super::*;

fn id(snapshot: ir::SnapshotId, index: u32) -> ir::DeclId {
    ir::DeclId { snapshot, index }
}
fn schema(budget: &ir::PlanBudget) -> tr::GenericSchema {
    tr::GenericSchema {
        env: Arc::new(ir::GenericEnv::new(&[], budget).unwrap()),
        signature: ir::Signature {
            params: vec![ir::Ty::Bool],
            result: ir::Ty::Unit,
            ref_contract: Arc::new(ir::RefContract::default()),
        },
        predicates: Arc::from([]),
    }
}
fn fixture(budget: &ir::PlanBudget) -> (Arc<SourceSchemas>, ir::DeclId) {
    let snapshot = ir::SnapshotId::allocate().unwrap();
    let declaration = id(snapshot, 0);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(declaration, schema(budget), budget)
        .unwrap();
    builder
        .region_modes
        .insert(declaration, tr::RegionContractMode::Positioned, budget)
        .unwrap();
    builder
        .assignment(declaration, id(snapshot, 1), ir::Ty::Bool, budget)
        .unwrap();
    (builder.seal(snapshot, budget).unwrap(), declaration)
}

#[test]
fn source_schemas_seal_moves_allocations_and_keeps_only_immutable_receipts() {
    let budget = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::allocate().unwrap();
    let declaration = id(snapshot, 0);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(declaration, schema(&budget), &budget)
        .unwrap();
    let params = builder.functions.values[&declaration]
        .signature
        .params
        .as_ptr();
    let retained = budget.usage_snapshot().reservations.retentions;
    let frozen = builder.seal(snapshot, &budget).unwrap();
    assert_eq!(
        params,
        frozen.functions[&declaration].signature.params.as_ptr()
    );
    assert_eq!(
        budget.usage_snapshot().reservations.retentions,
        retained + 1
    );
    let before = budget.usage_snapshot();
    let shared = frozen.clone();
    assert!(Arc::ptr_eq(&frozen, &shared));
    assert_eq!(budget.usage_snapshot().bytes, before.bytes);
    for receipt in frozen.receipts.iter().flatten() {
        assert!(receipt.authority().same_authority(budget.authority()));
        assert!(receipt.bytes() > 0);
    }
    assert_eq!(budget.usage_snapshot().reservations.current_reservations, 0);
}

#[test]
fn source_schemas_foreign_snapshot_and_equal_policy_foreign_payer_fail() {
    let budget = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&budget);
    let foreign = id(ir::SnapshotId::allocate().unwrap(), declaration.index);
    let other_budget = ir::PlanBudget::new(budget.limits());
    for (decl, payer) in [(foreign, &budget), (declaration, &other_budget)] {
        assert!(frozen.schema(decl, payer).is_err());
        assert!(frozen.region_mode(decl, payer).is_err());
        assert!(frozen.region_env(decl, true, payer).is_err());
        assert!(frozen.signature_arity(decl, payer).is_err());
        assert!(frozen.associated_rhs(decl, declaration, payer).is_err());
    }
    assert!(frozen.assignment_rows(&other_budget).is_err());
    assert!(
        frozen
            .schema(id(declaration.snapshot, 999), &budget)
            .is_err()
    );
    assert!(
        frozen
            .associated_rhs(declaration, id(declaration.snapshot, 999), &budget)
            .is_err()
    );
}

#[test]
fn source_schemas_missing_source_regions_do_not_become_empty_contracts() {
    let budget = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&budget);
    assert!(
        frozen
            .region_env(declaration, false, &budget)
            .unwrap_err()
            .contains("original lifetime")
    );
    let empty = frozen.region_env(declaration, true, &budget).unwrap();
    assert_eq!(empty.declaration, declaration);
    assert_eq!(empty.region_count, 0);
}

#[test]
fn source_schemas_budget_failure_is_sticky_on_hits_misses_and_zero_work_reads() {
    let budget = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&budget);
    assert!(budget.charge_work(usize::MAX).is_err());
    let failure = budget.first_resource_failure();
    for decl in [declaration, id(declaration.snapshot, 999)] {
        assert!(
            frozen
                .schema(decl, &budget)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .region_mode(decl, &budget)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .region_env(decl, true, &budget)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .signature_arity(decl, &budget)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .associated_rhs(decl, declaration, &budget)
                .unwrap_err()
                .contains("resource limit")
        );
    }
    assert!(frozen.assignment_rows(&budget).is_err());
    assert_eq!(budget.first_resource_failure(), failure);
}

#[test]
fn source_schemas_growth_is_prepaid_and_foreign_builder_payer_cannot_mutate() {
    let empty_budget = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..Default::default()
    });
    let mut map = PaidMap::<u32, u32>::default();
    assert!(map.insert(0, 0, &empty_budget).is_err());
    assert_eq!(map.values.capacity(), 0);
    assert!(map.values.is_empty());
    assert!(map.receipt.is_none());
    let budget = ir::PlanBudget::default();
    map.insert(0, 1, &budget).unwrap();
    let payer = ir::PlanBudget::new(budget.limits());
    assert!(map.insert(0, 2, &payer).is_err());
    assert_eq!(map.values[&0], 1);
    let mut builder = SourceSchemasBuilder::default();
    let snapshot = ir::SnapshotId::allocate().unwrap();
    let declaration = id(snapshot, 0);
    builder
        .assignment(declaration, declaration, ir::Ty::Bool, &budget)
        .unwrap();
    assert!(
        builder
            .assignment(declaration, declaration, ir::Ty::Unit, &payer)
            .is_err()
    );
    assert_eq!(builder.associated_type_rows.len(), 1);
    assert!(builder.seal(snapshot, &payer).is_err());
}

#[test]
fn source_schemas_assignment_duplicates_remain_visible_and_first_rhs_authoritative() {
    let budget = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::allocate().unwrap();
    let implementation = id(snapshot, 0);
    let associated = id(snapshot, 1);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .assignment(implementation, associated, ir::Ty::Bool, &budget)
        .unwrap();
    builder
        .assignment(implementation, associated, ir::Ty::Unit, &budget)
        .unwrap();
    let frozen = builder.seal(snapshot, &budget).unwrap();
    let rows = frozen.assignment_rows(&budget).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].rhs, ir::Ty::Unit);
    assert_eq!(
        frozen
            .associated_rhs(implementation, associated, &budget)
            .unwrap(),
        ir::Ty::Bool
    );
}

#[test]
fn source_schemas_metadata_callbacks_do_not_lock_mutable_nominal_state() {
    use super::super::{PlanCtx, SchemaAdapter, parse};
    use tr::SchemaCx;
    let budget = ir::PlanBudget::default();
    let program = parse("fn f(x:bool)->bool{x}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let original = plan.source_region_env(declaration).unwrap();
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        state: plan.state.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: budget.clone(),
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = plan.state.lock().unwrap();
        panic!("intentional nominal state poison");
    }));
    assert!(plan.state.is_poisoned());
    assert!(adapter.declaration_info(declaration).is_ok());
    assert!(adapter.declaration_schema(declaration).is_ok());
    assert!(Arc::ptr_eq(
        &adapter.source_region_env(declaration).unwrap(),
        &original
    ));
    let foreign = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    assert!(
        adapter
            .declaration_schema(foreign.function_decl(0).unwrap())
            .is_err()
    );
    assert!(
        adapter
            .source_region_env(foreign.function_decl(0).unwrap())
            .is_err()
    );
    assert!(
        adapter
            .declaration_info(id(declaration.snapshot, u32::MAX))
            .is_err()
    );
}
