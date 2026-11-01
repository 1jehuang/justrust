//! Preparation authority checks and an exact stock/native projected-Copy regression.
use super::*;

#[test]
fn nominal_comparison_constructor_equations_require_unique_canonical_impl() {
    for (source, inferred, op) in [
        (
            "#[derive(PartialEq)]struct W<T>{a:T}fn main(){}",
            true,
            BinaryOp::Eq,
        ),
        (
            "#[derive(PartialEq,PartialOrd)]struct W<T>{a:T}fn main(){}",
            true,
            BinaryOp::Gt,
        ),
        (
            "struct W<T>{a:T}trait FakeOrd{fn gt(&self,r:&Self)->bool;}impl<T> FakeOrd for W<T>{fn gt(&self,r:&Self)->bool{true}}fn main(){}",
            false,
            BinaryOp::Gt,
        ),
        (
            "struct W<T>{a:T}trait FakeEq<R>{fn eq(&self,r:&R)->bool;}impl<T> FakeEq<W<T>> for W<T>{fn eq(&self,r:&W<T>)->bool{true}}fn main(){}",
            false,
            BinaryOp::Eq,
        ),
        (
            "struct W<T>{a:T}impl<T> PartialEq<W<u8>> for W<T>{fn eq(&self,r:&W<u8>)->bool{true}}impl<T> PartialEq<W<u16>> for W<T>{fn eq(&self,r:&W<u16>)->bool{false}}fn main(){}",
            false,
            BinaryOp::Eq,
        ),
    ] {
        let source = format!("{source}struct Other<T>{{a:T}}");
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "nominal-comparison-equations.rs",
            &source,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let index = program
            .structs
            .iter()
            .position(|item| {
                item.name
                    .segments()
                    .last()
                    .is_some_and(|name| program.names.spelling(*name).unwrap() == "W")
            })
            .unwrap();
        let nominal = plan.nominal_declaration(index).unwrap();
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
        let absent_schema = |_| Ok(None);
        let missing = |_: &Demand| Ok(None);
        let inputs = Inputs::new(&absent_schema, &primitive_source, &missing);
        let mut solver = solve_body(&checker, &function.body, None, Some(&inputs)).unwrap();
        let known = solver.fixed(ir::Ty::Int(ir::IntTy::U8)).unwrap();
        let unknown = solver.fresh().unwrap();
        let left = solver.node(Domain::Nominal(nominal, vec![known])).unwrap();
        let right = solver
            .node(Domain::Nominal(nominal, vec![unknown]))
            .unwrap();
        let result = solver.operator_binary(op, left, right).unwrap();
        // No typed realization read is supplied. The impl may constrain a
        // constructor's actual, but must never publish a type or operator body.
        assert!(!solver.solve_operators().unwrap());
        assert_eq!(solver.operators.len(), 1);
        assert!(matches!(solver.domain(result), Domain::Unknown));
        if inferred {
            assert!(matches!(
                solver.domain(unknown),
                Domain::Fixed(ir::Ty::Int(ir::IntTy::U8))
            ));
        } else {
            assert!(
                matches!(solver.domain(unknown), Domain::Unknown),
                "{source}"
            );
        }
        let pending = solver.pending_nominals().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].declaration, nominal);
        assert!(matches!(
            pending[0].arguments.as_ref(),
            [Shape::Leaf(ir::Ty::Int(ir::IntTy::U8))]
        ));
        if inferred {
            // Inject a declaration-substituted RHS term with the same actual
            // list. The unique impl equation must preserve the nominal owner,
            // not treat similarly shaped source declarations as interchangeable.
            let other = program
                .structs
                .iter()
                .position(|item| {
                    item.name
                        .segments()
                        .last()
                        .is_some_and(|name| program.names.spelling(*name).unwrap() == "Other")
                })
                .unwrap();
            let other = plan.nominal_declaration(other).unwrap();
            let forged_actual = solver.fresh().unwrap();
            let forged = solver
                .node(Domain::Nominal(other, vec![forged_actual]))
                .unwrap();
            solver.operator_binary(op, left, forged).unwrap();
            let error = solver.solve_operators().unwrap_err();
            assert!(error.contains("E0308"), "{error}");
        }
        assert!(checker.exprs.is_empty());
        assert!(checker.locals.is_empty());
    }
}

