use super::*;

#[test]
fn opaque_pack_rejects_foreign_identity_and_changed_hidden_operand() {
    let snapshot = Arc::new(Snapshot {
        input: Input::Text("trait Marker {} struct Number { value:i32 } impl Marker for Number {} fn make()->impl Marker { Number { value:42 } } fn other(){} fn main(){}".into()),
        source_options: source::SourceOptions::default(),
        budget: ir::PlanBudget::default(),
    });
    let budget = snapshot.budget.clone();
    let engine = Engine::new(CompilerProvider(snapshot));
    let body = engine.request(crate::compiler::keys::item(TEMPLATE_BODY, 0));
    engine.run(1).unwrap();
    body.completion().unwrap();
    let template = engine
        .fact_of::<TemplateBodyFact>(&crate::compiler::keys::item(TEMPLATE_BODY, 0), 0)
        .unwrap();
    let types = engine
        .fact_of::<frontend::PlanCtx>(&crate::compiler::keys::SignaturesDebtRoot.key(), 0)
        .unwrap();
    let publication = template.publication.as_ref().unwrap();
    let facts = ir::FactsCtx::Validated(publication.facts());
    let (occurrence, opaque, hidden) = template
        .checked
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expression)| {
            if let ir::ExprKind::OpaqueWrap { opaque, value } = &expression.kind {
                Some((
                    ir::ExprId(index),
                    opaque.clone(),
                    template.checked.exprs[value.0].ty.clone(),
                ))
            } else {
                None
            }
        })
        .unwrap();
    let proof = ir::OpaquePackProof::seal(publication.clone(), occurrence, &budget).unwrap();
    let record = ir::OpaqueHiddenType::new(opaque.clone(), hidden.clone());
    ir::OpaqueDefinitionProof::seal(publication.clone(), &record, &budget).unwrap();
    proof.validate_shape(&hidden, facts, &budget).unwrap();
    assert!(proof.validate_shape(&ir::Ty::Bool, facts, &budget).is_err());
    let other = types
        .callable_facts(types.callable_decl(1).unwrap())
        .unwrap();
    assert!(
        proof
            .validate_shape(&hidden, ir::FactsCtx::Validated(&other), &budget)
            .is_err()
    );
    for changed in [
        ir::OpaqueTy::new(
            types.callable_decl(1).unwrap(),
            opaque.owner_args().into(),
            opaque.site(),
            opaque.bounds().into(),
            opaque.captured_regions().into(),
        ),
        ir::OpaqueTy::new(
            opaque.owner(),
            opaque.owner_args().into(),
            opaque.site() + 1,
            opaque.bounds().into(),
            opaque.captured_regions().into(),
        ),
        ir::OpaqueTy::new(
            opaque.owner(),
            Arc::from([ir::Ty::Bool]),
            opaque.site(),
            opaque.bounds().into(),
            opaque.captured_regions().into(),
        ),
        ir::OpaqueTy::new(
            opaque.owner(),
            opaque.owner_args().into(),
            opaque.site(),
            Arc::from([]),
            opaque.captured_regions().into(),
        ),
        ir::OpaqueTy::new(
            opaque.owner(),
            opaque.owner_args().into(),
            opaque.site(),
            opaque.bounds().into(),
            Arc::from([ir::SourceRegionId {
                declaration: opaque.owner(),
                index: 9,
            }]),
        ),
    ] {
        assert!(
            ir::OpaqueDefinitionProof::seal(
                publication.clone(),
                &ir::OpaqueHiddenType::new(Arc::new(changed), hidden.clone()),
                &budget
            )
            .is_err()
        );
    }
}

#[test]
fn opaque_pack_does_not_erase_reference_carriers() {
    let snapshot = Arc::new(Snapshot {
        input: Input::Text(
            "fn make<'a>(value: &'a i32) -> impl Sized { value } fn main() {}".into(),
        ),
        source_options: source::SourceOptions::default(),
        budget: ir::PlanBudget::default(),
    });
    let budget = snapshot.budget.clone();
    let engine = Engine::new(CompilerProvider(snapshot));
    let body = engine.request(crate::compiler::keys::item(TEMPLATE_BODY, 0));
    engine.run(1).unwrap();
    body.completion().unwrap();
    let template = engine
        .fact_of::<TemplateBodyFact>(&crate::compiler::keys::item(TEMPLATE_BODY, 0), 0)
        .unwrap();
    let publication = template.publication.as_ref().unwrap();
    let (occurrence, opaque, hidden) = template
        .checked
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expression)| {
            if let ir::ExprKind::OpaqueWrap { opaque, value } = &expression.kind {
                Some((
                    ir::ExprId(index),
                    opaque.clone(),
                    template.checked.exprs[value.0].ty.clone(),
                ))
            } else {
                None
            }
        })
        .unwrap();
    let _ = (opaque, hidden);
    let error = ir::OpaquePackProof::seal(publication.clone(), occurrence, &budget).unwrap_err();
    assert!(
        error.contains("explicit captured carrier mapping"),
        "unexpected rejection: {error}"
    );
}

