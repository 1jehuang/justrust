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
        crate::compiler::backend::value_size(&ir::Ty::Int(ir::IntTy::U8)).unwrap(),
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
    tr::SchemaCx::replay_packed_reserve_authority(&adapter, &cap, id).unwrap();
    let error =
        tr::SchemaCx::replay_packed_reserve_authority(&DenyPacked(&adapter), &cap, id).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("source authority is unavailable")
    );
}
