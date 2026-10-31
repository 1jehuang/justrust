use super::*;
use crate::compiler::{source, traits as tr};

fn genuine_plan() -> PlanCtx {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "packed-query-authority.rs",
            "fn main(){}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    let parsed = parse_prepared(&prepared).unwrap();
    let index = named(&parsed.program, "packed_reserve");
    let original = parsed.program.functions[index]
        .embedded_source
        .as_ref()
        .unwrap()
        .original()
        .unwrap();
    assert!(
        original.packed_query.is_some(),
        "genuine original query tokens must be retained"
    );
    let program = resolve(&parsed).unwrap();
    PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap()
}
fn named(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|ident| program.names.spelling(*ident).unwrap() == name)
        })
        .unwrap()
}
fn declaration(plan: &PlanCtx, name: &str) -> ir::DeclId {
    let index = named(&plan.program, name);
    plan.registry
        .declarations
        .iter()
        .find(|info| info.kind == ir::DeclKind::Function && info.source_index == index)
        .unwrap()
        .id
}

#[test]
fn packed_query_authority_genuine_loader_and_facts_replay() {
    let plan = genuine_plan();
    let declaration = declaration(&plan, "packed_reserve");
    let authority = plan.packed_reserve_authority(declaration).unwrap();
    authority
        .replay(&plan.program, &plan.registry, declaration, &plan.budget)
        .unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    // Exercise the actual checker ingress, not only the internal issuer API.
    type_callable_with_plan(&plan.program, declaration, &plan, &plan.budget).unwrap();
    facts.replay_packed_reserve_authority(&authority).unwrap();
    let query =
        ir::TypeAlignmentQuery::issue_packed(ir::Ty::Int(ir::IntTy::U8), &facts, authority.clone())
            .unwrap();
    assert_eq!(
        crate::compiler::memory_layout::query_value(
            &query,
            ir::FactsCtx::Validated(&facts),
            &plan.budget
        )
        .unwrap(),
        Some(1)
    );
    let target = ir::CallTarget {
        declaration,
        type_args: Arc::from([]),
    };
    let closed = plan.instance_facts(&target).unwrap();
    query
        .validate(ir::FactsCtx::Validated(&closed), &plan.budget)
        .unwrap();
    assert_eq!(
        crate::compiler::backend::value_size(&ir::Ty::Int(ir::IntTy::U8), &plan.budget).unwrap(),
        8
    );
    assert!(
        ir::TypeAlignmentQuery::issue_kind(
            ir::Ty::Int(ir::IntTy::U8),
            &facts,
            ir::TypeQueryKind::PackedByteReserveUnit
        )
        .is_err()
    );
    assert!(
        ir::TypeAlignmentQuery::issue_packed(ir::Ty::Int(ir::IntTy::U16), &facts, authority)
            .is_err()
    );
}

