//! Mint capture result recipes only from original checked occurrences and loans.
use super::super::capture_rewrite::CaptureProjection;
use super::*;

impl Checker<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler::frontend::closures) fn captured_closure_result(
        &self,
        owner: ir::DeclId,
        provider: ir::DeclId,
        site: u32,
        body: &ir::TypedBlock,
        result: &ir::Ty,
        projections: &[CaptureProjection],
        sources: &[ir::CaptureSource],
        prefix: &[ir::TypedStmt],
        tuple: ir::LocalId,
    ) -> Result<Option<Arc<ir::ClosureResultReceipt>>, String> {
        let plan = self.signatures.plan()?;
        let locations = ir::carrier_locations(result, &plan.budget)?;
        // A rigid symbolic result keeps its existing required-call witness.
        // An incidental captured callable receiver is not capture provenance
        // for that unrelated equality-dependent result family.
        if locations.is_empty() || matches!(result, ir::Ty::Param(_)) {
            return Ok(None);
        }
        let Some(tail) = body.tail else {
            return Ok(None);
        };
        // This probe chooses the issuer, not its authority. Every result channel
        // is subsequently traced through checked canonical return contracts.
        let argument_flow = matches!(
            self.exprs.get(tail.0).map(|expr| &expr.kind),
            Some(ir::ExprKind::Call { .. } | ir::ExprKind::Tuple(_))
        );
        let mut pending = vec![tail];
        let mut seen = std::collections::HashSet::new();
        let mut captured = false;
        while let Some(id) = pending.pop() {
            plan.budget.charge_work(1)?;
            if !seen.insert(id) {
                continue;
            }
            if argument_flow {
                captured |= input_value_result::value_origin(
                    &self.exprs,
                    &self.locals,
                    prefix,
                    tuple,
                    id,
                    &plan.budget,
                )?
                .is_some();
            }
            match &self
                .exprs
                .get(id.0)
                .ok_or("invalid capture result source")?
                .kind
            {
                ir::ExprKind::Borrow { place, .. } => {
                    // Issuer selection only. The returned canonical call row
                    // must actually reach this borrow before it is validated.
                    captured |= argument_flow && argument_local(prefix, place.local);
                    captured |= projections.iter().any(|capture| {
                        capture.local == place.local
                            && place.projection.starts_with(&capture.prefix)
                    });
                }
                ir::ExprKind::Call { args, .. } | ir::ExprKind::Tuple(args) => {
                    pending.extend(args.iter().copied())
                }
                ir::ExprKind::ArrayToSlice { reference } => pending.push(*reference),
                _ => {
                    if let Some(place) =
                        super::super::capture_use::expression_place(&self.exprs, id)?
                    {
                        captured |= projections.iter().any(|capture| {
                            capture.local == place.local
                                && place.projection.starts_with(&capture.prefix)
                        });
                    }
                }
            }
        }
        if !captured {
            return Ok(None);
        }
        if !body.statements.is_empty() {
            return Err(
                "unsupported: captured closure result requires straight-line source flow".into(),
            );
        }
        let facts = self
            .facts()
            .validated()
            .ok_or("captured closure result requires canonical caller facts")?;
        let mut nodes = Vec::new();
        let mut active = std::collections::HashSet::new();
        for location in locations.iter() {
            trace(
                self,
                tail,
                location,
                projections,
                sources,
                facts,
                &mut nodes,
                &mut active,
                prefix,
                tuple,
                0,
            )?;
        }
        let recipe = plan
            .registry
            .closure_recipes
            .get(&provider)
            .ok_or("capture result lacks original source recipe")?
            .clone();
        Ok(Some(Arc::new(ir::ClosureResultReceipt::captured(
            owner,
            provider,
            site,
            tail,
            ir::CaptureResultWitness {
                recipe,
                nodes: nodes.into(),
            },
            &plan.budget,
        )?)))
    }
}

