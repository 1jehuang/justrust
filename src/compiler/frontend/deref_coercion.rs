//! Deref coercion at expected-type sites: `&S` where `S: Deref<Target = T>`
//! coerces to `&T`, repeatedly (`&Box<Box<T>>` -> `&T`), exactly like stock
//! rustc. Each step is an ordinary proven `Deref::deref` or
//! `DerefMut::deref_mut` trait call, according to the required carrier.
use super::*;

/// Steps per occurrence; each has its own inventoried source site.
pub(super) const MAX_DEREF_STEPS: usize = 3;

/// Expressions that may yield a reference needing deref coercion.
pub(super) fn candidate(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Borrow { .. } | Expr::BorrowValue { .. } | Expr::Var(_) | Expr::Field { .. }
    )
}

pub(super) fn receiver_candidate(e: &Expr) -> bool {
    candidate(e) || type_check::genuine_call(e)
}

/// Tagged source pointer for coercion step `step` (1-based) of an expression.
/// Expr pointers are 8-aligned; tag 1 is reserved for statements.
pub(super) fn site(pointer: usize, step: usize) -> usize {
    pointer | (step << 1)
}

impl type_check::Checker<'_> {
    /// Coerce a checked reference through Deref or DerefMut, or `None`
    /// when no Deref chain reaches the expected referent.
    pub(super) fn deref_coercion(
        &mut self,
        source: &Expr,
        value: ir::ExprId,
        expected: &ir::Ty,
    ) -> Result<Option<ir::ExprId>, String> {
        let ir::Ty::Ref {
            kind,
            referent: want,
        } = expected
        else {
            return Ok(None);
        };
        // Built-in reference dereferencing copies the inner shared carrier,
        // not a fresh borrow of its pointee. The ordinary Deref expression
        // carries the original nested provenance into CFG ownership analysis.
        let mut builtin = value;
        for _ in 0..if *kind == ir::BorrowKind::Shared {
            MAX_DEREF_STEPS
        } else {
            0
        } {
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &self.exprs[builtin.0].ty
            else {
                break;
            };
            if referent.as_ref() == expected {
                let completion = self.exprs[builtin.0].completion;
                self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                let id = ir::ExprId(self.exprs.len());
                self.exprs.push(ir::TypedExpr {
                    ty: expected.clone(),
                    kind: ir::ExprKind::Deref { value: builtin },
                    completion,
                });
                return Ok(Some(id));
            }
            // This bounded prerequisite does not invent mutable reborrows
            // or a different referent type through a shared carrier.
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                ..
            } = referent.as_ref()
            else {
                break;
            };
            let ty = referent.as_ref().clone();
            let completion = self.exprs[builtin.0].completion;
            self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
            let id = ir::ExprId(self.exprs.len());
            self.exprs.push(ir::TypedExpr {
                ty,
                kind: ir::ExprKind::Deref { value: builtin },
                completion,
            });
            builtin = id;
        }
        let ir::Ty::Ref {
            kind: actual_kind,
            referent: have,
        } = &self.exprs[value.0].ty
        else {
            return Ok(None);
        };
        if *kind == ir::BorrowKind::Mut && *actual_kind != ir::BorrowKind::Mut {
            return Ok(None);
        }
        if !candidate(source)
            || have == want
            || !matches!(have.as_ref(), ir::Ty::Struct(_) | ir::Ty::Enum(_))
        {
            return Ok(None);
        }
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("deref coercion requires registry")?;
        let (trait_name, method_name) = if *kind == ir::BorrowKind::Mut {
            ("DerefMut", "deref_mut")
        } else {
            ("Deref", "deref")
        };
        let Ok((declaration, ident)) = self.operator_trait("ops", trait_name, method_name) else {
            return Ok(None);
        };
        let mut current = value;
        for step in 1..=MAX_DEREF_STEPS + 1 {
            plan.budget.charge_work(1)?;
            let ir::Ty::Ref {
                kind: current_kind,
                referent,
            } = &self.exprs[current.0].ty
            else {
                return Ok(None);
            };
            if *kind == ir::BorrowKind::Mut && *current_kind != ir::BorrowKind::Mut {
                return Ok(None);
            }
            if referent == want {
                return Ok((*current_kind == *kind).then_some(current));
            }
            if step > MAX_DEREF_STEPS
                || !matches!(referent.as_ref(), ir::Ty::Struct(_) | ir::Ty::Enum(_))
            {
                return Ok(None);
            }
            let trait_ref = ir::TraitRef {
                declaration,
                self_ty: referent.as_ref().clone(),
                args: Arc::from([]),
            };
            let Some((target, signature, _)) = plan.select_trait_with_scratch(
                &trait_ref,
                &ident,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                return Ok(None);
            };
            current = self.finish_checked_call(
                target,
                ident,
                signature,
                &[],
                Arc::from([]),
                Some((current, None)),
            )?;
            self.record_operator(site(source as *const Expr as usize, step), current)?;
        }
        Ok(None)
    }
}

