//! Deref coercion at expected-type sites: `&S` where `S: Deref<Target = T>`
//! coerces to `&T`, repeatedly (`&Box<Box<T>>` -> `&T`), exactly like stock
//! rustc. Each step is an ordinary proven `Deref::deref` trait call.
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

/// Tagged source pointer for coercion step `step` (1-based) of an expression.
/// Expr pointers are 8-aligned; tag 1 is reserved for statements.
pub(super) fn site(pointer: usize, step: usize) -> usize {
    pointer | (step << 1)
}

impl type_check::Checker<'_> {
    /// Coerce checked `value: &S` to `expected: &T` through Deref, or `None`
    /// when no Deref chain reaches the expected referent.
    pub(super) fn deref_coercion(
        &mut self,
        source: &Expr,
        value: ir::ExprId,
        expected: &ir::Ty,
    ) -> Result<Option<ir::ExprId>, String> {
        let ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: want,
        } = expected
        else {
            return Ok(None);
        };
        // Built-in reference dereferencing copies the inner shared carrier,
        // not a fresh borrow of its pointee. The ordinary Deref expression
        // carries the original nested provenance into CFG ownership analysis.
        let mut builtin = value;
        for _ in 0..MAX_DEREF_STEPS {
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
        let ir::Ty::Ref { referent: have, .. } = &self.exprs[value.0].ty else {
            return Ok(None);
        };
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
        let Ok((declaration, ident)) = self.operator_trait("ops", "Deref", "deref") else {
            return Ok(None);
        };
        let mut current = value;
        for step in 1..=MAX_DEREF_STEPS + 1 {
            plan.budget.charge_work(1)?;
            let ir::Ty::Ref { referent, .. } = &self.exprs[current.0].ty else {
                return Ok(None);
            };
            if referent == want {
                return Ok(Some(current));
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
            let Some((target, signature, _)) =
                plan.select_trait(&trait_ref, &ident, &[], self.facts(), false, &plan.budget)?
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
        if !candidate(receiver) {
            return Ok(None);
        }
        let (owner, behind_ref) = match receiver_ty {
            ir::Ty::Ref { referent, .. } => (referent.as_ref().clone(), true),
            ty => (ty.clone(), false),
        };
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
