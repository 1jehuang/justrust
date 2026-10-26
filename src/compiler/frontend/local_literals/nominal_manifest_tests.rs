//! Preparation authority checks and an exact stock/native projected-Copy regression.
use super::*;

#[test]
fn projected_copy_record_keeps_graph_identity_at_runtime() {
    // Exact unchanged tests/associated_types.rs case20_copy_record source.
    crate::compiler::frontend::parser_tests::stock_native_run(
        "trait Has{type Out;} #[derive(Clone,Copy)] struct Record{n:i32,b:bool} struct S; impl Has for S{type Out=Record;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=Record{n:42,b:true}; let y=pass(x); println!(\"{} {} {}\",x.n,y.n,y.b);}\n",
    );
    // Exact unchanged case34_literal_copy_record, including its trailing LF.
    crate::compiler::frontend::parser_tests::stock_native_run(
        "trait Has{type Out;} #[derive(Clone,Copy)] struct Record{n:i32,b:bool} struct S; impl Has for S{type Out=Record;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=Record{n:42,b:true}; let y=pass(x); if x.n==42{if x.b{if y.n==42{if y.b{println!(\"ok\");}}}}}\n",
    );
    // The normal form is structural: both nested nominal leaves need the
    // same graph reads as the corresponding constructor and local types.
    crate::compiler::frontend::parser_tests::stock_native_run(
        "trait Has{type Out;} #[derive(Clone,Copy)] struct Record{n:i32} struct S; impl Has for S{type Out=(Record,(bool,Record));} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=(Record{n:42},(true,Record{n:7})); let y=pass(x); println!(\"{} {} {}\",x.0.n,y.1.1.n,y.1.0);}\n",
    );
}

#[test]
fn projected_normal_form_requires_graph_read_and_preserves_missing() {
    let program =
        parse("struct Ingress{value:i32}fn value()->Ingress{Ingress{value:1}}fn main(){}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let plan = signatures.plan().unwrap();
    let source = plan
        .function_schema(plan.function_decl(0).unwrap())
        .unwrap()
        .signature
        .result;
    let function = &program.functions[1];
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    let schema = |_| Ok(None);
    let missing = |_: &Demand| Ok(None);
    let inputs = Inputs::new(&schema, &primitive_source, &missing);
    let solver = solve_body(&checker, &function.body, None, Some(&inputs)).unwrap();
    assert!(
        solver
            .resolve_projection_representation(&source, 0)
            .unwrap()
            .is_none()
    );
    let requested = solver.missing_nominals.borrow();
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].declaration, nominal_parts(&source).unwrap().0);
    assert!(requested[0].arguments.is_empty());
    drop(requested);

    // Even an authentic source application is not a typed graph read.
    let raw = |_: &Demand| Ok(Some(source.clone()));
    let inputs = Inputs::new(&schema, &primitive_source, &raw);
    let solver = solve_body(&checker, &function.body, None, Some(&inputs)).unwrap();
    let error = solver
        .resolve_projection_representation(&source, 0)
        .unwrap_err();
    assert!(
        error.contains("projected nominal lacks its graph read"),
        "{error}"
    );
    assert!(solver.missing_nominals.borrow().is_empty());

    // Equal nominal IDs must not hide forged source payloads.
    let ir::Ty::Struct(original) = &source else {
        panic!("struct source")
    };
    let mut definition = original.def().clone();
    let mut fields = definition.fields.read(&plan.budget).unwrap().to_vec();
    fields[0].ty = ir::Ty::Bool;
    definition.fields = fields.into();
    let forged = ir::Ty::Struct(ir::StructTy::new(definition));
    assert!(
        solver
            .resolve_projection_representation(&forged, 0)
            .is_err()
    );
}

