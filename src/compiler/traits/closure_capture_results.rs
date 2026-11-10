//! Replay captured result flow in the actual caller, world and owner instance.
use super::*;

fn channel_at_prefix(channel: &ir::CarrierChannel, prefix: u8) -> ir::CarrierChannel {
    match channel {
        ir::CarrierChannel::Fixed(_) => {
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: prefix })
        }
        ir::CarrierChannel::ParamTail { parameter, .. } => ir::CarrierChannel::ParamTail {
            parameter: *parameter,
            prefix,
        },
        ir::CarrierChannel::ProjectionTail { projection, .. } => {
            ir::CarrierChannel::ProjectionTail {
                projection: projection.clone(),
                prefix,
            }
        }
        ir::CarrierChannel::ArrayTail { array, .. } => ir::CarrierChannel::ArrayTail {
            array: array.clone(),
            prefix,
        },
        ir::CarrierChannel::OpaqueTail { opaque, .. } => ir::CarrierChannel::OpaqueTail {
            opaque: opaque.clone(),
            prefix,
        },
    }
}

pub(super) fn remove_result_reference(
    location: &ir::CarrierLocation,
    budget: &PlanBudget,
) -> Result<ir::CarrierLocation> {
    if let Some((ir::CarrierStep::Referent, rest)) = location.path.0.split_first() {
        retain::<ir::CarrierStep>(budget, rest.len())?;
        return Ok(ir::CarrierLocation {
            path: ir::CarrierPath(rest.into()),
            channel: location.channel.clone(),
        });
    }
    if location.path.0.is_empty() && location.channel.prefix() != 0 {
        return Ok(ir::CarrierLocation {
            path: location.path.clone(),
            channel: channel_at_prefix(&location.channel, location.channel.prefix() - 1),
        });
    }
    Err(FactsError::invalid(
        budget,
        "captured borrow result lost exact outer reference channel",
    ))
}

pub(super) fn captured_input(
    mode: ir::ClosureKind,
    field: usize,
    projections: &[ir::Projection],
    location: &ir::CarrierLocation,
    budget: &PlanBudget,
) -> Result<ir::InputCarrier> {
    let mut relative = Vec::new();
    for projection in projections {
        let step = match projection {
            ir::Projection::Deref => ir::CarrierStep::Referent,
            ir::Projection::Field(index) => ir::CarrierStep::Field(*index),
            ir::Projection::ConstantIndex(index) => ir::CarrierStep::ConstantIndex(*index),
            ir::Projection::ManuallyDropPayload => ir::CarrierStep::ManuallyDropPayload,
            _ => {
                return Err(FactsError::unsupported(
                    budget,
                    "captured result needs an exact static source projection",
                ));
            }
        };
        push(budget, &mut relative, step)?;
    }
    for step in location.path.0.iter() {
        push(budget, &mut relative, step.clone())?;
    }
    // Consecutive references at the final component are channels, not field
    // paths. In particular reading &[T] from a shared &&[T] capture is depth
    // one. The depth-zero reference merely borrows the reference binding.
    let mut prefix = location.channel.prefix();
    while relative.last() == Some(&ir::CarrierStep::Referent) {
        relative.pop();
        prefix = prefix.checked_add(1).ok_or_else(|| {
            FactsError::invalid(budget, "captured reference channel depth overflow")
        })?;
    }
    let mut path = Vec::new();
    if mode != ir::ClosureKind::FnOnce {
        push(budget, &mut path, ir::CarrierStep::Referent)?;
    }
    push(budget, &mut path, ir::CarrierStep::Field(field))?;
    for step in relative {
        push(budget, &mut path, step)?;
    }
    Ok(ir::InputCarrier {
        param: 0,
        location: ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel: channel_at_prefix(&location.channel, prefix),
        },
    })
}

