use super::*;

#[test]
fn marker_probe_defers_source_associated_constant_owners() {
    let text = "struct Number;impl Number{const GOOD:i32=7;const OTHER:i32=Self::GOOD;}fn read()->i32{Number::GOOD}fn main(){let _:i32=crate::Number::GOOD;}";
    let source = parse(text).unwrap();
    assert!(matches!(
        source.functions[0].body.tail.as_deref(),
        Some(Expr::AssociatedConst(_))
    ));
    assert!(matches!(
        &source.functions[1].body.statements[0],
        Stmt::LetPattern {
            pattern: Pattern::Wildcard,
            ty: Type::Int(ir::IntTy::I32),
            value: Expr::AssociatedConst(_),
        }
    ));
    let budget = ir::PlanBudget::default();
    PlanCtx::from_program(&source, &budget).unwrap();
    for workers in [1, 8] {
        crate::compiler::opaque_source_tests::source_check(text, workers).unwrap();
    }
}

#[test]
fn catalog_marker_does_not_create_a_source_nominal_schema() {
    let source = parse("struct Cell<T>{value:T}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&source, &budget).unwrap();
    let marker = plan.catalog.id(tr::CatalogItem::PhantomData);
    let info = plan.declaration(marker).unwrap();
    assert_eq!(info.param_env.arity(), 1);
    assert!(matches!(
        info.path.item,
        DeclItemName::Catalog(tr::CatalogItem::PhantomData)
    ));
    assert!(plan.nominal_schema(marker, &budget).is_err());
}

#[test]
fn phantom_data_catalog_aliases_preserve_source_and_canonical_argument_identity() {
    let source = parse("use core::marker::PhantomData as Renamed;fn a(x:core::marker::PhantomData<i32>){}fn b(x:std::marker::PhantomData<i32>){}fn c(x:Renamed<i32>){}fn main(){let x:Renamed<i32>=Renamed;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&source, &budget).unwrap();
    let mut actual = None;
    for index in 0..3 {
        let schema = plan
            .function_schema(plan.function_decl(index).unwrap())
            .unwrap();
        let ir::Ty::PhantomData(marker) = &schema.signature.params[0] else {
            panic!("canonical marker became a source nominal");
        };
        assert_eq!(
            marker.declaration(),
            plan.catalog.id(tr::CatalogItem::PhantomData)
        );
        assert_eq!(marker.argument(), &ir::Ty::Int(ir::IntTy::I32));
        if let Some(expected) = &actual {
            assert_eq!(&schema.signature.params[0], expected);
        }
        actual = Some(schema.signature.params[0].clone());
    }
    let Type::PhantomData(alias) = &source.functions[2].params[0].1 else {
        panic!("lost source marker use");
    };
    let written = alias.original.checked_path(source.names.as_ref()).unwrap();
    assert_eq!(
        source.names.spelling(written.single().unwrap()).unwrap(),
        "Renamed"
    );
    assert!(
        !alias
            .occurrence
            .source_occurrences(source.names.as_ref())
            .unwrap()
            .is_empty()
    );
    let Stmt::Let {
        value: Expr::PhantomData(constructor),
        ..
    } = &source.functions[3].body.statements[0]
    else {
        panic!("bare marker constructor became a source struct or function item");
    };
    assert!(constructor.argument.is_none());

    let foreign = parse("fn a(x:core::marker::PhantomData<i32>){}fn main(){}").unwrap();
    let mut transplanted = source.clone();
    transplanted.functions[0].params[0].1 = foreign.functions[0].params[0].1.clone();
    assert!(name_validation::validate_program_names(&transplanted).is_err());
    assert!(PlanCtx::from_program(&transplanted, &budget).is_err());

    let local = parse("struct PhantomData{n:i32}fn f(x:PhantomData){}fn main(){}").unwrap();
    assert!(matches!(local.functions[0].params[0].1, Type::Named(_)));
    assert!(parse("fn f(x:PhantomData<i32>){}fn main(){}").is_err());
}

#[test]
fn unsafe_trait_impl_safety_replays_original_pair_and_source_flags() {
    let source =
        parse("unsafe trait A{}trait B{}struct S;unsafe impl A for S{}impl B for S{}fn main(){}")
            .unwrap();
    assert!(source.traits[0].unsafety);
    assert!(!source.traits[1].unsafety);
    assert!(source.impls[0].unsafety);
    assert!(!source.impls[1].unsafety);
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&source, &budget).unwrap();
    let mut adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        nominal_schemas: plan.nominal_schemas.clone(),
        nominals: plan.nominals.clone(),
        contexts: plan.contexts.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: budget.clone(),
    };
    struct OmittedSafety<'a>(&'a SchemaAdapter);
    impl tr::SchemaCx for OmittedSafety<'_> {
        fn budget(&self) -> &ir::PlanBudget {
            &self.0.budget
        }
        fn registry_snapshot(&self) -> ir::SnapshotId {
            tr::SchemaCx::registry_snapshot(self.0)
        }
        fn core_catalog(&self) -> &Arc<tr::CoreCatalog> {
            &self.0.catalog
        }
        fn declaration_info(&self, id: ir::DeclId) -> Result<tr::SchemaDecl, tr::FactsError> {
            tr::SchemaCx::declaration_info(self.0, id)
        }
        fn declaration_schema(&self, id: ir::DeclId) -> Result<tr::GenericSchema, tr::FactsError> {
            tr::SchemaCx::declaration_schema(self.0, id)
        }
        fn source_region_env(
            &self,
            id: ir::DeclId,
        ) -> Result<Arc<ir::SourceRegionEnv>, tr::FactsError> {
            tr::SchemaCx::source_region_env(self.0, id)
        }
        fn instantiate_schema(
            &self,
            schema: &tr::GenericSchema,
            args: &[ir::Ty],
            caller: &ir::GenericEnv,
            budget: &ir::PlanBudget,
        ) -> Result<tr::InstantiatedSchema, tr::FactsError> {
            tr::SchemaCx::instantiate_schema(self.0, schema, args, caller, budget)
        }
        fn validate_nominal(
            &self,
            ty: &ir::Ty,
            env: &ir::GenericEnv,
            budget: &ir::PlanBudget,
        ) -> Result<(), tr::FactsError> {
            tr::SchemaCx::validate_nominal(self.0, ty, env, budget)
        }
    }
    let first = plan.registry.impl_ids[0];
    let second = plan.registry.impl_ids[1];
    let a = plan.registry.trait_ids[0];
    let b = plan.registry.trait_ids[1];
    assert_eq!(
        tr::SchemaCx::impl_trait_safety(&adapter, first, a).unwrap(),
        (true, true)
    );
    let omitted = tr::SchemaCx::impl_trait_safety(&OmittedSafety(&adapter), first, a).unwrap_err();
    assert!(format!("{omitted:?}").contains("unsupported"));
    assert_eq!(
        tr::SchemaCx::impl_trait_safety(&adapter, second, b).unwrap(),
        (false, false)
    );
    assert!(tr::SchemaCx::impl_trait_safety(&adapter, first, b).is_err());
    assert!(tr::SchemaCx::impl_trait_safety(&adapter, second, a).is_err());
    assert!(tr::SchemaCx::impl_trait_safety(&adapter, a, a).is_err());
    let mut erased = plan.program.as_ref().clone();
    erased.traits[0].unsafety = false;
    erased.impls[0].unsafety = false;
    adapter.program = Arc::new(erased);
    assert!(tr::SchemaCx::impl_trait_safety(&adapter, first, a).is_err());
    adapter.program = Arc::new(
        parse("trait A{}trait B{}struct S;impl A for S{}impl B for S{}fn main(){}").unwrap(),
    );
    assert!(tr::SchemaCx::impl_trait_safety(&adapter, first, a).is_err());
    for (text, diagnostic) in [
        (
            "unsafe trait A{}struct S;impl A for S{}fn main(){}",
            "E0200",
        ),
        (
            "trait A{}struct S;unsafe impl A for S{}fn main(){}",
            "E0199",
        ),
    ] {
        let source = parse(text).unwrap();
        let error = PlanCtx::from_program(&source, &ir::PlanBudget::default()).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
    assert!(
        parse("struct S;unsafe impl S{}fn main(){}")
            .unwrap_err()
            .contains("E0197")
    );
}

// Text selects an authored fixture witness only. Lookup authority is the
// checked path retained by that same parsed Program, never a rebuilt name.
fn fixture_function_key(program: &Program, expected: &str) -> PathKey {
    let path = &program
        .functions
        .iter()
        .find(|function| {
            function
                .name
                .display(program.names.as_ref())
                .unwrap()
                .as_str()
                .strip_prefix("crate::")
                == Some(expected)
        })
        .unwrap()
        .name;
    path.check(program.names.as_ref()).unwrap();
    path.key(Namespace::Value)
}

fn fixture_nominal_key(program: &Program, expected: &str) -> PathKey {
    let path = program
        .structs
        .iter()
        .map(|item| &item.name)
        .chain(program.enums.iter().map(|item| &item.name))
        .find(|path| {
            path.display(program.names.as_ref())
                .unwrap()
                .as_str()
                .strip_prefix("crate::")
                == Some(expected)
        })
        .unwrap();
    path.check(program.names.as_ref()).unwrap();
    path.key(Namespace::Type)
}

#[test]
fn canonical_copy_identity_and_source_supertraits_authorize_only_real_bounds() {
    let source = "trait Copy{fn fake(&self)->i32;}trait Clone{fn fake_clone(&self)->i32;}struct S{v:i32}impl core::clone::Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl core::marker::Copy for S{}fn twice<T:std::marker::Copy>(x:T)->(T,T){(x,x)}fn main(){let s=S{v:3};let p=twice::<S>(s);let n=p.0.v+p.1.v;}";
    let p = parse(source).unwrap();
    check(&p).unwrap();
    let p = parse("trait Base{fn base(&self)->i32;}trait Child:Base{fn child(&self)->i32;}struct S{v:i32}impl Base for S{fn base(&self)->i32{self.v}}impl Child for S{fn child(&self)->i32{self.v}}fn via<T:Child>(x:&T)->i32{x.base()+x.child()}fn main(){}").unwrap();
    check(&p).unwrap();
    let fake = parse("trait Copy{fn tag(&self)->i32;}struct S{v:i32}impl Copy for S{fn tag(&self)->i32{self.v}}fn main(){let s=S{v:1};let a=s;let b=s;let n=a.v+b.v;}").unwrap();
    assert!(check(&fake).is_err());
}

#[test]
fn canonical_clone_receiver_order_and_primitive_temporaries_use_real_borrows() {
    let p = parse("struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl S{fn set(&mut self,n:i32){self.v=n;}}fn carrier(x:&mut S)->&mut S{x}struct Opaque{v:i32}fn main(){let s=S{v:12};let r=&s;let value=r.clone();let owner=Opaque{v:13};let shared=&owner;let handle=shared.clone();let n=10i32.clone();let yes=true.clone();let unit=().clone();let used=n+value.v+handle.v;let mut temporary=S{v:1};carrier(&mut temporary).set(2);}").unwrap();
    let signatures = signature_table(&p).unwrap();
    let checked = type_function_with_signatures(&p, 1, &signatures).unwrap();
    let receiver = checked
        .locals
        .iter()
        .position(|local| local.name == "$method_receiver")
        .map(ir::LocalId)
        .expect("owned receiver temporary");
    assert!(checked.exprs.iter().any(|e| matches!(&e.kind, ir::ExprKind::Block(block) if block.statements.iter().any(|s| matches!(s, ir::TypedStmt::Declare { local } if *local == receiver)))));
    assert!(checked.exprs.iter().any(|e| matches!(&e.kind, ir::ExprKind::Block(block) if block.statements.iter().any(|s| matches!(s, ir::TypedStmt::Assign { local, .. } if *local == receiver)))));
    assert!(checked.exprs.iter().any(|e|matches!(&e.ty,ir::Ty::Ref{referent,..} if matches!(referent.as_ref(),ir::Ty::Struct(t) if t.def().name=="crate::Opaque"))));
    check(&p).unwrap();
}

#[test]
fn registered_recipe_bodies_share_counting_and_universal_verification() {
    let p = Arc::new(parse("fn main(){let tuple=(1i32,true);let array:[i32;0]=[];}").unwrap());
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_shared_program(p.clone(), &budget).unwrap();
    let mut recipes_seen = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        if let CallableProvider::Recipe { descriptor } =
            plan.callable_provider(declaration).unwrap()
        {
            recipes_seen += 1;
            assert!(
                matches!(plan.declaration_origin(declaration).unwrap(),tr::DeclOrigin::CoreCatalog{item:tr::CatalogItem::Recipe(d),..} if d==descriptor)
            );
            let env = plan.callable_env(declaration).unwrap();
            let target = ir::CallTarget {
                declaration,
                type_args: env.ordered_params().map(ir::Ty::Param).collect(),
            };
            let facts = plan.callable_facts(declaration).unwrap();
            let checked = plan.checked_body(&target, &facts, &budget).unwrap();
            let template =
                super::super::cfg::lower_template(Arc::new(checked), facts, &budget).unwrap();
            super::super::ownership::verify_template(template).unwrap();
        }
    }
    assert!(recipes_seen >= 14);
    let counts = recipes::counts(tr::RecipeDescriptor::CloneTuple { arity: 33 }, &budget).unwrap();
    assert!(matches!(
        super::super::ownership::preflight_recipe_counts(&counts, &budget).unwrap(),
        super::super::ownership::RecipeAdmission::IntrinsicLimit(_)
    ));
}

fn input_region(param: usize, region: u16) -> ir::InputRegion {
    ir::InputRegion {
        input: ir::InputReference {
            param,
            position: ir::ReferencePosition { depth: 0 },
        },
        region: ir::RegionParamId(region),
    }
}

#[test]
fn nested_shared_contracts_preserve_positions_aliases_and_implied_outlives() {
    let p = parse("fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32{*rr}fn renamed<'z,'w>(rr:&'z &'w i32)->&'w i32{*rr}fn id<'a,'b>(rr:&'a &'b i32)->&'a &'b i32{rr}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    let inner = &env.functions[&fixture_function_key(&p, "inner")].ref_contract;
    assert_eq!(
        inner,
        &env.functions[&fixture_function_key(&p, "renamed")].ref_contract
    );
    assert_eq!(
        inner.input_regions.as_ref(),
        &[
            input_region(0, 0),
            ir::InputRegion {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 1 }
                },
                region: ir::RegionParamId(1)
            },
        ]
    );
    assert_eq!(
        inner.outlives.as_ref(),
        &[(ir::RegionParamId(1), ir::RegionParamId(0))]
    );
    assert_eq!(inner.results[0].region, ir::RegionParamId(1));
    assert_eq!(
        inner.results[0].returns,
        ir::ReturnContract::SharedInputPaths(
            vec![ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 1 }
                },
                projection: Arc::from([])
            }]
            .into()
        )
    );
    let id = &env.functions[&fixture_function_key(&p, "id")].ref_contract;
    assert_eq!(id.results.len(), 2);
    for depth in 0..2 {
        assert_eq!(id.results[depth].position.depth, depth as u8);
        assert_eq!(
            id.results[depth].returns,
            ir::ReturnContract::SharedInputPaths(
                vec![ir::InputPath {
                    input: ir::InputReference {
                        param: 0,
                        position: ir::ReferencePosition { depth: depth as u8 }
                    },
                    projection: Arc::from([])
                }]
                .into()
            )
        );
    }
    for index in 0..p.functions.len() {
        check_function_with_signatures(&p, index, &env).unwrap();
    }
}