#[test]
fn nominal_recipe_selectors_reject_non_fields_and_out_of_range_fields() {
    let program = parse("struct Wrap<T>{value:T}fn main(){let w=Wrap{value:1u8};}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let function = &program.functions[0];
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    let declaration_schema = |declaration| {
        let plan = signatures.plan()?;
        let (env, _) = plan.nominal_schema(declaration, &plan.budget)?;
        let parameter = ir::ParamId {
            owner: env.binder(),
            index: 0,
        };
        Ok(Some(DeclarationRecipe {
            env,
            fields: DeclarationFields::Struct(vec![Shape::Leaf(ir::Ty::Param(parameter))].into()),
        }))
    };
    let nominal_type = |_: &Demand| Ok(None);
    let inputs = Inputs::new(&declaration_schema, &primitive_source, &nominal_type);
    let mut solver = solve_body(&checker, &function.body, None, Some(&inputs)).unwrap();
    let manifest = solver.solved_nominal_manifest(false).unwrap();
    let declaration = manifest.demands()[0].declaration;
    let actual = solver.fixed(ir::Ty::Int(ir::IntTy::U8)).unwrap();
    for selector in [FieldSelector::ConstantIndex(0), FieldSelector::Deref] {
        assert!(
            solver
                .prepare_field(declaration, &[actual], &selector)
                .unwrap_err()
                .contains("non-field selector in nominal recipe")
        );
    }
    assert!(
        solver
            .prepare_field(declaration, &[actual], &FieldSelector::Tuple(99))
            .unwrap_err()
            .contains("nominal recipe field index")
    );
    assert!(
        solver
            .prepare_field(declaration, &[], &FieldSelector::Tuple(0))
            .unwrap_err()
            .contains("nominal recipe argument arity")
    );
    assert!(checker.exprs.is_empty());
}

#[test]
fn canonical_source_parameters_are_seeded_without_runtime_bindings() {
    let program = parse("fn seed(x:u8){let y:u8=x;}fn main(){}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let index = program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "seed")
        })
        .unwrap();
    let plan = signatures.plan().unwrap();
    let declaration = plan.function_decl(index).unwrap();
    let function = plan.callable_body(&program, declaration).unwrap();
    let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    checker
        .install_lexical_scope(function.lexical_scope)
        .unwrap();
    checker.declaration = Some(declaration);
    let source_type = |source: &Type| {
        Ok(match source {
            Type::Unit => Some(Shape::Leaf(ir::Ty::Unit)),
            Type::Int(kind) => Some(Shape::Leaf(ir::Ty::Int(*kind))),
            _ => None,
        })
    };
    let declaration_schema = |_| Ok(None);
    let nominal_type = |_: &Demand| Ok(None);
    let inputs = Inputs::new(&declaration_schema, &source_type, &nominal_type);
    assert!(checker.locals.is_empty());
    assert!(matches!(
        prepare_callable(&checker, declaration, None, &inputs).unwrap(),
        Preparation::Solved(_)
    ));
    assert!(checker.locals.is_empty());
    assert!(checker.exprs.is_empty());
    assert!(matches!(
        prepare_callable_nominals(plan, declaration, None, &inputs).unwrap(),
        Preparation::Solved(_)
    ));
    assert!(matches!(
        finalized_callable_nominal_hints(plan, declaration, None, &inputs).unwrap(),
        PreparedHints::Ready(_)
    ));
    let NominalCallableOutput::Ready(body, _, _) = type_callable_source_with_nominal_inputs(
        &program,
        declaration,
        plan,
        &plan.budget,
        None,
        None,
        &inputs,
    )
    .unwrap() else {
        panic!("primitive callable has no missing nominal graph input");
    };
    assert_eq!(body.params.len(), 1);
    let constants = plan
        .prepare_body_constants(declaration, &plan.budget)
        .unwrap();
    assert!(constants.direct().is_empty());
    let constants = plan
        .seal_body_constants(constants, Vec::new(), &plan.budget)
        .unwrap();
    assert!(matches!(
        plan.publish_callable_body_with_nominal_inputs(
            declaration,
            constants,
            &plan.budget,
            None,
            &inputs,
        )
        .unwrap(),
        NominalCallablePublication::Ready(PublishedCallableBody::Source(_), _, _)
    ));
    checker.declaration = None;
    assert!(
        prepare_callable(&checker, declaration, None, &inputs)
            .err()
            .expect("foreign callable owner must be rejected")
            .contains("owner mismatch")
    );
}