impl type_check::Checker<'_> {
    /// One overloaded autoderef step of a method/field receiver: when the
    /// written receiver (or the nominal behind one reference) has no direct
    /// candidate, `recv.m()` / `recv.f` reads through its exact canonical
    /// `Deref::deref(&recv)` (or `DerefMut::deref_mut(&mut recv)`), recorded
    /// at the receiver's first inventoried coercion site. Returns the checked
    /// `&Target`/`&mut Target` carrier, or `None` when no impl applies.
    pub(super) fn receiver_overloaded_deref(
        &mut self,
        receiver: &Expr,
        receiver_ty: &ir::Ty,
        kind: ir::BorrowKind,
    ) -> Result<Option<ir::ExprId>, String> {
        if !receiver_candidate(receiver) {
            return Ok(None);
        }
        let (owner, behind_ref) = match receiver_ty {
            ir::Ty::Ref { referent, .. } => (referent.as_ref().clone(), true),
            ty => (ty.clone(), false),
        };
        if kind == ir::BorrowKind::Shared
            && let ir::Ty::Ref { .. } = &owner
        {
            // `&&..&N` receiver: builtin dereferences of the copied shared
            // carriers reach `&N`, then one overloaded `Deref::deref(&N)`.
            let mut id = self.expr_expected(receiver, Some(receiver_ty))?;
            loop {
                let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                    return Ok(None);
                };
                if !matches!(referent.as_ref(), ir::Ty::Ref { .. }) {
                    break;
                }
                let ty = referent.as_ref().clone();
                let completion = self.exprs[id.0].completion;
                self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
                let next = ir::ExprId(self.exprs.len());
                self.exprs.push(ir::TypedExpr {
                    kind: ir::ExprKind::Deref { value: id },
                    ty,
                    completion,
                });
                id = next;
            }
            let ir::Ty::Ref { referent, .. } = &self.exprs[id.0].ty else {
                return Ok(None);
            };
            let nominal = referent.as_ref().clone();
            if !matches!(nominal, ir::Ty::Struct(_) | ir::Ty::Enum(_))
                || self.deref_target_hint(&nominal)?.is_none()
            {
                return Ok(None);
            }
            let (declaration, name) = self.operator_trait("ops", "Deref", "deref")?;
            let plan = self.signatures.plan()?;
            let goal = ir::TraitRef {
                declaration,
                self_ty: nominal,
                args: Arc::from([]),
            };
            let Some((target, signature, _)) = plan.select_trait_with_scratch(
                &goal,
                &name,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                return Ok(None);
            };
            let call = self.finish_checked_call(
                target,
                name,
                signature,
                &[],
                Arc::from([]),
                Some((id, None)),
            )?;
            self.record_operator(site(receiver as *const Expr as usize, 1), call)?;
            return Ok(Some(call));
        }
        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_))
            || self.deref_target_hint(&owner)?.is_none()
        {
            return Ok(None);
        }
        let spec = if kind == ir::BorrowKind::Mut {
            ("ops", "DerefMut", "deref_mut")
        } else {
            ("ops", "Deref", "deref")
        };
        if !behind_ref && type_check::genuine_call(receiver) {
            // Method autoref keeps an owned call result alive through the full
            // expression, including its destructor. The original expression
            // remains the sole source occurrence for the recorded Deref call.
            let value = self.expr_expected(receiver, Some(&owner))?;
            let owner = self.exprs[value.0].ty.clone();
            let local = self.temporary(
                "$deref_receiver",
                owner.clone(),
                kind == ir::BorrowKind::Mut,
            )?;
            self.borrow_temporaries.push((local, false));
            let borrowed = self.borrow_expression(
                kind,
                ir::Place {
                    local,
                    projection: Vec::new(),
                },
            )?;
            let (declaration, name) = self.operator_trait(spec.0, spec.1, spec.2)?;
            let plan = self.signatures.plan()?;
            let goal = ir::TraitRef {
                declaration,
                self_ty: owner,
                args: Arc::from([]),
            };
            let Some((target, signature, _)) = plan.select_trait_with_scratch(
                &goal,
                &name,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                return Err("overloaded deref receiver has no selected implementation".into());
            };
            let call = self.finish_checked_call(
                target,
                name,
                signature,
                &[],
                Arc::from([]),
                Some((borrowed, None)),
            )?;
            self.record_operator(site(receiver as *const Expr as usize, 1), call)?;
            let ty = self.exprs[call.0].ty.clone();
            let completion = self.exprs[call.0].completion;
            self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 128)?;
            let result = ir::ExprId(self.exprs.len());
            self.exprs.push(ir::TypedExpr {
                ty,
                completion,
                kind: ir::ExprKind::Block(ir::TypedBlock {
                    statements: vec![ir::TypedStmt::Assign { local, value }],
                    tail: Some(call),
                }),
            });
            return Ok(Some(result));
        }
        let first = if behind_ref {
            if kind == ir::BorrowKind::Mut
                && !matches!(
                    receiver_ty,
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        ..
                    }
                )
            {
                return Err(
                    "E0596: cannot borrow data behind a shared reference as mutable".into(),
                );
            }
            super::operators::First::Value(receiver)
        } else {
            super::operators::First::Borrow(kind, receiver)
        };
        let call = self.operator_call(spec, owner, None, first, &[])?;
        self.record_operator(site(receiver as *const Expr as usize, 1), call)?;
        Ok(Some(call))
    }
}

