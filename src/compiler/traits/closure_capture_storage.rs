//! Exact storage comes from the original capture expression, not carrier types.
use super::*;

#[derive(Debug)]
pub(in crate::compiler) struct CapturedResultStorage {
    input: ir::InputCarrier,
    projection: Arc<[ir::Projection]>,
}
impl CapturedResultStorage {
    pub(in crate::compiler) fn input(&self) -> &ir::InputCarrier {
        &self.input
    }
    pub(in crate::compiler) fn projection(&self) -> &[ir::Projection] {
        &self.projection
    }
}

impl CarrierContractView {
    pub(in crate::compiler) fn captured_result_storage(
        &self,
        facts: &ValidatedFacts,
        callee: &Callee,
        signature: &Signature,
        location: &ir::CarrierLocation,
    ) -> Result<Option<CapturedResultStorage>> {
        let (closure, mode) = if let Some((closure, mode)) = &self.closure_result {
            (closure, *mode)
        } else if let Some(projected) = &self.projected {
            let goal = projected.goal();
            let Ty::Closure(closure) = &goal.self_ty else {
                return Ok(None);
            };
            let Some(mode) = facts.world.closure_trait_kind(goal.declaration)? else {
                return Ok(None);
            };
            (closure, mode)
        } else {
            return Ok(None);
        };
        let Some(receipt) = closure.result_receipt() else {
            return Ok(None);
        };
        let Some(witness) = receipt.capture_witness() else {
            return Ok(None);
        };
        let Some(storage) =
            source_storage(witness, receipt.tail(), mode, location, facts.budget(), 0)?
        else {
            return Ok(None);
        };
        // Replaying the whole view authenticates the owner, site, recipe,
        // source field and complete carrier mapping in this actual caller.
        self.validate_result_flow_with(
            facts,
            SignatureSubject::Call(callee),
            signature,
            location,
            &mut false,
        )?;
        if !matches!(
            charged(
                facts.budget(),
                ir::carrier_at(&signature.result, location, facts.budget())
            )?,
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                ..
            }
        ) {
            return Ok(None);
        }
        let row = self
            .contract
            .results
            .iter()
            .find(|row| row.location == *location)
            .ok_or_else(|| {
                FactsError::invalid(
                    facts.budget(),
                    "captured storage lost its original result channel",
                )
            })?;
        if row.sources.as_ref() != std::slice::from_ref(storage.input())
            || !self.contract.inputs.contains(storage.input())
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "captured storage differs from its exact original input carrier",
            ));
        }
        Ok(Some(storage))
    }
}

fn source_storage(
    witness: &ir::CaptureResultWitness,
    expression: ir::ExprId,
    mode: ir::ClosureKind,
    location: &ir::CarrierLocation,
    budget: &PlanBudget,
    depth: usize,
) -> Result<Option<CapturedResultStorage>> {
    charged(budget, budget.check_depth(depth))?;
    charged(budget, budget.charge_work(witness.nodes.len() + 1))?;
    let node = witness
        .nodes
        .iter()
        .find(|node| node.expression == expression)
        .ok_or_else(|| {
            FactsError::invalid(budget, "captured storage lost original expression identity")
        })?;
    match &node.kind {
        ir::CaptureResultKind::Tuple { fields, .. } => {
            let Some((ir::CarrierStep::Field(index), rest)) = location.path.0.split_first() else {
                return Ok(None);
            };
            let expression = *fields.get(*index).ok_or_else(|| {
                FactsError::invalid(
                    budget,
                    "captured storage names a foreign tuple result field",
                )
            })?;
            retain::<ir::CarrierStep>(budget, rest.len())?;
            source_storage(
                witness,
                expression,
                mode,
                &ir::CarrierLocation {
                    path: ir::CarrierPath(rest.into()),
                    channel: location.channel.clone(),
                },
                budget,
                depth + 1,
            )
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
            if source.kind != ir::CaptureKind::Shared || !original.projection.starts_with(prefix) {
                return Err(FactsError::invalid(
                    budget,
                    "captured storage changed original capture mode or prefix",
                ));
            }
            let suffix = &original.projection[prefix.len()..];
            let mut projection = Vec::new();
            push(budget, &mut projection, ir::Projection::Deref)?;
            for step in suffix {
                push(budget, &mut projection, *step)?;
            }
            let root_borrow = matches!(node.kind, ir::CaptureResultKind::Borrow { .. })
                && location.path.0.is_empty()
                && location.channel
                    == ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 });
            let (input, remaining) = if root_borrow {
                let dereference = projection
                    .iter()
                    .rposition(|step| *step == ir::Projection::Deref)
                    .ok_or_else(|| {
                        FactsError::invalid(
                            budget,
                            "captured storage has no original shared dereference",
                        )
                    })?;
                let remaining = &projection[dereference + 1..];
                if remaining.iter().any(|step| {
                    matches!(
                        step,
                        ir::Projection::DynamicIndex(_) | ir::Projection::Deref
                    )
                }) {
                    return Ok(None);
                }
                let input = super::capture::captured_input(
                    mode,
                    *field,
                    &projection[..dereference],
                    location,
                    budget,
                )?;
                retain::<ir::Projection>(budget, remaining.len())?;
                (input, Arc::from(remaining))
            } else {
                let location = if matches!(node.kind, ir::CaptureResultKind::Borrow { .. }) {
                    super::capture::remove_result_reference(location, budget)?
                } else {
                    location.clone()
                };
                (
                    super::capture::captured_input(mode, *field, &projection, &location, budget)?,
                    Arc::from([]),
                )
            };
            Ok(Some(CapturedResultStorage {
                input,
                projection: remaining,
            }))
        }
        // A canonical call carrier contract proves lifetime containment, not
        // identical returned storage. No Call or coercion can mint this proof.
        ir::CaptureResultKind::Call { .. } | ir::CaptureResultKind::ArrayToSlice { .. } => Ok(None),
    }
}