fn primitive_source(ty: &Type) -> Result<Option<Shape>, String> {
    Ok(match ty {
        Type::Unit => Some(Shape::Leaf(ir::Ty::Unit)),
        Type::Int(kind) => Some(Shape::Leaf(ir::Ty::Int(*kind))),
        _ => None,
    })
}

#[test]
fn missing_graph_read_blocks_after_later_numeric_constraint_without_emitting() {
    let program =
        parse("struct Wrap<T>{value:T}fn main(){let x=Wrap{value:1};let y:u8=x.value;}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let function = program
        .functions
        .iter()
        .find(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "main")
        })
        .unwrap();
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    let declaration_schema = |declaration| {
        let plan = signatures.plan()?;
        let (env, _) = plan.nominal_schema(declaration, &plan.budget)?;
        if env.arity() != 1 {
            return Err("missing test parameter".into());
        }
        let parameter = ir::ParamId {
            owner: env.binder(),
            index: 0,
        };
        Ok(Some(DeclarationRecipe {
            env,
            fields: DeclarationFields::Struct(vec![Shape::Leaf(ir::Ty::Param(parameter))].into()),
        }))
    };
    let nominal_type = |_demand: &Demand| Ok(None);
    let inputs = Inputs {
        declaration_schema: &declaration_schema,
        source_type: &primitive_source,
        nominal_type: &nominal_type,
    };
    let before_exprs = checker.exprs.len();
    let before_locals = checker.locals.len();
    let Preparation::Blocked {
        nominals,
        needs_source_schemas,
        unresolved_constraints,
        ..
    } = prepare(&checker, &function.body, None, &inputs).unwrap()
    else {
        panic!("an absent graph read cannot authorize completion");
    };
    assert!(!needs_source_schemas);
    assert!(!unresolved_constraints);
    assert_eq!(nominals.len(), 1);
    assert!(matches!(
        nominals[0].arguments.as_ref(),
        [Shape::Leaf(ir::Ty::Int(ir::IntTy::U8))]
    ));
    assert_eq!(checker.exprs.len(), before_exprs);
    assert_eq!(checker.locals.len(), before_locals);
    let PreparedHints::Blocked(Preparation::Blocked { nominals, .. }) =
        hints_with_inputs(&checker, &function.body, None, &inputs).unwrap()
    else {
        panic!("finalized hints must not fall back to eager nominal interning");
    };
    assert_eq!(nominals.len(), 1);
    assert_eq!(checker.exprs.len(), before_exprs);
    let index = program
        .functions
        .iter()
        .position(|candidate| std::ptr::eq(candidate, function))
        .unwrap();
    let plan = signatures.plan().unwrap();
    let declaration = plan.function_decl(index).unwrap();
    let NominalCallableOutput::Blocked(Preparation::Blocked { nominals, .. }) =
        type_callable_source_with_nominal_inputs(
            &program,
            declaration,
            plan,
            &plan.budget,
            None,
            None,
            &inputs,
        )
        .unwrap()
    else {
        panic!("actual consumer must propagate the exact absent graph read before lowering");
    };
    assert_eq!(nominals.len(), 1);
    assert!(matches!(
        nominals[0].arguments.as_ref(),
        [Shape::Leaf(ir::Ty::Int(ir::IntTy::U8))]
    ));
    let constants = plan
        .prepare_body_constants(declaration, &plan.budget)
        .unwrap();
    assert!(constants.direct().is_empty());
    let mut constants = plan
        .seal_body_constants(constants, Vec::new(), &plan.budget)
        .unwrap();
    let owner = constants.inventory.as_ref().unwrap().owner.clone();
    for _ in 0..2 {
        let NominalCallablePublication::Blocked {
            preparation,
            constants: retained,
        } = plan
            .publish_callable_body_with_nominal_inputs(
                declaration,
                constants,
                &plan.budget,
                None,
                &inputs,
            )
            .unwrap()
        else {
            panic!("missing nominal graph input must not publish a canonical body");
        };
        assert_eq!(retained.inventory.as_ref().unwrap().owner, owner);
        assert!(matches!(preparation, Preparation::Blocked { .. }));
        constants = *retained;
    }
}

