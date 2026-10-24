use super::*;

fn id(snapshot: ir::SnapshotId, index: u64) -> ir::DeclId {
    ir::DeclId { snapshot, index }
}
// Unit fixtures deliberately bypass registry minting to exercise rejection.
fn slot(id: ir::DeclId) -> DeclSlot {
    DeclSlot { id }
}
fn assignment_source_fixture(
    implementation: ir::DeclId,
    associated: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Arc<tr::AssociatedTypeSource> {
    Arc::new(tr::AssociatedTypeSource {
        implementation,
        associated,
        env: Arc::new(ir::GenericEnv::new(&[], budget).unwrap()),
        region_count: 0,
        outlives: Arc::from([]),
        references: Arc::from([]),
        parameter_tail: None,
    })
}
fn schema(b: &ir::PlanBudget) -> StoredSchema {
    let value = tr::GenericSchema {
        env: Arc::new(ir::GenericEnv::new(&[], b).unwrap()),
        signature: ir::Signature {
            params: vec![ir::Ty::Bool],
            result: ir::Ty::Unit,
            ref_contract: Arc::new(ir::RefContract::default()),
        },
        predicates: Arc::from([]),
    };
    StoredSchema::copy(&value, b).unwrap()
}
fn fixture(b: &ir::PlanBudget) -> (Arc<SourceSchemas>, ir::DeclId) {
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let declaration = id(snapshot, 0);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(slot(declaration), schema(b), b)
        .unwrap();
    builder
        .assignment(
            declaration,
            id(snapshot, 1),
            ir::Ty::Bool,
            assignment_source_fixture(declaration, id(snapshot, 1), b),
            b,
        )
        .unwrap();
    (builder.seal(snapshot, b).unwrap(), declaration)
}
#[test]
fn source_schemas_seal_moves_allocations_and_keeps_only_immutable_receipts() {
    let b = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let declaration = id(snapshot, 0);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(slot(declaration), schema(&b), &b)
        .unwrap();
    let params = builder.functions.values[0]
        .as_ref()
        .unwrap()
        .1
        .value
        .signature
        .params
        .as_ptr();
    let table = builder.functions.values.as_ptr();
    let retained = b.usage_snapshot().reservations.retentions;
    let frozen = builder.seal(snapshot, &b).unwrap();
    assert_eq!(
        params,
        frozen.functions[0]
            .as_ref()
            .unwrap()
            .1
            .value
            .signature
            .params
            .as_ptr()
    );
    assert_eq!(table, frozen.functions.as_ptr());
    assert_eq!(b.usage_snapshot().reservations.retentions, retained + 1);
    let before = b.usage_snapshot();
    assert!(Arc::ptr_eq(&frozen, &frozen.clone()));
    assert_eq!(b.usage_snapshot().bytes, before.bytes);
    for receipt in frozen.receipts.iter().flatten() {
        assert!(receipt.authority().same_authority(b.authority()));
        assert!(receipt.bytes() > 0);
    }
    assert_eq!(b.usage_snapshot().reservations.current_reservations, 0);
}
#[test]
fn source_schemas_foreign_snapshot_and_equal_policy_foreign_payer_fail() {
    let b = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&b);
    let foreign = id(ir::SnapshotId::standalone().unwrap(), declaration.index);
    let other = ir::PlanBudget::new(b.limits());
    for (decl, payer) in [(foreign, &b), (declaration, &other)] {
        assert!(frozen.schema(decl, payer).is_err());
        assert!(frozen.declaration_info(decl, payer).is_err());
        assert!(frozen.region_env(decl, true, payer).is_err());
        assert!(frozen.signature_arity(decl, payer).is_err());
        assert!(frozen.associated_rhs(decl, declaration, payer).is_err());
    }
    assert!(frozen.assignment_rows(&other).is_err());
    assert!(frozen.schema(id(declaration.snapshot, 999), &b).is_err());
    assert!(
        frozen
            .associated_rhs(declaration, id(declaration.snapshot, 999), &b)
            .is_err()
    );
}
#[test]
fn source_schemas_missing_source_regions_do_not_become_empty_contracts() {
    let b = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&b);
    assert!(
        frozen
            .region_env(declaration, false, &b)
            .unwrap_err()
            .contains("original lifetime")
    );
    let empty = frozen.region_env(declaration, true, &b).unwrap();
    assert_eq!(empty.declaration, declaration);
    assert_eq!(empty.region_count, 0);
}
#[test]
fn source_schemas_budget_failure_is_sticky_on_hits_misses_and_zero_work_reads() {
    let b = ir::PlanBudget::default();
    let (frozen, declaration) = fixture(&b);
    assert!(b.charge_work(usize::MAX).is_err());
    let failure = b.first_resource_failure();
    for decl in [declaration, id(declaration.snapshot, 999)] {
        assert!(
            frozen
                .schema(decl, &b)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .declaration_info(decl, &b)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .region_env(decl, true, &b)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .signature_arity(decl, &b)
                .unwrap_err()
                .contains("resource limit")
        );
        assert!(
            frozen
                .associated_rhs(decl, declaration, &b)
                .unwrap_err()
                .contains("resource limit")
        );
    }
    assert!(frozen.assignment_rows(&b).is_err());
    assert_eq!(b.first_resource_failure(), failure);
}
#[test]
fn source_schemas_growth_is_prepaid_and_foreign_builder_payer_cannot_mutate() {
    let empty = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..Default::default()
    });
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let declaration = id(snapshot, 0);
    let mut table = PaidDeclTable::<u32>::default();
    assert!(table.insert(slot(declaration), 0, &empty).is_err());
    assert_eq!(table.values.capacity(), 0);
    assert!(table.receipt.is_none());
    let b = ir::PlanBudget::default();
    table.insert(slot(declaration), 1, &b).unwrap();
    let other = ir::PlanBudget::new(b.limits());
    assert!(table.insert(slot(declaration), 2, &other).is_err());
    assert_eq!(table.values[0].as_ref().unwrap().1, 1);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .assignment(
            declaration,
            declaration,
            ir::Ty::Bool,
            assignment_source_fixture(declaration, declaration, &b),
            &b,
        )
        .unwrap();
    assert!(
        builder
            .assignment(
                declaration,
                declaration,
                ir::Ty::Unit,
                assignment_source_fixture(declaration, declaration, &other),
                &other
            )
            .is_err()
    );
    assert_eq!(builder.associated_type_rows.len(), 1);
    assert!(builder.seal(snapshot, &other).is_err());
}
#[test]
fn source_schemas_assignment_duplicates_remain_visible_and_first_rhs_authoritative() {
    let b = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let implementation = id(snapshot, 0);
    let associated = id(snapshot, 1);
    let mut builder = SourceSchemasBuilder::default();
    builder
        .assignment(
            implementation,
            associated,
            ir::Ty::Bool,
            assignment_source_fixture(implementation, associated, &b),
            &b,
        )
        .unwrap();
    builder
        .assignment(
            implementation,
            associated,
            ir::Ty::Unit,
            assignment_source_fixture(implementation, associated, &b),
            &b,
        )
        .unwrap();
    let frozen = builder.seal(snapshot, &b).unwrap();
    let rows = frozen.assignment_rows(&b).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].rhs, ir::Ty::Unit);
    assert_eq!(
        frozen
            .associated_rhs(implementation, associated, &b)
            .unwrap(),
        ir::Ty::Bool
    );
}
#[test]
fn source_schemas_metadata_callbacks_do_not_lock_mutable_nominal_state() {
    use super::super::{PlanCtx, SchemaAdapter, parse};
    use tr::SchemaCx;
    let b = ir::PlanBudget::default();
    let program = parse("fn f(x:bool)->bool{x}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &b).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let original = plan.source_region_env(declaration).unwrap();
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        nominal_schemas: plan.nominal_schemas.clone(),
        nominals: plan.nominals.clone(),
        contexts: plan.contexts.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: b.clone(),
    };
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
            .declaration_info(id(declaration.snapshot, u64::MAX))
            .is_err()
    );
}
#[test]
fn source_schemas_underpaid_replacement_is_transactional_and_recoverable() {
    let b = ir::PlanBudget::default();
    let mut values = Vec::<u32>::new();
    let mut receipt = None;
    grow(&mut values, &mut receipt, 2, &b).unwrap();
    values.push(7);
    let original = (
        values.as_ptr(),
        values.capacity(),
        receipt.as_ref().unwrap().bytes(),
    );
    let replacement = Vec::with_capacity(16);
    let lease = b.reserve_bytes(1).unwrap();
    assert!(commit_replacement(&mut values, &mut receipt, replacement, lease, &b).is_err());
    assert_eq!(
        (
            values.as_ptr(),
            values.capacity(),
            receipt.as_ref().unwrap().bytes()
        ),
        original
    );
    assert_eq!(values, [7]);
    grow(&mut values, &mut receipt, 16, &b).unwrap();
    assert_eq!(values, [7]);
    assert!(receipt.as_ref().unwrap().bytes() >= values.capacity() * size_of::<u32>());
    assert!(
        b.usage_snapshot().reservations.peak_bytes
            >= original.2 + values.capacity() * size_of::<u32>()
    );
}
#[test]
fn source_schemas_failed_growth_is_sticky_and_never_commits_new_capacity() {
    let b = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 16,
        ..Default::default()
    });
    let mut values = Vec::<u64>::new();
    let mut receipt = None;
    grow(&mut values, &mut receipt, 1, &b).unwrap();
    values.push(1);
    let before = (values.as_ptr(), values.capacity());
    assert!(grow(&mut values, &mut receipt, 8, &b).is_err());
    assert_eq!((values.as_ptr(), values.capacity()), before);
    assert!(grow(&mut values, &mut receipt, 1, &b).is_err());
    assert_eq!(values, [1]);
}
#[test]
fn source_schemas_forged_receipt_cannot_publish() {
    let b = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(slot(id(snapshot, 0)), schema(&b), &b)
        .unwrap();
    builder.functions.receipt = Some(b.reserve_bytes(1).unwrap().retain().unwrap());
    assert!(
        builder
            .seal(snapshot, &b)
            .unwrap_err()
            .contains("fully prepaid")
    );
}
#[test]
fn source_schemas_mixed_snapshot_rows_regions_and_region_keys_are_rejected() {
    let b = ir::PlanBudget::default();
    let snapshot = ir::SnapshotId::standalone().unwrap();
    let foreign = ir::SnapshotId::standalone().unwrap();
    let mut builder = SourceSchemasBuilder::default();
    builder
        .assignment(
            id(snapshot, 0),
            id(snapshot, 1),
            ir::Ty::Bool,
            assignment_source_fixture(id(snapshot, 0), id(snapshot, 1), &b),
            &b,
        )
        .unwrap();
    assert!(
        builder
            .assignment(
                id(snapshot, 0),
                id(foreign, 1),
                ir::Ty::Unit,
                assignment_source_fixture(id(snapshot, 0), id(foreign, 1), &b),
                &b
            )
            .is_err()
    );
    builder.associated_type_rows[0].associated = id(foreign, 1);
    assert!(builder.seal(snapshot, &b).is_err());
    let mut builder = SourceSchemasBuilder::default();
    builder
        .source_regions
        .insert(
            slot(id(snapshot, 0)),
            Arc::new(ir::SourceRegionEnv {
                carriers: Arc::from([]),
                declaration: id(foreign, 0),
                region_count: 0,
                outlives: Arc::from([]),
                normalized: Arc::from([]),
            }),
            &b,
        )
        .unwrap();
    assert!(
        builder
            .seal(snapshot, &b)
            .unwrap_err()
            .contains("declaration mismatch")
    );
    let mut builder = SourceSchemasBuilder::default();
    builder
        .functions
        .insert(slot(id(foreign, 0)), schema(&b), &b)
        .unwrap();
    assert!(builder.seal(snapshot, &b).is_err());
}
#[test]
fn source_schemas_const_slots_are_not_callable_and_indices_stay_source_ordered() {
    use super::super::{PlanCtx, parse};
    let b = ir::PlanBudget::default();
    let program = parse("const A:u64=1u64;fn f()->u64{A}const B:bool=true;fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &b).unwrap();
    assert_eq!(plan.registry.const_ids.len(), 2);
    for declaration in &plan.registry.const_ids {
        assert_eq!(
            plan.declaration(*declaration).unwrap().kind,
            ir::DeclKind::Const
        );
        assert!(plan.raw_schema(*declaration).is_err());
        assert!(plan.registry.function_position(*declaration).is_none());
        assert_eq!(
            plan.source_region_env(*declaration).unwrap().region_count,
            0
        );
    }
    assert_eq!(
        plan.function_decl(0).unwrap(),
        plan.registry.function_ids[0]
    );
    assert_eq!(
        plan.function_decl(1).unwrap(),
        plan.registry.function_ids[1]
    );
}
#[test]
fn source_schemas_do_not_keep_source_program_or_name_store_alive() {
    use super::super::{PlanCtx, parse};
    let b = ir::PlanBudget::default();
    let (schemas, program, names) = {
        let source = parse("struct S{x:bool}fn f(x:S)->S{x}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&source, &b).unwrap();
        (
            plan.source_schemas.clone(),
            Arc::downgrade(&plan.program),
            Arc::downgrade(&plan.program.names),
        )
    };
    assert!(program.upgrade().is_none());
    assert!(names.upgrade().is_none());
    assert!(!schemas.functions.is_empty());
}

#[test]
fn source_schemas_params_copy_has_exact_capacity_receipt_and_shared_arc_children() {
    let b = ir::PlanBudget::default();
    let original = schema(&b);
    let before = b.usage_snapshot();
    let copied = StoredSchema::copy(&original.value, &b).unwrap();
    let after = b.usage_snapshot();
    let bytes = copied.value.signature.params.capacity() * size_of::<ir::Ty>();
    assert_eq!(copied.params_storage.bytes(), bytes);
    assert_eq!(after.bytes - before.bytes, bytes);
    assert_eq!(
        after.work - before.work,
        original.value.signature.params.len() + 1
    );
    assert_ne!(
        copied.value.signature.params.as_ptr(),
        original.value.signature.params.as_ptr()
    );
    assert!(Arc::ptr_eq(&copied.value.env, &original.value.env));
    assert!(Arc::ptr_eq(
        &copied.value.predicates,
        &original.value.predicates
    ));
    assert!(Arc::ptr_eq(
        &copied.value.signature.ref_contract,
        &original.value.signature.ref_contract
    ));
    let empty = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..Default::default()
    });
    assert!(StoredSchema::params(1, &empty).is_err());
}