#[test]
fn packed_query_authority_occurrence_issuer_target_and_bound_transplants() {
    let plan = genuine_plan();
    let id = declaration(&plan, "packed_reserve");
    let cap = plan.packed_reserve_authority(id).unwrap();
    let reject = |changed: &PackedReserveAuthority| {
        assert!(
            changed
                .replay(&plan.program, &plan.registry, id, &plan.budget)
                .is_err()
        );
    };
    let mut changed = cap.clone();
    changed.query_event = changed.query_event.checked_add(1).unwrap();
    reject(&changed);
    changed = cap.clone();
    changed.query_token = original(&cap.source).unwrap().source;
    reject(&changed); // same issuer/file, wrong actual invocation range
    changed = cap.clone();
    changed.raw_realloc = id;
    reject(&changed);
    changed = cap.clone();
    changed.source = cap.callee_source.clone();
    reject(&changed);
    let foreign = genuine_plan();
    let foreign_id = declaration(&foreign, "packed_reserve");
    let foreign_cap = foreign.packed_reserve_authority(foreign_id).unwrap();
    reject(&foreign_cap);
    changed = cap.clone();
    changed.query_token = foreign_cap.query_token;
    reject(&changed);
    changed = cap.clone();
    changed.callee_source = foreign_cap.callee_source;
    reject(&changed);
    assert!(
        cap.replay(
            &foreign.program,
            &foreign.registry,
            foreign_id,
            &foreign.budget
        )
        .is_err()
    );
    let other = declaration(&plan, "raw_realloc");
    assert!(
        cap.replay(&plan.program, &plan.registry, other, &plan.budget)
            .is_err()
    );
    assert!(
        plan.callable_facts(other)
            .unwrap()
            .replay_packed_reserve_authority(&cap)
            .is_err()
    );
    let mut changed_program = plan.program.as_ref().clone();
    let index = cap.source.function_index;
    let Stmt::Let {
        value: Expr::TypeAlignment(queried),
        ..
    } = &mut changed_program.functions[index].body.statements[0]
    else {
        panic!("genuine query missing")
    };
    *queried = Type::Int(ir::IntTy::U16);
    assert!(
        cap.replay(&changed_program, &plan.registry, id, &plan.budget)
            .is_err()
    );
    let mut changed_program = plan.program.as_ref().clone();
    changed_program.functions[index].body.statements.swap(1, 2);
    assert!(
        cap.replay(&changed_program, &plan.registry, id, &plan.budget)
            .is_err()
    );
}

#[test]
fn packed_query_authority_raw_copy_spoof_and_generic_env_reject() {
    // Ordinary parse gets no loader seal, even when its body/module spellings
    // are byte-for-byte copies of the embedded helper.
    let copied = "fn packed_reserve(address:usize,old_count:usize,new_count:usize)->usize{let unit=__justrust_storage_stride!(u8);if new_count==0usize{return 0usize;}if old_count>18446744073709551615usize/unit||new_count>18446744073709551615usize/unit{return 0usize;}raw_realloc(address,old_count*unit,new_count*unit)}fn raw_realloc(a:usize,b:usize,c:usize)->usize{a}fn main(){}";
    let spoofed = format!("mod __justrust_std{{pub mod mem{{{copied}}}}}fn main(){{}}");
    for text in [copied, spoofed.as_str()] {
        let program = parse(text).unwrap();
        assert!(
            program
                .functions
                .iter()
                .all(|function| function.embedded_source.is_none())
        );
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        assert!(
            plan.packed_reserve_authority(declaration(&plan, "packed_reserve"))
                .is_err()
        );
    }
    let genuine = genuine_plan();
    let cap = genuine
        .packed_reserve_authority(declaration(&genuine, "packed_reserve"))
        .unwrap();
    let program = parse("fn f<T>(x:T)->T{x}fn main(){}").unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let facts = plan.callable_facts(declaration(&plan, "f")).unwrap();
    assert!(facts.replay_packed_reserve_authority(&cap).is_err());
    assert!(ir::TypeAlignmentQuery::issue_packed(ir::Ty::Int(ir::IntTy::U8), &facts, cap).is_err());
}