impl ValidatedFacts {
    /// Match a checked result occurrence to the canonical call contract. Nominal
    /// applications may be source-slot or graph realizations of the same exact
    /// declaration and ordered actuals. Their variant IDs are not interchangeable
    /// until both full types validate and canonical equality has been proved.
    pub(in crate::compiler) fn capture_call_result_sources(
        &self,
        callee: &Callee,
        actual: &Ty,
        location: &ir::CarrierLocation,
    ) -> Result<Arc<[ir::InputCarrier]>> {
        let view = self.carrier_contract(SignatureSubject::Call(callee))?;
        view.validate_for(self)?;
        self.validate_ty_metadata(actual)?;
        if !self.world.eq_ty(actual, &view.result_signature.result, 0)? {
            return Err(FactsError::invalid(
                self.budget(),
                "capture call result changed its canonical type",
            ));
        }
        let canonical =
            self.world
                .rebase_source_location(location, actual, &view.result_signature.result)?;
        for row in view.contract.results.iter() {
            if self.world.same_location(&row.location, &canonical)? {
                return Ok(row.sources.clone());
            }
        }
        Err(FactsError::invalid(
            self.budget(),
            "capture call result has no exact canonical return channel",
        ))
    }

    pub(in crate::compiler) fn capture_result_call(
        &self,
        receipt: &ir::ClosureResultReceipt,
        expression: ir::ExprId,
    ) -> Result<Callee> {
        self.world.check_facts(self, true)?;
        let closure = self.closure.as_ref().ok_or_else(|| {
            FactsError::invalid(
                self.budget(),
                "capture result lacks actual closure body context",
            )
        })?;
        self.world.validate_capture_receipt(closure, receipt)?;
        let node = receipt
            .capture_witness()
            .and_then(|w| w.nodes.iter().find(|node| node.expression == expression))
            .ok_or_else(|| {
                FactsError::invalid(self.budget(), "foreign capture result call occurrence")
            })?;
        let ir::CaptureResultKind::Call { target, .. } = &node.kind else {
            return Err(FactsError::invalid(
                self.budget(),
                "capture result occurrence is not a call",
            ));
        };
        self.world.specialize_capture_target(closure, target, self)
    }

    pub(in crate::compiler) fn capture_result_call_type_matches(
        &self,
        receipt: &ir::ClosureResultReceipt,
        expression: ir::ExprId,
        actual: &Ty,
    ) -> Result<bool> {
        // This authenticates the exact original occurrence and current owner
        // before replaying its type recipe, just as it does its callee recipe.
        self.capture_result_call(receipt, expression)?;
        let closure = self.closure.as_ref().expect("validated closure context");
        let node = receipt
            .capture_witness()
            .and_then(|w| w.nodes.iter().find(|node| node.expression == expression))
            .expect("validated source occurrence");
        let ir::CaptureResultKind::Call { source_ty, .. } = &node.kind else {
            unreachable!("validated call occurrence")
        };
        let mut types =
            self.world
                .specialize_capture_types(closure, vec![source_ty.clone()], self)?;
        self.validate_ty_metadata(actual)?;
        self.world
            .eq_ty(&types.pop().expect("one type recipe"), actual, 0)
    }
}

impl ValidatedTraitWorld {
    fn validate_capture_receipt(
        &self,
        closure: &ir::ClosureTy,
        receipt: &ir::ClosureResultReceipt,
    ) -> Result<()> {
        let invalid = |message| FactsError::invalid(&self.budget, message);
        let witness = receipt
            .capture_witness()
            .ok_or_else(|| invalid("missing capture result source witness"))?;
        if receipt.owner() != closure.owner()
            || receipt.provider() != closure.provider()
            || receipt.site() != closure.site()
            || closure
                .result_receipt()
                .is_none_or(|original| !original.same_source_shape(receipt))
            || self.schema.closure_recipe(closure.provider())?.as_ref() != Some(&witness.recipe)
        {
            return Err(invalid(
                "capture result has foreign owner, site, recipe or original receipt",
            ));
        }
        Ok(())
    }

