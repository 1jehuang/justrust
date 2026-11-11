//! `s[i][j] (op)= x` where `s` reaches a builtin slice `[R]` (for example
//! `s: &mut [Vec<T>]`) and the row `R` is indexed through an overloaded
//! `IndexMut`. Stock order: the right operand, then the builtin row bounds
//! check, then `IndexMut::index_mut(&mut (*s)[i], j)`, then the write through
//! the returned `&mut`. The row is borrowed in place, never copied.
use super::*;

impl type_check::Checker<'_> {
    pub(super) fn slice_row_index_assignment(
        &mut self,
        statement: &Stmt,
        statements: &mut Vec<ir::TypedStmt>,
    ) -> Result<Option<(ir::TypedStmt, bool)>, String> {
        let (site, place, op, value) = match statement {
            Stmt::AssignPlace { site, place, value } => (*site, place, None, value),
            Stmt::AssignOp {
                site,
                place,
                op,
                value,
            } => (*site, place, Some(*op), value),
            _ => return Ok(None),
        };
        let fields = &place.fields;
        // 1. The first constant index whose owner is a builtin slice reached
        // through references. Array owners keep the ordinary place rules.
        let mut split = None;
        for (at, selector) in fields.iter().enumerate() {
            self.signatures.plan()?.budget.charge_work(1)?;
            let FieldSelector::ConstantIndex(index) = selector else {
                continue;
            };
            let prefix = Place {
                local: place.local,
                fields: fields[..at].to_vec(),
            };
            let Ok((mut slice_place, mut ty, mut writable)) = self.checked_place(&prefix) else {
                return Ok(None);
            };
            let mut derefs = prefix
                .fields
                .iter()
                .filter(|selector| matches!(selector, FieldSelector::Deref))
                .count();
            while let ir::Ty::Ref { kind, referent } = &ty {
                if derefs >= ir::MAX_PATTERN_PROJECTION_DEPTH
                    || slice_place.projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH
                {
                    return Err("place projection nesting exceeds limit128".into());
                }
                writable = (derefs == 0 || writable) && *kind == ir::BorrowKind::Mut;
                derefs += 1;
                slice_place.projection.push(ir::Projection::Deref);
                ty = referent.as_ref().clone();
            }
            match ty {
                ir::Ty::Slice { element } => {
                    split = Some((at, *index, slice_place, element.as_ref().clone(), writable));
                    break;
                }
                ir::Ty::Array { .. } => continue,
                _ => return Ok(None),
            }
        }
        let Some((at, row_index, slice_place, element, slice_writable)) = split else {
            return Ok(None);
        };
        // 2. Selectors inside the row up to the overloaded index.
        // Builtin array indexes inside the row stay place projections; the
        // first index whose owner is nominal is the overloaded `IndexMut`.
        let rest = &fields[at + 1..];
        let row_carrier = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(element),
        };
        let mut found = None;
        for (position, selector) in rest.iter().enumerate() {
            self.signatures.plan()?.budget.charge_work(1)?;
            let FieldSelector::ConstantIndex(inner_index) = selector else {
                continue;
            };
            let mut row_fields = Vec::with_capacity(position + 2);
            row_fields.push(FieldSelector::Deref);
            row_fields.extend(rest[..position].iter().cloned());
            let (mut row_projection, mut owner, mut row_writable) =
                self.call_place_projection(&row_carrier, &row_fields)?;
            if let ir::Ty::Ref { kind, referent } = &owner
                && super::operators::overloadable(referent)
                && !matches!(referent.as_ref(), ir::Ty::Tuple(_))
            {
                row_writable &= *kind == ir::BorrowKind::Mut;
                row_projection.push(ir::Projection::Deref);
                owner = referent.as_ref().clone();
            }
            if super::operators::overloadable(&owner) && !matches!(owner, ir::Ty::Tuple(_)) {
                found = Some((position, *inner_index, row_projection, owner, row_writable));
                break;
            }
            if !matches!(owner, ir::Ty::Array { .. }) {
                return Ok(None);
            }
        }
        let Some((inner, inner_index, row_projection, owner, row_writable)) = found else {
            return Ok(None);
        };
        let index = Expr::Integer(IntegerLiteral {
            magnitude: inner_index as u128,
            suffix: None,
        });
        let Some(reference) = self.index_reference_hint(&owner, &index)? else {
            return Ok(None);
        };
        if !(slice_writable && row_writable) {
            return Err(
                "E0596: cannot borrow a slice element as mutable: it is behind a `&` reference"
                    .into(),
            );
        }
        let ir::Ty::Ref { referent, .. } = reference else {
            return Err("Index must return a reference".into());
        };
        let carrier_ty = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent,
        };
        let mut tail = Vec::with_capacity(rest.len() - inner);
        tail.push(FieldSelector::Deref);
        tail.extend(rest[inner + 1..].iter().cloned());
        let (projection, destination, writable) = self.call_place_projection(&carrier_ty, &tail)?;
        if !writable {
            return Err("E0594: cannot assign to data behind a shared reference".into());
        }
        // 3. Right operand first.
        let (rhs, rhs_flow) = if let Some(op) = op {
            self.compound_value(op, &destination, value)?
        } else {
            let rhs = self.expr_expected(value, Some(&destination))?;
            let flow = self.flow(rhs);
            self.require(&flow, &destination)?;
            (rhs, flow)
        };
        let rhs_ty = self.exprs[rhs.0].ty.clone();
        let saved = self.temporary(
            "$index_rhs",
            if rhs_ty == ir::Ty::Never {
                destination.clone()
            } else {
                rhs_ty.clone()
            },
            false,
        )?;
        self.charge_node(
            std::mem::size_of::<ir::TypedStmt>() * 4 + std::mem::size_of::<ir::TypedExpr>() * 2,
        )?;
        statements.push(ir::TypedStmt::Let {
            local: saved,
            value: rhs,
        });
        // 4. The builtin row index, bounds-checked before the row is borrowed.
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let literal = Expr::Integer(IntegerLiteral {
            magnitude: row_index as u128,
            suffix: Some(ir::IntTy::Usize),
        });
        let row_value = self.expr(&literal)?;
        let saved_index = self.temporary(
            &format!("$bi{}", self.locals.len()),
            usize_ty.clone(),
            false,
        )?;
        statements.push(ir::TypedStmt::Let {
            local: saved_index,
            value: row_value,
        });
        let checked = self.for_expr(usize_ty, ir::ExprKind::Local(saved_index))?;
        let probe = self.for_expr(
            ir::Ty::Unit,
            ir::ExprKind::BoundsCheck {
                array: slice_place.clone(),
                index: checked,
            },
        )?;
        if let Some(site) = site {
            self.record_runtime_span(site, probe)?;
        }
        statements.push(ir::TypedStmt::Expr(probe));
        let mut row_place = slice_place;
        if row_place.projection.len() + row_projection.len() > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        row_place
            .projection
            .push(ir::Projection::DynamicIndex(saved_index));
        // `call_place_projection` starts from the row carrier's own Deref.
        row_place
            .projection
            .extend(row_projection.into_iter().skip(1));
        // 5. IndexMut on the borrowed row, then the write.
        let receiver = self.borrow_expression(ir::BorrowKind::Mut, row_place)?;
        let call = self.operator_call(
            ("ops", "IndexMut", "index_mut"),
            owner,
            Some(ir::Ty::Int(ir::IntTy::Usize)),
            super::operators::First::Lowered(receiver),
            std::slice::from_ref(&index),
        )?;
        self.record_operator(statement as *const Stmt as usize | 1, call)?;
        let completes = rhs_flow.completes && self.flow(call).completes;
        let carrier = self.temporary("$index_carrier", self.exprs[call.0].ty.clone(), false)?;
        statements.push(ir::TypedStmt::Let {
            local: carrier,
            value: call,
        });
        let loaded = self.for_expr(rhs_ty, ir::ExprKind::Local(saved))?;
        let place = ir::Place {
            local: carrier,
            projection,
        };
        let result = match op {
            Some(op) => ir::TypedStmt::AssignOp {
                place,
                op,
                value: loaded,
            },
            None => ir::TypedStmt::AssignPlace {
                place,
                value: loaded,
            },
        };
        Ok(Some((result, completes)))
    }
}