/// Same real adapter for all ordinary schema services, deliberately no source
/// replay override. Its inherited default must deny even a genuine capability.
struct DenyPacked<'a>(&'a SchemaAdapter);
impl tr::SchemaCx for DenyPacked<'_> {
    fn impl_trait_safety(
        &self,
        implementation: ir::DeclId,
        trait_decl: ir::DeclId,
    ) -> std::result::Result<(bool, bool), tr::FactsError> {
        tr::SchemaCx::impl_trait_safety(self.0, implementation, trait_decl)
    }
    fn budget(&self) -> &ir::PlanBudget {
        &self.0.budget
    }
    fn registry_snapshot(&self) -> ir::SnapshotId {
        tr::SchemaCx::registry_snapshot(self.0)
    }
    fn core_catalog(&self) -> &Arc<tr::CoreCatalog> {
        &self.0.catalog
    }
    fn declaration_info(
        &self,
        id: ir::DeclId,
    ) -> std::result::Result<tr::SchemaDecl, tr::FactsError> {
        tr::SchemaCx::declaration_info(self.0, id)
    }
    fn declaration_schema(
        &self,
        id: ir::DeclId,
    ) -> std::result::Result<tr::GenericSchema, tr::FactsError> {
        tr::SchemaCx::declaration_schema(self.0, id)
    }
    fn source_region_env(
        &self,
        id: ir::DeclId,
    ) -> std::result::Result<Arc<ir::SourceRegionEnv>, tr::FactsError> {
        tr::SchemaCx::source_region_env(self.0, id)
    }
    fn instantiate_schema(
        &self,
        schema: &tr::GenericSchema,
        args: &[ir::Ty],
        caller: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> std::result::Result<tr::InstantiatedSchema, tr::FactsError> {
        tr::SchemaCx::instantiate_schema(self.0, schema, args, caller, budget)
    }
    fn validate_nominal(
        &self,
        ty: &ir::Ty,
        env: &ir::GenericEnv,
        budget: &ir::PlanBudget,
    ) -> std::result::Result<(), tr::FactsError> {
        tr::SchemaCx::validate_nominal(self.0, ty, env, budget)
    }
}
#[test]
fn packed_query_authority_alternate_schema_default_denies() {
    let plan = genuine_plan();
    let id = declaration(&plan, "packed_reserve");
    let cap = plan.packed_reserve_authority(id).unwrap();
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        nominal_schemas: plan.nominal_schemas.clone(),
        nominals: plan.nominals.clone(),
        contexts: plan.contexts.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: plan.budget.clone(),
    };
    let slot = declaration(&plan, "slot");
    assert!(
        tr::SchemaCx::native_sequence_slot(&adapter, slot)
            .unwrap()
            .is_some()
    );
    assert!(
        tr::SchemaCx::native_sequence_slot(&DenyPacked(&adapter), slot)
            .unwrap()
            .is_none()
    );
    tr::SchemaCx::replay_packed_reserve_authority(&adapter, &cap, id).unwrap();
    let error =
        tr::SchemaCx::replay_packed_reserve_authority(&DenyPacked(&adapter), &cap, id).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("source authority is unavailable")
    );
}

#[test]
fn native_swap_source_genuine_checked_pair_and_forged_cfg_reject() {
    let plan = genuine_plan();
    let declarations: Vec<_> = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .filter(|id| {
            native_sequence::issue_swap(&plan.registry, *id, &plan.budget)
                .unwrap()
                .is_some()
        })
        .collect();
    assert_eq!(
        declarations.len(),
        1,
        "only the exact issued Vec exchange recipe"
    );
    let id = declarations[0];
    let facts = plan.callable_facts(id).unwrap();
    assert!(facts.native_sequence_swap(id).unwrap().is_some());
    let checked = type_callable_with_plan(&plan.program, id, &plan, &plan.budget).unwrap();
    let template =
        crate::compiler::cfg::lower_template(Arc::new(checked), facts.clone(), &plan.budget)
            .unwrap();
    let proof = crate::compiler::ownership::verify_template(template.clone())
        .unwrap_or_else(|analysis| panic!("{:?}", analysis.diagnostics));
    assert!(proof.analysis().passed());
    assert!(
        proof.carrier_effects().is_some(),
        "paired storage exchange needs a real proof"
    );
    let original = template.function().as_ref();
    let first_slot = original
        .blocks
        .iter()
        .position(|block| {
            matches!(
                block.terminator.value,
                crate::compiler::cfg::Terminator::Call { .. }
            )
        })
        .map(crate::compiler::cfg::BlockId)
        .unwrap();
    assert_ne!(original.entry, first_slot);
    let mut bypassed_entry = original.clone();
    bypassed_entry.entry = first_slot;
    assert!(
        bypassed_entry
            .validate_provenance(&facts, &plan.budget)
            .is_err()
    );

    let mut changed_guard_edges = 0usize;
    let mut changed_guards = 0usize;
    for (index, block) in original.blocks.iter().enumerate().take(first_slot.0) {
        let crate::compiler::cfg::Terminator::Switch {
            then_block,
            else_block,
            ..
        } = &block.terminator.value
        else {
            continue;
        };
        changed_guards += 1;
        for (change_then, successor) in [(true, *then_block), (false, *else_block)] {
            if successor == first_slot {
                continue;
            }
            let mut bypassed_guard = original.clone();
            let crate::compiler::cfg::Terminator::Switch {
                then_block,
                else_block,
                ..
            } = &mut bypassed_guard.blocks[index].terminator.value
            else {
                unreachable!();
            };
            if change_then {
                *then_block = first_slot;
            } else {
                *else_block = first_slot;
            }
            assert!(
                bypassed_guard
                    .validate_provenance(&facts, &plan.budget)
                    .is_err()
            );
            changed_guard_edges += 1;
        }
    }
    assert!(
        changed_guards >= 2,
        "bounds and alias guards must both be covered"
    );
    assert!(
        changed_guard_edges >= 2,
        "bounds and alias successor bypass controls"
    );

    let mut forged = template.function().as_ref().clone();
    let block = forged
        .blocks
        .iter_mut()
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(&statement.value,crate::compiler::cfg::Statement::Assign {destination,..}
            if destination.projection.as_slice()==[ir::Projection::Deref])
            })
        })
        .unwrap();
    let store = block
        .statements
        .iter()
        .find(|statement| {
            matches!(&statement.value,crate::compiler::cfg::Statement::Assign {destination,..}
            if destination.projection.as_slice()==[ir::Projection::Deref])
        })
        .unwrap()
        .clone();
    block.statements.push(store);
    assert!(forged.validate_provenance(&facts, &plan.budget).is_err());
}