#[test]
fn source_schemas_final_params_capacity_is_paid_independently_of_plan_state() {
    use super::super::{PlanCtx, parse};
    let b = ir::PlanBudget::default();
    let frozen = {
        let program = parse("struct S{x:bool}impl S{fn get(&self,x:u64)->u64{x}}fn f(a:bool,b:u64)->bool{a}fn main(){}").unwrap();
        PlanCtx::from_program(&program, &b)
            .unwrap()
            .source_schemas
            .clone()
    };
    for (_, stored) in frozen.functions.iter().flatten() {
        assert_eq!(
            stored.params_storage.bytes(),
            stored.value.signature.params.capacity() * size_of::<ir::Ty>()
        );
        assert!(
            stored
                .params_storage
                .authority()
                .same_authority(b.authority())
        );
    }
    let (_, decl) = fixture(&b);
    let empty = SourceSchemasBuilder::default()
        .seal(decl.snapshot, &b)
        .unwrap();
    let before = b.usage_snapshot().bytes;
    empty.region_env(decl, true, &b).unwrap();
    assert_eq!(
        b.usage_snapshot().bytes - before,
        arc_storage::<ir::SourceRegionEnv>().unwrap() + 2 * arc_storage::<()>().unwrap()
    );
}

#[test]
fn source_schemas_projection_rejects_same_id_wrong_payload_before_seal() {
    use super::super::{PlanCtx, parse};
    let b = ir::PlanBudget::default();
    let program = parse("fn f(x:bool)->bool{x}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &b).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let foreign = ir::SnapshotId::standalone().unwrap();
    let other_program = parse("fn other(){}fn main(){}").unwrap();
    let other_plan = PlanCtx::from_program(&other_program, &b).unwrap();
    let foreign_module = other_plan
        .declaration(other_plan.function_decl(0).unwrap())
        .unwrap()
        .path
        .module;
    for variant in 0..8 {
        let mut builder = SourceSchemasBuilder::default();
        builder
            .set_region_mode(
                declaration,
                tr::RegionContractMode::Positioned,
                &plan.registry,
                &b,
            )
            .unwrap();
        let position = table_position(&builder.region_modes.values, declaration).unwrap();
        let info = &mut builder.region_modes.values[position]
            .as_mut()
            .unwrap()
            .1
            .info;
        match variant {
            0 => info.declaration = id(foreign, declaration.index),
            1 => info.kind = ir::DeclKind::Const,
            2 => info.owner = Some(declaration),
            3 => {
                info.visibility = if info.visibility.is_public() {
                    ir::Visibility::Restricted(info.module)
                } else {
                    ir::Visibility::Public
                }
            }
            4 => {
                info.own_env = Arc::new(
                    ir::ParamEnv::new(ir::BinderId { declaration }, info.own_env.arity()).unwrap(),
                )
            }
            5 => info.env = Arc::new((*info.env).clone()),
            6 => info.module = foreign_module,
            7 => info.receiver = ir::ReceiverMode::Value,
            _ => unreachable!(),
        }
        assert!(
            builder.seal_from_registry(&plan.registry, &b).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn source_schemas_projection_hit_replaces_lookup_and_missing_const_uses_registry() {
    use super::super::{PlanCtx, SchemaAdapter, parse};
    use tr::SchemaCx;
    let b = ir::PlanBudget::default();
    let program = parse("trait Read{fn read(&self)->bool;}struct S;const C:u64=1u64;fn f(x:bool)->bool{x}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &b).unwrap();
    let function = plan.function_decl(0).unwrap();
    let constant = plan.registry.const_ids[0];
    let required = plan.registry.trait_items[0][0];
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        nominal_schemas: plan.nominal_schemas.clone(),
        nominals: plan.nominals.clone(),
        contexts: plan.contexts.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: b.clone(),
    };
    let before = b.usage_snapshot().work;
    let info = adapter.declaration_info(function).unwrap();
    assert_eq!(
        b.usage_snapshot().work - before,
        2,
        "one slot lookup and one allocation-free projection clone"
    );
    assert_eq!(info.declaration, function);
    assert!(Arc::ptr_eq(
        &info.env,
        &plan.declaration(function).unwrap().env
    ));
    for declaration in [constant, required] {
        assert!(
            plan.source_schemas
                .declaration_info(declaration, &b)
                .unwrap()
                .is_none()
        );
        let before = b.usage_snapshot().work;
        let info = adapter.declaration_info(declaration).unwrap();
        assert_eq!(
            b.usage_snapshot().work - before,
            3,
            "miss plus authoritative registry lookup and inline visibility clone"
        );
        assert_eq!(info.kind, plan.declaration(declaration).unwrap().kind);
        assert_eq!(
            info.region_contract_mode,
            tr::RegionContractMode::Positioned
        );
    }
    assert!(adapter.declaration_schema(constant).is_err());
    assert!(adapter.declaration_schema(required).is_ok());
    assert!(b.charge_work(usize::MAX).is_err());
    assert!(adapter.declaration_info(function).is_err());
    assert!(adapter.declaration_info(constant).is_err());
}
