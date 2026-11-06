//! Replay one canonical adjustment of an authored local borrow.
use super::*;

impl Replay<'_> {
    pub(super) fn borrow_coercion_site(
        original: &OriginalSource,
        inventory: &SourceInventory,
        node: &sr::SourceNode,
        kind: ir::BorrowKind,
        root: AstIdent,
        budget: &ir::PlanBudget,
    ) -> Result<usize, FactsError> {
        Self::borrow_coercion_field_site(original, inventory, node, kind, root, &[], budget)
    }

    pub(super) fn borrow_coercion_field_site(
        original: &OriginalSource,
        inventory: &SourceInventory,
        node: &sr::SourceNode,
        kind: ir::BorrowKind,
        root: AstIdent,
        fields: &[FieldSelector],
        budget: &ir::PlanBudget,
    ) -> Result<usize, FactsError> {
        validate_original_occurrence(original, node, budget)?;
        let function = original
            .plan
            .callable_body(&original.plan.program, original.declaration)
            .map_err(|e| source_error(budget, &e))?;
        let mut found = None;
        walk_depth_inspecting(
            DepthNode::Block(&function.body),
            None,
            Some(&inventory.names),
            Some(&mut |part, depth| {
                budget.check_depth(depth)?;
                budget.charge_work(1)?;
                let DepthNode::Expr(expr) = part else {
                    return Ok(());
                };
                let Expr::Borrow {
                    kind: source_kind,
                    place,
                } = expr
                else {
                    return Ok(());
                };
                let LocalName::Named(name) = place.local else {
                    return Ok(());
                };
                let class = crate::compiler::semantic_name::LookupClass::Local;
                if *source_kind != kind
                    || place.fields.len() != fields.len()
                    || inventory
                        .names
                        .key(name, class)
                        .map_err(native_adapter::name_diagnostic)?
                        != inventory
                            .names
                            .key(root, class)
                            .map_err(native_adapter::name_diagnostic)?
                {
                    return Ok(());
                }
                for (source, expected) in place.fields.iter().zip(fields) {
                    budget.charge_work(1)?;
                    let equal = match (source, expected) {
                        (FieldSelector::Deref, FieldSelector::Deref) => true,
                        (FieldSelector::ConstantIndex(a), FieldSelector::ConstantIndex(b))
                        | (FieldSelector::Tuple(a), FieldSelector::Tuple(b)) => a == b,
                        (FieldSelector::Named(a), FieldSelector::Named(b)) => {
                            let class = crate::compiler::semantic_name::LookupClass::Member;
                            inventory
                                .names
                                .key(*a, class)
                                .map_err(native_adapter::name_diagnostic)?
                                == inventory
                                    .names
                                    .key(*b, class)
                                    .map_err(native_adapter::name_diagnostic)?
                        }
                        _ => false,
                    };
                    if !equal {
                        return Ok(());
                    }
                }
                let pointer = *expr as *const Expr as usize;
                let Some(index) = inventory
                    .call_pointers
                    .get(&deref_coercion::site(pointer, 1))
                    .copied()
                else {
                    return Ok(());
                };
                let Some(site) = inventory.calls.get(index) else {
                    return Err("explicit borrow coercion operator role outside inventory".into());
                };
                if site.node != *node {
                    return Ok(());
                }
                if !site.operator_candidate || site.node.owner != original.identity {
                    return Err("explicit borrow coercion operator role changed".into());
                }
                for step in 2..=deref_coercion::MAX_DEREF_STEPS {
                    let Some(other) = inventory
                        .call_pointers
                        .get(&deref_coercion::site(pointer, step))
                        .copied()
                    else {
                        return Err("explicit borrow coercion reserved step missing".into());
                    };
                    let Some(other_site) = inventory.calls.get(other) else {
                        return Err(
                            "explicit borrow coercion reserved step outside inventory".into()
                        );
                    };
                    if other == index || !other_site.operator_candidate || other_site.node != *node
                    {
                        return Err(
                            "explicit borrow coercion first step aliases another role".into()
                        );
                    }
                }
                if found.replace(index).is_some() {
                    return Err("explicit borrow coercion original operand is not unique".into());
                }
                Ok(())
            }),
        )
        .map_err(|e| source_error(budget, &e))?;
        found.ok_or_else(|| {
            FactsError::invalid(
                budget,
                "explicit borrow coercion original first step missing",
            )
        })
    }

    pub(super) fn explicit_borrow_coercion(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let Expr::Borrow { kind, place } = source else {
            return Err(FactsError::invalid(
                self.budget,
                "coercion lost authored borrow syntax",
            ));
        };
        let LocalName::Named(root) = place.local else {
            return Err(FactsError::unsupported(
                self.budget,
                "coercion requires an authored named local root",
            ));
        };
        let kind = *kind;
        let (local, source_ty) = self.operand(node, depth + 1)?;
        let owner = &source_ty;
        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_)) {
            return Err(FactsError::unsupported(
                self.budget,
                "explicit borrow coercion requires one nominal deref step",
            ));
        }
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        let (trait_name, member) = if kind == ir::BorrowKind::Mut {
            ("DerefMut", "deref_mut")
        } else {
            ("Deref", "deref")
        };
        let (declaration, name) = checker
            .operator_trait("ops", trait_name, member)
            .map_err(|e| source_error(self.budget, &e))?;
        let goal = ir::TraitRef {
            declaration,
            self_ty: owner.clone(),
            args: Arc::from([]),
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
                    "explicit borrow coercion has no canonical Deref implementation",
                )
            })?;
        // Selection yields a trait-item call. Resolve it under the original
        // source facts exactly as finish_checked_call does, before comparing
        // the retained capsule or checked target. Neither can select the impl.
        let mut scratch = tr::TraitScratch::default();
        self.original.facts.validate_signature_with(
            ir::SignatureSubject::Call(&selected_target),
            &selected,
            &mut scratch,
        )?;
        let ir::Callee::TraitItem(call) = &selected_target else {
            return Err(FactsError::unsupported(
                self.budget,
                "explicit borrow coercion requires a source trait-item selection",
            ));
        };
        let resolved = self.original.facts.service().resolve_trait_call_with(
            call,
            &self.original.facts,
            &mut scratch,
        )?;
        let selected_target = resolved.callee().clone();
        let selected = resolved.signature().clone();
        let input = self.reference(kind, owner.clone())?;
        let ir::Ty::Ref {
            kind: result_kind, ..
        } = &selected.result
        else {
            return Err(FactsError::invalid(
                self.budget,
                "selected borrow coercion did not return a reference",
            ));
        };
        if *result_kind != kind {
            return Err(FactsError::invalid(
                self.budget,
                "selected borrow coercion changed authored reference kind",
            ));
        }
        if selected.params.len() != 1 || !self.equal(&selected.params[0], &input)? {
            return Err(FactsError::invalid(
                self.budget,
                "selected explicit borrow coercion parameter changed",
            ));
        }
        let source_index = if place.fields.is_empty() {
            Self::borrow_coercion_site(
                self.original,
                self.inventory,
                node,
                kind,
                root,
                self.budget,
            )?
        } else {
            Self::borrow_coercion_field_site(
                self.original,
                self.inventory,
                node,
                kind,
                root,
                &place.fields,
                self.budget,
            )?
        };
        sr::work(self.budget, self.original.call_mapping.len())?;
        let mut positions = self
            .original
            .call_mapping
            .iter()
            .enumerate()
            .filter(|(_, (index, _, _))| *index == source_index);
        let (position, (_, expression, original_target)) = positions.next().ok_or_else(|| {
            FactsError::invalid(
                self.budget,
                "explicit borrow coercion original operator capsule missing",
            )
        })?;
        if positions.next().is_some() || *expression != id {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion original operator occurrence changed",
            ));
        }
        validate_original_callee(original_target, &selected_target, self.budget)?;
        let actual = self.checked.exprs.get(id.0).ok_or_else(|| {
            FactsError::invalid(self.budget, "explicit borrow coercion call outside arena")
        })?;
        let ir::ExprKind::Call {
            target,
            signature,
            args,
            ..
        } = &actual.kind
        else {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion operator is not a call",
            ));
        };
        validate_original_callee(&selected_target, target, self.budget)?;
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
        if args.len() != 1
            || signature.params.len() != 1
            || canonical.params.len() != 1
            || !self.equal(&selected.params[0], &canonical.params[0])?
            || !self.equal(&selected.params[0], &signature.params[0])?
            || !self.equal(&selected.result, &canonical.result)?
            || !self.equal(&selected.result, &signature.result)?
            || !self.equal(&selected.result, &actual.ty)?
            || selected.ref_contract != canonical.ref_contract
            || selected.ref_contract != signature.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion selected signature changed",
            ));
        }
        let argument = self.checked.exprs.get(args[0].0).ok_or_else(|| {
            FactsError::invalid(
                self.budget,
                "explicit borrow coercion operand outside arena",
            )
        })?;
        let exact = matches!(&argument.kind,
            ir::ExprKind::Borrow { kind: actual_kind, place, timing: ir::BorrowTiming::Immediate }
                if *actual_kind == kind && place.local == local && place.projection.is_empty());
        if !exact || !self.equal(&argument.ty, &input)? {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion original operand adjustment changed",
            ));
        }
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let mut rows = self
            .checked
            .call_evidence
            .iter()
            .filter(|row| row.call == id);
        let evidence = rows.next().ok_or_else(|| {
            FactsError::invalid(
                self.budget,
                "explicit borrow coercion original evidence missing",
            )
        })?;
        if rows.next().is_some()
            || evidence.arguments.len() != 1
            || evidence.arguments[0].argument != 0
            || evidence.arguments[0].value != args[0]
            || evidence.arguments[0].expectation != (ir::CallExpectation::FirmReference { kind })
        {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion original evidence changed",
            ));
        }
        let ir::Callee::Direct(direct) = &selected_target else {
            return Err(FactsError::unsupported(
                self.budget,
                "explicit borrow coercion requires an exact source implementation",
            ));
        };
        // A type-erased reference cannot recover an authored named lifetime.
        // This branch proves reference-free actuals, never guessed Elided rows.
        for actual in direct.type_args.iter() {
            if !ir::carrier_locations(actual, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty()
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "explicit borrow coercion requires independently reference-free generic actuals",
                ));
            }
        }
        if !evidence.type_actual_regions.is_empty() {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion original actual regions changed",
            ));
        }
        let carrier = self
            .original
            .facts
            .carrier_contract(ir::SignatureSubject::Call(&selected_target))?;
        if carrier.ref_contract().as_ref() != selected.ref_contract.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "explicit borrow coercion original carrier contract changed",
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