#[test]
fn nominal_comparison_missing_metadata_preserves_old_lookup_frontier() {
    // A reduced source probe has no prelude operator trait capability. An
    // optional inference equation must not require it before the original
    // receiver graph read is available, nor suppress the required diagnostic.
    let program = parse("struct W<T>{a:T}fn main(){}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let plan = signatures.plan().unwrap();
    let nominal = plan.nominal_declaration(0).unwrap();
    let function = &program.functions[0];
    let checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    assert!(
        checker
            .available_operator_trait("cmp", "PartialEq", "eq")
            .unwrap()
            .is_none()
    );
    assert!(
        checker
            .operator_trait("cmp", "PartialEq", "eq")
            .unwrap_err()
            .contains("unsupported: operator trait")
    );
    let absent_schema = |_| Ok(None);
    let missing = |_: &Demand| Ok(None);
    let inputs = Inputs::new(&absent_schema, &primitive_source, &missing);
    let mut solver = solve_body(&checker, &function.body, None, Some(&inputs)).unwrap();
    let known = solver.fixed(ir::Ty::Int(ir::IntTy::U8)).unwrap();
    let unknown = solver.fresh().unwrap();
    let left = solver.node(Domain::Nominal(nominal, vec![known])).unwrap();
    let right = solver
        .node(Domain::Nominal(nominal, vec![unknown]))
        .unwrap();
    solver.operator_binary(BinaryOp::Eq, left, right).unwrap();
    assert!(!solver.solve_operators().unwrap());
    assert!(matches!(solver.domain(unknown), Domain::Unknown));
    assert_eq!(solver.pending_nominals().unwrap().len(), 1);
    assert!(checker.exprs.is_empty());
    assert!(checker.locals.is_empty());
}

#[test]
fn nominal_comparison_constructor_receiver_hint_retains_source_equation() {
    for (text, expected) in [
        (
            "#[derive(PartialEq)]struct W<T>{a:T}fn main(){let x:W<u8>=W{a:1};let same=W{a:1}==x;}",
            vec![ir::IntTy::U8],
        ),
        (
            "struct L<T>{a:T}struct R<T>{a:T}impl<T:PartialEq> PartialEq<R<T>> for L<T>{fn eq(&self,r:&R<T>)->bool{self.a==r.a}}fn main(){let x:L<u8>=L{a:7};let same=x==R{a:7};}",
            vec![ir::IntTy::U8],
        ),
        (
            "#[derive(PartialEq,PartialOrd)]struct W<T>{a:T}#[derive(PartialEq)]enum G<T>{N,S(T)}fn main(){let x:W<u8>=W{a:1};let g:G<u16>=G::S(3);println!(\"{} {} {} {}\",W{a:1}==x,W{a:2}>x,G::S(3)==g,G::N==g);}",
            vec![ir::IntTy::U8, ir::IntTy::U8, ir::IntTy::U16, ir::IntTy::U16],
        ),
    ] {
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "nominal-comparison-receiver.rs",
            text,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let index = program
            .functions
            .iter()
            .position(|function| {
                function
                    .name
                    .segments()
                    .last()
                    .is_some_and(|name| program.names.spelling(*name).unwrap() == "main")
            })
            .unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let function = plan.callable_body(&program, declaration).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        checker
            .install_lexical_scope(function.lexical_scope)
            .unwrap();
        checker.declaration = Some(declaration);
        let schema = |owner| {
            let (env, template) = plan.nominal_schema(owner, &plan.budget)?;
            let shapes = |fields: &ir::NominalFields| -> Result<Arc<[Shape]>, String> {
                Ok(fields
                    .read(&plan.budget)?
                    .iter()
                    .map(|field| Shape::Leaf(field.ty.clone()))
                    .collect::<Vec<_>>()
                    .into())
            };
            let fields = match template {
                ir::Ty::Struct(value) => DeclarationFields::Struct(shapes(&value.def().fields)?),
                ir::Ty::Enum(value) => DeclarationFields::Enum(
                    value
                        .def()
                        .variants
                        .iter()
                        .map(|variant| shapes(&variant.fields))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                ),
                _ => return Err("test source recipe must be nominal".into()),
            };
            Ok(Some(DeclarationRecipe { env, fields }))
        };
        let source = |ty: &Type| plan.nominal_source_shape(declaration, ty);
        let missing = |_: &Demand| Ok(None);
        let inputs = Inputs::new(&schema, &source, &missing);
        let solver = solve_callable(&checker, declaration, None, &inputs).unwrap();
        assert_eq!(solver.receiver_sites.len(), expected.len());
        for ((_, receiver), integer) in solver.receiver_sites.iter().zip(expected) {
            let Domain::Nominal(_, arguments) = solver.domain(*receiver) else {
                panic!("constructor hint must retain its canonical nominal owner");
            };
            assert_eq!(arguments.len(), 1);
            assert!(
                matches!(solver.domain(arguments[0]), Domain::Fixed(ir::Ty::Int(actual)) if actual == integer)
            );
            assert!(solver.resolve(*receiver, 0).unwrap().is_none());
        }
        assert!(!solver.pending_nominals().unwrap().is_empty());
        assert!(checker.exprs.is_empty());
        assert!(checker.locals.is_empty());
    }
}

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
        &mut tr::TraitScratch::default(),
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
            &mut tr::TraitScratch::default(),
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
fn constructor_closure_equations_preserve_source_and_missing_graph_authority() {
    const SOURCE: &str = "struct Wrap<T>{value:T}impl<T> Wrap<T>{fn apply<U,F:FnOnce(T)->U>(self,f:F)->U{f(self.value)}}fn probe(){let outer=(7u8,);let wrapped=Wrap{value:&outer};let result=wrapped.apply(|node|node.0);let expected:u8=result;}fn main(){}";
    const FRONTIER: &str = "struct Wrap<T>{value:T}impl<T:PartialEq> PartialEq for Wrap<T>{fn eq(&self,other:&Self)->bool{self.value==other.value}}impl<T> Wrap<T>{fn apply<U,F:FnOnce(T)->U>(self,f:F)->Wrap<U>{Wrap{value:f(self.value)}}}fn probe(){let outer=(7u8,);let wrapped=Wrap{value:&outer};let result=wrapped.apply(|node|node.0);let expected=result==Wrap{value:7u8};}fn main(){}";
    for source in [
        SOURCE.to_owned(),
        SOURCE.replace(
            "let result=wrapped.apply(|node|node.0);",
            "let f=|node|node.0;let result=7u8;",
        ),
        SOURCE.replace("|node|", "|node: &(u16,)|"),
        SOURCE.replace("let expected:u8=result;", "let expected:bool=result;"),
        FRONTIER.to_owned(),
    ] {
        let stock = crate::compiler::frontend::parser_tests::stock_compile(
            &source,
            std::path::Path::new("/dev/null"),
            true,
        );
        assert_eq!(
            stock.status.success(),
            source == SOURCE || source == FRONTIER,
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "constructor-closure-equations.rs",
            &source,
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let signatures = signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let index = program
            .functions
            .iter()
            .position(|function| {
                function
                    .name
                    .segments()
                    .last()
                    .is_some_and(|name| program.names.spelling(*name).unwrap() == "probe")
            })
            .unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let function = plan.callable_body(&program, declaration).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        checker
            .install_lexical_scope(function.lexical_scope)
            .unwrap();
        checker.declaration = Some(declaration);
        let declaration_schema = |declaration| {
            let (env, _) = plan.nominal_schema(declaration, &plan.budget)?;
            let parameter = ir::ParamId {
                owner: env.binder(),
                index: 0,
            };
            Ok(Some(DeclarationRecipe {
                env,
                fields: DeclarationFields::Struct(
                    vec![Shape::Leaf(ir::Ty::Param(parameter))].into(),
                ),
            }))
        };
        let source_type = |ty: &Type| plan.nominal_source_shape(declaration, ty);
        let missing = |_: &Demand| Ok(None);
        let inputs = Inputs::new(&declaration_schema, &source_type, &missing);
        if source == FRONTIER {
            let empty = Block {
                statements: Vec::new(),
                tail: None,
            };
            let mut solver = solve_body(&checker, &empty, None, Some(&inputs)).unwrap();
            solver.block(&function.body, 0).unwrap();
            let methods = std::mem::take(&mut solver.methods);
            let projections = std::mem::take(&mut solver.projections);
            assert_eq!(solver.coercions.len(), 1);
            let (actual, expected) = solver.coercions[0];
            assert!(matches!(solver.domain(expected), Domain::Unknown));
            assert!(
                solver
                    .local_lazy_inputs_ready(&methods, &projections)
                    .unwrap()
            );
            // The readiness closure describes dependencies, never equations.
            assert!(matches!(solver.domain(expected), Domain::Unknown));
            // Split the same deferred dependency into a reverse-ordered chain.
            // A bounded fixed point must not depend on queue insertion order.
            let intermediate = solver.fresh().unwrap();
            solver.coercions = vec![(intermediate, expected), (actual, intermediate)];
            assert!(
                solver
                    .local_lazy_inputs_ready(&methods, &projections)
                    .unwrap()
            );
            assert!(matches!(solver.domain(intermediate), Domain::Unknown));
            assert!(matches!(solver.domain(expected), Domain::Unknown));
            // Even the same tail dependency cannot authorize an unrelated
            // closure parameter or an independently delayed producer result.
            solver.delayed_results.push(intermediate);
            assert!(
                !solver
                    .local_lazy_inputs_ready(&methods, &projections)
                    .unwrap()
            );
            solver.delayed_results.pop();
            solver.constructor_coercions.clear();
            assert!(
                !solver
                    .local_lazy_inputs_ready(&methods, &projections)
                    .unwrap()
            );
            assert!(checker.exprs.is_empty());
            assert!(checker.locals.is_empty());
            continue;
        }
        let solved = solve_body(&checker, &function.body, None, Some(&inputs));
        if source.contains("node: &(u16,)") || source.contains("let expected:bool=result;") {
            assert!(
                solved.is_err(),
                "a source equation must not erase a written closure/output type"
            );
            continue;
        }
        let solver = solved.unwrap();
        assert_eq!(solver.constructor_coercions.len(), 1);
        if source != SOURCE {
            let error = solver.closure_hints().unwrap_err();
            assert!(error.contains("E0282"), "{error}");
            continue;
        }
        let hints = solver.closure_hints().unwrap();
        let signature = hints.values().next().unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(signature.result, ir::Ty::Int(ir::IntTy::U8));
        assert_eq!(
            signature.params,
            vec![ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::U8)].into())),
            }]
        );
        // A solved source equation is still not permission to materialize its
        // nominal owner. Replaying the same source with the same absent read
        // must demand the same exact application and emit no executable IR.
        for _ in 0..2 {
            let PreparedHints::Blocked(Preparation::Blocked { nominals, .. }) =
                finalize_callable(&checker, declaration, None, &inputs).unwrap()
            else {
                panic!("constructor inference cannot manufacture a graph realization");
            };
            assert_eq!(nominals.len(), 1);
            assert!(
                matches!(nominals[0].arguments.as_ref(), [Shape::Ref(ir::BorrowKind::Shared, inner)]
                if matches!(inner.as_ref(), Shape::Tuple(fields)
                    if matches!(fields.as_ref(), [Shape::Leaf(ir::Ty::Int(ir::IntTy::U8))])))
            );
        }
        assert!(checker.exprs.is_empty());
        assert!(checker.locals.is_empty());
    }
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
            &mut tr::TraitScratch::default(),
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
    let mut retry_scratch = tr::TraitScratch::default();
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
                &mut retry_scratch,
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