#[test]
fn native_swap_source_wrong_owner_recipe_context_issuer_and_poison_reject() {
    let plan = genuine_plan();
    let id = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|id| {
            native_sequence::issue_swap(&plan.registry, *id, &plan.budget)
                .unwrap()
                .is_some()
        })
        .unwrap();
    let position = plan.registry.function_position(id).unwrap();
    let CallableSource::Associated {
        implementation,
        item,
    } = plan.registry.callable_sources[position]
    else {
        panic!("swap must be associated")
    };
    assert!(
        native_sequence::issue_swap(&plan.registry, declaration(&plan, "slot"), &plan.budget)
            .unwrap()
            .is_none()
    );
    let foreign = genuine_plan();
    assert!(matches!(
        native_sequence::issue_swap(&foreign.registry, id, &foreign.budget),
        Ok(None) | Err(_)
    ));
    let registry_copy = || {
        let mut registry = plan.registry.empty_like();
        registry.declarations = plan.registry.declarations.clone();
        registry.slots = plan.registry.slots.clone();
        registry.functions = plan.registry.functions.clone();
        registry.function_ids = plan.registry.function_ids.clone();
        registry.callable_sources = plan.registry.callable_sources.clone();
        assert!(
            native_sequence::issue_swap(&registry, id, &plan.budget)
                .unwrap()
                .is_some()
        );
        registry
    };
    for mutation in 0..4 {
        let mut registry = registry_copy();
        let mut program = plan.program.as_ref().clone();
        let function = &mut program.impls[implementation].items[item].function;
        match mutation {
            0 => function.body.statements.swap(2, 3),
            1 => {
                let Some(Expr::Unsafe(block)) = function.body.tail.as_deref_mut() else {
                    panic!("unsafe recipe")
                };
                block.statements.swap(1, 2);
            }
            2 => {
                let Some(Expr::Unsafe(block)) = function.body.tail.as_deref_mut() else {
                    panic!("unsafe recipe")
                };
                block.statements.push(block.statements[1].clone());
            }
            3 => {
                let Stmt::ImplicitExpr(Expr::If { condition, .. }) =
                    &mut function.body.statements[1]
                else {
                    panic!("alias guard")
                };
                let Expr::Binary { op, .. } = condition.as_mut() else {
                    panic!("alias compare")
                };
                *op = BinaryOp::Ne;
            }
            _ => unreachable!(),
        }
        registry.program = Arc::new(program);
        assert!(
            native_sequence::issue_swap(&registry, id, &plan.budget)
                .unwrap()
                .is_none(),
            "mutation={mutation}"
        );
    }
    let mut registry = registry_copy();
    let mut program = plan.program.as_ref().clone();
    program.embedded_issuers = Arc::from([]);
    registry.program = Arc::new(program);
    assert!(
        native_sequence::issue_swap(&registry, id, &plan.budget)
            .unwrap()
            .is_none()
    );
    let poison = ir::PlanBudget::default();
    poison.poison_ledger_for_tests();
    assert!(native_sequence::issue_swap(&plan.registry, id, &poison).is_err());
}