#[test]
fn opaque_representation_requires_exact_body_and_snapshot_witness() {
    struct Probe(CompilerProvider);
    impl Provider for Probe {
        fn outputs(&self, key: &Key) -> usize {
            self.0.outputs(key)
        }
        fn start(&self, cx: Context, key: Key) -> TaskFuture {
            if key.kind != 9000 {
                return self.0.start(cx, key);
            }
            let budget = self.0.0.budget.clone();
            Box::pin(async move {
                let hidden: Arc<OpaqueHiddenTypesFact> = cx
                    .read_fact(
                        cx.request(crate::compiler::keys::item(TEMPLATE_BODY, 0))
                            .get(2),
                    )
                    .await?;
                let params = instances::graph_key::encode_opaque(
                    hidden.representations[0].opaque(),
                    &budget,
                )
                .map_err(Error::new)?;
                let _: Arc<OpaqueRepresentationFact> = cx
                    .read_fact(
                        cx.request(crate::compiler::keys::words(OPAQUE_REPRESENTATION, params)?)
                            .get(0),
                    )
                    .await?;
                cx.publish_fact(0, ())
            })
        }
    }
    fn fixture(budget: &ir::PlanBudget) -> (Engine, Arc<frontend::PlanCtx>) {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("trait Marker {} struct Number { value:i32 } struct Other { value:i32 } impl Marker for Number {} impl Marker for Other {} fn make()->impl Marker { Number {value:42} } fn other()->impl Marker { Other {value:7} } fn same()->impl Marker { Number {value:9} } fn plain()->Number { Number {value:1} } fn main(){}".into()),
            source_options: source::SourceOptions::default(), budget: budget.clone(),
        });
        let engine = Engine::new(Probe(CompilerProvider(snapshot)));
        let probe = engine.request(crate::compiler::keys::params(9000, 0, vec![]));
        let requests: Vec<_> = (0..4)
            .map(|index| engine.request(crate::compiler::keys::item(TEMPLATE_BODY, index)))
            .collect();
        engine.run(1).unwrap();
        probe.completion().unwrap();
        for request in requests {
            request.completion().unwrap();
        }
        let types = engine
            .fact_of::<frontend::PlanCtx>(&crate::compiler::keys::SignaturesDebtRoot.key(), 0)
            .unwrap();
        (engine, types)
    }
    let budget = ir::PlanBudget::default();
    let (engine, types) = fixture(&budget);
    let templates: Vec<_> = (0..4)
        .map(|index| {
            engine
                .fact_of::<TemplateBodyFact>(&crate::compiler::keys::item(TEMPLATE_BODY, index), 0)
                .unwrap()
        })
        .collect();
    let publication = templates[0].publication.as_ref().unwrap();
    let records = engine
        .fact_of::<OpaqueHiddenTypesFact>(&crate::compiler::keys::item(TEMPLATE_BODY, 0), 2)
        .unwrap();
    let record = &records.representations[0];
    let proof = ir::OpaqueDefinitionProof::seal(publication.clone(), record, &budget).unwrap();
    let mut context = types.as_ref().clone();
    let closed = proof
        .instantiate(record.opaque(), publication, &mut context, &budget)
        .unwrap();
    assert!(ir::identical_type(closed.hidden(), record.hidden(), &budget).unwrap());
    // The replacement independently satisfies Marker. Bounds alone are insufficient.
    let other_records = engine
        .fact_of::<OpaqueHiddenTypesFact>(&crate::compiler::keys::item(TEMPLATE_BODY, 1), 2)
        .unwrap();
    let replacement = ir::OpaqueHiddenType::new(
        record.opaque().clone(),
        other_records.representations[0].hidden().clone(),
    );
    assert!(ir::OpaqueDefinitionProof::seal(publication.clone(), &replacement, &budget).is_err());
    // Same concrete hidden type in a different defining callable is not evidence.
    assert!(
        ir::OpaqueDefinitionProof::seal(
            templates[2].publication.as_ref().unwrap().clone(),
            record,
            &budget
        )
        .is_err()
    );
    assert!(
        ir::OpaqueDefinitionProof::seal(
            templates[3].publication.as_ref().unwrap().clone(),
            record,
            &budget
        )
        .is_err()
    );
    let occurrence = publication
        .source()
        .exprs
        .iter()
        .position(|expr| matches!(expr.kind, ir::ExprKind::OpaqueWrap { .. }))
        .unwrap();
    let pack =
        ir::OpaquePackProof::seal(publication.clone(), ir::ExprId(occurrence), &budget).unwrap();
    pack.validate_occurrence(publication, Some(ir::ExprId(occurrence)), &budget)
        .unwrap();
    assert!(
        pack.validate_occurrence(publication, None, &budget)
            .is_err()
    );
    assert!(
        pack.validate_occurrence(
            templates[2].publication.as_ref().unwrap(),
            Some(ir::ExprId(occurrence)),
            &budget
        )
        .is_err()
    );
    let lowered = cfg::lower_source_template(publication, &budget).unwrap();
    let mut changed = lowered.function().as_ref().clone();
    let statement = changed
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.value,
                cfg::Statement::Assign {
                    value: cfg::Rvalue::OpaquePack { .. },
                    ..
                }
            )
        })
        .unwrap();
    statement.origin = None;
    assert!(
        changed
            .validate_source_publication(publication, &budget)
            .is_err()
    );
    let mut unsealed = lowered.function().as_ref().clone();
    unsealed.lowering_proof = None;
    assert!(
        unsealed
            .validate_provenance(publication.facts(), &budget)
            .is_err()
    );
    let operand = match publication.source().exprs[occurrence].kind {
        ir::ExprKind::OpaqueWrap { value, .. } => value,
        _ => unreachable!(),
    };
    assert!(ir::OpaquePackProof::seal(publication.clone(), operand, &budget).is_err());
    assert!(
        ir::OpaquePackProof::seal(publication.clone(), ir::ExprId(usize::MAX), &budget).is_err()
    );
    let (foreign_engine, foreign_types) = fixture(&budget);
    let foreign_template = foreign_engine
        .fact_of::<TemplateBodyFact>(&crate::compiler::keys::item(TEMPLATE_BODY, 0), 0)
        .unwrap();
    let foreign_publication = foreign_template.publication.as_ref().unwrap();
    assert!(
        pack.validate_occurrence(foreign_publication, Some(ir::ExprId(occurrence)), &budget)
            .is_err()
    );
    assert!(
        proof
            .instantiate(record.opaque(), foreign_publication, &mut context, &budget)
            .is_err()
    );
    let frontend::PublishedCallableBody::Source(republished) = types
        .publish_callable_body(record.opaque().owner(), &budget)
        .unwrap()
    else {
        panic!("Source publication required")
    };
    assert_ne!(publication.identity(), republished.identity());
    assert!(
        pack.validate_occurrence(&republished, Some(ir::ExprId(occurrence)), &budget)
            .is_err()
    );
    assert!(
        proof
            .instantiate(record.opaque(), &republished, &mut context, &budget)
            .is_err()
    );
    assert!(
        proof
            .instantiate(
                record.opaque(),
                publication,
                &mut foreign_types.as_ref().clone(),
                &budget
            )
            .is_err()
    );
    let foreign_facts = foreign_types
        .callable_facts(foreign_types.callable_decl(0).unwrap())
        .unwrap();
    assert!(
        pack.validate_shape(
            record.hidden(),
            ir::FactsCtx::Validated(&foreign_facts),
            &budget
        )
        .is_err()
    );
    // The actual producer consumes both template outputs and publishes this same record.
    let key = crate::compiler::keys::words(
        OPAQUE_REPRESENTATION,
        instances::graph_key::encode_opaque(record.opaque(), &budget).unwrap(),
    )
    .unwrap();
    let actual = engine.fact_of::<OpaqueRepresentationFact>(&key, 0).unwrap();
    assert!(ir::identical_type(actual.record.hidden(), record.hidden(), &budget).unwrap());
    let replayed = actual
        .definition
        .instantiate(record.opaque(), publication, &mut context, &budget)
        .unwrap();
    assert!(ir::identical_type(replayed.hidden(), actual.record.hidden(), &budget).unwrap());
    assert!(
        actual
            .definition
            .instantiate(record.opaque(), foreign_publication, &mut context, &budget)
            .is_err()
    );
    assert!(
        actual
            .definition
            .instantiate(record.opaque(), &republished, &mut context, &budget)
            .is_err()
    );
}
