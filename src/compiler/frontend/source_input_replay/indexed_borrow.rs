//! Source-owned replay of `&local[N]`: a builtin constant array projection or
//! one canonical `Index<usize>::index(&local, N)` call on a nominal owner.
use super::*;

/// Shared `&local[N]` with a local-shaped root and nothing else. Mutable
/// index borrows keep their existing routes.
pub(super) fn authored_index_borrow(source: &Expr) -> Option<(ir::BorrowKind, &Expr, usize)> {
    let Expr::BorrowValue {
        kind: kind @ ir::BorrowKind::Shared,
        value,
    } = source
    else {
        return None;
    };
    let Expr::Index { base, index, .. } = value.as_ref() else {
        return None;
    };
    matches!(base.as_ref(), Expr::Var(path) if is_local_shaped_path(path)).then_some((
        *kind,
        base.as_ref(),
        *index,
    ))
}

/// Lexical node of the root local inside `&local[N]`.
pub(super) fn root_node(
    node: &sr::SourceNode,
    budget: &ir::PlanBudget,
) -> Result<sr::SourceNode, FactsError> {
    node.child(11, 0, budget)?.child(13, 0, budget)
}

impl Replay<'_> {
    pub(super) fn index_borrow(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let (kind, _, index) = authored_index_borrow(source)
            .ok_or_else(|| FactsError::invalid(self.budget, "index borrow lost authored syntax"))?;
        validate_original_occurrence(self.original, node, self.budget)?;
        let (local, owner) = self.operand(&root_node(node, self.budget)?, depth + 1)?;
        let actual = self
            .checked
            .exprs
            .get(id.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "index borrow outside arena"))?;
        if actual.completion != ir::Completion::Returns {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow changed completion",
            ));
        }
        // Builtin fixed array: the place itself, never a copied element.
        if let ir::Ty::Array { .. } = &owner {
            let context = self
                .original
                .plan
                .callable_context(self.original.declaration)
                .map_err(|e| source_error(self.budget, &e))?;
            let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
            let element = checker
                .index_type(&owner, index)
                .map_err(|e| source_error(self.budget, &e))?;
            let result = self.reference(kind, element)?;
            let exact = matches!(&actual.kind,
                ir::ExprKind::Borrow { kind: actual_kind, place, timing: ir::BorrowTiming::Immediate }
                    if *actual_kind == kind
                        && place.local == local
                        && place.projection.as_ref() == [ir::Projection::ConstantIndex(index)]);
            if !exact || !self.equal(&actual.ty, &result)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "array index borrow changed its source place",
                ));
            }
            return Ok(result);
        }
        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_)) {
            return Err(FactsError::unsupported(
                self.budget,
                "inferred index borrow requires an owned array or nominal root",
            ));
        }
        let ir::ExprKind::Call {
            target,
            signature,
            args,
            ..
        } = &actual.kind
        else {
            return Err(FactsError::invalid(
                self.budget,
                "nominal index borrow is not an operator call",
            ));
        };
        // The exact original operator occurrence of this authored borrow.
        sr::work(self.budget, self.original.call_mapping.len())?;
        let mut rows = self
            .original
            .call_mapping
            .iter()
            .enumerate()
            .filter(|(_, (_, expression, _))| *expression == id);
        let (position, (source_index, _, original_target)) = rows.next().ok_or_else(|| {
            FactsError::invalid(self.budget, "index borrow original operator missing")
        })?;
        if rows.next().is_some()
            || !self
                .inventory
                .calls
                .get(*source_index)
                .is_some_and(|site| site.operator_candidate && site.node == *node)
        {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow original operator occurrence changed",
            ));
        }
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        let (declaration, name) = checker
            .operator_trait("ops", "Index", "index")
            .map_err(|e| source_error(self.budget, &e))?;
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let goal = ir::TraitRef {
            declaration,
            self_ty: owner.clone(),
            args: Arc::from([usize_ty.clone()]),
        };
        let (selected_target, selected, _) = self
            .original
            .plan
            .select_trait_with_scratch(
                &goal,
                &name,
                &[],
                ir::FactsCtx::Validated(&self.original.facts),
                false,
                self.budget,
                &std::cell::RefCell::new(tr::TraitScratch::default()),
            )
            .map_err(|e| source_error(self.budget, &e))?
            .ok_or_else(|| {
                FactsError::unsupported(
                    self.budget,
                    "index borrow has no canonical Index<usize> implementation",
                )
            })?;
        let mut scratch = tr::TraitScratch::default();
        self.original.facts.validate_signature_with(
            ir::SignatureSubject::Call(&selected_target),
            &selected,
            &mut scratch,
        )?;
        let ir::Callee::TraitItem(call) = &selected_target else {
            return Err(FactsError::unsupported(
                self.budget,
                "index borrow requires a source trait-item selection",
            ));
        };
        let resolved = self.original.facts.service().resolve_trait_call_with(
            call,
            &self.original.facts,
            &mut scratch,
        )?;
        let selected_target = resolved.callee().clone();
        let selected = resolved.signature().clone();
        validate_original_callee(original_target, &selected_target, self.budget)?;
        validate_original_callee(&selected_target, target, self.budget)?;
        let input = self.reference(kind, owner.clone())?;
        let canonical = nominal_call_receipts::original_call_signature(
            self.original,
            position,
            id,
            target,
            signature,
            &self.original.facts,
            self.budget,
            &mut tr::TraitScratch::default(),
        )?;
        if args.len() != 2
            || selected.params.len() != 2
            || signature.params.len() != 2
            || canonical.params.len() != 2
            || !matches!(&selected.result, ir::Ty::Ref { kind: result_kind, .. } if *result_kind == kind)
            || !self.equal(&selected.params[0], &input)?
            || !self.equal(&selected.params[1], &usize_ty)?
            || !self.equal(&selected.params[0], &signature.params[0])?
            || !self.equal(&selected.params[0], &canonical.params[0])?
            || !self.equal(&selected.params[1], &signature.params[1])?
            || !self.equal(&selected.params[1], &canonical.params[1])?
            || !self.equal(&selected.result, &signature.result)?
            || !self.equal(&selected.result, &canonical.result)?
            || !self.equal(&selected.result, &actual.ty)?
            || selected.ref_contract != signature.ref_contract
            || selected.ref_contract != canonical.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow selected signature changed",
            ));
        }
        let receiver = self.checked.exprs.get(args[0].0).ok_or_else(|| {
            FactsError::invalid(self.budget, "index borrow receiver outside arena")
        })?;
        let operand = self.checked.exprs.get(args[1].0).ok_or_else(|| {
            FactsError::invalid(self.budget, "index borrow operand outside arena")
        })?;
        let exact_receiver = matches!(&receiver.kind,
            ir::ExprKind::Borrow { kind: actual_kind, place, timing: ir::BorrowTiming::Immediate }
                if *actual_kind == kind && place.local == local && place.projection.is_empty());
        let exact_operand = matches!(&operand.kind,
            ir::ExprKind::Int(value) if value.ty == ir::IntTy::Usize && value.bits == index as u128);
        if !exact_receiver
            || !exact_operand
            || !self.equal(&receiver.ty, &input)?
            || !self.equal(&operand.ty, &usize_ty)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow changed its source root or constant",
            ));
        }
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let mut evidence = self
            .checked
            .call_evidence
            .iter()
            .filter(|row| row.call == id);
        let row = evidence.next().ok_or_else(|| {
            FactsError::invalid(self.budget, "index borrow original evidence missing")
        })?;
        if evidence.next().is_some()
            || row.arguments.len() != 2
            || row.arguments[0].argument != 0
            || row.arguments[0].value != args[0]
            || row.arguments[0].expectation != (ir::CallExpectation::FirmReference { kind })
            || row.arguments[1].argument != 1
            || row.arguments[1].value != args[1]
            || row.arguments[1].expectation != ir::CallExpectation::NoFirmReference
            || !row.type_actual_regions.is_empty()
        {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow original evidence changed",
            ));
        }
        let ir::Callee::Direct(direct) = &selected_target else {
            return Err(FactsError::unsupported(
                self.budget,
                "index borrow requires an exact source implementation",
            ));
        };
        for actual in direct.type_args.iter() {
            if !ir::carrier_locations(actual, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty()
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "index borrow requires reference-free owner actuals",
                ));
            }
        }
        let carrier = self
            .original
            .facts
            .carrier_contract(ir::SignatureSubject::Call(&selected_target))?;
        if carrier.ref_contract().as_ref() != selected.ref_contract.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "index borrow carrier contract changed",
            ));
        }
        ir::validate_callee(
            &selected_target,
            &selected,
            ir::FactsCtx::Validated(&self.original.facts),
        )
        .map_err(|e| source_error(self.budget, &e))?;
        Ok(selected.result)
    }
}
