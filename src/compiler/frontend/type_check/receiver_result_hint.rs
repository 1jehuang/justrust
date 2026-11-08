//! Source result evidence at an already chosen canonical receiver probe.
use super::*;

pub(super) enum InherentMethodCandidate {
    Missing,
    Ambiguous,
    Unique(ir::DeclId, HashMap<ir::ParamId, ir::Ty>),
}

impl Checker<'_> {
    /// Stored fields win over autoderef, exactly as for a local-rooted place.
    /// The caller uses this only to plan projections; checked DerefMut and the
    /// owned temporary's lifetime remain in normal assignment lowering.
    pub(super) fn owned_call_assignment_target(
        &self,
        carrier: &Expr,
        owner: &ir::Ty,
        fields: &[FieldSelector],
    ) -> Result<Option<ir::Ty>, String> {
        if !genuine_call(carrier) || !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_)) {
            return Ok(None);
        }
        let explicit = matches!(fields.first(), Some(FieldSelector::Deref));
        if !explicit {
            let Some(field @ (FieldSelector::Named(_) | FieldSelector::Tuple(_))) = fields.first()
            else {
                return Ok(None);
            };
            let ir::Ty::Struct(nominal) = owner else {
                return Ok(None);
            };
            if self.has_member(nominal.def().source_decl, field)? {
                return Ok(None);
            }
        }
        let target = self.deref_target_hint(owner)?;
        if explicit && target.is_none() {
            return Err("E0614: call assignment carrier cannot be dereferenced".into());
        }
        Ok(target)
    }

    /// Share the inherent inference probe, including its ambiguity rule. Dot
    /// callers supply the receiver mode chosen by the canonical search.
    pub(super) fn inherent_method_candidate(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        filter: Option<ReceiverKind>,
    ) -> Result<InherentMethodCandidate, String> {
        let plan = self.signatures.plan()?;
        let Some((nominal, _)) = nominal_parts(receiver) else {
            return Ok(InherentMethodCandidate::Missing);
        };
        let mut candidate = None;
        for id in &plan.registry.function_ids {
            plan.budget.charge_work(1)?;
            let info = plan.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            if filter.is_some_and(|mode| receiver_mode(mode) != receiver_mode(associated.receiver))
                || associated.trait_decl.is_some()
                || associated.nominal != nominal
                || !plan.registry.source_member_equal(info.path.item, *name)?
            {
                continue;
            }
            let Some(bindings) =
                plan.match_inherent_head(*id, receiver, self.facts(), &plan.budget)?
            else {
                continue;
            };
            // Do not use the result type to choose between declarations.
            if candidate.replace((*id, bindings)).is_some() {
                return Ok(InherentMethodCandidate::Ambiguous);
            }
        }
        Ok(match candidate {
            Some((id, bindings)) => InherentMethodCandidate::Unique(id, bindings),
            None => InherentMethodCandidate::Missing,
        })
    }

    /// A callback's type need not be known when the enclosing impl alone
    /// determines the result. This does not select a callee or discharge any
    /// predicate: the actual call still infers and validates every argument.
    pub(super) fn receiver_bound_method_result(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        argument_count: usize,
        mode: ReceiverKind,
    ) -> Result<Option<ir::Ty>, String> {
        let plan = self.signatures.plan()?;
        let InherentMethodCandidate::Unique(id, bindings) =
            self.inherent_method_candidate(receiver, name, Some(mode))?
        else {
            return Ok(None);
        };
        let schema = plan.raw_schema(id)?;
        if schema.signature.params.len().checked_sub(1) != Some(argument_count) {
            return Ok(None);
        }
        let solver = Inference::new(self, plan.param_env(id)?)?;
        let source_receiver = self.method_owner_term(
            &solver.schema(&schema.signature.params[0], 0)?,
            &bindings,
            0,
        )?;
        let expected_receiver = match mode {
            ReceiverKind::Value { .. } => receiver.clone(),
            ReceiverKind::Borrowed { kind } => ir::Ty::Ref {
                kind,
                referent: Arc::new(receiver.clone()),
            },
            ReceiverKind::None => return Ok(None),
        };
        let Some(source_receiver) = solver.resolve(&source_receiver, false, 0)? else {
            return Ok(None);
        };
        if !metered_type_equal(&source_receiver, &expected_receiver, &plan.budget, 0)? {
            return Ok(None);
        }
        let result =
            self.method_owner_term(&solver.schema(&schema.signature.result, 0)?, &bindings, 0)?;
        // No defaulting or fabricated actuals. An own method slot anywhere in
        // the result leaves it open, even inside references or nominal actuals.
        let Some(result) = solver.resolve(&result, false, 0)? else {
            return Ok(None);
        };
        if ir::contains_associated_types(&result, &plan.budget)? {
            return Ok(None);
        }
        Ok(Some(result))
    }
}
