//! Real graph-read constructor ingress authority and stock/native acceptance.
use super::*;
use crate::compiler::nominal_realization::{ReadNominal, RealizationKey, read_manifest};
use crate::compiler::{CompilerProvider, Input, SIGNATURES, Snapshot};
use crate::engine::{Context, Engine, Key, Provider, TaskFuture};

struct IngressProvider {
    snapshot: Arc<Snapshot>,
    signatures: Option<PlanCtx>,
}
impl Provider for IngressProvider {
    fn outputs(&self, key: &Key) -> usize {
        if key.kind == 997 {
            1
        } else {
            CompilerProvider(self.snapshot.clone()).outputs(key)
        }
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        if key.kind == SIGNATURES
            && let Some(plan) = self.signatures.clone()
        {
            return Box::pin(async move { cx.publish_fact(0, plan) });
        }
        if key.kind != 997 {
            return CompilerProvider(self.snapshot.clone()).start(cx, key);
        }
        Box::pin(async move {
            let handle = cx
                .request(crate::compiler::keys::SignaturesDebtRoot.key())
                .get(0);
            let plan: Arc<PlanCtx> = cx.read_fact(handle).await?;
            let declaration = plan
                .registry
                .declarations
                .iter()
                .find(|d| {
                    d.kind == ir::DeclKind::Struct && d.name.rsplit("::").next() == Some("Ingress")
                })
                .expect("exact source Ingress declaration")
                .id;
            let world = plan.trait_world().map_err(crate::engine::Error::new)?;
            let key =
                RealizationKey::encode(world.nominal_realization_world(), declaration, &[], 32)?;
            let mut capabilities = read_manifest(&cx, &[key], world.budget()).await?;
            cx.publish_fact(
                0,
                (plan.as_ref().clone(), declaration, capabilities.remove(0)),
            )
        })
    }
}

#[test]
fn missing_constructor_nominal_is_demanded_before_native_match() {
    crate::compiler::frontend::parser_tests::stock_native_run(
        "enum E{A(i32),B(i32)}fn main(){let value=E::A(42);match value{E::A(x)=>println!(\"{}\",x),E::B(x)=>println!(\"{}\",x)}}",
    );
}