#[test]
fn nested_shared_source_authority_and_closed_boundaries_remain_distinct() {
    let p = parse("trait Read{fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32;}fn legacy(r:&i32){}fn named<'a>(r:&i32){}fn nested<'a,'b>(r:&'a &'b i32){}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let adapter = SchemaAdapter {
        program: plan.program.clone(),
        catalog: plan.catalog.clone(),
        registry: plan.registry.clone(),
        nominal_schemas: plan.nominal_schemas.clone(),
        nominals: plan.nominals.clone(),
        contexts: plan.contexts.clone(),
        source_schemas: plan.source_schemas.clone(),
        budget: budget.clone(),
    };
    for (index, mode) in [
        (0, tr::RegionContractMode::LegacyUnnamedFreeNoReturn),
        (1, tr::RegionContractMode::Positioned),
        (2, tr::RegionContractMode::Positioned),
    ] {
        let decl = plan.function_decl(index).unwrap();
        assert_eq!(
            tr::SchemaCx::declaration_info(&adapter, decl)
                .unwrap()
                .region_contract_mode,
            mode
        );
        assert_eq!(
            tr::SchemaCx::declaration_schema(&adapter, decl)
                .unwrap()
                .signature,
            plan.raw_schema(decl).unwrap().signature
        );
    }
    let item = plan.registry.trait_items[0][0];
    assert_eq!(
        tr::SchemaCx::declaration_info(&adapter, item)
            .unwrap()
            .region_contract_mode,
        tr::RegionContractMode::Positioned
    );
    assert_eq!(
        tr::SchemaCx::declaration_schema(&adapter, item)
            .unwrap()
            .signature
            .ref_contract
            .input_regions
            .len(),
        2
    );
    for source in [
        "fn f(x:&&i32)->&i32{*x}fn main(){}",
        "fn f<'a>(x:&'a &'a i32)->&i32{*x}fn main(){}",
    ] {
        assert!(
            signature_table(&parse(source).unwrap())
                .unwrap_err()
                .contains("missing lifetime specifier")
        );
    }
    parse("struct Holder<'a>{r:&'a i32}fn main(){}").unwrap();
    assert!(
        parse("struct Holder<'fn>{r:i32}fn main(){}")
            .unwrap_err()
            .contains("keyword lifetime names")
    );
    for source in [
        "fn f(x:&&mut i32){}fn main(){}",
        "fn f(x:&mut &i32){}fn main(){}",
        "fn f(x:&&&i32){}fn main(){}",
        "fn f<T>(x:T){}fn main(){let x=1;let r=&x;f::<&i32>(r);}",
        "fn main(){let x=1;let r=&x;let rr=(&r,);}",
    ] {
        parse(source)
            .and_then(|p| check(&p))
            .unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    // A reference field without a declared lifetime is still rejected.
    let source = "struct S{x:&&i32}fn main(){}";
    assert!(
        parse(source).and_then(|p| check(&p)).is_err(),
        "accepted {source}"
    );
}

#[test]
fn nested_shared_real_call_loads_are_once_only_and_named_locals_stay_excluded() {
    check(&parse("fn nested<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}fn main(){let x=11;let r=&x;let rr=nested(&r);let n=**rr;}").unwrap()).unwrap();
    let p = parse("fn inner<'a,'b>(rr:&'a &'b i32)->&'b i32{*rr}fn id<'a,'b>(rr:&'a &'b i32)->&'a &'b i32{rr}fn main(){let x=7;let r=&x;let n=*inner(&r);let m=**id(&r);let rr:&&i32=&r;let copied=*rr;let y=*copied;}").unwrap();
    let env = signature_table(&p).unwrap();
    let main = type_function_with_signatures(&p, 2, &env).unwrap();
    assert_eq!(
        main.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
    assert!(main.exprs.iter().any(|e| matches!(e.kind, ir::ExprKind::Deref { value } if matches!(main.exprs[value.0].kind, ir::ExprKind::Deref { .. }))));
    check_function_with_signatures(&p, 2, &env).unwrap();
    let p = parse("fn bad<'a>(x:&'a i32){let r:&&'a i32=&x;}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert!(
        type_function_with_signatures(&p, 0, &env)
            .unwrap_err()
            .contains("named lifetime annotations on locals")
    );
}

#[test]
fn generic_carrier_call_traces_preserve_firm_reborrows_and_opaque_moves() {
    let p = parse("fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id::<T>(x)}fn firm<'a>(x:&'a mut i32)->&'a mut i32{id::<&'a mut i32>(x)}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let forward =
        type_callable_with_plan(&p, plan.function_decl(1).unwrap(), &plan, &budget).unwrap();
    let firm_decl = plan.function_decl(2).unwrap();
    let firm = type_callable_with_plan(&p, firm_decl, &plan, &budget).unwrap();
    assert_eq!(forward.call_evidence.len(), 1);
    assert_eq!(firm.call_evidence.len(), 1);
    let opaque = &forward.call_evidence[0].arguments[0];
    assert_eq!(opaque.expectation, ir::CallExpectation::NoFirmReference);
    assert!(matches!(
        forward.exprs[opaque.value.0].kind,
        ir::ExprKind::Local(_)
    ));
    let explicit = &firm.call_evidence[0].arguments[0];
    assert_eq!(
        explicit.expectation,
        ir::CallExpectation::FirmReference {
            kind: ir::BorrowKind::Mut
        }
    );
    assert!(matches!(
        firm.exprs[explicit.value.0].kind,
        ir::ExprKind::Borrow {
            timing: ir::BorrowTiming::TwoPhase { .. },
            ..
        }
    ));
    assert_eq!(firm.call_evidence[0].type_actual_regions.len(), 1);
    assert_eq!(
        firm.call_evidence[0].type_actual_regions[0].source,
        ir::ActualRegion::Named(ir::SourceRegionId {
            declaration: firm_decl,
            index: 0
        })
    );
}
#[test]
fn generic_carrier_original_region_env_retains_unused_names_and_required_items() {
    let p = parse("trait Get{fn get<'a,'unused>(&'a self)->&'a Self;}fn keep<'a,'b:'a,'unused,T>(x:&'a T)->&'a T{x}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let env = plan.source_region_env(declaration).unwrap();
    assert_eq!(env.declaration, declaration);
    assert_eq!(env.region_count, 3);
    assert_eq!(env.normalized.len(), 3);
    assert!(env.normalized[0].is_some());
    assert_eq!(env.normalized[1..], [None, None]);
    assert_eq!(
        env.outlives.as_ref(),
        &[(
            ir::SourceRegionId {
                declaration,
                index: 1
            },
            ir::SourceRegionId {
                declaration,
                index: 0
            }
        )]
    );
    let item = plan.registry.trait_items[0][0];
    let item_env = plan.source_region_env(item).unwrap();
    assert_eq!(item_env.region_count, 2);
    assert!(item_env.normalized[0].is_some());
    assert_eq!(item_env.normalized[1], None);
    let foreign =
        parse("fn id<T>(x:T)->T{x}fn bad(x:&i32)->&i32{id::<&'missing i32>(x)}fn main(){}")
            .unwrap();
    let foreign_plan = PlanCtx::from_program(&foreign, &ir::PlanBudget::default()).unwrap();
    assert!(
        type_callable_with_plan(
            &foreign,
            foreign_plan.function_decl(1).unwrap(),
            &foreign_plan,
            &foreign_plan.budget
        )
        .unwrap_err()
        .contains("undeclared lifetime")
    );
}
#[test]
fn generic_carrier_internal_nominal_transport_does_not_relax_source_actuals() {
    let p = parse("struct Wrap<T>{value:T}enum E<T>{One{value:T}}fn wrap<T>(x:T)->E<T>{let w=Wrap{value:x};E::One{value:w.value}}fn main(){let x=1;let e=wrap::<&i32>(&x);}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let body = type_callable_with_plan(&p, plan.function_decl(0).unwrap(), &plan, &budget).unwrap();
    assert!(
        body.locals
            .iter()
            .any(|local| matches!(&local.ty, ir::Ty::Struct(ty) if ty.def().name == "crate::Wrap"))
    );
    let caller =
        type_callable_with_plan(&p, plan.function_decl(1).unwrap(), &plan, &budget).unwrap();
    assert!(caller.exprs.iter().any(|expr| matches!(&expr.ty, ir::Ty::Enum(ty) if matches!(ty.def().variants[0].fields.read(&budget).unwrap()[0].ty, ir::Ty::Ref { .. }))));
    for source in [
        "struct Wrap<T>{value:T}fn main(){let x=1;let w=Wrap::<&i32>{value:&x};}",
        "fn id<T>(x:T)->T{x}fn main(){let x=1i32;let r=id::<&i32>(&x);}",
    ] {
        // Reference generic arguments are admitted (G3 aggregates).
        parse(source)
            .and_then(|program| check(&program))
            .unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}
#[test]
fn generic_reference_declarations_keep_rigid_envelopes_and_canonical_regions() {
    check(&parse("fn id<T>(x:&T)->&T{x}fn wrap(x:&i32)->&i32{id(x)}fn main(){let x=7;let r=wrap(&x);let n=*r;}").unwrap()).unwrap();
    let p=parse("fn first<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let _=y;x}fn renamed<'z,'w,T,U>(x:&'z T,y:&'w U)->&'z T{let _=y;x}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let schema = plan.signature(declaration).unwrap();
    assert_eq!(
        schema.signature.ref_contract,
        plan.signature(plan.function_decl(1).unwrap())
            .unwrap()
            .signature
            .ref_contract
    );
    assert!(
        matches!(&schema.signature.ref_contract.results[0].returns,ir::ReturnContract::SharedInputEnvelopes(rows) if rows.len()==1&&rows[0].input.param==0)
    );
    let closed = plan
        .instantiate_signature(
            &ir::CallTarget {
                declaration,
                type_args: vec![ir::Ty::Int(ir::IntTy::I32); 2].into(),
            },
            ir::FactsCtx::Concrete,
            &budget,
        )
        .unwrap();
    assert_eq!(closed.ref_contract, schema.signature.ref_contract);
    for i in 0..plan.callable_count() {
        let d = plan.callable_decl(i).unwrap();
        type_callable_with_plan(&p, d, &plan, &budget).unwrap();
    }
    for source in [
        "fn id<T>(x:T)->T{x}fn main(){let x=1;let r=id::<&i32>(&x);}",
        "fn bad<T>(x:&&T){}fn main(){}",
        "struct W<T>{x:T}fn main(){let x=1;let w=W::<&i32>{x:&x};}",
    ] {
        let result = parse(source).and_then(|p| check(&p));
        result.unwrap();
    }
}
#[test]
fn borrowed_receiver_and_argument_adjustments_bind_exact_calls_without_owner_moves() {
    let p=parse("struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,n:i32){self.x=n;}}fn put(r:&mut i32,n:i32){*r=n;}fn main(){let mut s=S{x:1};s.set(s.get()+1);let mut x=1;let r=&mut x;put(r,*r+1);}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let main = plan.callable_decl(1).unwrap();
    let typed = type_callable_with_plan(&p, main, &plan, &budget).unwrap();
    let mut receiver = false;
    let mut argument = false;
    for (index, expr) in typed.exprs.iter().enumerate() {
        if let ir::ExprKind::Borrow {
            timing:
                ir::BorrowTiming::TwoPhase {
                    activation,
                    argument: slot,
                    reason,
                },
            ..
        } = &expr.kind
        {
            let ir::ExprKind::Call { args, .. } = &typed.exprs[activation.0].kind else {
                panic!("missing activation");
            };
            assert_eq!(args[*slot], ir::ExprId(index));
            match reason {
                ir::ImplicitBorrowKind::MutableReceiver => receiver = true,
                ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier } => {
                    argument = true;
                    assert!(matches!(
                        typed.exprs[carrier.0].kind,
                        ir::ExprKind::Local(_)
                    ));
                }
            }
        }
    }
    assert!(receiver && argument);
    assert!(typed.exprs.iter().any(|e| matches!(
        e.kind,
        ir::ExprKind::Borrow {
            timing: ir::BorrowTiming::Immediate,
            ..
        }
    )));
    check(&p).unwrap();
}
#[test]
fn receiver_regions_call_carriers_and_explicit_immediate_conflicts_remain_distinct() {
    let p=parse("trait Get{fn get<'a>(&'a self)->&'a i32;}struct S{x:i32}impl Get for S{fn get<'z>(&'z self)->&'z i32{&self.x}}impl S{fn choose(&self,other:&i32)->&i32{let _=other;&self.x}}fn id<T>(x:&mut T)->&mut T{x}fn main(){let mut a=[1];*id::<i32>(&mut a[0])=2;let s=S{x:3};let r;{let other=1;r=s.choose(&other);}let n=*r;let p=<S as Get>::get(&s);}").unwrap();
    check(&p).unwrap();
    for source in [
        "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,n:i32){self.x=n;}}fn main(){let mut s=S{x:1};S::set(&mut s,s.get());}",
        "fn put(r:&mut i32,n:i32){*r=n;}fn main(){let mut x=1;let r=&mut x;put(&mut *r,*r);}",
        "fn bad<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;y}fn main(){}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err());
    }
}

#[test]
fn projected_mutable_reborrow_rejects_forged_carrier_and_place() {
    let p =
        parse("fn put(r:&mut i32){*r=1;}fn relay(r:&mut &mut i32){put(*r);}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let typed =
        type_callable_with_plan(&p, plan.callable_decl(1).unwrap(), &plan, &budget).unwrap();
    typed.validate().unwrap();
    let (borrow, carrier) = typed
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expr)| match &expr.kind {
            ir::ExprKind::Borrow {
                timing:
                    ir::BorrowTiming::TwoPhase {
                        reason: ir::ImplicitBorrowKind::MutableArgumentReborrow { carrier },
                        ..
                    },
                ..
            } => Some((index, *carrier)),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        typed.exprs[carrier.0].kind,
        ir::ExprKind::Deref { .. }
    ));
    let mut forged = typed.clone();
    if let ir::ExprKind::Deref { value } = &mut forged.exprs[carrier.0].kind {
        *value = carrier;
    }
    assert!(forged.validate().is_err(), "cyclic carrier accepted");
    let mut forged = typed.clone();
    if let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[borrow].kind {
        place.projection.pop();
    }
    assert!(forged.validate().is_err(), "shortened projection accepted");
}

#[test]
fn call_result_reborrow_rejects_forged_place_and_mutability() {
    let p =
        parse("fn get(v:&i32)->&i32{v}fn main(){let n=1;let r=&*get(&n);let copy=*r;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let typed =
        type_callable_with_plan(&p, plan.callable_decl(1).unwrap(), &plan, &budget).unwrap();
    typed.validate().unwrap();
    let borrow = typed
        .exprs
        .iter()
        .position(|expr| {
            matches!(&expr.kind,
                ir::ExprKind::Borrow { place, .. } if place.projection == [ir::Projection::Deref]
            )
        })
        .unwrap();
    let mut forged = typed.clone();
    if let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[borrow].kind {
        place.projection.clear();
    }
    assert!(
        forged.validate().is_err(),
        "reference slot mistaken for its referent"
    );
    let mut forged = typed.clone();
    if let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[borrow].kind {
        *kind = ir::BorrowKind::Mut;
    }
    assert!(
        forged.validate().is_err(),
        "shared returned reference made mutable"
    );
}

#[test]
fn trait_default_sources_have_separate_identity_and_rigid_self_facts() {
    let p =
        parse("mod m{fn helper()->i32{7}pub trait V{fn value(&self)->i32{helper()}}}fn main(){}")
            .unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let trait_decl = plan.registry.trait_ids[0];
    let item = plan.registry.trait_items[0][0];
    let default = plan
        .registry
        .callable_sources
        .iter()
        .position(|s| {
            matches!(
                s,
                CallableSource::TraitDefault {
                    trait_index: 0,
                    item: 0
                }
            )
        })
        .map(|i| plan.registry.function_ids[i])
        .unwrap();
    assert_ne!(default, item);
    assert_eq!(plan.declaration(default).unwrap().owner, Some(trait_decl));
    assert_eq!(
        plan.declaration(default).unwrap().path.module,
        p.traits[0].module
    );
    assert_eq!(
        plan.callable_context(default).unwrap().module,
        p.traits[0].module
    );
    assert_eq!(
        p.traits[0].name.display(p.names.as_ref()).unwrap().as_str(),
        "crate::m::V"
    );
    let module_atoms: Vec<_> = plan
        .persistent_decl_path(default)
        .unwrap()
        .modules()
        .map(|module| match module {
            PersistentModule::Source(atom) => atom,
            PersistentModule::Catalog(_) => panic!("expected source trait module"),
        })
        .collect();
    assert_eq!(module_atoms, ["m"]);
    assert_eq!(plan.callable_env(default).unwrap().total_arity(), 1);
    let self_ty = plan.callable_context(default).unwrap().self_ty.unwrap();
    assert!(plan.callable_facts(default).unwrap().predicates().predicates().iter().any(|p| matches!(p, ir::Predicate::Implements(r) if r.declaration == trait_decl && r.self_ty == self_ty)));
    assert!(matches!(
        plan.publish_callable_body(default, &budget).unwrap(),
        PublishedCallableBody::Source(_)
    ));
    check(&p).unwrap();
    for source in [
        "trait V{fn v(&self)->i32{true}}fn main(){}",
        "trait V:Sized{fn v(self)->(Self,Self){(self,self)}}fn main(){}",
        "trait V{fn v(&mut self)->i32{let a=&mut *self;let b=&mut *self;a.v()+b.v()}}fn main(){}",
        "trait V{fn v(&self)->i32{missing()}}fn main(){}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
    }
}
#[test]
fn trait_required_schemas_and_bound_calls_retain_authoritative_identities() {
    let p=parse("struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(mut self)->i32{self.x=self.x+1;self.x}}fn read<T:Value>(x:T)->i32{<T as Value>::value(x)}fn main(){let n=read::<S>(S{x:6});}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    assert_eq!(
        (0..plan.callable_count())
            .filter(|&i| matches!(
                plan.callable_provider(plan.callable_decl(i).unwrap())
                    .unwrap(),
                CallableProvider::Source { .. }
            ))
            .count(),
        3
    );
    let generic = plan.callable_decl(0).unwrap();
    let facts = plan.callable_facts(generic).unwrap();
    assert_eq!(facts.predicates().predicates().len(), 2);
    let typed = type_callable_with_plan(&p, generic, &plan, &budget).unwrap();
    assert!(typed.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::Call{target:ir::Callee::TraitItem(call),..} if matches!(call.trait_ref.self_ty,ir::Ty::Param(_)))));
    let trait_decl = plan.registry.trait_ids[0];
    let item = plan.registry.trait_items[0][0];
    assert!(plan.callable_index(item).is_err());
    let schema = plan.trait_world().unwrap().schema_facts(item).unwrap();
    assert_eq!(schema.phase(), tr::FactsPhase::Schema);
    assert!(schema.must_copy(&ir::Ty::Int(ir::IntTy::I32)).is_err());
    assert_eq!(plan.declaration(item).unwrap().owner, Some(trait_decl));
    check(&p).unwrap();
}
#[test]
fn trait_unused_conformance_and_direct_obligations_are_not_body_witnesses() {
    for source in [
        "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{}fn main(){}",
        "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{fn v(self)->u8{1u8}}fn main(){}",
        "struct S{x:i32}trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{1}}impl V for S{fn v(self)->i32{2}}fn main(){}",
        "trait V{fn v(self)->i32;}fn bad<T:V>(x:T)->(T,T){(x,x)}fn main(){}",
        "struct S{x:i32}trait V{fn v(self)->i32;}fn need<T:V>(x:T){}fn main(){if false{need::<S>(S{x:1});}}",
        "trait V{fn v(self)->i32;}fn need<T:V>(x:T){}fn bad<T>(x:T){need::<T>(x)}fn main(){}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
}
#[test]
fn trait_scope_inherent_precedence_and_predicate_limits_stay_explicit() {
    for source in [
        "mod m{pub struct S{pub x:i32}pub trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{self.x}}}use m::V;fn main(){let n=m::S{x:1}.v();}",
        "struct S{x:i32}trait A{fn v(self)->i32;}trait B{fn v(self)->i32;}impl A for S{fn v(self)->i32{1}}impl B for S{fn v(self)->i32{2}}impl S{fn v(self)->i32{3}}fn main(){let n=S{x:1}.v();}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "mod m{pub struct S{pub x:i32}pub trait V{fn v(self)->i32;}impl V for S{fn v(self)->i32{self.x}}}fn main(){let n=m::S{x:1}.v();}",
        "struct S{x:i32}trait A{fn v(self)->i32;}trait B{fn v(self)->i32;}impl A for S{fn v(self)->i32{1}}impl B for S{fn v(self)->i32{2}}fn main(){let n=S{x:1}.v();}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err());
    }
    let bounds = std::iter::repeat_n("V", 257).collect::<Vec<_>>().join("+");
    assert!(
        parse(&format!(
            "trait V{{fn v(self)->i32;}}fn f<T:{bounds}>(x:T){{}}fn main(){{}}"
        ))
        .is_err()
    );
}
#[test]
fn generic_paths_are_structured_and_binders_are_rigid() {
    let p=parse("struct W<T>{v:T}fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn shadow<i32>(x:i32)->i32{x}fn main(){let w:W<W<u8>>=W::<W<u8>>{v:W::<u8>{v:7}};let n=1+id::<u8>(w.v.v);}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let a = plan
        .function_schema(plan.function_decl(0).unwrap())
        .unwrap();
    let b = plan
        .function_schema(plan.function_decl(1).unwrap())
        .unwrap();
    assert_ne!(a.signature.params[0], b.signature.params[0]);
    assert!(matches!(a.signature.params[0], ir::Ty::Param(_)));
    assert!(matches!(p.functions[2].params[0].1, Type::Param(_)));
    for index in 0..p.functions.len() {
        type_function_with_plan(&p, index, &plan, &budget).unwrap();
    }
    check(&p).unwrap();
}

#[test]
fn associated_callable_registry_preserves_impl_method_binders_and_self() {
    let p=parse("struct W<T>{x:T}impl<T> W<T>{fn make(x:T)->Self{Self{x}}fn pair<U>(self,u:U)->(T,U){(self.x,u)}fn forward<U>(self,u:U)->(T,U){self.pair::<U>(u)}}fn main(){let p=W::<i32>::make(3).forward::<u8>(4u8);}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    assert_eq!(p.functions.len(), 1);
    assert_eq!(
        (0..plan.callable_count())
            .filter(|&i| matches!(
                plan.callable_provider(plan.callable_decl(i).unwrap())
                    .unwrap(),
                CallableProvider::Source { .. }
            ))
            .count(),
        4
    );
    for index in 0..plan.callable_count() {
        let decl = plan.callable_decl(index).unwrap();
        assert_eq!(plan.callable_index(decl).unwrap(), index);
        let f = type_callable_with_plan(&p, decl, &plan, &budget).unwrap();
        if !matches!(
            plan.callable_provider(decl).unwrap(),
            CallableProvider::Source { .. }
        ) {
            continue;
        }
        let context = plan.callable_context(decl).unwrap();
        if context.receiver != ReceiverKind::None {
            assert_eq!(
                f.params.len(),
                plan.callable_body(&p, decl).unwrap().params.len() + 1
            );
            assert_eq!(f.locals[f.params[0].0].name, "self");
        }
        if matches!(
            plan.persistent_decl_path(decl).unwrap().item(),
            PersistentDeclItem::Name("pair")
        ) {
            let env = plan.callable_env(decl).unwrap();
            assert_eq!(env.scopes().len(), 2);
            assert_eq!(env.total_arity(), 2);
            assert_ne!(env.scopes()[0].binder(), env.scopes()[1].binder());
        }
    }
    check(&p).unwrap();
    let singleton = parse("enum E{Item(i32)}impl E{fn take(self)->i32{let Self::Item(x)=self;x}}fn main(){let n=E::Item(7).take();}").unwrap();
    check(&singleton).unwrap();
}

#[test]
fn inherent_coherence_and_definition_privacy_do_not_depend_on_demand() {
    for source in [
        "struct W<T>{x:T}impl W<u8>{fn take(self)->u8{self.x}}impl W<u16>{fn take(self)->u16{self.x}}fn main(){let a=W::<u8>{x:1u8}.take();}",
        "struct P<T,U>{a:T,b:U}impl<T>P<T,T>{fn score(self)->i32{1}}impl P<i32,u8>{fn score(self)->i32{2}}fn main(){}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "struct W<T>{x:T}impl<T> W<T>{fn clash(self){}}impl W<i32>{fn clash(self){}}fn main(){}",
        "struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn main(){}",
        "struct S{x:i32}impl<T>S{fn unused(self){}}fn main(){}",
        "mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn main(){}",
        "mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn main(){let n=a::S{x:1}.take();}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn associated_receivers_are_checked_once_and_owning_calls_stay_direct() {
    let p=parse("struct S{x:i32}impl S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn make()->S{S{x:3}}fn arg()->i32{4}fn main(){let n=make().add(arg());}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let decl = plan.callable_decl(2).unwrap();
    let function = type_callable_with_plan(&p, decl, &plan, &budget).unwrap();
    let calls = function
        .exprs
        .iter()
        .filter_map(|e| {
            if let ir::ExprKind::Call { target, args, .. } = &e.kind {
                Some((target, args))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert_eq!(
            calls
                .iter()
                .filter(
                    |(target, _)|matches!(target,ir::Callee::Direct(t) if matches!(plan.persistent_decl_path(t.declaration).unwrap().item(), PersistentDeclItem::Name("make")))
                )
                .count(),
            1
        );
    let (target, args) = calls.last().unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(
        plan.declaration(match target {
            ir::Callee::Direct(t) => t.declaration,
            _ => panic!("expected direct call"),
        })
        .unwrap()
        .kind,
        ir::DeclKind::AssociatedFunction
    );
    check(&p).unwrap();
    let moved = "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn main(){let s=S{x:1};let n=s.take();let x=s.x;}";
    assert!(parse(moved).and_then(|p| check(&p)).is_err());
    let inferred =
        "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn main(){let n=S{x:1}.choose(2);}";
    let stock = super::parser_tests::stock_compile(inferred, std::path::Path::new("unused"), true);
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    check(&parse(inferred).unwrap()).unwrap();
}

#[test]
fn omitted_generic_inference_uses_all_evidence_before_integer_fallback() {
    let p = parse("struct W<T>{v:T}enum E<T>{A(T),B}fn id<T>(x:T)->T{x}fn same<T>(x:T,y:T)->T{y}fn main(){let a=id(1);let b:u8=id(2);let c=same(3,4u8);let d=id(1)+2u8;let e=1+id(2u8);let p=id((1,2));let z:[u8;0]=id([]);let w=W{v:7u16};let en=E::A(8u8);let explicit=id::<i32>(id(1));let cast=id(256) as u8;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let function = type_function_with_plan(&p, 2, &plan, &budget).unwrap();
    let local = |name: &str| {
        function
            .locals
            .iter()
            .find(|l| l.name == name)
            .unwrap()
            .ty
            .clone()
    };
    assert_eq!(local("a"), ir::Ty::Int(ir::IntTy::I32));
    for name in ["b", "c", "d", "e", "cast"] {
        assert_eq!(local(name), ir::Ty::Int(ir::IntTy::U8));
    }
    assert_eq!(
        local("p"),
        ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32); 2].into())
    );
    assert_eq!(
        local("z"),
        ir::Ty::Array {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            len: 0
        }
    );
    assert!(
        matches!(local("w"), ir::Ty::Struct(t) if t.def().type_args.as_ref() == [ir::Ty::Int(ir::IntTy::U16)])
    );
    assert!(
        matches!(local("en"), ir::Ty::Enum(t) if t.def().type_args.as_ref() == [ir::Ty::Int(ir::IntTy::U8)])
    );
    check(&p).unwrap();
}

#[test]
fn omitted_generic_inference_keeps_opaque_sites_and_universal_binders_rigid() {
    for source in [
        "fn id<T>(x:T)->T{x}fn main(){let x:u8=id({let a=1;2});}",
        "fn id<T>(x:T)->T{x}fn same<T>(x:T,y:T)->T{y}fn main(){let x=same(id(1),2u8);}",
        "fn f<T>(x:T,n:i32)->T{if n==0{x}else{f(x,n-1)}}fn main(){let x=f(1u8,1);}",
        "fn id<T>(x:T)->T{x}fn main(){let x=id({let a=1;2});}",
        "fn id<T>(x:T)->T{x}fn main(){let x=id(id(1));}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "fn same<T>(x:T,y:T)->T{y}fn main(){let x=same(1u8,2u16);}",
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err());
    }
}

#[test]
fn omitted_generic_hints_do_not_commit_numeric_defaults_or_expression_effects() {
    let p = parse("fn id<T>(x:T)->T{x}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    let signatures = plan
        .environment(Some(plan.function_decl(1).unwrap()))
        .unwrap();
    let checker = Checker::new(&signatures, ir::Ty::Unit, p.functions[1].module);
    assert_eq!(
        p.functions[0]
            .name
            .display(p.names.as_ref())
            .unwrap()
            .as_str(),
        "crate::id"
    );
    let call = |suffix| Expr::Call {
        name: p.functions[0].name.clone(),
        args: vec![Expr::Integer(IntegerLiteral {
            magnitude: 1,
            suffix,
        })],
    };
    let untyped = call(None);
    let typed = call(Some(ir::IntTy::U8));
    assert_eq!(checker.scalar_hint(&untyped).unwrap(), None);
    assert_eq!(
        checker.scalar_hint(&typed).unwrap(),
        Some(ir::Ty::Int(ir::IntTy::U8))
    );
    assert!(checker.locals.is_empty());
    assert!(checker.exprs.is_empty());
    assert!(checker.reachable);
    assert_eq!(checker.scalar_hint(&untyped).unwrap(), None);
}

#[test]
fn generic_all_body_semantics_do_not_specialize_away_invalid_templates() {
    for source in [
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){}",
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){let n=bad::<i32>(7);}",
        "fn bad<T>(x:T)->T{x+1}fn main(){}",
        "fn bad<T>(x:T)->T{7i32}fn main(){}",
        "fn id<T>(x:T)->T{x}fn main(){let n=id::<i32,u8>(1);}",
        "fn bad<T>(x:T){return;let r=&x;}fn main(){}",
    ] {
        if source.contains("return;let r=&x;") {
            check(&parse(source).unwrap()).unwrap();
            continue;
        }
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    check(&parse("struct P<T>{a:T,b:T}fn split<T>(p:P<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn main(){let p=split::<u8>(P::<u8>{a:3,b:4});}").unwrap()).unwrap();
}

#[test]
fn generic_nominal_applications_are_canonical_and_share_resource_authority() {
    let p = parse("struct W<T>{v:T}enum E<T>{A(T),B}fn id<T>(x:T)->T{x}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&p, &budget).unwrap();
    assert_eq!(
        p.structs[0]
            .name
            .display(p.names.as_ref())
            .unwrap()
            .as_str(),
        "crate::W"
    );
    let id = *plan
        .registry
        .nominals
        .get(&p.structs[0].name.key(Namespace::Type))
        .unwrap();
    let a = plan
        .intern_nominal(
            id,
            &[ir::Ty::Int(ir::IntTy::U8)],
            ir::FactsCtx::Concrete,
            &budget,
        )
        .unwrap();
    let b = plan
        .clone()
        .intern_nominal(
            id,
            &[ir::Ty::Int(ir::IntTy::U8)],
            ir::FactsCtx::Concrete,
            &budget.clone(),
        )
        .unwrap();
    let c = plan
        .intern_nominal(
            id,
            &[ir::Ty::Int(ir::IntTy::I32)],
            ir::FactsCtx::Concrete,
            &budget,
        )
        .unwrap();
    assert_eq!(a, b);
    assert!(
        plan.intern_nominal(
            id,
            &[ir::Ty::Int(ir::IntTy::U8)],
            ir::FactsCtx::Concrete,
            &ir::PlanBudget::default()
        )
        .unwrap_err()
        .contains("authority")
    );
    assert_ne!(a, c);
    let foreign = ir::ParamId {
        owner: ir::BinderId {
            declaration: plan.function_decl(0).unwrap(),
        },
        index: 0,
    };
    assert!(
        plan.intern_nominal(
            id,
            &[ir::Ty::Param(foreign)],
            ir::FactsCtx::Concrete,
            &budget
        )
        .is_err()
    );
    let limited = ir::PlanBudget::new(ir::PlanLimits {
        applications: 0,
        ..Default::default()
    });
    assert!(
        PlanCtx::from_program(&p, &limited)
            .unwrap_err()
            .contains("resource")
    );
    for source in [
        "struct Bad<T>{x:i32}fn main(){}",
        "struct Bad<T>{x:Bad<(T,)>}fn main(){}",
    ] {
        assert!(parse(source).and_then(|p| type_env(&p)).is_err());
    }
}

#[test]
fn compound_statement_tokens_types_and_expression_boundaries() {
    for name in [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ] {
        let source = format!(
            "fn main(){{let mut x:{name}=4;x+=1;x-=1;x*=2;x/=2;x%=3;x&=3;x|=1;x^=2;x<<=1u64;x>>=1i8;}}"
        );
        let p = parse(&source).unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        assert_eq!(
            f.body
                .statements
                .iter()
                .filter(|s| matches!(s, ir::TypedStmt::AssignOp { .. }))
                .count(),
            10
        );
    }
    check(&parse("fn main(){let mut b=true;b&=false;b|=true;b^=true;let mut a=[1u8];let i=0usize;a[i]+=2;}").unwrap()).unwrap();
    let source = "fn main(){let mut x=1;let y=(x+=1);}";
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    check(&parse(source).unwrap()).unwrap();
    for source in [
        "fn main(){let mut x=1;x \"+=\" 1;}",
        "fn main(){let mut x=1u8;x+=1u16;}",
        "fn main(){let mut x=true;x+=false;}",
        "fn main(){let mut x=1;x<<=true;}",
        "fn main(){let mut x=(1,2);x+=x;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    let mut p = parse("fn main(){let mut x=1;x+=2;}").unwrap();
    let Stmt::AssignOp { op, .. } = &mut p.functions[0].body.statements[1] else {
        panic!("compound root place")
    };
    *op = BinaryOp::Eq;
    assert!(check(&p).unwrap_err().contains("operator"));
}

#[test]
fn compound_current_place_rhs_initialization_and_permission_checks() {
    for source in [
        "fn main(){let mut x:i32;x+={x=10;1};}",
        "fn main(){let mut x=1;let mut y=10;let mut p=&mut x;*p+={p=&mut y;2};}",
        "fn main(){let mut a:[i32;1];let i=0usize;a[i]+={a=[10];2};}",
        "fn main(){let mut a=[1];a[{a=[10];0usize}]+={a=[3];2};}",
        "fn main(){let mut x=1;return;x+=1;}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "fn main(){let mut x:i32;x+=1;}",
        "fn main(){let x=1;x+=1;}",
        "fn main(){let x=1;let r=&x;*r+=1;}",
        "fn main(){let a=[1];let i=0usize;a[i]+=1;}",
        "fn main(){let mut x=1;let r=&x;x+=1;let y=*r;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    let p = parse("fn rhs()->i32{1}fn idx()->usize{0}fn main(){let mut a=[1];a[idx()]+=rhs();}")
        .unwrap();
    let f = type_function_with_signatures(&p, 2, &signature_table(&p).unwrap()).unwrap();
    assert_eq!(
        f.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
    assert!(
        f.body
            .statements
            .iter()
            .any(|s| matches!(s, ir::TypedStmt::AssignIndexOp { .. }))
    );
}

#[test]
fn indexed_compound_autodereferences_mutable_array_carriers() {
    for source in [
        "fn shift(a:&mut [u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
        "fn shift(a:&mut [u8;2],i:usize,n:u128){(*a)[i]>>=n;}fn main(){}",
        "fn shift(a:&mut ([u8;2],),i:usize,n:u128){a.0[i]<<=n;}fn main(){}",
        "fn shift(a:&mut [u128;2],i:usize,n:u128){a[i]+=n;}fn main(){}",
        "fn main(){let mut a=[1u8,2u8];let r=&mut a;let i=0usize;r[i]<<=1u128;}",
    ] {
        check(&parse(source).unwrap()).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    for source in [
        "fn shift(a:&[u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
        "fn shift(mut a:&[u8;2],i:usize,n:u128){a[i]<<=n;}fn main(){}",
        "fn shift(a:&[u8;2],i:usize,n:u128){(*a)[i]<<=n;}fn main(){}",
        "fn main(){let a=[1u8,2u8];let r=&a;let i=0usize;r[i]<<=1u128;}",
        "fn main(){let mut a=[1u8,2u8];let r=&mut a;let s=&*r;let i=0usize;r[i]<<=1u128;let x=*s;}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
    }
    let p = parse(
        "fn rhs()->u128{1}fn idx()->usize{0}fn shift(a:&mut [u8;2]){a[idx()]<<=rhs();}fn main(){}",
    )
    .unwrap();
    let f = type_function_with_signatures(&p, 2, &signature_table(&p).unwrap()).unwrap();
    let ir::TypedStmt::AssignIndexOp {
        array,
        index,
        value,
        ..
    } = f
        .body
        .statements
        .iter()
        .find(|statement| matches!(statement, ir::TypedStmt::AssignIndexOp { .. }))
        .expect("expected indexed compound assignment")
    else {
        unreachable!()
    };
    assert_eq!(array.projection, [ir::Projection::Deref]);
    assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::U128));
    assert_eq!(f.exprs[index.0].ty, ir::Ty::Int(ir::IntTy::Usize));
    for statement in &f.body.statements {
        if let ir::TypedStmt::Let { local, value } = statement {
            assert_eq!(f.locals[local.0].ty, f.exprs[value.0].ty);
        }
    }
    let rhs = f
        .exprs
        .iter()
        .position(|e| {
            matches!(e.kind, ir::ExprKind::Call { .. }) && e.ty == ir::Ty::Int(ir::IntTy::U128)
        })
        .unwrap();
    let index_call = f
        .exprs
        .iter()
        .position(|e| {
            matches!(e.kind, ir::ExprKind::Call { .. }) && e.ty == ir::Ty::Int(ir::IntTy::Usize)
        })
        .unwrap();
    assert!(rhs < index_call, "RHS must be typed before index");
    assert_eq!(
        f.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
}

#[test]
fn compound_shift_queries_share_count_memo_and_check_dead_statements() {
    for source in [
        "fn main(){let mut x=1u8;x<<=8u64;}",
        "fn main(){let mut x=1u8;x>>=-1i16;}",
        "fn main(){let mut a=[1u8];let i=0usize;a[i]<<=3u16+5u16;}",
        "fn main(){let mut x=1u8;return;x<<=8u64;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    let p =
        parse("fn main(){let mut x=1u8;let n=8u64;x<<=n;x>>={x=3;1u32};let y=1u8<<1u64;}").unwrap();
    check(&p).unwrap();
    let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
    let ir::TypedStmt::AssignOp { value, .. } = f.body.statements[2] else {
        panic!("compound shift")
    };
    assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::U64));
    let literal = ir::TypedExpr {
        kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
        ty: ir::Ty::Int(ir::IntTy::I32),
        completion: ir::Completion::Returns,
    };
    let queries = vec![(ir::IntTy::U8, ir::ExprId(0)); COUNT_EVAL_WORK / 2 + 1];
    assert!(
        validate_shift_counts(&[literal], &queries)
            .unwrap_err()
            .contains("work")
    );
}

#[test]
fn shifts_keep_independent_integer_operands_and_parser_precedence() {
    let names = [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ];
    for lhs in names {
        for rhs in names {
            let source = format!("fn f(x:{lhs},n:{rhs})->{lhs}{{(x<<n)>>n}}fn main(){{}}");
            let p = parse(&source).unwrap();
            check(&p).unwrap();
            let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
            for expr in &f.exprs {
                if let ir::ExprKind::Binary {
                    op: BinaryOp::Shl | BinaryOp::Shr,
                    left,
                    right,
                } = expr.kind
                {
                    assert_eq!(expr.ty, f.exprs[left.0].ty);
                    assert_eq!(f.exprs[right.0].ty, f.locals[f.params[1].0].ty);
                }
            }
        }
    }
    let p = parse("fn f()->u8{1<<1u64}fn g(x:u8)->u8{x<<1}fn main(){let x=1<<1u64;let y=(1i16 as u8)<<1;let z=4i32 as u8>>1;let p=1 | 2 & 3 << 4+1*2;}").unwrap();
    check(&p).unwrap();
    let env = signature_table(&p).unwrap();
    let g = type_function_with_signatures(&p, 1, &env).unwrap();
    let ir::ExprKind::Binary { right, .. } = g.exprs[g.body.tail.unwrap().0].kind else {
        panic!("shift")
    };
    assert_eq!(g.exprs[right.0].ty, ir::Ty::Int(ir::IntTy::I32));
    let main = type_function_with_signatures(&p, 2, &env).unwrap();
    assert_eq!(main.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
    for source in [
        "fn main(){let x=true<<1;}",
        "fn main(){let x=1>>false;}",
        "fn main(){let x=1i32 as u8<<1;}",
        "fn main(){let x=1 \"<<\" 1;}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err());
    }
    for op in ["<<=", ">>="] {
        check(&parse(&format!("fn main(){{let mut x=1;x{op}1;}} ")).unwrap()).unwrap();
    }
}

#[test]
fn shifts_static_count_grammar_and_runtime_leaves_are_distinct() {
    for count in [
        "8u8",
        "-1i16",
        "(3u16+5u16)",
        "(1u64<<3u8)",
        "(!0u8)",
        "((-1i8) as u64)",
        "{8u32}",
        "(255u8+1u8)",
        "(1u8/0u8)",
        "((-128i8)/(-1i8))",
    ] {
        let source = format!("fn main(){{let x=1u8<<({count});}}");
        assert!(
            parse(&source).and_then(|p| check(&p)).is_err(),
            "accepted {count}"
        );
    }
    check(&parse("fn count()->u64{64}fn f(x:u8,n:u64)->u8{x<<n}fn main(){let n=8u64;let x=1u8<<n;let y=1u64>>count();let z=1u8<<((3u16^1u16) as u64);let a=1u64<<((18446744073709551615u64*0u64) as u8);}").unwrap()).unwrap();
    assert!(
        parse("fn main(){let x=1u8<<(if true{1}else{2});}")
            .and_then(|p| check(&p))
            .unwrap_err()
            .contains("unsupported")
    );
    check(&parse("fn main(){let mut bits=3u8;let snapshot=bits<<{bits=9u8;1u32};}").unwrap())
        .unwrap();
    let p = parse("fn dead()->u8{1u8<<({return 7u8;1u32})}fn once(n:u8)->u8{1u8<<n}fn main(){}")
        .unwrap();
    check(&p).unwrap();
    let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
    assert_eq!(
        f.exprs[f.body.tail.unwrap().0].completion,
        ir::Completion::Never
    );
}

#[test]
fn shift_count_memo_bounds_cycles_and_never_values() {
    let literal = ir::TypedExpr {
        kind: ir::ExprKind::Int(ir::IntValue::i32(0)),
        ty: ir::Ty::Int(ir::IntTy::I32),
        completion: ir::Completion::Returns,
    };
    let mut arena = vec![literal.clone()];
    for index in 0..100 {
        arena.push(ir::TypedExpr {
            kind: ir::ExprKind::Binary {
                op: BinaryOp::BitOr,
                left: ir::ExprId(index),
                right: ir::ExprId(index),
            },
            ..literal.clone()
        });
    }
    let mut eval = CountEvaluator::new(&arena).unwrap();
    assert!(matches!(
        eval.evaluate(ir::ExprId(100)).unwrap(),
        CountValue::Known(_)
    ));
    assert!(eval.work < 1000);
    eval.work = COUNT_EVAL_WORK;
    assert!(eval.evaluate(ir::ExprId(0)).unwrap_err().contains("work"));
    let mut deep = vec![literal.clone()];
    for index in 0..130 {
        deep.push(ir::TypedExpr {
            kind: ir::ExprKind::IntCast {
                value: ir::ExprId(index),
                to: ir::IntTy::I32,
            },
            ..literal.clone()
        });
    }
    let mut eval = CountEvaluator::new(&deep).unwrap();
    eval.evaluate(ir::ExprId(100)).unwrap();
    assert!(
        eval.evaluate(ir::ExprId(130))
            .unwrap_err()
            .contains("depth")
    );
    let cyclic = [ir::TypedExpr {
        kind: ir::ExprKind::IntCast {
            value: ir::ExprId(0),
            to: ir::IntTy::I32,
        },
        ..literal.clone()
    }];
    assert!(
        CountEvaluator::new(&cyclic)
            .unwrap()
            .evaluate(ir::ExprId(0))
            .unwrap_err()
            .contains("cycle")
    );
    let never = [ir::TypedExpr {
        completion: ir::Completion::Never,
        ..literal
    }];
    assert!(matches!(
        CountEvaluator::new(&never)
            .unwrap()
            .evaluate(ir::ExprId(0))
            .unwrap(),
        CountValue::Dynamic
    ));
}

#[test]
fn bitwise_exact_integer_and_boolean_types_preserve_operand_hints() {
    for name in [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ] {
        let source = format!("fn f(x:{name})->{name}{{(x & 3) | (1 ^ x)}}fn main(){{}}");
        let p = parse(&source).unwrap();
        check(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
        for expr in &f.exprs {
            if matches!(
                expr.kind,
                ir::ExprKind::Binary {
                    op: BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor,
                    ..
                }
            ) {
                assert_eq!(expr.ty, f.result);
            }
        }
    }
    check(&parse("fn f(x:bool)->bool{(x & true) | (false ^ x)}fn main(){let x=(1i8<2i8)&(3i16<4i16);let y=(200+100)|1;let z=((200+100)|1) as u8;}").unwrap()).unwrap();
    for source in [
        "fn main(){let x=1u8 & 2u16;}",
        "fn main(){let x=true | 1;}",
        "fn main(){let x=() ^ ();}",
        "fn main(){let x=(1,2) & (3,4);}",
        // `r|r` (one shared layer) is stock-valid since a2c25ee46. Stock still
        // rejects mixed depths (E0277) and two layers (E0369).
        "fn main(){let x=1;let r=&x;let z=r|&r;}",
        "fn main(){let x=1;let r=&x;let rr=&r;let z=rr|rr;}",
        "fn main(){let x:u8=256 | 0;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn bitwise_pratt_precedence_and_compound_boundaries() {
    let p = parse("fn main(){let x=1 | 2 ^ 3 & 4 + 5 * 6i16 as i32;}").unwrap();
    let Stmt::Let {
        value:
            Expr::Binary {
                op: BinaryOp::BitOr,
                right,
                ..
            },
        ..
    } = &p.functions[0].body.statements[0]
    else {
        panic!("OR precedence")
    };
    let Expr::Binary {
        op: BinaryOp::BitXor,
        right,
        ..
    } = right.as_ref()
    else {
        panic!("XOR precedence")
    };
    let Expr::Binary {
        op: BinaryOp::BitAnd,
        right,
        ..
    } = right.as_ref()
    else {
        panic!("AND precedence")
    };
    let Expr::Binary {
        op: BinaryOp::Add,
        right,
        ..
    } = right.as_ref()
    else {
        panic!("addition precedence")
    };
    let Expr::Binary {
        op: BinaryOp::Mul,
        right,
        ..
    } = right.as_ref()
    else {
        panic!("multiplication precedence")
    };
    assert!(matches!(right.as_ref(), Expr::Cast { .. }));
    check(&parse("fn main(){let x=1i32;let r=&x;let a=*r & -1 | !0 ^ 3;let b=1|2|3;let c=1|2==3;let d=true || false & true && false;}").unwrap()).unwrap();
    for op in ["&=", "|=", "^="] {
        let source = format!("fn main(){{let mut x=1;x{op}2;}}");
        check(&parse(&source).unwrap()).unwrap();
    }
    for source in [
        "fn main(){let x=1 \"|\" 2;}",
        "fn main(){let x=1 \"^\" 2;}",
        "fn main(){let x=1<2<3;}",
    ] {
        assert!(parse(source).is_err());
    }
}

#[test]
fn bitwise_eager_completion_and_once_only_operands() {
    let p = parse("fn source()->bool{true}fn f()->bool{source() & source()}fn dead()->bool{false & {return true;false}}fn main(){}").unwrap();
    check(&p).unwrap();
    let env = signature_table(&p).unwrap();
    let f = type_function_with_signatures(&p, 1, &env).unwrap();
    assert_eq!(
        f.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
    let dead = type_function_with_signatures(&p, 2, &env).unwrap();
    assert_eq!(
        dead.exprs[dead.body.tail.unwrap().0].completion,
        ir::Completion::Never
    );
    // An eager RHS read is checked even when the left Boolean decides the value.
    assert!(check(&parse("fn main(){let x:bool;let y=false & x;}").unwrap()).is_err());
    assert!(check(&parse("fn main(){let x:bool;let y=true | x;}").unwrap()).is_err());
}

#[test]
fn cast_soft_expectations_preserve_concrete_sources_and_drop_at_operators() {
    use ir::IntTy::*;
    for (body, source_ty) in [
        ("300i32 as u8", I32),
        ("{let x=300;x} as u8", I32),
        ("(200+100) as u8", I32),
        ("(if true{300}else{301}) as u8", I32),
        ("(match true{true=>300,false=>301}) as u8", I32),
        ("(loop{break 300;}) as u8", I32),
        ("!0 as u8", U8),
        ("-128i8 as u8", I8),
    ] {
        // Parenthesize the outer operand when the body begins a naked block.
        let body = if body.starts_with('{') {
            format!("({body})")
        } else {
            body.to_string()
        };
        let source = format!("fn f()->u8{{{body}}}fn main(){{}}");
        let p = parse(&source).unwrap();
        check(&p).unwrap();
        let env = signature_table(&p).unwrap();
        let f = type_function_with_signatures(&p, 0, &env).unwrap();
        let ir::ExprKind::IntCast { value, to } = f.exprs[f.body.tail.unwrap().0].kind else {
            panic!("not a cast: {source}")
        };
        assert_eq!(to, U8);
        assert_eq!(f.exprs[value.0].ty, ir::Ty::Int(source_ty), "{source}");
    }
    for source in [
        "fn main(){let x=300 as u8;}",
        "fn main(){let x=({300}) as u8;}",
        "fn main(){let x=(-1) as u8;}",
        "fn main(){let x=300 as u8 as u16;}",
        "fn main(){let x=(1u8+300i32) as u16;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    check(&parse("fn wide()->u64{18446744073709551615 as u64}fn f()->u16{(-1i8) as u16}fn main(){let x:u16=300i32 as u8 as u16;}").unwrap()).unwrap();
}

#[test]
fn cast_parser_preserves_precedence_statement_boundaries_and_typed_tokens() {
    let p=parse("fn main(){let x=-1i16 as u8*2;let y=1i32 as u8 as u16;let z=1+2i32 as u8;let b=(1i32 as u8)<2;}").unwrap();
    let Stmt::Let {
        value:
            Expr::Binary {
                op: BinaryOp::Mul,
                left,
                ..
            },
        ..
    } = &p.functions[0].body.statements[0]
    else {
        panic!("multiplication precedence")
    };
    assert!(
        matches!(left.as_ref(),Expr::Cast{value,..} if matches!(value.as_ref(),Expr::Unary{op:UnaryOp::Neg,..}))
    );
    let Stmt::Let {
        value: Expr::Cast { value, .. },
        ..
    } = &p.functions[0].body.statements[1]
    else {
        panic!("cast chain")
    };
    assert!(matches!(value.as_ref(), Expr::Cast { .. }));
    let Stmt::Let {
        value:
            Expr::Binary {
                op: BinaryOp::Add,
                right,
                ..
            },
        ..
    } = &p.functions[0].body.statements[2]
    else {
        panic!("addition precedence")
    };
    assert!(matches!(right.as_ref(), Expr::Cast { .. }));
    for source in [
        "fn main(){let x=1 \"as\" u8;}",
        "fn main(){let x=1 as \"u8\";}",
        "fn main(){let x:\"u8\"=1;}",
        "fn main(){let x=1i32 as u8<2;}",
        "fn main(){let x=1i32 as u8.0;}",
        "fn main(){let x=1i32 as u8[0];}",
        "fn main(){let x=1i32 as u8();}",
        "fn f()->u8{{1} as u8}fn main(){}",
        "fn f()->u8{if true{1}else{2} as u8}fn main(){}",
        "fn f()->u8{match true{_=>1} as u8}fn main(){}",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
    assert!(
        parse("fn main(){let x=1i32 as (u8);}")
            .unwrap_err()
            .contains("unsupported")
    );
    check(&parse("fn f()->u8{({1}) as u8}fn g()->u8{(if true{1}else{2}) as u8}fn main(){let b=(1i32 as u8)<2;}").unwrap()).unwrap();
}

#[test]
fn cast_completion_and_once_only_operands_do_not_create_values_or_origins() {
    let p=parse("fn source()->i64{300}fn once()->u8{source() as u8}fn typed_dead()->u8{({return 1u8;300i32}) as u8}fn read(r:&i16)->u8{*r as u8}fn main(){}").unwrap();
    check(&p).unwrap();
    let env = signature_table(&p).unwrap();
    let once = type_function_with_signatures(&p, 1, &env).unwrap();
    assert_eq!(
        once.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        1
    );
    let dead = type_function_with_signatures(&p, 2, &env).unwrap();
    let cast = &dead.exprs[dead.body.tail.unwrap().0];
    assert_eq!(cast.ty, ir::Ty::Int(ir::IntTy::U8));
    assert_eq!(cast.completion, ir::Completion::Never);
    let ir::ExprKind::IntCast { value, .. } = cast.kind else {
        panic!("expected typed dead cast")
    };
    assert_eq!(dead.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::I32));
    assert_eq!(dead.exprs[value.0].completion, ir::Completion::Never);
    // Stock accepts diverging and bool operands.
    for source in [
        "fn main(){let x=(loop{}) as u8;}",
        "fn main(){let x=true as u8;}",
    ] {
        parse(source).and_then(|p| check(&p)).unwrap();
    }
    {
        let source = "fn main(){let x=1i32;let r=&x;let n=r as usize;}";
        assert!(
            parse(source)
                .and_then(|p| check(&p))
                .unwrap_err()
                .contains("E0606")
        );
    }
    let within = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(120));
    check(&parse(&within).unwrap()).unwrap();
    let source = format!("fn main(){{let x=1i32{};}}", " as u8".repeat(140));
    assert!(parse(&source).unwrap_err().contains("nesting"));
}

#[test]
fn wide_count_evaluation_preserves_sign_high_limbs_and_checked_bounds() {
    use ir::IntTy::{I8, I128, U8, U64, U128};
    let int = |ty, bits| ir::IntValue { ty, bits };
    let known = |op, left, right| {
        let CountValue::Known(value) = count_binary(op, left, right).unwrap() else {
            panic!("expected exact static value")
        };
        value
    };
    let min = int(I128, 1u128 << 127);
    let max = int(I128, i128::MAX as u128);
    let umax = int(U128, u128::MAX);
    assert_eq!(count_integer(min), i128::MIN);
    assert_eq!(count_integer(max), i128::MAX);
    assert_eq!(count_cast(int(I8, 255), U128), u128::MAX);
    assert_eq!(count_cast(int(U8, 255), I128), 255);
    assert_eq!(count_cast(umax, U64), u64::MAX as u128);
    assert_eq!(count_checked_signed(i128::MIN, I128).unwrap(), min.bits);
    assert_eq!(count_checked_signed(i128::MAX, I128).unwrap(), max.bits);
    assert!(count_checked_signed(128, I8).is_err());
    assert!(count_checked_signed(-129, I8).is_err());
    assert_eq!(known(BinaryOp::Sub, umax, int(U128, 1)).bits, u128::MAX - 1);
    assert_eq!(
        known(BinaryOp::Add, min, int(I128, 1)).bits,
        (1u128 << 127) + 1
    );
    assert_eq!(known(BinaryOp::Shr, min, int(U128, 127)).bits, u128::MAX);
    assert_eq!(
        known(BinaryOp::Shl, int(U128, 1), int(I128, 127)).bits,
        1u128 << 127
    );
    assert_eq!(
        known(BinaryOp::Div, min, int(I128, 2)).bits,
        (i128::MIN / 2) as u128
    );
    assert_eq!(known(BinaryOp::Rem, umax, int(U128, 2)).bits, 1);
    for (op, lhs, rhs) in [
        (BinaryOp::Add, max, int(I128, 1)),
        (BinaryOp::Sub, min, int(I128, 1)),
        (BinaryOp::Mul, min, int(I128, 2)),
        (BinaryOp::Add, umax, int(U128, 1)),
        (BinaryOp::Sub, int(U128, 0), int(U128, 1)),
        (BinaryOp::Mul, umax, int(U128, 2)),
        (BinaryOp::Div, min, int(I128, u128::MAX)),
        (BinaryOp::Rem, min, int(I128, u128::MAX)),
        (BinaryOp::Div, umax, int(U128, 0)),
        (BinaryOp::Rem, umax, int(U128, 0)),
        (BinaryOp::Shl, int(U128, 1), int(U128, 128)),
        (BinaryOp::Shr, min, int(I128, u128::MAX)),
    ] {
        assert!(
            count_binary(op, lhs, rhs).is_err(),
            "{op:?}: {lhs:?} {rhs:?}"
        );
    }
    for value in [umax, min, int(U128, 1u128 << 64), int(I128, u128::MAX)] {
        assert!(count_shift_amount(value, U8).is_err());
    }
    assert_eq!(count_shift_amount(int(U128, 7), U8).unwrap(), 7);
    let arena = vec![
        ir::TypedExpr {
            ty: ir::Ty::Int(I128),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Int(min),
        },
        ir::TypedExpr {
            ty: ir::Ty::Int(I128),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Unary {
                op: UnaryOp::Neg,
                value: ir::ExprId(0),
            },
        },
    ];
    assert!(
        CountEvaluator::new(&arena)
            .unwrap()
            .evaluate(ir::ExprId(1))
            .is_err()
    );
}

#[test]
fn wide_source_casts_and_static_counts_are_exact() {
    let names = [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ];
    for from in names {
        for to in names {
            let source = format!("fn f(x:{from})->{to}{{x as {to}}}fn main(){{}}");
            check(&parse(&source).unwrap()).unwrap();
        }
    }
    for integer in ["i128", "u128"] {
        for float in ["f32", "f64"] {
            let source = format!(
                "fn f(x:{integer})->{float}{{x as {float}}}fn g(x:{float})->{integer}{{x as {integer}}}fn main(){{}}"
            );
            check(&parse(&source).unwrap()).unwrap();
        }
    }
    for count in [
        "127u128",
        "(340282366920938463463374607431768211455u128%128u128)",
        "((340282366920938463463374607431768211455u128*0u128) as u8)",
        "((-170141183460469231731687303715884105728i128>>127u128)&127i128)",
        "(((-1i8) as u128)&127u128)",
        "((18446744073709551616u128 as u64) as i128)",
    ] {
        let source = format!("fn main(){{let x=1u128<<{count};let mut y=1i128;y>>={count};}}");
        check(&parse(&source).unwrap()).unwrap();
    }
    for count in [
        "128u128",
        "18446744073709551616u128",
        "340282366920938463463374607431768211455u128",
        "-170141183460469231731687303715884105728i128",
        "(-(-170141183460469231731687303715884105728i128))",
        "(170141183460469231731687303715884105727i128+1i128)",
        "(-170141183460469231731687303715884105728i128-1i128)",
        "(340282366920938463463374607431768211455u128+1u128)",
        "(340282366920938463463374607431768211455u128*2u128)",
        "((-170141183460469231731687303715884105728i128)/(-1i128))",
        "((-170141183460469231731687303715884105728i128)%(-1i128))",
        "(1u128<<128u128)",
    ] {
        for body in [
            format!("let x=1u128<<{count};"),
            format!("let mut x=1u128;return;x<<={count};"),
            format!("let x=1u8<<{count};"),
        ] {
            let source = format!("fn main(){{{body}}}");
            assert!(parse(&source).and_then(|p| check(&p)).is_err(), "{source}");
        }
    }
}

#[test]
fn wide_literals_and_patterns_keep_all_128_bits() {
    for text in [
        "340_282_366_920_938_463_463_374_607_431_768_211_455u128",
        "0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffffu128",
        "0b11111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111u128",
        "0o3777777777777777777777777777777777777777777u128",
    ] {
        let literal = parse_integer_literal(text).unwrap();
        assert_eq!(literal.magnitude, u128::MAX, "{text}");
        assert_eq!(
            resolve_integer(&literal, false, None).unwrap().bits,
            u128::MAX
        );
    }
    for text in [
        "340282366920938463463374607431768211456u128",
        "0x100000000000000000000000000000000u128",
        "1u128bad",
        "0b102u128",
    ] {
        assert!(parse_integer_literal(text).is_err(), "{text}");
    }
    for source in [
        "fn main(){let x=170141183460469231731687303715884105728i128;}",
        "fn main(){let x=-170141183460469231731687303715884105729i128;}",
        "fn main(){let x=-1u128;}",
        "fn main(){let x:u64=18446744073709551616;}",
        "fn main(){let x:i128=340282366920938463463374607431768211455;}",
        "fn main(){let x=18446744073709551616;}",
    ] {
        assert!(parse(source).and_then(|p| check(&p)).is_err(), "{source}");
    }
    for source in [
        "fn main(){let x=170141183460469231731687303715884105728i128;}",
        "fn main(){let x=-170141183460469231731687303715884105729i128;}",
        "fn main(){let x=0x80000000000000000000000000000000i128;}",
        "fn main(){let x:i128=170141183460469231731687303715884105728;}",
    ] {
        let error = parse(source).and_then(|p| check(&p)).unwrap_err();
        assert!(error.contains("out of range for i128"), "{error}");
        assert!(
            !error.contains("unsupported"),
            "invalid Rust is not an unsupported feature: {error}"
        );
    }
    assert!(
        parse("fn main(){let x=300 as u8;}")
            .and_then(|p| check(&p))
            .unwrap_err()
            .contains("unsupported")
    );
    let source = "fn f(x:u128)->i32{match x{0=>1,18446744073709551616=>2,340282366920938463463374607431768211455=>3,_=>4}}fn g(x:i128)->i32{match x{-170141183460469231731687303715884105728=>1,170141183460469231731687303715884105727=>2,_=>3}}fn main(){}";
    let p = parse(source).unwrap();
    check(&p).unwrap();
    let f = type_function_with_signatures(&p, 0, &signature_table(&p).unwrap()).unwrap();
    let ir::ExprKind::Match { arms, .. } = &f.exprs[f.body.tail.unwrap().0].kind else {
        panic!("expected match")
    };
    let bits = arms
        .iter()
        .filter_map(|arm| match arm.pattern {
            ir::MatchPattern::Int(value) => Some(value.bits),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(bits, [0, 1u128 << 64, u128::MAX]);
}

#[test]
fn integer_literal_bounds_contexts_and_grouped_minima_are_exact() {
    use ir::IntTy::*;
    for ty in [
        I8, I16, I32, I64, I128, Isize, U8, U16, U32, U64, U128, Usize,
    ] {
        let max = if ty.is_signed() {
            (1u128 << (ty.bits() - 1)) - 1
        } else {
            ty.mask()
        };
        let source = format!(
            "fn f()->{}{{{max}}}fn main(){{let x:{}={max};let y={max}{};let z=1+y;let n=!y;}}",
            ty.name(),
            ty.name(),
            ty.name()
        );
        check(&parse(&source).unwrap()).unwrap();
        let value = resolve_integer(
            &IntegerLiteral {
                magnitude: max,
                suffix: Some(ty),
            },
            false,
            None,
        )
        .unwrap();
        assert_eq!(value, ir::IntValue { ty, bits: max });
        if max < u128::MAX {
            assert!(
                resolve_integer(
                    &IntegerLiteral {
                        magnitude: max + 1,
                        suffix: Some(ty)
                    },
                    false,
                    None
                )
                .is_err()
            );
        }
        if ty.is_signed() {
            let magnitude = 1u128 << (ty.bits() - 1);
            let source = format!(
                "fn f()->{}{{-((({magnitude}{})))}}fn main(){{let x:{}=-(({magnitude}));}}",
                ty.name(),
                ty.name(),
                ty.name()
            );
            let program = parse(&source).unwrap();
            check(&program).unwrap();
            let env = signature_table(&program).unwrap();
            let function = type_function_with_signatures(&program, 0, &env).unwrap();
            assert!(
                matches!(function.exprs[function.body.tail.unwrap().0].kind,ir::ExprKind::Int(ir::IntValue{ty:actual,bits}) if actual==ty&&bits==magnitude)
            );
            assert!(
                check(
                    &parse(&format!(
                        "fn main(){{let x=-{{{magnitude}{}}};}}",
                        ty.name()
                    ))
                    .unwrap()
                )
                .is_err()
            );
            assert!(
                resolve_integer(
                    &IntegerLiteral {
                        magnitude: magnitude + 1,
                        suffix: Some(ty)
                    },
                    true,
                    None
                )
                .is_err()
            );
        } else {
            assert!(
                resolve_integer(
                    &IntegerLiteral {
                        magnitude: 0,
                        suffix: Some(ty)
                    },
                    true,
                    None
                )
                .is_err()
            );
        }
    }
    check(&parse("fn main(){let x:i64=-2147483648;let a:u16=1+2u16;let b=2u16+1;let x:[u8;0x2_usize]=[0b1_u8,0o7_u8];let y=[0xff_u8;0b10];let v=y[0x0];let b=(1i8<2i8)==(3i16<4i16);}").unwrap()).unwrap();
    for literal in [
        "0xff_u8",
        "0o177_i16",
        "0b_1010__u32",
        "18_446_744_073_709_551_615u64",
    ] {
        let literal = parse_integer_literal(literal).unwrap();
        resolve_integer(&literal, false, None).unwrap();
    }
    for literal in [
        "0x_",
        "0b2",
        "0o8",
        "340282366920938463463374607431768211456u128",
        "0x1_0000_0000_0000_0000_0000_0000_0000_0000u128",
    ] {
        assert!(
            parse_integer_literal(literal).is_err(),
            "accepted {literal}"
        );
    }
    for source in [
        "fn main(){let x=18446744073709551616u64;}",
        "fn main(){let x=0x1_0000_0000_0000_0000u64;}",
        "fn main(){let x:u8=1i8;}",
        "fn main(){let x:i64=1isize;}",
        "fn main(){let a=[1];let i=0u64;let x=a[i];}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
    }
}

#[test]
fn integer_patterns_preserve_exact_bits_and_bounded_exhaustiveness() {
    check(&parse("fn f(x:i8)->i32{match x{-128i8=>1,127=>2,_=>3}}fn g(x:u64)->i32{match x{18446744073709551615u64=>1,_=>2}}fn main(){}").unwrap()).unwrap();
    for ty in ["u8", "i8"] {
        let range = if ty == "i8" { -128..128 } else { 0..256 };
        let arms = range.map(|n| format!("{n}=>(),")).collect::<String>();
        check(&parse(&format!("fn f(x:{ty}){{match x{{{arms}}};}}fn main(){{}}")).unwrap())
            .unwrap();
    }
    for source in [
        "fn main(){match 1u8{1i8=>(),_=>()};}",
        "fn main(){match 1u8{256=>(),_=>()};}",
        "fn main(){match 1i8{-129=>(),_=>()};}",
        "fn main(){match 1u64{0=>(),1=>()};}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
    }
    assert!(parse("fn main(){match 1i8{\"1\"=>(),_=>()};}").is_err());
    let p = parse("fn main(){match 1u8{1=>(),0x01u8=>(),_=>()};}").unwrap();
    check(&p).unwrap();
}

#[test]
fn e3_mixed_contracts_preserve_formal_kinds_and_real_shared_return_reborrows() {
    let p=parse("fn weaken<'a>(r:&'a mut i32)->&'a i32{r}fn mix<'a>(s:&'a i32,m:&'a mut i32,b:bool)->&'a i32{if b{s}else{m}}fn mut_target<'a>(s:&'a i32,m:&'a mut i32)->&'a mut i32{m}fn no_mut_target<'a>(s:&'a i32)->&'a mut i32{loop{}}fn pin<'a>(m:&'a mut bool,s:&'a i32)->&'a i32{s}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    let empty = |param| ir::InputPath {
        input: ir::InputReference {
            param,
            position: ir::ReferencePosition { depth: 0 },
        },
        projection: vec![].into(),
    };
    assert_eq!(
        env.functions[&fixture_function_key(&p, "mix")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(vec![empty(0), empty(1)].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "mut_target")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::MutInputPaths(vec![empty(1)].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "no_mut_target")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::MutInputPaths(vec![].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "pin")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(vec![empty(1)].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "pin")]
            .ref_contract
            .input_regions
            .as_ref(),
        &[input_region(0, 0), input_region(1, 0)]
    );
    assert!(matches!(
        env.functions[&fixture_function_key(&p, "weaken")].params[0],
        ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            ..
        }
    ));
    let weaken = type_function_with_signatures(&p, 0, &env).unwrap();
    assert!(
        matches!(&weaken.exprs[weaken.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place,..} if place.local==weaken.params[0]&&place.projection==vec![ir::Projection::Deref])
    );
    assert!(
        !weaken
            .exprs
            .iter()
            .any(|e| matches!(e.kind,ir::ExprKind::Local(local) if local==weaken.params[0]))
    );
    for index in 0..p.functions.len() {
        // Stock accepts every item, including the diverging `no_mut_target`.
        check_function_with_signatures(&p, index, &env).unwrap();
    }
}

#[test]
fn e3_all_input_positions_regions_and_deferred_adjustments_remain_enforced() {
    let p = parse("fn ambiguous<'a>(s:&'a i32,m:&'a mut bool)->&i32{s}fn main(){}").unwrap();
    assert!(
        signature_table(&p)
            .unwrap_err()
            .contains("missing lifetime specifier")
    );
    let p = parse("fn rigid<'a>(s:&bool,m:&mut i32)->&'a i32{&*m}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert_eq!(
        env.functions[&fixture_function_key(&p, "rigid")]
            .ref_contract
            .input_regions
            .as_ref(),
        &[input_region(0, 0), input_region(1, 1)]
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "rigid")]
            .ref_contract
            .results[0]
            .region,
        ir::RegionParamId(2)
    );
    assert!(check_function_with_signatures(&p, 0, &env).is_err());
    let names = (0..63)
        .map(|i| format!("'r{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(&format!(
        "fn cap<{names}>(other:&bool,r:&mut i32)->&'r0 mut i32{{loop{{}}}}fn main(){{}}"
    ))
    .unwrap();
    assert!(signature_table(&p).unwrap_err().contains("limit64"));
    let p = parse("fn invalid<'a>(s:&'a i32)->&'a mut i32{s}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert!(check_function_with_signatures(&p, 0, &env).is_err());
    for source in [
        "fn deferred<'a>(m:&'a mut i32)->&'a i32{&mut *m}fn main(){}",
        "struct S{x:i32}fn deferred<'a>(m:&'a mut S)->&'a i32{&mut m.x}fn main(){}",
    ] {
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        assert!(
            type_function_with_signatures(&p, 0, &env)
                .unwrap_err()
                .contains("unsupported")
        );
    }
}

#[test]
fn e3_branch_join_keeps_mutable_formal_and_emits_shared_reborrow() {
    for body in [
        "if b{s}else{m}",
        "if b{m}else{s}",
        "if let true=b{s}else{m}",
        "match b{true=>m,false=>s}",
    ] {
        let source =
            format!("fn pick<'a>(s:&'a i32,m:&'a mut i32,b:bool)->&'a i32{{{body}}}fn main(){{}}");
        let program = parse(&source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, 0, &signatures)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert!(matches!(
            checked.locals[checked.params[0].0].ty,
            ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                ..
            }
        ));
        assert!(matches!(
            checked.locals[checked.params[1].0].ty,
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                ..
            }
        ));
        assert!(
            checked.exprs.iter().any(|expression| matches!(
                &expression.kind,
                ir::ExprKind::Borrow { kind: ir::BorrowKind::Shared, place, .. }
                    if place.local == checked.params[1]
                        && place.projection == [ir::Projection::Deref]
            )),
            "{source}: missing genuine shared reborrow"
        );
        checked.validate().unwrap();
        check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
}

#[test]
fn loop_reference_context_preserves_break_and_reborrow_authority() {
    for body in [
        "loop{break m;}",
        "loop{if b{break s;}break m;}",
        "loop{if b{break m;}break s;}",
        "'out: loop{loop{break 'out m;}}",
        "'out:{if b{break 'out s;}m}",
        "if b{loop{}}else{m}",
    ] {
        let source =
            format!("fn pick<'a>(s:&'a i32,m:&'a mut i32,b:bool)->&'a i32{{{body}}}fn main(){{}}");
        let program = parse(&source).unwrap();
        check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        let signatures = signature_table(&program).unwrap();
        let checked = type_function_with_signatures(&program, 0, &signatures).unwrap();
        checked.validate().unwrap();
        assert!(
            checked.exprs.iter().any(|expression| matches!(
                &expression.kind,
                ir::ExprKind::Borrow { kind: ir::BorrowKind::Shared, place, .. }
                    if place.local == checked.params[1]
                        && place.projection == [ir::Projection::Deref]
            )),
            "{source}: loop result must use a real shared reborrow"
        );
    }
    // A substituted reference actual is not a written reference coercion site:
    // the field is declared as T, so the loop break must move p unchanged.
    // The legacy cloned-Program typed adapter fails closed on inferred generic
    // reference actuals, so observe the move through the canonical checker:
    // the valid form checks, and reusing p after the break is rejected below.
    super::parser_tests::stock_native_run(
        "struct Wrap<T>{value:T}fn main(){let mut n=1;let p=&mut n;let wrapped:Wrap<&mut i32>=Wrap{value:loop{break p;}};*wrapped.value=7;println!(\"{}\",n);}",
    );
    for source in [
        "fn bad<'a>()->&'a i32{loop{let n=1;break &n;}}fn main(){}",
        "fn bad<'a>(s:&'a i32)->&'a mut i32{loop{break s;}}fn main(){}",
        "fn main(){let mut n=1;let p=&mut n;let q=loop{break p;};let x=*p;let y=*q;}",
        "struct Wrap<T>{value:T}fn main(){let mut n=1;let p=&mut n;let wrapped:Wrap<&mut i32>=Wrap{value:loop{break p;}};let x=*p;let y=*wrapped.value;}",
        "fn pick<'a>(s:&'a i32,m:&'a mut i32)->&'a i32{loop{break m;}}fn main(){let x=1;let mut n=2;let p=&mut n;let q=pick(&x,p);*p=3;let y=*q;}",
    ] {
        let stock =
            super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
        assert!(!stock.status.success(), "stock accepted {source}");
        assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
    }
}

#[test]
fn e2_implicit_and_explicit_fields_have_identical_ir_and_current_carrier_places() {
    let p=parse("struct S{x:i32,y:i32}fn implicit(r:&mut S){let n=r.x;r.x=n+1;let q=&mut r.y;*q=4;}fn explicit(r:&mut S){let n=(*r).x;(*r).x=n+1;let q=&mut (*r).y;*q=4;}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    check_function_with_signatures(&p, 0, &env).unwrap();
    check_function_with_signatures(&p, 1, &env).unwrap();
    let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
    let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
    assert_eq!(
        format!("{:?}", implicit.exprs),
        format!("{:?}", explicit.exprs)
    );
    assert_eq!(
        format!("{:?}", implicit.body),
        format!("{:?}", explicit.body)
    );
    let p=parse("struct S{x:i32}fn implicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;r.x={r=&mut y;3};}fn explicit(){let mut x=S{x:1};let mut y=S{x:2};let mut r=&mut x;(*r).x={r=&mut y;3};}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    check_function_with_signatures(&p, 0, &env).unwrap();
    check_function_with_signatures(&p, 1, &env).unwrap();
    let implicit = type_function_with_signatures(&p, 0, &env).unwrap();
    let explicit = type_function_with_signatures(&p, 1, &env).unwrap();
    assert_eq!(
        format!("{:?}", implicit.exprs),
        format!("{:?}", explicit.exprs)
    );
    assert_eq!(
        format!("{:?}", implicit.body),
        format!("{:?}", explicit.body)
    );
    assert!(implicit.body.statements.iter().any(|s|matches!(s,ir::TypedStmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
}

#[test]
fn e2_field_reads_use_one_genuine_call_temp_and_usize_peer_hints() {
    let p=parse("struct Wide{u:usize}fn get<'a>(r:&'a Wide)->&'a Wide{r}fn get_mut<'a>(r:&'a mut Wide)->&'a mut Wide{r}fn main(){let mut s=Wide{u:4294967296usize};let r=&s;let sum=1+r.u;let bigger=r.u>1;let x=get(&s).u;let y=get_mut(&mut s).u;let mut tuple=(1,2);let t=&mut tuple;t.0=3;let n=t.1;}").unwrap();
    let env = signature_table(&p).unwrap();
    check_function_with_signatures(&p, 2, &env).unwrap();
    let main = type_function_with_signatures(&p, 2, &env).unwrap();
    assert_eq!(
        main.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
    assert!(
        main.exprs
            .iter()
            .filter(|e| matches!(
                e.kind,
                ir::ExprKind::Int(ir::IntValue {
                    ty: ir::IntTy::Usize,
                    bits: 1
                })
            ))
            .count()
            >= 2
    );
    for expr in &main.exprs {
        if let ir::ExprKind::Field { base, .. } = expr.kind {
            assert!(matches!(
                main.exprs[base.0].kind,
                ir::ExprKind::Deref { .. }
            ));
        }
    }
}

#[test]
fn e2_field_normalization_preserves_authority_privacy_and_deferred_boundaries() {
    for source in [
        "struct S{x:i32}fn main(){let mut s=S{x:1};let r=&s;r.x=2;}",
        "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let q=&mut r.x;}",
        "struct T{x:i32}struct S{child:T}fn main(){let s=S{child:T{x:1}};let r=&s;let moved=r.child;}",
        "mod m{pub struct S{x:i32}pub fn make()->S{S{x:1}}}fn main(){let s=m::make();let r=&s;let n=r.x;}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    for source in [
        "struct S{a:[i32;1]}fn main(){let s=S{a:[1]};let r=&s;let i=0usize;let _=r.a[i];}",
        "struct S{a:[i32;1]}fn main(){let mut s=S{a:[1]};let r=&mut s;let i=0usize;r.a[i]=2;}",
        "struct S{x:i32}fn main(){let s=S{x:1};let r=&s;let n=({r}).x;}",
    ] {
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        assert!(
            type_function_with_signatures(&p, p.functions.len() - 1, &env)
                .unwrap_err()
                .contains("unsupported"),
            "lost boundary {source}"
        );
    }
    check(&parse("struct S{x:i32}fn main(){let s=S{x:1};let r=&s;return;r.x=2;}").unwrap())
        .unwrap();
    // Variant matches through a projected dereference switch on the referent.
    for source in [
        "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;match r.e{E::A=>()};}",
        "enum E{A}struct S{e:E}fn main(){let s=S{e:E::A};let r=&s;let E::A=r.e;}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
}

#[test]
fn e1_elision_matches_explicit_contracts_and_keeps_anonymous_inputs_rigid() {
    let p=parse("fn elided(x:&i32,n:i32)->&i32{x}fn explicit<'a>(x:&'a i32,n:i32)->&'a i32{x}fn mut_elided(x:&mut i32)->&mut i32{x}fn mut_explicit<'a>(x:&'a mut i32)->&'a mut i32{x}fn unused_names<'a,'b>(x:&i32)->&i32{x}fn rigid<'a>(x:&i32,y:&i32)->&'a i32{x}fn repeated<'a>(x:&'a i32,y:&'a bool)->&'a i32{x}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert_eq!(
        env.functions[&fixture_function_key(&p, "elided")].ref_contract,
        env.functions[&fixture_function_key(&p, "explicit")].ref_contract
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "mut_elided")].ref_contract,
        env.functions[&fixture_function_key(&p, "mut_explicit")].ref_contract
    );
    let unused = &env.functions[&fixture_function_key(&p, "unused_names")].ref_contract;
    assert_eq!(unused.region_count, 1);
    assert_eq!(unused.input_regions.as_ref(), &[input_region(0, 0)]);
    assert_eq!(unused.results[0].region, ir::RegionParamId(0));
    let rigid = &env.functions[&fixture_function_key(&p, "rigid")].ref_contract;
    assert_eq!(
        rigid.input_regions.as_ref(),
        &[input_region(0, 0), input_region(1, 1)]
    );
    assert_eq!(rigid.results[0].region, ir::RegionParamId(2));
    assert!(rigid.outlives.is_empty());
    assert_eq!(
        rigid.results[0].returns,
        ir::ReturnContract::SharedInputPaths(vec![].into())
    );
    type_function_with_signatures(&p, 5, &env).unwrap();
    assert!(check_function_with_signatures(&p, 5, &env).is_err());
    for index in [0, 1, 2, 3, 4, 6] {
        check_function_with_signatures(&p, index, &env).unwrap();
    }
}

#[test]
fn e1_output_elision_counts_reference_positions_not_distinct_region_ids() {
    for source in [
        "fn missing()->&i32{loop{}}fn main(){}",
        "fn ambiguous(x:&i32,y:&i32)->&i32{x}fn main(){}",
        "fn repeated<'a>(x:&'a i32,y:&'a i32)->&i32{x}fn main(){}",
        "fn repeated<'a>(x:&'a mut i32,y:&'a mut i32)->&mut i32{x}fn main(){}",
        "fn different<'a>(x:&'a i32,y:&'a bool)->&i32{x}fn main(){}",
    ] {
        let p = parse(source).unwrap();
        assert!(
            signature_table(&p)
                .unwrap_err()
                .contains("missing lifetime specifier"),
            "accepted {source}"
        );
    }
}

#[test]
fn e1_combined_region_cap_does_not_limit_erased_no_reference_parameters() {
    let names = |count: usize| {
        (0..count)
            .map(|i| format!("'r{i}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let p = parse(&format!(
        "fn at_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
        names(63)
    ))
    .unwrap();
    let env = signature_table(&p).unwrap();
    assert_eq!(
        env.functions[&fixture_function_key(&p, "at_limit")]
            .ref_contract
            .region_count,
        1
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "at_limit")]
            .ref_contract
            .results[0]
            .region,
        ir::RegionParamId(0)
    );
    let p = parse(&format!(
        "fn over_limit<{}>(x:&i32)->&i32{{x}}fn main(){{}}",
        names(64)
    ))
    .unwrap();
    assert!(signature_table(&p).unwrap_err().contains("limit64"));
    let params = (0..4096)
        .map(|i| format!("p{i}:&i32"))
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(&format!("fn erased({params}){{}}fn main(){{}}")).unwrap();
    let env = signature_table(&p).unwrap();
    let contract = &env.functions[&fixture_function_key(&p, "erased")].ref_contract;
    assert_eq!(contract.region_count, 0);
    assert!(contract.input_regions.is_empty());
    assert!(contract.results.is_empty());
    assert!(contract.outlives.is_empty());
    assert_eq!(
        env.functions[&fixture_function_key(&p, "erased")]
            .params
            .len(),
        4096
    );
}

#[test]
fn projected_contract_paths_keep_source_privacy_and_argument_layouts() {
    let p=parse("mod m{pub struct S{nested:([i32;1],)}pub fn get<'a>(p:&'a S)->&'a i32{&(*p).nested.0[0]}pub fn get_mut<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).nested.0[0]}pub fn make()->S{S{nested:([7],)}}}fn choose<'a>(flag:bool,a:&'a(i32,bool),b:&'a(bool,i32))->&'a i32{if flag{&(*a).0}else{&(*b).1}}fn main(){let mut s=m::make();let r=m::get(&s);let n=*r;let q=m::get_mut(&mut s);*q=9;}").unwrap();
    let env = signature_table(&p).unwrap();
    let nested = ir::InputPath {
        input: ir::InputReference {
            param: 0,
            position: ir::ReferencePosition { depth: 0 },
        },
        projection: vec![
            ir::Projection::Field(0),
            ir::Projection::Field(0),
            ir::Projection::ConstantIndex(0),
        ]
        .into(),
    };
    assert_eq!(
        env.functions[&fixture_function_key(&p, "m::get")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(vec![nested.clone()].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "m::get_mut")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::MutInputPaths(vec![nested].into())
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "choose")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(
            vec![
                ir::InputPath {
                    input: ir::InputReference {
                        param: 1,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![ir::Projection::Field(0)].into()
                },
                ir::InputPath {
                    input: ir::InputReference {
                        param: 2,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![ir::Projection::Field(1)].into()
                },
            ]
            .into()
        )
    );
    for index in 0..p.functions.len() {
        type_function_with_signatures(&p, index, &env).unwrap();
    }
    let invalid = parse(
        "mod m{pub struct S{x:i32}pub fn make()->S{S{x:7}}}fn main(){let s=m::make();let r=&s.x;}",
    )
    .unwrap();
    let env = signature_table(&invalid).unwrap();
    assert!(
        type_function_with_signatures(&invalid, 1, &env)
            .unwrap_err()
            .contains("private")
    );
}

#[test]
fn b3b_mutable_contracts_keep_complete_original_indices_and_return_reborrows() {
    let p=parse("fn pick<'a>(flag:bool,s:&'a mut bool,x:&'a mut i32,y:&'a mut i32)->&'a mut i32{if flag{x}else{y}}fn moved<'a>(r:&'a mut i32)->&'a mut i32{let moved=r;moved}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert_eq!(
        env.functions[&fixture_function_key(&p, "pick")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::MutInputPaths(
            vec![
                ir::InputPath {
                    input: ir::InputReference {
                        param: 2,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![].into()
                },
                ir::InputPath {
                    input: ir::InputReference {
                        param: 3,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![].into()
                }
            ]
            .into()
        )
    );
    let pick = type_function_with_signatures(&p, 0, &env).unwrap();
    assert_eq!(
        pick.exprs
            .iter()
            .filter(|e| matches!(
                e.kind,
                ir::ExprKind::Borrow {
                    kind: ir::BorrowKind::Mut,
                    ..
                }
            ))
            .count(),
        2
    );
    let moved = type_function_with_signatures(&p, 1, &env).unwrap();
    let ir::TypedStmt::Let { local, value } = moved.body.statements[0] else {
        panic!("expected genuine untyped handle let")
    };
    assert!(matches!(moved.exprs[value.0].kind,ir::ExprKind::Local(id) if id==moved.params[0]));
    assert!(
        matches!(&moved.exprs[moved.body.tail.unwrap().0].kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Mut,place,..} if place.local==local && place.projection==vec![ir::Projection::Deref])
    );
    assert!(std::sync::Arc::ptr_eq(
        &moved.ref_contract,
        &env.functions[&fixture_function_key(&p, "moved")].ref_contract
    ));
}

#[test]
fn b3b_mutable_call_temp_copy_reads_preserve_computed_place_boundaries() {
    let p=parse("struct S{x:i32}fn id<'a>(r:&'a mut S)->&'a mut S{r}fn main(){let mut s=S{x:7};let n=(*id(&mut s)).x;}").unwrap();
    let env = signature_table(&p).unwrap();
    let main = type_function_with_signatures(&p, 1, &env).unwrap();
    assert_eq!(
        main.exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        1
    );
    assert!(
        main.exprs
            .iter()
            .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
    );
    for source in [
        "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;*id(&mut x)=2;}",
        "fn id<'a>(r:&'a mut i32)->&'a mut i32{r}fn main(){let mut x=1;let q=&mut *id(&mut x);}",
    ] {
        // Call-result assignment and explicit reborrowing both preserve the
        // original referent loan. Execution and rejection are covered by the
        // reference-return differential acceptance suite.
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "fn bad<'a>(x:&'a i32)->&'a mut i32{loop{}}fn main(){}",
        "fn bad<'a>(x:&'a mut i32,y:&'a i32)->&'a mut i32{x}fn main(){}",
        "fn bad<'a>(x:&'a mut i32)->&'a i32{&*x}fn main(){}",
    ] {
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        // Stock accepts all three: a diverging body satisfies any result.
        check_function_with_signatures(&p, 0, &env).unwrap();
    }
}

#[test]
fn b3a_contracts_are_alpha_canonical_and_keep_original_parameter_indices() {
    let source = "fn pick<'a>(flag:bool,s:&'a bool,x:&'a i32,y:&'a i32)->&'a i32{if flag{x}else{y}}fn renamed<'z>(flag:bool,s:&'z bool,x:&'z i32,y:&'z i32)->&'z i32{if flag{x}else{y}}fn main(){}";
    let p = parse(source).unwrap();
    let env = signature_table(&p).unwrap();
    let first = &env.functions[&fixture_function_key(&p, "pick")];
    assert_eq!(
        first.ref_contract,
        env.functions[&fixture_function_key(&p, "renamed")].ref_contract
    );
    assert_eq!(
        first.ref_contract.input_regions.as_ref(),
        &[input_region(1, 0), input_region(2, 0), input_region(3, 0)]
    );
    assert_eq!(
        first.ref_contract.results[0].returns,
        ir::ReturnContract::SharedInputPaths(
            vec![
                ir::InputPath {
                    input: ir::InputReference {
                        param: 2,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![].into()
                },
                ir::InputPath {
                    input: ir::InputReference {
                        param: 3,
                        position: ir::ReferencePosition { depth: 0 }
                    },
                    projection: vec![].into()
                }
            ]
            .into()
        )
    );
    let checked = type_function_with_signatures(&p, 0, &env).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &first.ref_contract,
        &checked.ref_contract
    ));
    assert_eq!(
        env.functions[&fixture_function_key(&p, "main")]
            .ref_contract
            .input_regions
            .len(),
        0
    );
}

#[test]
fn b3a_outlives_bounds_and_call_result_dereference_are_structural() {
    let p=parse("struct S{x:i32}fn id<'a>(x:&'a S)->&'a S{x}fn short<'a,'b>(x:&'a i32)->&'b i32 where 'a:'b{x}fn missing<'a,'b>(x:&'a i32)->&'b i32{x}fn main(){let s=S{x:7};let n=(*id(&s)).x;}").unwrap();
    let env = signature_table(&p).unwrap();
    assert_eq!(
        env.functions[&fixture_function_key(&p, "short")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(
            vec![ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 0 }
                },
                projection: vec![].into()
            }]
            .into()
        )
    );
    assert_eq!(
        env.functions[&fixture_function_key(&p, "missing")]
            .ref_contract
            .results[0]
            .returns,
        ir::ReturnContract::SharedInputPaths(vec![].into())
    );
    let checked = type_function_with_signatures(&p, 3, &env).unwrap();
    assert_eq!(
        checked
            .exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        1
    );
    assert!(
        checked
            .exprs
            .iter()
            .any(|e| matches!(e.kind, ir::ExprKind::Deref { .. }))
    );
    // Missing universal outlives relations are analyzer obligations, not type inference.
    type_function_with_signatures(&p, 2, &env).unwrap();
}

#[test]
fn b3a_rejects_unbound_duplicate_and_deferred_lifetime_forms() {
    for source in [
        "fn f<'a,'a>(x:&'a i32)->&'a i32{x}fn main(){}",
        "fn f<'a>(x:&'b i32)->&'a i32{x}fn main(){}",
        "fn f<'a:'b>(x:&'a i32)->&'a i32{x}fn main(){}",
        "fn f<'a>(x:&'a i32){}fn main(){}",
        "fn f<'fn>(x:&'fn i32)->&'fn i32{x}fn main(){}",
    ] {
        if source == "fn f<'a>(x:&'a i32){}fn main(){}" {
            check(&parse(source).unwrap()).unwrap();
            continue;
        }
        assert!(
            parse(source).and_then(|p| signature_table(&p)).is_err(),
            "accepted {source}"
        );
    }
    let p = parse("fn f<'a>(x:&'a i32)->&'a i32{let q:&'a i32=x;q}fn main(){}").unwrap();
    let env = signature_table(&p).unwrap();
    assert!(
        type_function_with_signatures(&p, 0, &env)
            .unwrap_err()
            .contains("unsupported")
    );
}

#[test]
fn b2_expected_reference_sites_reborrow_the_carrier_without_a_prior_move() {
    let p=parse("fn read(r:&i32)->i32{*r}fn bump(r:&mut i32){*r=*r+1;}fn main(){let mut x=1;let mut r=&mut x;bump(r);bump(r);read(r);let t:&mut i32=r;*t=3;let mut y=2;let s=&mut y;r=s;let a:&i32={r};let b:&i32=if true{r}else{r};let c:&i32=match true{_=>r};let moved=r;}").unwrap();
    let env = signature_table(&p).unwrap();
    let typed = type_function_with_signatures(&p, 2, &env).unwrap();
    let carrier = typed.locals.iter().position(|l| l.name == "r").unwrap();
    let reborrows=typed.exprs.iter().filter(|e|matches!(&e.kind,ir::ExprKind::Borrow{place,..} if place.local==ir::LocalId(carrier)&&place.projection==vec![ir::Projection::Deref])).count();
    assert!(
        reborrows >= 7,
        "missing call/let/assignment/control-context reborrows: {reborrows}"
    );
    let moved = typed
        .body
        .statements
        .iter()
        .find_map(|s| {
            if let ir::TypedStmt::Let { local, value } = s
                && typed.locals[local.0].name == "moved"
            {
                Some(*value)
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        matches!(typed.exprs[moved.0].kind,ir::ExprKind::Local(local) if local==ir::LocalId(carrier))
    );
    for i in 0..2 {
        type_function_with_signatures(&p, i, &env).unwrap();
    }
}

#[test]
fn b2_noescape_restrictions_hold_for_unused_declarations_and_explicit_borrows() {
    for source in [
        "fn unused(r:&[&i32;0]){}fn main(){}",
        "fn unused(r:(&i32,)){}fn main(){}",
        "fn unused(r:&&i32){}fn main(){}",
        "fn unused()->&i32{loop{}}fn main(){}",
        "struct S{r:&i32}fn main(){}",
        "fn read(r:&i32){}fn main(){let mut x=1;read(&mut x);}",
        "fn main(){let mut x=1;let r=&mut x;let q:&mut i32=loop{break r;};}",
    ] {
        if matches!(
            source,
            "fn unused(r:&&i32){}fn main(){}"
                | "fn unused(r:&[&i32;0]){}fn main(){}"
                | "fn unused(r:(&i32,)){}fn main(){}"
                | "fn main(){let mut x=1;let r=&mut x;let q:&mut i32=loop{break r;};}"
        ) {
            check(&parse(source).unwrap()).unwrap();
            continue;
        }
        assert!(
            parse(source)
                .and_then(|p| {
                    let env = signature_table(&p)?;
                    type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
                })
                .is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn blocklike_statements_do_not_consume_a_following_deref_as_multiplication() {
    for source in [
        "fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r}fn main(){let n=choose(true);}",
        "fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}fn main(){let n=run();}",
        "fn main(){let x=1;let r=&x;if true{}else if false{}else{} *r;}",
        "fn main(){let x={1}*2;({1})*2;let a=({[1]})[0];}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
}

#[test]
fn b1_types_explicit_borrows_reborrows_and_static_deref_projections() {
    let program=parse("struct S{x:i32,y:i32}fn main(){let mut s=S{x:1,y:2};let r:&mut S=&mut s;(*r).x=3;let c=(*r).x;let p=&mut (*r).y;*p=4;let q=&*r;let z=(*q).x;let mut a=[1,2];let ar=&mut a;(*ar)[0]=3;let n=(*ar)[0];let owner=1;let sh=&owner;let copied=sh;let shared_s=S{x:7,y:8};let shared_r=&shared_s;let shared_n=(*shared_r).x;let moved=r;}").unwrap();
    let env = signature_table(&program).unwrap();
    let typed = type_function_with_signatures(&program, 0, &env).unwrap();
    assert!(typed.body.statements.iter().any(|s|matches!(s,ir::TypedStmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::Field(0)])));
    assert!(typed.body.statements.iter().any(|s|matches!(s,ir::TypedStmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Deref,ir::Projection::ConstantIndex(0)])));
    assert!(typed.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::Borrow{kind:ir::BorrowKind::Shared,place,..} if place.projection==vec![ir::Projection::Deref])));
    for source in [
        "fn main(){let mut x=1;let r=&mut x;let moved=r;let _=*r;match *r{_=>()};}",
        "fn main(){let r:&i32;let _=*r;match *r{_=>()};}",
        "fn main(){let x=1;return;let _=&mut x;}",
        "fn main(){let x=1;let r=&x;return;*r=2;}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
}

#[test]
fn b1_rejects_forbidden_reference_locations_authority_and_implicit_coercions() {
    for source in [
        "fn main(){let x=1;let _=&mut x;}",
        "fn main(){let x=1;let r=&x;*r=2;}",
        "fn main(){let mut x=1;let r=&x;let p=&mut *r;}",
    ] {
        let p = parse(source).unwrap();
        let env = signature_table(&p).unwrap();
        type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(check(&p).is_err(), "analysis accepted {source}");
    }
    for source in [
        "fn unused()->&i32{loop{}}fn main(){}",
        "struct S{x:(&i32,)}fn main(){}",
        "enum E{V([&i32;0])}fn main(){}",
        "fn main(){let x=1;let r=&x;let a=[r;0];}",
        "fn main(){let x=1;let r=&x;let t=(r,);}",
        "fn main(){let x=1;let r=&x;let nested=&r;}",
        "fn main(){let r=&1;}",
        "fn main(){let x=1;let a=&x;let b=&x;let eq=a==b;}",
        "fn main(){let a=[1];let i=0usize;let p=&a[i];}",
        "fn main(){let a=[1];let p=&a;let i=0usize;let n=(*p)[i];}",
    ] {
        let result = parse(source).and_then(|p| {
            let env = signature_table(&p)?;
            type_function_with_signatures(&p, p.functions.len() - 1, &env).map(|_| ())
        });
        if matches!(
            source,
            "fn main(){let x=1;let r=&x;let nested=&r;}"
                | "fn main(){let x=1;let r=&x;let a=[r;0];}"
                | "fn main(){let x=1;let r=&x;let t=(r,);}"
                | "fn main(){let r=&1;}"
                | "fn main(){let a=[1];let i=0usize;let p=&a[i];}"
                // a2c25ee46: comparisons peel equal reference depths.
                | "fn main(){let x=1;let a=&x;let b=&x;let eq=a==b;}"
        ) {
            let stock =
                super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
            assert!(
                stock.status.success(),
                "{}",
                String::from_utf8_lossy(&stock.stderr)
            );
            result.unwrap();
            check(&parse(source).unwrap()).unwrap();
        } else {
            assert!(result.is_err(), "accepted {source}");
        }
    }
}

#[test]
fn usize_retains_full_width_and_uses_forward_and_binary_peer_contexts() {
    let p=parse("fn id(x:usize)->usize{x}fn high()->usize{18446744073709551615}fn main(){let u:usize=4294967296;let s=18446744073709551615usize;let b=1<u;let c=u>1;let r:usize=1+2;let q=1+u;let not=!0usize;let x=id(4294967296);let a:[usize;2]=[1,2];let mut b=[1,2];let i:usize=0;b[i]=3;let n=b[i];let min=-2147483648i32;}").unwrap();
    let env = signature_table(&p).unwrap();
    let typed = type_function_with_signatures(&p, 2, &env).unwrap();
    assert!(typed.exprs.iter().any(|e| matches!(
        e.kind,
        ir::ExprKind::Int(ir::IntValue {
            ty: ir::IntTy::Usize,
            bits: 18446744073709551615
        })
    )));
    assert!(
        typed
            .exprs
            .iter()
            .any(|e| matches!(e.kind, ir::ExprKind::DynamicIndex { .. }))
    );
    assert!(
        typed
            .body
            .statements
            .iter()
            .any(|s| matches!(s, ir::TypedStmt::AssignIndex { .. }))
    );
    assert_eq!(
        type_function_with_signatures(&p, 1, &env).unwrap().result,
        ir::Ty::Int(ir::IntTy::Usize)
    );
}

#[test]
fn usize_does_not_convert_i32_or_erase_runtime_index_pattern_boundaries() {
    for source in [
        "fn main(){let x=0usize;match x{0=>(),_=>()};}",
        "fn main(){match 0usize{0usize=>(),_=>()};}",
        // Nested runtime row indexing is admitted (NESTED-INDEX.md); stock
        // accepts this unchanged source.
        "fn main(){let a=[[1]];let i=0usize;let x=a[i][0];}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "fn main(){let a=[1];let i:i32=0;let x=a[i];}",
        "fn main(){let x:usize=1i32;}",
        "fn main(){let x:i32=1usize;}",
        "fn main(){let x:usize=-1;}",
        "fn main(){let x=18446744073709551616usize;}",
        "struct S{x:i32}fn main(){let a=[S{x:1}];let i=0usize;let S{x:_}=a[i];}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn arrays_contextualize_empty_literals_and_keep_constant_index_projection() {
    let source = "struct S{x:[i32;0]}enum E{V([i32;0])}fn make()->[i32;0]{[]}fn explicit()->[i32;0]{return [];}fn take(x:[i32;0]){}fn main(){let z:[i32;0]=[];take([]);let s=S{x:[]};let e=E::V([]);let nested:[[i32;0];2]=[[],[]];let mut a=[1,2];a[0]=3;let n=a[0];let t:([i32;0],)=([],);let q:[i32;0]=if true{[]}else{[]};let r:[i32;0]=match true{_=>[]};}";
    let program = parse(source).unwrap();
    check(&program).unwrap();
    let function = type_function_with_signatures(
        &program,
        program.functions.len() - 1,
        &signature_table(&program).unwrap(),
    )
    .unwrap();
    assert!(function.body.statements.iter().any(|s|matches!(s,ir::TypedStmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::ConstantIndex(0)])));
}

#[test]
fn arrays_reject_bounds_types_repeat_moves_and_unsupported_inference() {
    let source = "fn main(){let a=[1];let i=0;let n=a[i];}";
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    check(&parse(source).unwrap()).unwrap();
    // Large scalar repeats are valid type-level inputs. Per-element expansion
    // and backend storage admission are separate from this CHECK boundary.
    let large_scalar = "fn main(){let a=[1;65536];}";
    let stock =
        super::parser_tests::stock_compile(large_scalar, std::path::Path::new("unused"), true);
    assert!(stock.status.success());
    check(&parse(large_scalar).unwrap()).unwrap();
    for source in [
        "fn main(){let a=[1,2];let n=a[2];}",
        "fn main(){let a=[1,true];}",
        "fn main(){let a:[i32;1]=[];}",
        "struct S{x:i32}fn main(){let a=[S{x:1};2];}",
        "struct S{x:i32}fn main(){let a=[S{x:1}];let s=a[0];}",
        "fn main(){let a=[1;16777217];}",
        "fn main(){let a=[];}",
    ] {
        assert!(
            parse(source).and_then(|p| check(&p)).is_err(),
            "accepted {source}"
        );
    }
    check(
        &parse("struct S{x:i32}fn main(){let a=[S{x:1};0];let b=[S{x:2};1];let n=b[0].x;}")
            .unwrap(),
    )
    .unwrap();
}

#[test]
fn payload_variants_preserve_constructor_shape_source_order_and_projection_ids() {
    let source = "struct S{x:i32} enum E{Unit,Tuple(S,bool),Named{a:i32,b:S},Zero(),Empty{}} use E::Named as Alias;fn main(){let e=Alias{b:S{x:2},a:1};let n=match e{Alias{b:S{x:n},a:_,..}=>n,E::Tuple(S{x:n},_)=>n,E::Unit=>0,E::Zero()=>0,E::Empty{}=>0};let z=E::Zero();let q=E::Empty{};}";
    let program = parse(source).unwrap();
    let env = type_env(&program).unwrap();
    let checked = type_function_with_env(&program, 0, &env).unwrap();
    checked.validate().unwrap();
    let enumeration = &env.enums[&fixture_nominal_key(&program, "E")];
    assert!(enumeration.has_payload());
    assert_eq!(enumeration.def().variants[3].shape, ir::VariantShape::Tuple);
    assert_eq!(
        enumeration.def().variants[4].shape,
        ir::VariantShape::Struct
    );
    assert!(checked.exprs.iter().any(|e|matches!(&e.kind,ir::ExprKind::EnumVariant{variant:2,fields,..} if fields.iter().map(|(i,_)|*i).collect::<Vec<_>>()==vec![1,0])));
    let arms = checked
        .exprs
        .iter()
        .find_map(|e| match &e.kind {
            ir::ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .unwrap();
    let ir::MatchPattern::EnumVariant { bindings, .. } = &arms[0].pattern else {
        panic!("variant pattern")
    };
    assert_eq!(
        bindings[0].projection,
        vec![
            ir::Projection::Downcast(ir::VariantId {
                owner: enumeration.def().id,
                index: 2
            }),
            ir::Projection::Field(1),
            ir::Projection::Field(0)
        ]
    );
    check(&program).unwrap();
}

#[test]
fn singleton_payload_let_and_nested_irrefutable_patterns_are_typed() {
    let source = "enum Inner{One(i32)} enum Outer{One{inner:Inner,flag:bool}}fn main(){let Outer::One{inner:Inner::One(mut n),flag:_}=Outer::One{flag:true,inner:Inner::One(3)};n=4;let e=Outer::One{inner:Inner::One(n),flag:false};let n=match e{Outer::One{inner:Inner::One(n),..}=>n};}";
    let program = parse(source).unwrap();
    let env = type_env(&program).unwrap();
    let checked = type_function_with_env(&program, 0, &env).unwrap();
    checked.validate().unwrap();
    check(&program).unwrap();
}

#[test]
fn mixed_nominal_payload_dag_validates_unused_cycles_fields_and_privacy() {
    for source in [
        "struct S{x:E}enum E{A(S),B}fn main(){}",
        "enum A{V(B)}enum B{V(A)}fn main(){}",
        "enum A{V(Missing)}fn main(){}",
        "enum A{V{x:i32,x:bool}}fn main(){}",
        "mod m{struct H{x:i32}}enum E{V(m::H)}fn main(){}",
        "enum E{V(i32)}fn main(){E::V(true);}",
        "enum E{V{x:i32}}fn main(){E::V{};}",
        "enum E{V(i32)}fn main(){match E::V(1){E::V()=>()};}",
        "enum E{V{i:i32}}fn main(){match E::V{i:1}{E::V{i:x,i:y}=>()};}",
    ] {
        assert!(
            parse(source).and_then(|program| check(&program)).is_err(),
            "accepted {source}"
        );
    }
    let program =
        parse("enum E{V(S)}struct S{x:i32}fn main(){let E::V(S{x:n})=E::V(S{x:1});}").unwrap();
    check(&program).unwrap();
}

#[test]
fn variant_imports_reserve_type_namespace_without_becoming_types() {
    for variant in ["V", "V(i32)", "V{x:i32}"] {
        for conflict in ["struct A{x:i32}", "enum A{Unit}", "mod A{}"] {
            let source = format!("{conflict} enum E{{{variant}}}use E::V as A;fn main(){{}}");
            assert!(parse(&source).is_err(), "accepted {source}");
        }
        let source = format!("enum E{{{variant}}}use E::V as A;fn f(x:A){{}}fn main(){{}}");
        assert!(parse(&source).is_err(), "variant used as type: {source}");
    }
    check(&parse("fn A()->i32{1}enum E{V{x:i32}}use E::V as A;fn main(){let n=A();let e=A{x:n};match e{A{x:n}=>{}}}").unwrap()).unwrap();
    check(&parse("struct A{x:i32}fn A()->i32{1}fn main(){let n=A();let a=A{x:n};}").unwrap())
        .unwrap();
    check(&parse("enum E{V{x:i32}}use E::V;fn f(V:i32)->i32{V}fn main(){let V:i32;V=1;let n=f(V);let V=2;let n=V;}").unwrap()).unwrap();
}

#[test]
fn typed_deferred_declarations_publish_fresh_uninitialized_locals() {
    let source = "enum E{A,B} struct S{e:E} fn main(){let x=1;let x:i32;let mut y:(i32,bool);let s:S;x=2;y=(x,true);s=S{e:E::A};let x=true;match s.e{E::A=>(),E::B=>()};}";
    let program = parse(source).unwrap();
    let env = type_env(&program).unwrap();
    let checked = type_function_with_env(&program, 0, &env).unwrap();
    assert_eq!(checked.locals.len(), 5);
    assert_eq!(checked.locals[0].name, "x");
    assert_eq!(checked.locals[1].name, "x");
    assert!(!checked.locals[1].mutable);
    assert!(checked.locals[2].mutable);
    assert!(matches!(
        checked.body.statements[1],
        ir::TypedStmt::Declare {
            local: ir::LocalId(1)
        }
    ));
    checked.validate().unwrap();
    check(&program).unwrap();
}

#[test]
fn deferred_declarations_keep_flow_checks_outside_pure_typing() {
    for source in [
        "fn main(){let x:i32;x=1;let y=x;}",
        "fn f(b:bool)->i32{let x:i32;if b{x=1;}else{x=2;}x}fn main(){}",
        "fn main(){let mut x:i32;x=1;x=2;}",
        "fn main(){return;let x=1;x=2;}",
        "fn main(){let x=1;return;x=2;}",
    ] {
        check(&parse(source).unwrap()).unwrap();
    }
    for source in [
        "fn main(){let x:i32;let y=x;}",
        "fn main(){let x:i32;x=1;x=2;}",
        "fn main(){let x=1;x=2;}",
        "fn f(b:bool){let x:i32;if b{x=1;}let y=x;}fn main(){}",
        "fn main(){let x:i32;loop{x=1;}}",
    ] {
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        type_function_with_env(&program, 0, &env).unwrap();
        assert!(
            check_function_with_signatures(&program, 0, &env).is_err(),
            "accepted {source}"
        );
        assert!(check_function(&program, 0).is_err());
        assert!(check(&program).is_err());
    }
    for source in [
        "fn main(){let x;}",
        "fn main(){let mut x;}",
        "fn main(){let (x,y):(i32,i32);}",
        "fn main(){let _:i32;}",
        "enum E{A}use E::A;fn main(){let A:E;}",
    ] {
        if matches!(source, "fn main(){let x;}" | "fn main(){let mut x;}") {
            assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
        } else {
            assert!(parse(source).is_err(), "accepted {source}");
        }
    }
    let source = "fn main(){let x:_;}";
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(!stock.status.success());
    assert!(String::from_utf8_lossy(&stock.stderr).contains("E0282"));
    let program = parse(source).unwrap();
    assert!(
        check(&program)
            .unwrap_err()
            .contains("untyped deferred reference declaration remains underconstrained")
    );
    let source = "fn main(){let x:!;}";
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(!stock.status.success());
    assert!(String::from_utf8_lossy(&stock.stderr).contains("E0658"));
    assert!(parse(source).is_err(), "accepted unstable local never type");
}

#[test]
fn admitted_carrier_and_copy_pattern_shapes_match_stock() {
    for source in [
        "fn main(){let whole @ (a,b)=(3i32,4i32);assert!(whole.0+whole.1==a+b);println!(\"7\");}",
        "fn id<T>(x:T)->T{x}fn main(){let n=7;let pair=id::<(&i32,bool)>((&n,true));assert!(*pair.0==7);println!(\"7\");}",
        "fn id<T>(x:T)->T{x}fn main(){let empty:[&i32;0]=[];let out=id::<[&i32;0]>(empty);assert!(out.len()==0);println!(\"7\");}",
    ] {
        let stock =
            super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
        assert!(
            stock.status.success(),
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        super::parser_tests::stock_native_run(source);
    }
    let source = "fn main(){let mut n=7;let whole @ (part,)=(&mut n,);let _=whole;let _=part;}";
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(!stock.status.success());
    assert!(check(&parse(source).unwrap()).is_err());
}

#[test]
fn enum_match_types_paths_scopes_and_nominal_identity() {
    let source = "mod m{pub enum E{A,B} pub fn make()->E{E::A}} use m::E as Alias; use Alias::A as First; struct S{e:Alias} fn f(x:Alias)->i32{match x{First=>1,Alias::B=>2}} fn main(){let s=S{e:First};let n=f(s.e);match true{true=>{},false=>{}} match (){()=>{}} let n=match n{-2147483648=>0,x=>x};let _=m::make();}";
    let program = parse(source).unwrap();
    let env = type_env(&program).unwrap();
    assert_ne!(
        env.structs[&fixture_nominal_key(&program, "S")].def().id,
        env.enums[&fixture_nominal_key(&program, "m::E")].def().id
    );
    assert!(!ir::Ty::Enum(env.enums[&fixture_nominal_key(&program, "m::E")].clone()).is_copy());
    check(&program).unwrap();
    let checked = type_function_with_env(&program, 1, &env).unwrap();
    assert!(
        checked
            .exprs
            .iter()
            .any(|expr| matches!(&expr.kind,ir::ExprKind::Match{arms,..} if arms.len()==2))
    );
}

#[test]
fn enum_match_exhaustiveness_and_dead_arm_obligations() {
    for source in [
        "enum E{A,B} fn main(){match E::A{E::A=>()};}",
        "fn main(){match true{true=>()};}",
        "fn main(){match 0{0=>()};}",
        "enum E{A} enum F{A} fn main(){match E::A{F::A=>()};}",
        "fn main(){match true{_=>1,true=>false};}",
        "fn main(){match true{_=>(),false=>{let x:bool=1;}};}",
        "fn main(){match 1{x=>x};x;}",
        "enum E{A,B} fn main(){let E::A=E::A;}",
        "fn f()->i32{loop{match true{_=>break 1,true=>break false}}} fn main(){}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "accepted {source}");
    }
    assert!(parse("enum E{A} use E::A; fn main(){match E::A{mut A=>()};}").is_err());
    for source in [
        "enum E{A,B} fn main(){match E::A{E::A=>(),E::A=>(),E::B=>(),_=>()};}",
        "enum E{A} use E::A; fn main(){let A=E::A;match A{A=>()};}",
        "fn f()->i32{match loop{} {}} fn main(){}",
        "enum E{A,B} fn f(e:E)->i32{match e{E::A{}=>return 1,E::B{}=>return 2}} fn main(){let e=E::A{};let n=f(e);}",
        "fn f()->i32{loop {match true{true=>{break 1;},false=>{break 2;},_=>{break 3;}}}} fn main(){}",
    ] {
        check(&parse(source).unwrap()).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    for source in [
        "enum E{A,B}use E::A;fn f(A:E){}fn main(){}",
        "enum E{A}use E::A;fn f(A:E)->E{A}fn main(){}",
    ] {
        assert!(
            parse(source)
                .unwrap_err()
                .contains("parameters are unsupported")
        );
    }
}

fn typed(source: &str, index: usize) -> ir::CheckedFunction {
    let program = parse(source).unwrap();
    let signatures = signature_table(&program).unwrap();
    let checked = type_function_with_signatures(&program, index, &signatures).unwrap();
    checked.validate().unwrap();
    checked
}

#[test]
fn tuple_grouping_and_singleton_syntax_are_distinct() {
    let program = parse("fn f(x:((i32)),y:(bool,))->((i32,bool),()) { ((x,y.0),()) } fn main(){let a=(1);let b=(1,);}").unwrap();
    let f = &program.functions[0];
    assert!(matches!(f.params[0].1, Type::Int(ir::IntTy::I32)));
    assert!(
        matches!(&f.params[1].1, Type::Tuple(fields) if matches!(fields.as_slice(), [Type::Bool]))
    );
    assert!(matches!(&f.result, Type::Tuple(fields)
            if matches!(fields.as_slice(), [Type::Tuple(inner), Type::Unit]
                if matches!(inner.as_slice(), [Type::Int(ir::IntTy::I32), Type::Bool]))));
    check(&program).unwrap();
    let f = typed("fn main(){let a=(1);let b=(1,);let z=((),((),));}", 0);
    assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
    assert_eq!(
        f.locals[1].ty,
        ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32)].into())
    );
    assert_eq!(
        f.locals[2].ty,
        ir::Ty::Tuple(vec![ir::Ty::Unit, ir::Ty::Tuple(vec![ir::Ty::Unit].into())].into())
    );
}

#[test]
fn tuple_places_resolve_parenthesized_local_roots_and_shadowing() {
    let f = typed(
        "fn main(){let mut x=((1,true),());((x)).0.1=false;let mut x=(2,);x.0=3;}",
        0,
    );
    let places = f
        .body
        .statements
        .iter()
        .filter_map(|s| match s {
            ir::TypedStmt::AssignPlace { place, .. } => Some(place),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(places[0].local, ir::LocalId(0));
    assert_eq!(
        places[0].projection,
        vec![ir::Projection::Field(0), ir::Projection::Field(1)]
    );
    assert_eq!(places[1].local, ir::LocalId(1));
    assert_eq!(places[1].projection, vec![ir::Projection::Field(0)]);
    assert_eq!(f.locals[0].name, f.locals[1].name);
}

#[test]
fn tuple_aggregate_flows_preserve_each_actual_type_and_projection_completion() {
    let f = typed("fn f()->i32 {({return 3;},0).1} fn main(){}", 0);
    let projected = &f.exprs[f.body.tail.unwrap().0];
    assert_eq!(projected.ty, ir::Ty::Int(ir::IntTy::I32));
    assert_eq!(projected.completion, ir::Completion::Never);
    let ir::ExprKind::Field { base, .. } = projected.kind else {
        panic!("expected projection")
    };
    assert_eq!(
        f.exprs[base.0].ty,
        ir::Ty::Tuple(vec![ir::Ty::Never, ir::Ty::Int(ir::IntTy::I32)].into())
    );
    let f = typed("fn main(){let x:(bool,i32)=({return;},0);}", 0);
    assert_eq!(
        f.locals[0].ty,
        ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
    );
    let f = typed("fn main(){let x=({return;false},0);}", 0);
    assert_eq!(
        f.locals[0].ty,
        ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)].into())
    );
    for source in [
        "fn main(){let x:(i32,i32)=({return;false},0);}",
        "fn main(){let x=if true {({return;false},0)} else {(1,0)};}",
        "fn main(){let x=loop {break ({return;false},0);break (1,0);};}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn tuple_calls_and_loop_breaks_keep_structural_signatures() {
    let f = typed(
        "mod m {pub fn f(x:(i32,bool))->(i32,bool){x}} fn main(){let t=loop {break if true {m::f((1,true))} else {(2,false)};};}",
        1,
    );
    let call = f
        .exprs
        .iter()
        .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    let ty = ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool].into());
    assert_eq!(call.ty, ty);
    let ir::ExprKind::Call {
        name, signature, ..
    } = &call.kind
    else {
        unreachable!()
    };
    assert_eq!(name, "crate::m::f");
    assert_eq!(signature.params, vec![ty.clone()]);
    assert_eq!(signature.result, ty);
    assert_eq!(f.locals[0].ty, ty);
}

#[test]
fn tuple_invalid_fields_assignments_and_operators_are_rejected() {
    for source in [
        "fn main(){let x=(1,);x.0=2;}",
        "fn main(){let mut x=(1,);x.0=true;}",
        "fn main(){let mut x=(1,);x.1=2;}",
        "fn main(){let x=(1,);let y=x.0.0;}",
        "fn main(){let x=().0;}",
        "fn main(){let x=(1,)==(1,);}",
        "fn main(){let x=(1,)<(2,);}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "{source}");
    }
    for source in [
        "fn main(){(1,).0=2;}",
        "fn main(){let x=(1,).00;}",
        "fn main(){let x=(1,).0usize;}",
        "fn main(){let x=(1,).-1;}",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    check(&parse("fn main(){let (x,..)=(1,2);}").unwrap()).unwrap();
}

#[test]
fn tuple_type_nesting_and_nested_inference_are_checked() {
    // Direct parser calls run where graph workers do: on a worker-sized stack.
    std::thread::Builder::new()
        .stack_size(crate::engine::WORKER_STACK_BYTES)
        .spawn(tuple_type_nesting_and_nested_inference_body)
        .unwrap()
        .join()
        .unwrap();
}
fn tuple_type_nesting_and_nested_inference_body() {
    let nested = format!("{}i32{}", "(".repeat(140), ",)".repeat(140));
    assert!(
        parse(&format!("fn f(x:{nested}){{}}fn main(){{}}"))
            .unwrap_err()
            .contains("nesting")
    );
    let mut program = parse("fn f(x:(i32,)){}fn main(){}").unwrap();
    program.functions[0].params[0].1 = Type::Tuple(vec![Type::Infer]);
    assert!(
        signature_table(&program)
            .unwrap_err()
            .contains("cannot be inferred")
    );
}

#[test]
fn loop_control_has_distinct_resolved_targets_and_scalar_types() {
    let f = typed(
        "fn main(){ let x=loop { while true { continue; break; } loop { break; } break 3; }; let y=loop { break true; }; }",
        0,
    );
    assert_eq!(
        f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
        vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool]
    );
    let breaks = f
        .exprs
        .iter()
        .filter_map(|e| match e.kind {
            ir::ExprKind::Break { target, .. } => {
                assert_eq!(e.flow(), None);
                Some(target)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        breaks,
        vec![ir::LoopId(1), ir::LoopId(2), ir::LoopId(0), ir::LoopId(3)]
    );
    assert!(f.exprs.iter().any(|e| matches!(
        e.kind,
        ir::ExprKind::Continue {
            target: ir::LoopId(1)
        }
    )));
}

#[test]
fn divergent_break_operand_contributes_only_its_nested_breaks() {
    let f = typed(
        "fn f()->i32 { loop { break { break 3; }; } } fn main(){}",
        0,
    );
    let root = &f.exprs[f.body.tail.unwrap().0];
    assert!(matches!(root.kind, ir::ExprKind::Loop { .. }));
    assert_eq!(root.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
    let f = typed(
        "fn f()->i32 { loop { break { return 3; }; } } fn main(){}",
        0,
    );
    assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
    let f = typed("fn f()->i32 { loop { continue; break 3; } } fn main(){}", 0);
    assert_eq!(
        f.exprs[f.body.tail.unwrap().0].ty,
        ir::Ty::Int(ir::IntTy::I32)
    );
    assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
    let f = typed("fn f()->i32 { loop { continue; } } fn main(){}", 0);
    assert_eq!(f.exprs[f.body.tail.unwrap().0].flow(), None);
}

#[test]
fn typed_unreachable_tails_preserve_types_without_creating_loop_exits() {
    let f = typed(
        "fn f()->i32 { loop { break {return 3; 0}; } } fn main(){}",
        0,
    );
    let value = &f.exprs[f.body.tail.unwrap().0];
    assert_eq!(value.ty, ir::Ty::Int(ir::IntTy::I32));
    assert_eq!(value.completion, ir::Completion::Never);
    let f = typed("fn f()->i32 { !{return 3;} } fn main(){}", 0);
    let value = &f.exprs[f.body.tail.unwrap().0];
    assert_eq!(value.ty, ir::Ty::Never);
    assert_eq!(value.completion, ir::Completion::Never);
    let f = typed("fn main(){ let x={return; true}; }", 0);
    assert_eq!(f.locals[0].ty, ir::Ty::Bool);
    for source in [
        "fn f()->i32 {return 3; ()} fn main(){}",
        "fn main(){loop {break 1; false}}",
        "fn main(){let x:bool=loop {break {return; 0};};}",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn explicit_semicolons_discard_blocklike_values() {
    let program = parse("fn main(){ {1}; if true {1} else {2}; loop {break 3;}; }").unwrap();
    check(&program).unwrap();
    assert!(
        program.functions[0]
            .body
            .statements
            .iter()
            .all(|s| matches!(s, Stmt::Expr(_)))
    );
    let program = parse("fn main(){ loop {break;} let x=1; }").unwrap();
    assert!(matches!(
        program.functions[0].body.statements[0],
        Stmt::ImplicitExpr(Expr::Loop { .. })
    ));
    check(&program).unwrap();
    for source in [
        "fn main(){ {1} let x=1; }",
        "fn main(){ if true {1} else {2} let x=1; }",
        "fn main(){ loop {break 3;} let x=1; }",
    ] {
        assert!(check(&parse(source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn loop_obligations_reject_unreachable_conflicts_and_while_values() {
    for source in [
        "fn main(){ loop {break 3; break true;} }",
        "fn main(){ loop {break; break 3;} }",
        "fn main(){ loop {continue; break 3; break true;} }",
        "fn main(){ while true {break ();} }",
        "fn main(){ while true {break {return;};} }",
        "fn main(){ break; }",
        "fn main(){ continue; }",
        "fn main(){ loop {1} }",
    ] {
        let p = parse(source).unwrap();
        assert!(check(&p).is_err(), "{source}");
    }
}

#[test]
fn while_conditions_are_control_target_barriers() {
    for source in [
        "fn main(){ while {break;} {} }",
        "fn main(){ while {continue;} {} }",
        "fn main(){ loop {while {break;} {}} }",
        "fn main(){ loop {while {continue;} {}} }",
    ] {
        let error = check(&parse(source).unwrap()).unwrap_err();
        assert!(error.contains("E0590"), "{error}");
    }
    let f = typed("fn main(){ while {loop {break true;}} {break;} }", 0);
    let ir::TypedStmt::While { id, .. } = f.body.statements[0] else {
        panic!("expected while")
    };
    assert_eq!(id, ir::LoopId(0));
    let breaks = f
        .exprs
        .iter()
        .filter_map(|e| match e.kind {
            ir::ExprKind::Break { target, .. } => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(breaks, vec![ir::LoopId(1), ir::LoopId(0)]);
}

#[test]
fn loop_walkers_resolve_break_value_calls_and_bound_depth() {
    let f = typed(
        "mod m {pub fn f()->i32{3}} fn main(){let x=loop {break m::f();};}",
        1,
    );
    assert!(
        f.exprs
            .iter()
            .any(|e| matches!(&e.kind, ir::ExprKind::Call { name, .. } if name == "crate::m::f"))
    );
    let nested_breaks = "break ".repeat(140);
    let source = format!("fn main(){{{nested_breaks}0;}}");
    assert!(parse(&source).unwrap_err().contains("nesting"));
    let labeled = typed("fn main(){'outer: loop {break 'outer;}}", 0);
    assert!(labeled.exprs.iter().any(|e| matches!(
        e.kind,
        ir::ExprKind::Break {
            target: ir::LoopId(0),
            value: None
        }
    )));
}

#[test]
fn inferred_shadowing_allocates_distinct_typed_locals() {
    let f = typed(
        "fn f(x:i32)->i32 { let x=x+1; let x=true; { let x=3; x; }; if x { 4 } else { 5 } } fn main(){}",
        0,
    );
    assert_eq!(f.params, vec![ir::LocalId(0)]);
    assert_eq!(
        f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
        vec![
            ir::Ty::Int(ir::IntTy::I32),
            ir::Ty::Int(ir::IntTy::I32),
            ir::Ty::Bool,
            ir::Ty::Int(ir::IntTy::I32)
        ]
    );
    assert!(f.locals.iter().all(|l| l.name == "x"));
    let reads = f
        .exprs
        .iter()
        .filter_map(|e| match e.kind {
            ir::ExprKind::Local(id) => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(3), ir::LocalId(2)]);
}

#[test]
fn assignment_and_initializer_resolve_before_shadowing() {
    let f = typed("fn main(){ let mut x=1; x=x+1; let x=x==2; x; }", 0);
    assert!(f.locals[0].mutable);
    assert!(!f.locals[1].mutable);
    assert_eq!(f.locals[1].ty, ir::Ty::Bool);
    assert!(matches!(
        f.body.statements[1],
        ir::TypedStmt::Assign {
            local: ir::LocalId(0),
            ..
        }
    ));
    let reads = f
        .exprs
        .iter()
        .filter_map(|e| match e.kind {
            ir::ExprKind::Local(id) => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reads, vec![ir::LocalId(0), ir::LocalId(0), ir::LocalId(1)]);
}

#[test]
fn canonical_calls_keep_validated_signature() {
    let f = typed(
        "mod m { pub fn f(x:bool)->i32 { if x { 1 } else { 2 } } } use m::f as g; fn main(){ let n=g(true); }",
        1,
    );
    let call = f
        .exprs
        .iter()
        .find(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    assert_eq!(call.flow(), Some(ir::Ty::Int(ir::IntTy::I32)));
    let ir::ExprKind::Call {
        name,
        args,
        signature,
        target,
    } = &call.kind
    else {
        unreachable!()
    };
    assert_eq!(name, "crate::m::f");
    assert!(matches!(target,ir::Callee::Direct(t) if t.type_args.is_empty()));
    assert_eq!(signature.params, vec![ir::Ty::Bool]);
    assert_eq!(signature.result, ir::Ty::Int(ir::IntTy::I32));
    assert_eq!(f.exprs[args[0].0].ty, ir::Ty::Bool);
    assert_eq!(f.locals[0].ty, ir::Ty::Int(ir::IntTy::I32));
}

#[test]
fn divergence_and_short_circuit_keep_existing_completion() {
    let f = typed(
        "fn f()->bool { let x=false && { return true; }; let y=true || { return false; }; let z={ return x; }; } fn main(){}",
        0,
    );
    assert_eq!(
        f.locals.iter().map(|l| l.ty.clone()).collect::<Vec<_>>(),
        vec![ir::Ty::Bool, ir::Ty::Bool, ir::Ty::Never]
    );
    let binaries = f
        .exprs
        .iter()
        .filter(|e| matches!(e.kind, ir::ExprKind::Binary { .. }))
        .collect::<Vec<_>>();
    assert!(binaries.iter().all(|e| e.flow() == Some(ir::Ty::Bool)));
    let never = f
        .exprs
        .iter()
        .filter(|e| e.completion == ir::Completion::Never)
        .collect::<Vec<_>>();
    assert_eq!(never.len(), 3);
    assert!(
        never
            .iter()
            .all(|e| e.ty == ir::Ty::Never && e.flow().is_none())
    );
}

#[test]
fn lowering_and_checking_reject_unreachable_errors_identically() {
    for source in [
        "fn main(){ return; let x=true+1; }",
        "fn main(){ return; let x:bool=1; }",
        "fn main(){ if true { 1 } else { 2 } let x=0; }",
        "fn f()->i32 { while true { return 1; } } fn main(){}",
    ] {
        let program = parse(source).unwrap();
        let signatures = signature_table(&program).unwrap();
        let lowered = type_function_with_signatures(&program, 0, &signatures).unwrap_err();
        assert_eq!(
            check_function_with_signatures(&program, 0, &signatures).unwrap_err(),
            lowered
        );
    }
}