#[test]
fn missing_source_certificate_is_explicit_not_eager_fallback() {
    let program = parse("fn main(){let x:u8=1;}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let function = &program.functions[0];
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    let declaration_schema = |_declaration| -> Result<Option<DeclarationRecipe>, String> {
        panic!("no nominal source requested")
    };
    let source_type = |_ty: &Type| Ok(None);
    let nominal_type = |_demand: &Demand| -> Result<Option<ir::Ty>, String> {
        panic!("no nominal type requested")
    };
    let inputs = Inputs {
        declaration_schema: &declaration_schema,
        source_type: &source_type,
        nominal_type: &nominal_type,
    };
    let Err(error) = prepare(&checker, &function.body, None, &inputs) else {
        panic!("unidentified source requests cannot be resumed or eagerly checked");
    };
    assert!(
        error.contains(
            "unsupported: source type preparation requires an authenticated source recipe"
        )
    );
    assert!(checker.exprs.is_empty());
    assert!(checker.locals.is_empty());
}

#[test]
fn recipe_comparison_preserves_order_and_propagates_meter_failure() {
    let left = Shape::Tuple(
        vec![
            Shape::Leaf(ir::Ty::Int(ir::IntTy::U8)),
            Shape::Leaf(ir::Ty::Int(ir::IntTy::U16)),
        ]
        .into(),
    );
    let right = Shape::Tuple(
        vec![
            Shape::Leaf(ir::Ty::Int(ir::IntTy::U16)),
            Shape::Leaf(ir::Ty::Int(ir::IntTy::U8)),
        ]
        .into(),
    );
    assert!(left.same_with(&left, 0, &mut |_| Ok(())).unwrap());
    assert!(!left.same_with(&right, 0, &mut |_| Ok(())).unwrap());
    let error = left
        .same_with(&left, 0, &mut |depth| {
            if depth > 1 {
                Err("test meter exhausted".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "test meter exhausted");
}

#[test]
fn missing_declaration_inventory_is_exact_and_deduplicated() {
    let program =
        parse("struct Wrap<T>{value:T}fn main(){let a=Wrap{value:1u8};let b=Wrap{value:2u16};}")
            .unwrap();
    let signatures = signature_table(&program).unwrap();
    let function = &program.functions[0];
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    let requested = std::cell::RefCell::new(Vec::new());
    let declaration_schema = |declaration| {
        requested.borrow_mut().push(declaration);
        Ok(None)
    };
    let nominal_type = |_demand: &Demand| Ok(None);
    let inputs = Inputs {
        declaration_schema: &declaration_schema,
        source_type: &primitive_source,
        nominal_type: &nominal_type,
    };
    let Preparation::Blocked {
        declarations,
        needs_source_schemas,
        ..
    } = prepare(&checker, &function.body, None, &inputs).unwrap()
    else {
        panic!("missing declaration facts must block");
    };
    assert!(needs_source_schemas);
    assert_eq!(requested.borrow().len(), 2);
    assert_eq!(declarations.len(), 1);
    assert!(
        requested
            .borrow()
            .iter()
            .all(|declaration| *declaration == declarations[0])
    );
    assert!(checker.exprs.is_empty());
}
