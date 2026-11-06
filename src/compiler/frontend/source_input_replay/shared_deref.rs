//! One shared local receiver adjustment, selected from its authored owner.
use super::*;

impl Replay<'_> {
    pub(super) fn shared_deref_site(
        original: &OriginalSource,
        inventory: &SourceInventory,
        node: &sr::SourceNode,
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
                if !matches!(expr, Expr::Var(path) if is_local_shaped_path(path)) {
                    return Ok(());
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
                    return Err("shared receiver operator role outside inventory".into());
                };
                if site.node != *node {
                    return Ok(());
                }
                if !site.operator_candidate || site.node.owner != original.identity {
                    return Err("shared receiver operator role changed".into());
                }
                for step in 2..=deref_coercion::MAX_DEREF_STEPS {
                    let Some(other) = inventory
                        .call_pointers
                        .get(&deref_coercion::site(pointer, step))
                        .copied()
                    else {
                        return Err("shared receiver reserved step missing".into());
                    };
                    let Some(other_site) = inventory.calls.get(other) else {
                        return Err("shared receiver reserved step outside inventory".into());
                    };
                    if other == index || !other_site.operator_candidate || other_site.node != *node
                    {
                        return Err("shared receiver first step aliases another role".into());
                    }
                }
                if found.replace(index).is_some() {
                    return Err("shared receiver original operand is not unique".into());
                }
                Ok(())
            }),
        )
        .map_err(|e| source_error(budget, &e))?;
        found.ok_or_else(|| {
            FactsError::invalid(budget, "shared receiver original first step missing")
        })
    }

    pub(super) fn shared_deref_receiver(
        &self,
        node: &sr::SourceNode,
        id: ir::ExprId,
        local: ir::LocalId,
        source_ty: &ir::Ty,
        depth: usize,
    ) -> Result<(ir::Ty, ir::Ty), FactsError> {
        sr::depth(self.budget, depth)?;
        let owner = match source_ty {
            ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } => referent.as_ref(),
            ir::Ty::Ref { .. } => {
                return Err(FactsError::unsupported(
                    self.budget,
                    "shared receiver requires an owned or shared local",
                ));
            }
            owner => owner,
        };
        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_)) {
            return Err(FactsError::unsupported(
                self.budget,
                "shared receiver requires one nominal deref step",
            ));
        }
        let source_index =
            Self::shared_deref_site(self.original, self.inventory, node, self.budget)?;
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
                "shared receiver original operator capsule missing",
            )
        })?;
        if positions.next().is_some() || *expression != id {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original operator occurrence changed",
            ));
        }
        let actual = self.checked.exprs.get(id.0).ok_or_else(|| {
            FactsError::invalid(self.budget, "shared receiver call outside arena")
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
                "shared receiver operator is not a call",
            ));
        };
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
        // A retained source-call receipt authenticates its original parameter,
        // including source-slot to graph realization transport. It supplies no
        // inference seed: the independently reconstructed lexical owner must
        // first prove canonical equality in that same original facts world.
        let Some(ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: canonical_owner,
        }) = canonical.params.first()
        else {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original parameter is not shared",
            ));
        };
        if canonical.params.len() != 1
            || !root_type_equal(owner, canonical_owner, &self.original.facts, self.budget)?
        {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original owner changed",
            ));
        }
        let owner = canonical_owner.as_ref();
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        let (declaration, name) = checker
            .operator_trait("ops", "Deref", "deref")
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
                    "shared receiver has no canonical Deref implementation",
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
                "shared receiver requires a source trait-item selection",
            ));
        };
        let resolved = self.original.facts.service().resolve_trait_call_with(
            call,
            &self.original.facts,
            &mut scratch,
        )?;
        let selected_target = resolved.callee().clone();
        let selected = resolved.signature().clone();
        let input = self.reference(ir::BorrowKind::Shared, owner.clone())?;
        let ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent,
        } = &selected.result
        else {
            return Err(FactsError::invalid(
                self.budget,
                "selected shared receiver did not return a shared reference",
            ));
        };
        let result_owner = referent.as_ref().clone();
        if selected.params.len() != 1 || !self.equal(&selected.params[0], &input)? {
            return Err(FactsError::invalid(
                self.budget,
                "selected shared receiver parameter changed",
            ));
        }
        validate_original_callee(original_target, &selected_target, self.budget)?;
        validate_original_callee(&selected_target, target, self.budget)?;
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
                "shared receiver selected signature changed",
            ));
        }
        let argument = self.checked.exprs.get(args[0].0).ok_or_else(|| {
            FactsError::invalid(self.budget, "shared receiver operand outside arena")
        })?;
        let exact = match (&argument.kind, source_ty) {
            (
                ir::ExprKind::Local(actual),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    ..
                },
            ) => *actual == local,
            (
                ir::ExprKind::Borrow {
                    kind: ir::BorrowKind::Shared,
                    place,
                    timing: ir::BorrowTiming::Immediate,
                },
                ty,
            ) if !matches!(ty, ir::Ty::Ref { .. }) => {
                place.local == local && place.projection.is_empty()
            }
            _ => false,
        };
        if !exact || !self.equal(&argument.ty, &input)? {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original operand adjustment changed",
            ));
        }
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let mut rows = self
            .checked
            .call_evidence
            .iter()
            .filter(|row| row.call == id);
        let evidence = rows.next().ok_or_else(|| {
            FactsError::invalid(self.budget, "shared receiver original evidence missing")
        })?;
        if rows.next().is_some()
            || evidence.arguments.len() != 1
            || evidence.arguments[0].argument != 0
            || evidence.arguments[0].value != args[0]
            || evidence.arguments[0].expectation
                != (ir::CallExpectation::FirmReference {
                    kind: ir::BorrowKind::Shared,
                })
        {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original evidence changed",
            ));
        }
        let ir::Callee::Direct(direct) = &selected_target else {
            return Err(FactsError::unsupported(
                self.budget,
                "shared receiver requires an exact source implementation",
            ));
        };
        for actual in direct.type_args.iter() {
            if !ir::carrier_locations(actual, self.budget)
                .map_err(|e| source_error(self.budget, &e))?
                .is_empty()
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "shared receiver requires independently reference-free generic actuals",
                ));
            }
        }
        if !evidence.type_actual_regions.is_empty() {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original actual regions changed",
            ));
        }
        let carrier = self
            .original
            .facts
            .carrier_contract(ir::SignatureSubject::Call(&selected_target))?;
        if carrier.ref_contract().as_ref() != selected.ref_contract.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "shared receiver original carrier contract changed",
            ));
        }
        ir::validate_callee(
            &selected_target,
            &selected,
            ir::FactsCtx::Validated(&self.original.facts),
        )
        .map_err(|e| source_error(self.budget, &e))?;
        Ok((result_owner, selected.result))
    }
}
