//! Source-derived method results. Checked actuals are comparison targets.
//! Generic owners remain zero-argument. Authored arguments and mutable local
//! receivers require a closed original header, including fixed formal inputs.
use super::*;

impl Replay<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn generic_method_result(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        direct: &ir::CallTarget,
        signature: &ir::Signature,
        args: &[ir::ExprId],
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let (receiver, name, type_args, written, authored_owner, receiver_tag, argument_offset) =
            match source {
                Expr::MethodCall {
                    receiver,
                    name,
                    type_args,
                    args,
                } => (
                    receiver.as_ref(),
                    name,
                    type_args,
                    args.as_slice(),
                    None,
                    10,
                    0,
                ),
                Expr::AssociatedCall {
                    owner,
                    name,
                    type_args,
                    args,
                } => {
                    let (receiver, written) = args.split_first().ok_or_else(|| {
                        FactsError::unsupported(
                            self.budget,
                            "associated method requires an explicit source receiver",
                        )
                    })?;
                    (receiver, name, type_args, written, Some(owner), 9, 1)
                }
                _ => {
                    return Err(FactsError::invalid(
                        self.budget,
                        "generic method lost original syntax",
                    ));
                }
            };
        if !type_args.is_empty() {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires inferred source type arguments",
            ));
        }
        if args.len() != written.len() + 1 {
            return Err(FactsError::invalid(
                self.budget,
                "generic method changed its checked receiver arity",
            ));
        }
        validate_original_occurrence(self.original, node, self.budget)?;
        let source_index = authored_call_index(self.inventory, node, self.budget)?;
        sr::work(self.budget, self.original.call_mapping.len())?;
        let (position, (_, original_expr, original_target)) = self
            .original
            .call_mapping
            .iter()
            .enumerate()
            .find(|(_, (index, _, _))| *index == source_index)
            .ok_or_else(|| {
                FactsError::invalid(
                    self.budget,
                    "generic method original correspondence missing",
                )
            })?;
        let target = ir::Callee::Direct(direct.clone());
        if *original_expr != id {
            return Err(FactsError::invalid(
                self.budget,
                "generic method expression changed",
            ));
        }
        validate_original_callee(original_target, &target, self.budget)?;
        let view = self
            .original
            .facts
            .original_callable_schema(tr::OriginalCallableSubject::Direct(direct.declaration))?;
        view.validate_for(&self.original.facts)?;
        if !self.original.facts.is_source_callable(direct.declaration)? {
            return Err(FactsError::unsupported(
                self.budget,
                "generic method requires an original source header",
            ));
        }
        let info = self
            .original
            .plan
            .registry
            .declaration(direct.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let associated = info
            .path
            .associated
            .as_ref()
            .ok_or_else(|| FactsError::invalid(self.budget, "generic method owner missing"))?;
        let closed_header = view.generic_env().total_arity() == 0 && direct.type_args.is_empty();
        if !written.is_empty()
            && (!closed_header || !matches!(info.receiver, ReceiverKind::Borrowed { .. }))
        {
            return Err(FactsError::unsupported(
                self.budget,
                "authored method arguments require a closed source header",
            ));
        }
        if info.param_env.arity() != 0
            || !self
                .original
                .plan
                .registry
                .source_member_equal(info.path.item, *name)
                .map_err(|e| source_error(self.budget, &e))?
            || view.signature().params.len() != args.len()
        {
            return Err(FactsError::unsupported(
                self.budget,
                "input method requires its exact source member and arity",
            ));
        }
        let mut env = None;
        for scope in view.generic_env().scopes() {
            sr::work(self.budget, 1)?;
            if scope.arity() != 0
                && (scope.binder().declaration != associated.impl_decl
                    || env.replace(scope.clone()).is_some())
            {
                return Err(FactsError::unsupported(
                    self.budget,
                    "generic method requires one original owner binder",
                ));
            }
        }
        let receiver_node = node.child(receiver_tag, 0, self.budget)?;
        validate_original_occurrence(self.original, &receiver_node, self.budget)?;
        let argument = self.checked.exprs.get(args[0].0).ok_or_else(|| {
            FactsError::invalid(self.budget, "generic method receiver outside checked arena")
        })?;
        let (owner, input, expectation) = if let Some(authored_owner) = authored_owner {
            let ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared,
            } = info.receiver
            else {
                return Err(FactsError::unsupported(
                    self.budget,
                    "associated input method requires a shared source receiver",
                ));
            };
            let expectation = ir::CallExpectation::FirmReference {
                kind: ir::BorrowKind::Shared,
            };
            let formal = &view.signature().params[0];
            let input = self.argument(
                receiver,
                &receiver_node,
                args[0],
                id,
                0,
                expectation,
                formal.is_closed_shallow().then_some(formal),
                depth + 1,
            )?;
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &input
            else {
                return Err(FactsError::invalid(
                    self.budget,
                    "associated method lost its source shared receiver",
                ));
            };
            let owner = referent.as_ref().clone();
            self.associated_method_owner(authored_owner, associated.nominal, &owner)?;
            (owner, input, expectation)
        } else {
            match info.receiver {
                ReceiverKind::Value { .. } => {
                    let owner =
                        self.expression(receiver, &receiver_node, args[0], None, depth + 1)?;
                    (owner.clone(), owner, ir::CallExpectation::NoFirmReference)
                }
                ReceiverKind::Borrowed { kind }
                    if kind == ir::BorrowKind::Shared || closed_header =>
                {
                    if !matches!(receiver, Expr::Var(path) if is_local_shaped_path(path)) {
                        return Err(FactsError::unsupported(
                            self.budget,
                            "borrowed input method requires an original local receiver",
                        ));
                    }
                    let (local, source_ty) = self.operand(&receiver_node, depth + 1)?;
                    if kind == ir::BorrowKind::Shared
                        && matches!(argument.kind, ir::ExprKind::Call { .. })
                    {
                        let (owner, input) = self.shared_deref_receiver(
                            &receiver_node,
                            args[0],
                            local,
                            &source_ty,
                            depth + 1,
                        )?;
                        (owner, input, ir::CallExpectation::FirmReference { kind })
                    } else {
                        let mut owner = source_ty.clone();
                        let mut projection = Vec::new();
                        while let ir::Ty::Ref { referent, .. } = &owner {
                            sr::depth(self.budget, depth + projection.len())?;
                            sr::push(&mut projection, ir::Projection::Deref, self.budget)?;
                            owner = referent.as_ref().clone();
                        }
                        let input = self.reference(kind, owner.clone())?;
                        let exact = match &argument.kind {
                            ir::ExprKind::Borrow {
                                kind: actual_kind,
                                place,
                                timing,
                            } => {
                                let exact_timing = match (kind, timing) {
                                    (ir::BorrowKind::Shared, ir::BorrowTiming::Immediate) => true,
                                    (
                                        ir::BorrowKind::Mut,
                                        ir::BorrowTiming::TwoPhase {
                                            activation,
                                            argument,
                                            reason: ir::ImplicitBorrowKind::MutableReceiver,
                                        },
                                    ) => *activation == id && *argument == 0,
                                    _ => false,
                                };
                                *actual_kind == kind
                                    && exact_timing
                                    && place.local == local
                                    && place.projection == projection
                            }
                            ir::ExprKind::Local(actual) if kind == ir::BorrowKind::Shared => {
                                *actual == local && self.equal(&source_ty, &input)?
                            }
                            _ => false,
                        };
                        if !exact || !self.equal(&argument.ty, &input)? {
                            return Err(FactsError::invalid(
                                self.budget,
                                "generic method changed its source receiver adjustment",
                            ));
                        }
                        (owner, input, ir::CallExpectation::FirmReference { kind })
                    }
                }
                _ => {
                    return Err(FactsError::unsupported(
                        self.budget,
                        "generic input method mutable receiver is not admitted",
                    ));
                }
            }
        };
        let owner_decl = match &owner {
            ir::Ty::Struct(value) => value.def().source_decl,
            ir::Ty::Enum(value) => value.def().source_decl,
            _ => {
                return Err(FactsError::unsupported(
                    self.budget,
                    "generic input method requires a nominal source owner",
                ));
            }
        };
        if owner_decl != associated.nominal {
            return Err(FactsError::invalid(
                self.budget,
                "generic method changed independently reconstructed owner",
            ));
        }
        let mut state = sr::CallInferenceState::new(env, self.budget)?;
        if closed_header {
            if !self.equal(&view.signature().params[0], &input)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "closed method receiver differs from original header",
                ));
            }
        } else {
            state.constrain_input(&view.signature().params[0], &input)?;
        }
        let mut inputs = Vec::new();
        sr::push(&mut inputs, input.clone(), self.budget)?;
        for (index, source) in written.iter().enumerate() {
            let formal = &view.signature().params[index + 1];
            let expectation = match formal {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let actual = self.argument(
                source,
                &node.child(9, index + argument_offset, self.budget)?,
                args[index + 1],
                id,
                index + 1,
                expectation,
                Some(formal),
                depth + 1,
            )?;
            if !self.equal(formal, &actual)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "closed method argument differs from original header",
                ));
            }
            sr::push(&mut inputs, actual, self.budget)?;
        }
        let mut materialize = |decl, args: &[ir::Ty]| {
            self.original
                .plan
                .intern_nominal(
                    decl,
                    args,
                    ir::FactsCtx::Validated(&self.original.facts),
                    self.budget,
                )
                .map_err(|e| source_error(self.budget, &e))
        };
        let actuals = state.finish(false, &mut materialize)?.ok_or_else(|| {
            FactsError::unsupported(
                self.budget,
                "generic source receiver leaves owner arguments unresolved",
            )
        })?;
        if actuals.len() != direct.type_args.len() {
            return Err(FactsError::invalid(
                self.budget,
                "generic method inferred owner arity changed",
            ));
        }
        for (source, checked) in actuals.iter().zip(direct.type_args.iter()) {
            if !self.equal(source, checked)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "generic method changed independently inferred owner arguments",
                ));
            }
        }
        sr::retain(
            self.budget,
            actuals.len().saturating_mul(std::mem::size_of::<ir::Ty>()) + 32,
        )?;
        let inferred_target = ir::CallTarget {
            declaration: direct.declaration,
            type_args: actuals.into(),
        };
        let inferred = self
            .original
            .plan
            .instantiate_signature(
                &inferred_target,
                ir::FactsCtx::Validated(&self.original.facts),
                self.budget,
            )
            .map_err(|e| source_error(self.budget, &e))?;
        let canonical = nominal_call_receipts::original_call_signature(
            self.original,
            position,
            id,
            &target,
            signature,
            &self.original.facts,
            self.budget,
            &mut tr::TraitScratch::default(),
        )?;
        if inferred.params.len() != args.len()
            || canonical.params.len() != args.len()
            || signature.params.len() != args.len()
            || !self.equal(&inferred.result, &canonical.result)?
            || !self.equal(&inferred.result, &signature.result)?
            || inferred.ref_contract != canonical.ref_contract
            || inferred.ref_contract != signature.ref_contract
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic method changed independently instantiated signature",
            ));
        }
        for (index, input) in inputs.iter().enumerate() {
            if !self.equal(&inferred.params[index], input)?
                || !self.equal(&inferred.params[index], &canonical.params[index])?
                || !self.equal(&inferred.params[index], &signature.params[index])?
            {
                return Err(FactsError::invalid(
                    self.budget,
                    "method source parameter changed",
                ));
            }
        }
        sr::work(self.budget, self.checked.call_evidence.len())?;
        let evidence = self
            .checked
            .call_evidence
            .iter()
            .find(|row| row.call == id)
            .ok_or_else(|| {
                FactsError::invalid(
                    self.budget,
                    "generic method original argument trace missing",
                )
            })?;
        if evidence.arguments.len() != args.len()
            || evidence.arguments[0].argument != 0
            || evidence.arguments[0].value != args[0]
            || evidence.arguments[0].expectation != expectation
        {
            return Err(FactsError::invalid(
                self.budget,
                "generic method original receiver trace changed",
            ));
        }
        for (index, argument) in args.iter().enumerate().skip(1) {
            let expectation = match &view.signature().params[index] {
                ir::Ty::Ref { kind, .. } => ir::CallExpectation::FirmReference { kind: *kind },
                _ => ir::CallExpectation::NoFirmReference,
            };
            let row = &evidence.arguments[index];
            if row.argument != index || row.value != *argument || row.expectation != expectation {
                return Err(FactsError::invalid(
                    self.budget,
                    "method authored argument trace changed",
                ));
            }
        }
        let mut regions = Vec::new();
        for (argument, actual) in inferred_target.type_args.iter().enumerate() {
            let locations = ir::carrier_locations(actual, self.budget)
                .map_err(|e| source_error(self.budget, &e))?;
            for location in locations.iter() {
                if matches!(location.channel, ir::CarrierChannel::Fixed(_)) {
                    sr::push(
                        &mut regions,
                        ir::TypeActualRegion {
                            argument,
                            location: location.clone(),
                            source: ir::ActualRegion::Elided,
                        },
                        self.budget,
                    )?;
                }
            }
        }
        if regions.as_slice() != evidence.type_actual_regions.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "generic method changed original owner-region occurrences",
            ));
        }
        let carrier = self
            .original
            .facts
            .carrier_contract(ir::SignatureSubject::Call(&target))?;
        if carrier.ref_contract().as_ref() != inferred.ref_contract.as_ref() {
            return Err(FactsError::invalid(
                self.budget,
                "generic method lost exact declaration-owned carrier contract",
            ));
        }
        ir::validate_callee(
            &target,
            signature,
            ir::FactsCtx::Validated(&self.original.facts),
        )
        .map_err(|e| source_error(self.budget, &e))?;
        Ok(inferred.result)
    }
}