impl type_check::Checker<'_> {
    /// Autoref of a field receiver read through an overloaded Deref
    /// (`rc.field.m()` where the field belongs to `Rc`'s target): the
    /// carrier `Deref::deref(&rc)` is saved once in a reference temporary and
    /// the receiver borrows `(*tmp).field`, as stock's autoderef place does.
    pub(super) fn deref_field_receiver(
        &mut self,
        receiver: &Expr,
        kind: ir::BorrowKind,
        selected: &MethodSelection,
    ) -> Result<Option<(ir::ExprId, Option<ir::ImplicitBorrowKind>)>, String> {
        let Expr::Field { base, field } = receiver else {
            return Ok(None);
        };
        let Some(hint) = self.method_receiver_hint(base)? else {
            return Ok(None);
        };
        let owner = match &hint {
            ir::Ty::Ref { referent, .. } => referent.as_ref(),
            ty => ty,
        };
        // Canonical Box dereferences are owning places, not an overloaded
        // Deref call saved in a reference temporary. checked_place preserves
        // this identity and emits the actual receiver autoref directly.
        if owner.owned_box_pointee().is_some() {
            return Ok(None);
        }
        let ir::Ty::Struct(nominal) = (match &hint {
            ir::Ty::Ref { referent, .. } => referent.as_ref(),
            ty => ty,
        }) else {
            return Ok(None);
        };
        if self.has_member(nominal.def().source_decl, field)? {
            return Ok(None);
        }
        let Some(carrier) = self.receiver_overloaded_deref(base, &hint, kind)? else {
            return Ok(None);
        };
        let carrier_ty = self.exprs[carrier.0].ty.clone();
        let ir::Ty::Ref { referent, .. } = &carrier_ty else {
            return Err("overloaded deref must return a reference".into());
        };
        let (index, _) = self.field_type(referent, field)?;
        let local = self.temporary("$deref_field", carrier_ty.clone(), false)?;
        self.borrow_temporaries.push((local, false));
        let derefs = selected.depth
            + usize::from(matches!(selected.adjustment, ReceiverAdjustment::Identity));
        self.charge_node((derefs + 2) * std::mem::size_of::<ir::Projection>() + 128)?;
        let mut projection = vec![ir::Projection::Deref, ir::Projection::Field(index)];
        projection.extend(std::iter::repeat_n(ir::Projection::Deref, derefs));
        let borrowed = self.borrow_expression(kind, ir::Place { local, projection })?;
        let ty = self.exprs[borrowed.0].ty.clone();
        let completion = self.exprs[carrier.0].completion;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign {
                    local,
                    value: carrier,
                }],
                tail: Some(borrowed),
            }),
        });
        Ok(Some((
            id,
            (kind == ir::BorrowKind::Mut).then_some(ir::ImplicitBorrowKind::MutableReceiver),
        )))
    }
}
