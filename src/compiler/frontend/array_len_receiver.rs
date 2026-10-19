//! Native array receiver adjustment into the existing shared slice operation.
use super::*;

pub(super) fn receiver_is_array(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Array { .. } => true,
        ir::Ty::Ref { referent, .. } => matches!(referent.as_ref(), ir::Ty::Array { .. }),
        _ => false,
    }
}

pub(super) fn explicit_mut_array_borrow(receiver: &Expr) -> bool {
    matches!(
        receiver,
        Expr::Borrow {
            kind: ir::BorrowKind::Mut,
            ..
        } | Expr::BorrowValue {
            kind: ir::BorrowKind::Mut,
            ..
        }
    )
}

impl type_check::Checker<'_> {
    /// Preserve the original explicit mutable borrow, then take an ordinary
    /// shared reborrow of its saved reference. The existing temporary protocol
    /// keeps that carrier alive through the complete enclosing statement.
    pub(super) fn shared_array_reference_temporary(
        &mut self,
        receiver: &Expr,
    ) -> Result<ir::ExprId, String> {
        if !explicit_mut_array_borrow(receiver) {
            return Err("array reference temporary requires an explicit mutable borrow".into());
        }
        let value = self.expr(receiver)?;
        let flow = self.flow(value);
        if !flow.completes {
            return Ok(value);
        }
        if !matches!(&flow.ty, ir::Ty::Ref { kind: ir::BorrowKind::Mut, referent }
            if matches!(referent.as_ref(), ir::Ty::Array { .. }))
        {
            return Err("array reference temporary changed checked type".into());
        }
        let local = self.temporary("$array_reference", flow.ty, true)?;
        self.charge_node(
            std::mem::size_of::<ir::LocalId>() + std::mem::size_of::<ir::Projection>(),
        )?;
        self.borrow_temporaries.push((local, false));
        let borrowed = self.borrow_expression(
            ir::BorrowKind::Shared,
            ir::Place {
                local,
                projection: vec![ir::Projection::Deref],
            },
        )?;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: self.exprs[borrowed.0].ty.clone(),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Block(ir::Block {
                statements: vec![ir::Stmt::Assign { local, value }],
                tail: Some(borrowed),
            }),
        });
        Ok(id)
    }

    pub(super) fn array_len_receiver(
        &mut self,
        receiver: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let Some(ty) = self.method_receiver_hint(receiver)? else {
            return Ok(None);
        };
        match ty {
            ir::Ty::Array { .. } => {
                let borrowed = if let Ok(source) = type_check::static_source_place(receiver) {
                    let (place, _, _) = self.checked_place(&source)?;
                    self.borrow_expression(ir::BorrowKind::Shared, place)?
                } else {
                    self.borrow_value(ir::BorrowKind::Shared, receiver, None)?
                };
                Ok(Some(borrowed))
            }
            ir::Ty::Ref { kind, referent } if matches!(referent.as_ref(), ir::Ty::Array { .. }) => {
                let borrowed = if kind == ir::BorrowKind::Shared {
                    self.expr(receiver)?
                } else if explicit_mut_array_borrow(receiver) {
                    self.shared_array_reference_temporary(receiver)?
                } else {
                    let source = type_check::static_source_place(receiver).map_err(|_| {
                        "unsupported: mutable array reference length requires a source place"
                            .to_string()
                    })?;
                    let (mut place, _, _) = self.checked_place(&source)?;
                    if place.projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("place projection nesting exceeds limit128".into());
                    }
                    self.charge_node(std::mem::size_of::<ir::Projection>())?;
                    place.projection.push(ir::Projection::Deref);
                    self.borrow_expression(ir::BorrowKind::Shared, place)?
                };
                Ok(Some(borrowed))
            }
            _ => Ok(None),
        }
    }
}
