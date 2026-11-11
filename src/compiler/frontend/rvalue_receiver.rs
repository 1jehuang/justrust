//! Borrowed method receivers whose base is an owned rvalue, not a place.
//! The value is stored once in a full-expression temporary (dropped at the
//! end of the enclosing statement, as in stock rustc) and the receiver is a
//! reborrow through that temporary after the selected autoderef steps.
use super::*;

impl type_check::Checker<'_> {
    pub(super) fn rvalue_reference_receiver(
        &mut self,
        receiver: &Expr,
        kind: ir::BorrowKind,
        selected: &MethodSelection,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::ExprId, Option<ir::ImplicitBorrowKind>), String> {
        let value = self.expr_expected(receiver, expected)?;
        let value_ty = self.exprs[value.0].ty.clone();
        let derefs = selected
            .depth
            .checked_add(usize::from(matches!(
                selected.adjustment,
                ReceiverAdjustment::Identity
            )))
            .ok_or_else(|| "resource limit: rvalue receiver depth overflow".to_string())?;
        if derefs >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        // Every dereferenced layer must permit the requested borrow kind.
        let mut layer = &value_ty;
        for _ in 0..derefs {
            let ir::Ty::Ref {
                kind: carrier,
                referent,
            } = layer
            else {
                return Err("invalid rvalue receiver dereference".into());
            };
            if kind == ir::BorrowKind::Mut && *carrier == ir::BorrowKind::Shared {
                return Err("E0596: cannot borrow as mutable through a shared reference".into());
            }
            layer = referent.as_ref();
        }
        // Mutating an owned temporary in place (depth 0) needs a mutable slot.
        let mutable = derefs == 0 && kind == ir::BorrowKind::Mut;
        let local = self.temporary("$rvalue_receiver", value_ty, mutable)?;
        self.register_borrow_temporary(local, false, None)?;
        self.charge_node(derefs * std::mem::size_of::<ir::Projection>() + 128)?;
        let projection = std::iter::repeat_n(ir::Projection::Deref, derefs).collect();
        let borrowed = self.borrow_expression(kind, ir::Place { local, projection })?;
        let ty = self.exprs[borrowed.0].ty.clone();
        let completion = self.exprs[value.0].completion;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign { local, value }],
                tail: Some(borrowed),
            }),
        });
        Ok((id, None))
    }
}
