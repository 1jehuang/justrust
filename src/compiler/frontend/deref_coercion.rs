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