#[test]
fn native_slot_source_genuine_and_universal_initialized_store() {
    let plan = genuine_plan();
    let slot = declaration(&plan, "slot");
    assert!(
        native_sequence::issue_slot(&plan.registry, slot, &plan.budget)
            .unwrap()
            .is_some()
    );
    let push =
        (0..plan.callable_count())
            .map(|index| plan.callable_decl(index).unwrap())
            .find(|declaration| {
                plan.registry
                    .source_callable(*declaration)
                    .is_some_and(|function| {
                        function.name.segments().last().is_some_and(|name| {
                            plan.program.names.spelling(*name).unwrap() == "push"
                        })
                    })
            })
            .unwrap();
    let facts = plan.callable_facts(push).unwrap();
    assert!(facts.native_sequence_slot(slot).unwrap().is_some());
    let checked = type_callable_with_plan(&plan.program, push, &plan, &plan.budget).unwrap();
    let template =
        crate::compiler::cfg::lower_template(Arc::new(checked), facts, &plan.budget).unwrap();
    let proof = crate::compiler::ownership::verify_template(template)
        .unwrap_or_else(|analysis| panic!("{:?}", analysis.diagnostics));
    assert!(proof.analysis().passed());
    assert!(
        proof.carrier_effects().is_some(),
        "issued initialized slot store requires a real effect proof"
    );
}

#[test]
fn native_slot_source_rejects_cross_kind_stride_even_when_values_coincide() {
    let plan = genuine_plan();
    let slot = declaration(&plan, "slot");
    for wrong in [
        "size_of",
        "storage_stride",
        "storage_alignment",
        "pointer_stride",
    ] {
        let mut program = plan.program.as_ref().clone();
        let wrong_path = program.functions[named(&plan.program, wrong)].name.clone();
        let Some(Expr::Cast { value, .. }) = program.functions[named(&plan.program, "slot")]
            .body
            .tail
            .as_deref_mut()
        else {
            panic!("slot cast")
        };
        let Expr::Binary { right, .. } = value.as_mut() else {
            panic!("slot sum")
        };
        let Expr::Binary { right, .. } = right.as_mut() else {
            panic!("slot product")
        };
        let Expr::Apply { path, value } = right.as_mut() else {
            panic!("stride application")
        };
        let arguments = path.segments.last().unwrap().args.clone();
        path.anchor = wrong_path.anchor();
        path.resolved_use = None;
        path.segments = wrong_path
            .segments()
            .iter()
            .map(|name| PathSegment {
                name: *name,
                region_args: Vec::new(),
                args: Vec::new(),
                associated_types: Vec::new(),
            })
            .collect();
        path.segments.last_mut().unwrap().args = arguments;
        let Expr::Call { name, .. } = value.as_mut() else {
            panic!("stride call")
        };
        *name = wrong_path;
        // Exercise the sealed role check itself, not an earlier registration
        // failure. Only genuine registry rows are copied into this malformed
        // fixture. It never becomes a PlanCtx or graph publication.
        let mut registry = plan.registry.empty_like();
        registry.declarations = plan.registry.declarations.clone();
        registry.slots = plan.registry.slots.clone();
        registry.functions = plan.registry.functions.clone();
        registry.function_ids = plan.registry.function_ids.clone();
        registry.callable_sources = plan.registry.callable_sources.clone();
        registry.program = Arc::new(program);
        assert!(
            native_sequence::issue_slot(&registry, slot, &plan.budget)
                .unwrap()
                .is_none(),
            "{wrong} cannot grant element-slot authority"
        );
    }
    let mut program = plan.program.as_ref().clone();
    program.functions[named(&plan.program, "element_stride")]
        .body
        .tail = Some(Box::new(Expr::TypeAlignment(Type::Int(ir::IntTy::U8))));
    let mut registry = plan.registry.empty_like();
    registry.declarations = plan.registry.declarations.clone();
    registry.slots = plan.registry.slots.clone();
    registry.functions = plan.registry.functions.clone();
    registry.function_ids = plan.registry.function_ids.clone();
    registry.callable_sources = plan.registry.callable_sources.clone();
    registry.program = Arc::new(program);
    assert!(
        native_sequence::issue_slot(&registry, slot, &plan.budget)
            .unwrap()
            .is_none()
    );
}