#[allow(clippy::too_many_arguments)]
fn trace(
    checker: &Checker<'_>,
    expression: ir::ExprId,
    location: &ir::CarrierLocation,
    projections: &[CaptureProjection],
    sources: &[ir::CaptureSource],
    facts: &tr::ValidatedFacts,
    nodes: &mut Vec<ir::CaptureResultNode>,
    active: &mut std::collections::HashSet<(ir::ExprId, ir::CarrierLocation)>,
    prefix: &[ir::TypedStmt],
    tuple: ir::LocalId,
    depth: usize,
) -> Result<(), String> {
    let budget = facts.budget();
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if !active.insert((expression, location.clone())) {
        return Err("cyclic captured closure result flow".into());
    }
    let expr = checker
        .exprs
        .get(expression.0)
        .ok_or("invalid captured closure result expression")?;
    let input = input_value_result::value_origin(
        &checker.exprs,
        &checker.locals,
        prefix,
        tuple,
        expression,
        budget,
    )?;
    let kind = if let Some(origin) = input {
        if !facts
            .must_copy(&expr.ty)
            .map_err(|error| error.into_legacy_diagnostic(budget))?
        {
            return Err(
                "unsupported: closure result call input requires an original Copy carrier value"
                    .into(),
            );
        }
        let Some(ir::Ty::Tuple(parameters)) = checker.locals.get(tuple.0).map(|local| &local.ty)
        else {
            return Err("closure result input lost its canonical argument tuple".into());
        };
        if !origin
            .value_carriers(parameters, budget)?
            .iter()
            .any(|row| row.location == *location)
        {
            return Err("closure result input names a foreign argument carrier".into());
        }
        ir::CaptureResultKind::InputValue(origin)
    } else {
        match &expr.kind {
            ir::ExprKind::ArrayToSlice { reference } => {
                if !location.path.0.is_empty()
                    || location.channel
                        != ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
                {
                    return Err(
                        "unsupported: captured array-to-slice result contains nested carriers"
                            .into(),
                    );
                }
                let source = checker
                    .exprs
                    .get(reference.0)
                    .ok_or("captured array-to-slice lost original reference")?;
                let Some((ir::BorrowKind::Shared, from, _)) = source.ty.array_ref_element_len()
                else {
                    return Err(
                        "capture result unsizing requires original shared array reference".into(),
                    );
                };
                let Some((ir::BorrowKind::Shared, to)) = expr.ty.slice_ref_element() else {
                    return Err("capture result unsizing changed shared slice result".into());
                };
                if from != to {
                    return Err("capture result unsizing changed canonical array element".into());
                }
                ir::validate_shared_slice_element(from, ir::FactsCtx::Validated(facts), budget)?;
                trace(
                    checker,
                    *reference,
                    location,
                    projections,
                    sources,
                    facts,
                    nodes,
                    active,
                    prefix,
                    tuple,
                    depth + 1,
                )?;
                ir::CaptureResultKind::ArrayToSlice {
                    reference: *reference,
                }
            }
            ir::ExprKind::Call {
                target,
                args,
                signature,
                ..
            } => {
                if !checker
                    .call_evidence
                    .iter()
                    .any(|evidence| evidence.call == expression)
                {
                    return Err(
                        "captured closure result lacks original call occurrence evidence".into(),
                    );
                }
                facts
                    .validate_signature(ir::SignatureSubject::Call(target), signature)
                    .map_err(|e| {
                        format!(
                            "captured result original call signature: {}",
                            e.into_legacy_diagnostic(budget)
                        )
                    })?;
                let returned = facts
                    .capture_call_result_sources(target, &expr.ty, location)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?;
                if returned.is_empty() {
                    return Err(
                        "captured closure result has no checked input loan provenance".into(),
                    );
                }
                for source in returned.iter() {
                    let argument = *args
                        .get(source.param)
                        .ok_or("capture result return contract names foreign argument")?;
                    trace(
                        checker,
                        argument,
                        &source.location,
                        projections,
                        sources,
                        facts,
                        nodes,
                        active,
                        prefix,
                        tuple,
                        depth + 1,
                    )?;
                }
                budget.retain_bytes(args.len() * std::mem::size_of::<ir::ExprId>())?;
                ir::CaptureResultKind::Call {
                    target: target.clone(),
                    arguments: args.clone().into(),
                    source_ty: expr.ty.clone(),
                }
            }
            ir::ExprKind::Tuple(fields) => {
                let Some((ir::CarrierStep::Field(field), rest)) = location.path.0.split_first()
                else {
                    return Err("capture result tuple has foreign carrier path".into());
                };
                let value = *fields
                    .get(*field)
                    .ok_or("capture result tuple field is invalid")?;
                let inner = ir::CarrierLocation {
                    path: ir::CarrierPath(rest.into()),
                    channel: location.channel.clone(),
                };
                trace(
                    checker,
                    value,
                    &inner,
                    projections,
                    sources,
                    facts,
                    nodes,
                    active,
                    prefix,
                    tuple,
                    depth + 1,
                )?;
                budget.retain_bytes(fields.len() * std::mem::size_of::<ir::ExprId>())?;
                ir::CaptureResultKind::Tuple {
                    fields: fields.clone().into(),
                    source_ty: expr.ty.clone(),
                }
            }
            ir::ExprKind::Borrow {
                kind: ir::BorrowKind::Shared,
                place,
                timing: ir::BorrowTiming::Immediate,
            } => {
                if !ir::carrier_locations(&expr.ty, budget)?.contains(location) {
                    return Err("capture result borrow names a foreign source carrier".into());
                }
                if argument_local(prefix, place.local) {
                    let (declaration, borrow, original, argument, depth) =
                        result_contract::argument_borrow_origin(
                            &checker.exprs,
                            &checker.locals,
                            prefix,
                            &[],
                            tuple,
                            expression,
                            facts,
                        )?;
                    if declaration.is_some() || borrow != expression {
                        return Err("aggregate argument borrow changed source occurrence".into());
                    }
                    let origin = ir::ClosureArgumentOrigin {
                        argument,
                        tuple,
                        borrow,
                        local: original.local,
                        place: original,
                        depth,
                    };
                    let Some(ir::Ty::Tuple(parameters)) =
                        checker.locals.get(tuple.0).map(|local| &local.ty)
                    else {
                        return Err("aggregate argument borrow lost original tuple".into());
                    };
                    if checker.locals.get(origin.local.0).map(|local| &local.ty)
                        != parameters.get(origin.argument)
                        || origin.projected_result(parameters, budget)? != expr.ty
                    {
                        return Err("aggregate argument borrow changed projected type".into());
                    }
                    budget.charge_work(nodes.len())?;
                    if !nodes.iter().any(|node| node.expression == expression) {
                        budget.retain_bytes(std::mem::size_of::<ir::CaptureResultNode>())?;
                        nodes.push(ir::CaptureResultNode {
                            expression,
                            kind: ir::CaptureResultKind::ArgumentBorrow(origin),
                        });
                    }
                    active.remove(&(expression, location.clone()));
                    return Ok(());
                }
                let capture = projections
                    .iter()
                    .find(|capture| {
                        capture.local == place.local
                            && place.projection.starts_with(&capture.prefix)
                    })
                    .ok_or("closure result borrows local-owned or foreign storage")?;
                let source = sources
                    .get(capture.field)
                    .ok_or("capture result names a foreign environment slot")?;
                if source.kind == ir::CaptureKind::Value
                    && !place.projection[capture.prefix.len()..]
                        .iter()
                        .any(|step| matches!(step, ir::Projection::Deref))
                {
                    return Err(
                        "closure reference result borrows local-owned captured storage".into(),
                    );
                }
                if !capture.indirect || source.kind != ir::CaptureKind::Shared {
                    return Err(
                        "unsupported: closure result requires an original shared capture loan"
                            .into(),
                    );
                }
                budget.retain_bytes(
                    (place.projection.len() + capture.prefix.len() + source.place.projection.len())
                        * std::mem::size_of::<ir::Projection>(),
                )?;
                ir::CaptureResultKind::Borrow {
                    field: capture.field,
                    source: source.clone(),
                    original: place.clone(),
                    prefix: capture.prefix.clone().into(),
                }
            }
            _ => {
                let place =
                    super::super::capture_use::expression_place(&checker.exprs, expression)?
                        .ok_or(
                            "unsupported: captured closure result has no original source place",
                        )?;
                let capture = projections
                    .iter()
                    .find(|capture| {
                        capture.local == place.local
                            && place.projection.starts_with(&capture.prefix)
                    })
                    .ok_or("closure result reads local-owned or foreign storage")?;
                let source = sources
                    .get(capture.field)
                    .ok_or("capture read names foreign environment field")?;
                if !capture.indirect || source.kind != ir::CaptureKind::Shared {
                    return Err(
                    "unsupported: closure reference results require checked capture carrier contracts: result read requires original shared capture capability"
                        .into(),
                );
                }
                if !facts
                    .must_copy(&expr.ty)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
                    || !ir::carrier_locations(&expr.ty, budget)?.contains(location)
                {
                    return Err(
                        "closure result cannot copy a moved or mutable captured carrier".into(),
                    );
                }
                budget.retain_bytes(
                    (place.projection.len() + capture.prefix.len() + source.place.projection.len())
                        * std::mem::size_of::<ir::Projection>(),
                )?;
                ir::CaptureResultKind::Read {
                    field: capture.field,
                    source: source.clone(),
                    original: place,
                    prefix: capture.prefix.clone().into(),
                }
            }
        }
    };
    budget.charge_work(nodes.len())?;
    if !nodes.iter().any(|node| node.expression == expression) {
        budget.retain_bytes(std::mem::size_of::<ir::CaptureResultNode>())?;
        nodes.push(ir::CaptureResultNode { expression, kind });
    }
    active.remove(&(expression, location.clone()));
    Ok(())
}