    fn specialize_capture_target(
        &self,
        closure: &ir::ClosureTy,
        target: &Callee,
        facts: &ValidatedFacts,
    ) -> Result<Callee> {
        let metadata = target.metadata_types().cloned().collect::<Vec<_>>();
        let mut types = self
            .specialize_capture_types(closure, metadata, facts)?
            .into_iter();
        let mut take = || {
            types.next().ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "capture call substitution lost source type occurrence",
                )
            })
        };
        let result = match target {
            Callee::Direct(call) => {
                let mut args = Vec::new();
                for _ in &*call.type_args {
                    push(&self.budget, &mut args, take()?)?;
                }
                Callee::Direct(ir::CallTarget {
                    declaration: call.declaration,
                    type_args: args.into(),
                })
            }
            Callee::TraitItem(call) => {
                let mut method_args = Vec::new();
                for _ in &*call.method_args {
                    push(&self.budget, &mut method_args, take()?)?;
                }
                let self_ty = take()?;
                let mut trait_args = Vec::new();
                for _ in &*call.trait_ref.args {
                    push(&self.budget, &mut trait_args, take()?)?;
                }
                Callee::TraitItem(TraitCall {
                    item: call.item,
                    method_args: method_args.into(),
                    trait_ref: TraitRef {
                        declaration: call.trait_ref.declaration,
                        self_ty,
                        args: trait_args.into(),
                    },
                })
            }
            _ => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "captured closure result requires canonical direct or trait calls",
                ));
            }
        };
        Ok(result)
    }

    fn specialize_capture_types(
        &self,
        closure: &ir::ClosureTy,
        metadata: Vec<Ty>,
        facts: &ValidatedFacts,
    ) -> Result<Vec<Ty>> {
        let mut schema = self.callable_schema(closure.owner())?;
        // A closed source owner has no substitution to perform. Preserve its
        // original graph-issued metadata rather than asking an identity-only
        // adapter to mint or retain a nominal realization. The enclosing
        // receipt replay has already checked the original recipe and current
        // facts, and the resulting call is still discharged canonically.
        if schema.env.total_arity() == 0 && closure.owner_args().is_empty() {
            return Ok(metadata);
        }
        // These are source metadata occurrences, not a callable signature. The
        // checked structural substitution adapter replays canonical binders and
        // nominal graph identities. Actual membership is discharged below.
        schema.signature = Signature {
            params: metadata,
            result: Ty::Unit,
            ref_contract: Arc::new(ir::RefContract::default()),
        };
        schema.predicates = Arc::from([]);
        let instantiated = self.substitute_projected_schema_with(
            &schema,
            closure.owner_args(),
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        Ok(instantiated.signature.params)
    }

    fn capture_call_signature(&self, target: &Callee, facts: &ValidatedFacts) -> Result<Signature> {
        match target {
            Callee::Direct(target) => Ok(self.discharge_direct(target, facts)?.signature),
            Callee::TraitItem(call) => self.instantiate_trait_signature(call, facts),
            _ => Err(FactsError::unsupported(
                &self.budget,
                "unsupported capture result dispatch",
            )),
        }
    }

    pub(super) fn captured_result_carriers(
        &self,
        closure: &Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
        signature: &Signature,
        facts: &ValidatedFacts,
    ) -> Result<Arc<[ir::CarrierReturn]>> {
        self.check_facts(facts, true)?;
        let receipt = closure
            .result_receipt()
            .ok_or_else(|| FactsError::invalid(&self.budget, "missing captured result receipt"))?;
        self.validate_capture_receipt(closure, receipt)?;
        let mut returns = Vec::new();
        let mut active = Vec::new();
        for location in charged(
            &self.budget,
            ir::carrier_locations(&signature.result, &self.budget),
        )?
        .iter()
        {
            let (ty, sources) = self.capture_result_flow(
                closure,
                mode,
                facts,
                receipt.tail(),
                location,
                &mut active,
                0,
            )?;
            if !self.eq_ty(&ty, &signature.result, 0)? || sources.is_empty() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "capture result differs from original canonical source call or loan provenance",
                ));
            }
            push(
                &self.budget,
                &mut returns,
                ir::CarrierReturn {
                    location: location.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        Ok(returns.into())
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_result_flow(
        &self,
        closure: &Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
        facts: &ValidatedFacts,
        expression: ir::ExprId,
        location: &ir::CarrierLocation,
        active: &mut Vec<(ir::ExprId, ir::CarrierLocation)>,
        depth: usize,
    ) -> Result<(Ty, Vec<ir::InputCarrier>)> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(active.len() + 1))?;
        if active
            .iter()
            .any(|(id, path)| *id == expression && path == location)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "cyclic capture result source flow",
            ));
        }
        push(&self.budget, active, (expression, location.clone()))?;
        let receipt = closure.result_receipt().expect("validated receipt");
        let witness = receipt
            .capture_witness()
            .expect("validated captured receipt");
        charged(&self.budget, self.budget.charge_work(witness.nodes.len()))?;
        let node = witness
            .nodes
            .iter()
            .find(|node| node.expression == expression)
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "capture result has no original source occurrence for current channel",
                )
            })?;
        let mut output = Vec::new();
        let ty = match &node.kind {
            ir::CaptureResultKind::ArgumentBorrow(origin) => {
                let ty = charged(
                    &self.budget,
                    origin.projected_result(closure.params(), &self.budget),
                )?;
                let root = ir::CarrierLocation {
                    path: ir::CarrierPath(Arc::from([])),
                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
                };
                if *location != root || origin.borrow != expression {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "argument borrow result changed exact source channel",
                    ));
                }
                retain::<ir::CarrierStep>(&self.budget, 1)?;
                let input = ir::InputCarrier {
                    param: 1,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(Arc::from([ir::CarrierStep::Field(origin.argument)])),
                        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                            depth: origin.depth,
                        }),
                    },
                };
                retain::<Ty>(&self.budget, closure.params().len())?;
                let tuple_ty = Ty::Tuple(closure.params().into());
                if !charged(&self.budget, ir::carrier_locations(&tuple_ty, &self.budget))?
                    .contains(&input.location)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "argument borrow is outside actual input carrier domain",
                    ));
                }
                push(&self.budget, &mut output, input)?;
                ty
            }
            ir::CaptureResultKind::InputValue(origin) => {
                let ty = charged(
                    &self.budget,
                    origin.projected_type(closure.params(), &self.budget),
                )?;
                if !facts.must_copy(&ty)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "closure result call cannot copy a moved argument carrier",
                    ));
                }
                let rows = charged(
                    &self.budget,
                    origin.value_carriers(closure.params(), &self.budget),
                )?;
                let row = rows
                    .iter()
                    .find(|row| row.location == *location)
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "closure result call lost its exact argument channel",
                        )
                    })?;
                for source in row.sources.iter() {
                    push(&self.budget, &mut output, source.clone())?;
                }
                ty
            }
            ir::CaptureResultKind::ArrayToSlice { reference } => {
                if !location.path.0.is_empty()
                    || location.channel
                        != ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
                {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "captured array-to-slice result contains nested carriers",
                    ));
                }
                let (source_ty, sources) = self.capture_result_flow(
                    closure,
                    mode,
                    facts,
                    *reference,
                    location,
                    active,
                    depth + 1,
                )?;
                let Some((ir::BorrowKind::Shared, element, _)) = source_ty.array_ref_element_len()
                else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "captured unsizing lost original shared array reference",
                    ));
                };
                charged(
                    &self.budget,
                    ir::validate_shared_slice_element(
                        element,
                        ir::FactsCtx::Validated(facts),
                        &self.budget,
                    ),
                )?;
                output = sources;
                retain::<Ty>(&self.budget, 2)?;
                Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(Ty::Slice {
                        element: Arc::new(element.clone()),
                    }),
                }
            }
            ir::CaptureResultKind::Call {
                target,
                arguments,
                source_ty,
            } => {
                let target = self.specialize_capture_target(closure, target, facts)?;
                let signature = self.capture_call_signature(&target, facts)?;
                let mut types =
                    self.specialize_capture_types(closure, vec![source_ty.clone()], facts)?;
                let actual = types.pop().ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "capture call lost its original result type recipe",
                    )
                })?;
                let returned = facts.capture_call_result_sources(&target, &actual, location)?;
                if returned.is_empty() || arguments.len() != signature.params.len() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "capture result call changed argument or loan domain",
                    ));
                }
                for source in returned.iter() {
                    let argument = *arguments.get(source.param).ok_or_else(|| {
                        FactsError::invalid(&self.budget, "capture result names foreign call input")
                    })?;
                    let (ty, sources) = self.capture_result_flow(
                        closure,
                        mode,
                        facts,
                        argument,
                        &source.location,
                        active,
                        depth + 1,
                    )?;
                    if !self.eq_ty(&ty, &signature.params[source.param], 0)? {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "capture result changed exact source argument type",
                        ));
                    }
                    for source in sources {
                        if !output.contains(&source) {
                            push(&self.budget, &mut output, source)?;
                        }
                    }
                }
                actual
            }
            ir::CaptureResultKind::Borrow {
                field,
                source,
                original,
                prefix,
            }
            | ir::CaptureResultKind::Read {
                field,
                source,
                original,
                prefix,
            } => {
                if closure.sources().get(*field) != Some(source)
                    || source.kind != ir::CaptureKind::Shared
                    || !original.projection.starts_with(prefix)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "capture result changed original shared capture loan or carrier path",
                    ));
                }
                let Some(Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                }) = closure.captures().get(*field)
                else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "capture result lost original shared environment field",
                    ));
                };
                let suffix = &original.projection[prefix.len()..];
                let mut projected = referent.as_ref().clone();
                for step in suffix {
                    if matches!(step, ir::Projection::Deref)
                        && !matches!(
                            &projected,
                            Ty::Ref {
                                kind: ir::BorrowKind::Shared,
                                ..
                            }
                        )
                    {
                        return Err(FactsError::unsupported(
                            &self.budget,
                            "capture result cannot derive loan authority through mutable or raw dereference",
                        ));
                    }
                    projected = facts.projected_ty(&projected, std::slice::from_ref(step))?;
                }
                let borrowing = matches!(&node.kind, ir::CaptureResultKind::Borrow { .. });
                let ty = if borrowing {
                    retain::<Ty>(&self.budget, 1)?;
                    Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(projected),
                    }
                } else {
                    if !facts.must_copy(&projected)? {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "capture result read cannot move a captured carrier",
                        ));
                    }
                    projected
                };
                if !charged(&self.budget, ir::carrier_locations(&ty, &self.budget))?
                    .contains(location)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "capture result names foreign source carrier depth or path",
                    ));
                }
                let mut projections = Vec::new();
                push(&self.budget, &mut projections, ir::Projection::Deref)?;
                for step in suffix {
                    push(&self.budget, &mut projections, *step)?;
                }
                let input = if borrowing
                    && location.path.0.is_empty()
                    && location.channel
                        == ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
                {
                    let owner = projections
                        .iter()
                        .rposition(|step| matches!(step, ir::Projection::Deref))
                        .expect("shared capture indirection");
                    captured_input(mode, *field, &projections[..owner], location, &self.budget)?
                } else {
                    let inner = if borrowing {
                        remove_result_reference(location, &self.budget)?
                    } else {
                        location.clone()
                    };
                    captured_input(mode, *field, &projections, &inner, &self.budget)?
                };
                push(&self.budget, &mut output, input)?;
                ty
            }
            ir::CaptureResultKind::Tuple { fields, source_ty } => {
                let Some((ir::CarrierStep::Field(index), rest)) = location.path.0.split_first()
                else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "captured tuple has foreign result path",
                    ));
                };
                let field = *fields.get(*index).ok_or_else(|| {
                    FactsError::invalid(&self.budget, "captured tuple has foreign field occurrence")
                })?;
                let inner = ir::CarrierLocation {
                    path: ir::CarrierPath(rest.into()),
                    channel: location.channel.clone(),
                };
                let (field_ty, sources) = self.capture_result_flow(
                    closure,
                    mode,
                    facts,
                    field,
                    &inner,
                    active,
                    depth + 1,
                )?;
                output = sources;
                let mut types =
                    self.specialize_capture_types(closure, vec![source_ty.clone()], facts)?;
                let ty = types.pop().ok_or_else(|| {
                    FactsError::invalid(&self.budget, "captured tuple lost original type recipe")
                })?;
                let expected = charged(&self.budget, ty.field_ty(*index, &self.budget))?
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "captured tuple field is outside original result",
                        )
                    })?;
                if !self.eq_ty(&expected, &field_ty, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "captured tuple changed original field type",
                    ));
                }
                ty
            }
        };
        active.pop();
        Ok((ty, output))
    }
}