#[test]
fn constructor_ingress_replays_source_application_snapshot_and_actuals() {
    let budget = ir::PlanBudget::default();
    let snapshot = Arc::new(Snapshot {
        input: Input::Text("struct Ingress;struct Other;fn main(){}".into()),
        source_options: crate::compiler::source::SourceOptions::default(),
        budget: budget.clone(),
    });
    let engine = Engine::new(IngressProvider {
        snapshot: snapshot.clone(),
        signatures: None,
    });
    let demand = engine.request(crate::compiler::keys::test_key(997, vec![]));
    engine.run(1).unwrap();
    demand.completion().unwrap();
    let fixture = demand
        .fact::<(PlanCtx, ir::DeclId, ReadNominal)>(0)
        .unwrap();
    let (plan, declaration, capability) = fixture.as_ref();
    let realized = capability.definition().unwrap();
    let (_, source) = plan.nominal_schema(*declaration, &budget).unwrap();
    let main = plan
        .registry
        .declarations
        .iter()
        .find(|d| d.kind == ir::DeclKind::Function && d.name.rsplit("::").next() == Some("main"))
        .expect("exact source main")
        .id;
    let signatures = plan.environment(Some(main)).unwrap();
    let function = plan.callable_body(&plan.program, main).unwrap();
    let declaration_schema = |_| Ok(None);
    let source_type = |_: &Type| Ok(None);
    let nominal_type = |_: &NominalDemand| Ok(Some(realized.clone()));
    let inputs = NominalPreparationInputs::new(&declaration_schema, &source_type, &nominal_type);
    let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    checker.declaration = Some(main);
    checker
        .install_lexical_scope(function.lexical_scope)
        .unwrap();
    checker.nominal_inputs = Some(&inputs);
    let converted = checker.realize_constructor_source(&source).unwrap();
    assert_eq!(converted, realized);
    {
        let absent = |_: &NominalDemand| Ok(None);
        let missing_inputs =
            NominalPreparationInputs::new(&declaration_schema, &source_type, &absent);
        let mut pending = Checker::new(&signatures, ir::Ty::Unit, function.module);
        pending.declaration = Some(main);
        pending
            .install_lexical_scope(function.lexical_scope)
            .unwrap();
        pending.nominal_inputs = Some(&missing_inputs);
        assert!(
            pending
                .realize_constructor_source(&source)
                .unwrap_err()
                .contains("missing graph input")
        );
        let blocked = pending.nominal_blocked.borrow();
        let Some(NominalPreparation::Blocked {
            nominals,
            declarations,
            needs_source_schemas,
            unresolved_constraints,
        }) = blocked.as_ref()
        else {
            panic!("missing constructor did not request its graph input")
        };
        assert_eq!(nominals.len(), 1);
        assert_eq!(nominals[0].declaration, *declaration);
        assert!(nominals[0].arguments.is_empty());
        assert!(declarations.is_empty());
        assert!(!needs_source_schemas && !unresolved_constraints);
    }
    assert_eq!(
        checker.realize_constructor_source(&realized).unwrap(),
        realized
    );
    // Reuse the exact plan/world, but issue an independently read realization in
    // another engine. This isolates engine authority from snapshot/world checks.
    let foreign_engine = Engine::new(IngressProvider {
        snapshot,
        signatures: Some(plan.clone()),
    });
    let foreign_demand = foreign_engine.request(crate::compiler::keys::test_key(997, vec![]));
    foreign_engine.run(1).unwrap();
    foreign_demand.completion().unwrap();
    let foreign_fixture = foreign_demand
        .fact::<(PlanCtx, ir::DeclId, ReadNominal)>(0)
        .unwrap();
    let foreign_graph = foreign_fixture.2.definition().unwrap();
    assert_eq!(foreign_fixture.1, *declaration);
    assert!(
        plan.trait_world().unwrap().nominal_realization_world()
            == foreign_fixture
                .0
                .trait_world()
                .unwrap()
                .nominal_realization_world()
    );
    assert!(
        checker.realize_constructor_source(&foreign_graph).is_err(),
        "constructor ingress laundered a same-world foreign-engine graph read"
    );
    // Source-to-graph transport recurses through structural actuals, but must
    // never replace an already graph-owned child with a current-engine lookup.
    for shape in 0..5 {
        let wrap = |ty: ir::Ty| match shape {
            0 => ir::Ty::Tuple(vec![ty].into()),
            1 => ir::Ty::Array {
                element: Arc::new(ty),
                len: 2,
            },
            2 => ir::Ty::Slice {
                element: Arc::new(ty),
            },
            3 => ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(ty),
            },
            _ => ir::Ty::RawPtr {
                kind: ir::BorrowKind::Shared,
                pointee: Arc::new(ty),
            },
        };
        assert_eq!(
            checker
                .realize_constructor_actual(&wrap(source.clone()), 0)
                .unwrap(),
            wrap(realized.clone()),
            "source actual transport failed for shape {shape}"
        );
        assert_eq!(
            checker
                .realize_constructor_actual(&wrap(realized.clone()), 0)
                .unwrap(),
            wrap(realized.clone()),
            "current graph actual changed for shape {shape}"
        );
        assert!(
            checker
                .realize_constructor_actual(&wrap(foreign_graph.clone()), 0)
                .is_err(),
            "foreign-engine graph actual laundered under shape {shape}"
        );
    }
    let ir::Ty::Struct(original) = &source else {
        panic!("struct source")
    };
    let other = plan
        .registry
        .declarations
        .iter()
        .find(|d| d.kind == ir::DeclKind::Struct && d.name.rsplit("::").next() == Some("Other"))
        .unwrap()
        .id;
    let (_, other_source) = plan.nominal_schema(other, &budget).unwrap();
    let ir::Ty::Struct(other_source) = other_source else {
        panic!("other struct")
    };
    let foreign = PlanCtx::from_program(
        &parse("struct Ingress;fn main(){}").unwrap(),
        &ir::PlanBudget::default(),
    )
    .unwrap();
    let foreign_declaration = foreign
        .registry
        .declarations
        .iter()
        .find(|d| d.kind == ir::DeclKind::Struct && d.name.rsplit("::").next() == Some("Ingress"))
        .unwrap()
        .id;
    let (_, foreign_source) = foreign
        .nominal_schema(foreign_declaration, &foreign.budget)
        .unwrap();
    for mode in 0..3 {
        let changed = if mode == 1 {
            foreign_source.clone()
        } else {
            let mut definition = original.def().clone();
            if mode == 0 {
                definition.id = other_source.id();
            } else {
                definition.type_args = Arc::from([ir::Ty::Bool]);
            }
            ir::Ty::Struct(ir::StructTy::new(definition))
        };
        assert!(
            checker.realize_constructor_source(&changed).is_err(),
            "constructor ingress accepted source forgery {mode}"
        );
    }
    // A genuine different declaration cannot be redirected to Ingress's typed output.
    assert!(
        checker
            .realize_constructor_source(&ir::Ty::Struct(other_source))
            .is_err()
    );
}

#[test]
fn raw_projection_inventory_uses_exact_closed_component_without_admitting_it() {
    for workers in [1, 8] {
        let budget = ir::PlanBudget::default();
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("struct Ingress{next:Option<Box<Ingress>>}fn main(){}".into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: budget.clone(),
        });
        let engine = Engine::new(IngressProvider {
            snapshot,
            signatures: None,
        });
        let demand = engine.request(crate::compiler::keys::test_key(997, vec![]));
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let fixture = demand
            .fact::<(PlanCtx, ir::DeclId, ReadNominal)>(0)
            .unwrap();
        let (_, declaration, capability) = fixture.as_ref();
        assert!(capability.is_component());
        let realized = capability.definition().unwrap();
        assert!(!raw_type_has_projection(&realized, &budget, 0).unwrap());
        // Projection inventory is not executable admission under raw facts.
        assert!(validate_argument(&realized, ir::FactsCtx::Concrete, &budget, 0).is_err());
        let ir::Ty::Struct(original) = &realized else {
            panic!("expected Ingress struct")
        };
        let mut changed = original.def().clone();
        changed.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged =
            ir::Ty::Struct(ir::StructTy::new(changed).with_realization(capability.clone()));
        assert!(raw_type_has_projection(&forged, &budget, 0).is_err());
        let projection = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: ir::TraitRef {
                declaration: *declaration,
                self_ty: realized.clone(),
                args: Arc::from([]),
            },
            item: *declaration,
        }));
        let wrapped = ir::Ty::Tuple(Arc::from([realized.clone(), projection]));
        assert!(raw_type_has_projection(&wrapped, &budget, 0).unwrap());
        // The finite height remains subject to the caller's unchanged depth limit.
        assert!(capability.component_metadata_height().unwrap() > 0);
        assert!(raw_type_has_projection(&realized, &budget, budget.limits().depth).is_err());
    }
}