fn argument_local(prefix: &[ir::TypedStmt], local: ir::LocalId) -> bool {
    prefix.iter().any(|statement| match statement {
        ir::TypedStmt::Let { local: bound, .. } => *bound == local,
        ir::TypedStmt::LetPattern { bindings, .. } => {
            bindings.iter().any(|binding| binding.local == local)
        }
        _ => false,
    })
}

pub(super) fn validate_source(
    receipt: &ir::ClosureResultReceipt,
    source: &ir::CheckedFunction,
    facts: &tr::ValidatedFacts,
) -> Result<(), String> {
    let budget = facts.budget();
    let (closure, mode) = facts
        .closure_lowering_identity()
        .ok_or("capture result lacks actual closure body authority")?;
    if closure.owner() != receipt.owner()
        || closure.provider() != receipt.provider()
        || closure.site() != receipt.site()
        || closure
            .result_receipt()
            .is_none_or(|original| !original.same_source_shape(receipt))
    {
        return Err("capture result has foreign source owner, provider, site or receipt".into());
    }
    let witness = receipt
        .capture_witness()
        .ok_or("missing capture result witness")?;
    let mut params = Vec::new();
    for parameter in &source.params {
        let local = source
            .locals
            .get(parameter.0)
            .ok_or("capture result lost actual parameter declaration")?;
        budget.retain_bytes(std::mem::size_of::<ir::Ty>())?;
        params.push(local.ty.clone());
    }
    facts
        .validate_signature(
            ir::SignatureSubject::Body,
            &ir::Signature {
                params,
                result: source.result.clone(),
                ref_contract: source.ref_contract.clone(),
            },
        )
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    for statement in &source.body.statements {
        let ir::TypedStmt::Let { value, .. } = statement else {
            return Err("capture result changed original straight-line parameter entry".into());
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Field { base, .. },
            ..
        }) = source.exprs.get(value.0)
        else {
            return Err("capture result changed original parameter projection".into());
        };
        if !matches!(source.exprs.get(base.0).map(|expr| &expr.kind), Some(ir::ExprKind::Local(local)) if *local == source.params[1])
        {
            return Err("capture result parameter entry belongs to a foreign tuple".into());
        }
    }
    for node in witness.nodes.iter() {
        budget.charge_work(1)?;
        let expr = source
            .exprs
            .get(node.expression.0)
            .ok_or("capture result lost original expression occurrence")?;
        match (&node.kind, &expr.kind) {
            (ir::CaptureResultKind::ArgumentBorrow(origin), _) => {
                let (declaration, borrow, place, argument, depth) =
                    result_contract::argument_borrow_origin(
                        &source.exprs,
                        &source.locals,
                        &source.body.statements,
                        &[],
                        source.params[1],
                        node.expression,
                        facts,
                    )?;
                if declaration.is_some()
                    || borrow != node.expression
                    || origin.borrow != borrow
                    || origin.place != place
                    || origin.local != place.local
                    || origin.argument != argument
                    || origin.depth != depth
                    || origin.tuple != source.params[1]
                    || source.locals.get(origin.local.0).map(|local| &local.ty)
                        != closure.params().get(origin.argument)
                    || origin.projected_result(closure.params(), budget)? != expr.ty
                {
                    return Err(
                        "aggregate result changed its original argument borrow occurrence".into(),
                    );
                }
            }
            (ir::CaptureResultKind::InputValue(origin), _) => {
                let replayed = input_value_result::value_origin(
                    &source.exprs,
                    &source.locals,
                    &source.body.statements,
                    source.params[1],
                    node.expression,
                    budget,
                )?;
                if source.params[1] != origin.tuple
                    || replayed.as_ref() != Some(origin)
                    || origin.projected_type(closure.params(), budget)? != expr.ty
                    || !facts
                        .must_copy(&expr.ty)
                        .map_err(|error| error.into_legacy_diagnostic(budget))?
                {
                    return Err(
                        "closure result call changed its original Copy input occurrence".into(),
                    );
                }
            }
            (
                ir::CaptureResultKind::ArrayToSlice { reference },
                ir::ExprKind::ArrayToSlice { reference: actual },
            ) if reference == actual => {}
            (
                ir::CaptureResultKind::Call { arguments, .. },
                ir::ExprKind::Call {
                    target,
                    args,
                    signature,
                    ..
                },
            ) => {
                let expected = facts
                    .capture_result_call(receipt, node.expression)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
                if !facts
                    .capture_result_call_type_matches(receipt, node.expression, &expr.ty)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                    || target != &expected
                    || args.as_slice() != arguments.as_ref()
                    || !source
                        .call_evidence
                        .iter()
                        .any(|trace| trace.call == node.expression)
                {
                    return Err(
                        "capture result changed original call member or argument occurrence".into(),
                    );
                }
                facts
                    .validate_signature(ir::SignatureSubject::Call(target), signature)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
            }
            (ir::CaptureResultKind::Tuple { fields, .. }, ir::ExprKind::Tuple(actual))
                if fields.as_ref() == actual.as_slice() => {}
            (
                ir::CaptureResultKind::Borrow {
                    field,
                    source: original_source,
                    original,
                    prefix,
                },
                ir::ExprKind::Borrow {
                    kind: ir::BorrowKind::Shared,
                    place,
                    timing: ir::BorrowTiming::Immediate,
                },
            ) => {
                if closure.sources().get(*field) != Some(original_source)
                    || original_source.kind != ir::CaptureKind::Shared
                    || !original.projection.starts_with(prefix)
                {
                    return Err(
                        "capture result changed original capture source or loan mode".into(),
                    );
                }
                let mut projection = Vec::new();
                if mode != ir::ClosureKind::FnOnce {
                    projection.push(ir::Projection::Deref);
                }
                projection.push(ir::Projection::Field(*field));
                projection.push(ir::Projection::Deref);
                projection.extend_from_slice(&original.projection[prefix.len()..]);
                if place.local != source.params[0] || place.projection != projection {
                    return Err(
                        "capture result changed original rewritten capture borrow occurrence"
                            .into(),
                    );
                }
            }
            (
                ir::CaptureResultKind::Read {
                    field,
                    source: original_source,
                    original,
                    prefix,
                },
                _,
            ) => {
                if closure.sources().get(*field) != Some(original_source)
                    || original_source.kind != ir::CaptureKind::Shared
                    || !original.projection.starts_with(prefix)
                {
                    return Err("capture read changed original source binding or capability".into());
                }
                let place =
                    super::super::capture_use::expression_place(&source.exprs, node.expression)?
                        .ok_or("capture read lost exact rewritten source place")?;
                let mut projection = Vec::new();
                if mode != ir::ClosureKind::FnOnce {
                    projection.push(ir::Projection::Deref);
                }
                projection.push(ir::Projection::Field(*field));
                projection.push(ir::Projection::Deref);
                projection.extend_from_slice(&original.projection[prefix.len()..]);
                if place.local != source.params[0]
                    || place.projection != projection
                    || !facts
                        .must_copy(&expr.ty)
                        .map_err(|e| e.into_legacy_diagnostic(budget))?
                {
                    return Err(
                        "capture read changed exact original projection, local or copy capability"
                            .into(),
                    );
                }
            }
            _ => return Err("capture result changed original source expression recipe".into()),
        }
    }
    let view = facts
        .carrier_contract(ir::SignatureSubject::Body)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    view.validate_for(facts)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    Ok(())
}