#[test]
fn native_slot_source_copy_wrong_recipe_context_and_poison_reject() {
    let plan = genuine_plan();
    let slot = declaration(&plan, "slot");
    assert!(
        native_sequence::issue_slot(&plan.registry, declaration(&plan, "size_of"), &plan.budget)
            .unwrap()
            .is_none()
    );
    let foreign = genuine_plan();
    assert!(matches!(
        native_sequence::issue_slot(&foreign.registry, slot, &foreign.budget),
        Ok(None) | Err(_)
    ));
    let mut changed = plan.program.as_ref().clone();
    changed.functions[named(&plan.program, "slot")].body.tail =
        Some(Box::new(Expr::Integer(IntegerLiteral {
            magnitude: 0,
            suffix: Some(ir::IntTy::Usize),
        })));
    let changed = PlanCtx::from_program(&changed, &ir::PlanBudget::default()).unwrap();
    assert!(
        native_sequence::issue_slot(
            &changed.registry,
            declaration(&changed, "slot"),
            &changed.budget
        )
        .unwrap()
        .is_none()
    );
    let mut unissued = plan.program.as_ref().clone();
    unissued.embedded_issuers = Arc::from([]);
    let registration_error =
        PlanCtx::from_program(&unissued, &ir::PlanBudget::default()).unwrap_err();
    assert!(
        registration_error
            .contains("embedded function registration changed original source or occurrence"),
        "{registration_error}"
    );
    // Exercise the issuer boundary independently of the earlier registration
    // rejection. Copy only genuine registry rows, then remove the issuer from
    // its source snapshot. This malformed fixture never becomes a PlanCtx.
    let mut unissued_registry = plan.registry.empty_like();
    unissued_registry.declarations = plan.registry.declarations.clone();
    unissued_registry.slots = plan.registry.slots.clone();
    unissued_registry.functions = plan.registry.functions.clone();
    unissued_registry.function_ids = plan.registry.function_ids.clone();
    unissued_registry.callable_sources = plan.registry.callable_sources.clone();
    assert!(
        native_sequence::issue_slot(&unissued_registry, slot, &plan.budget)
            .unwrap()
            .is_some()
    );
    unissued_registry.program = Arc::new(unissued);
    assert!(
        native_sequence::issue_slot(&unissued_registry, slot, &plan.budget)
            .unwrap()
            .is_none()
    );
    let copied = parse("fn size_of<T>()->usize{let base=4096usize as *const[T;2];let second=unsafe{&(*base)[1]}as *const T;second as usize-4096usize}fn slot<T>(base:*mut T,index:usize)->*mut T{(base as usize+index*size_of::<T>())as *mut T}fn main(){}").unwrap();
    let copied = PlanCtx::from_program(&copied, &ir::PlanBudget::default()).unwrap();
    assert!(
        native_sequence::issue_slot(
            &copied.registry,
            declaration(&copied, "slot"),
            &copied.budget
        )
        .unwrap()
        .is_none()
    );
    let poison = ir::PlanBudget::default();
    poison.poison_ledger_for_tests();
    assert!(native_sequence::issue_slot(&plan.registry, slot, &poison).is_err());
}
