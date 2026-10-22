//! Builtin scalar binary checking outside the large general expression frame.
use super::*;
impl Checker<'_> {
    pub(super) fn builtin_binary_expression(
        &mut self,
        source: &Expr,
        expected: Option<&ir::Ty>,
    ) -> Result<ir::ExprId, String> {
        let Expr::Binary {
            op, left, right, ..
        } = source
        else {
            return Err(
                "contract: builtin binary checker requires its original binary source".into(),
            );
        };
        let (kind, flow) = {
            let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
            let shifting = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
            let hint = if logical {
                Some(ir::Ty::Bool)
            } else {
                let mut hint = expected
                    .filter(|t| matches!(t, ir::Ty::Int(_) | ir::Ty::Float(_)))
                    .cloned();
                if hint.is_none() {
                    hint = self.scalar_hint(left)?.map(scalar_referent);
                }
                if hint.is_none() && !shifting {
                    hint = self.scalar_hint(right)?.map(scalar_referent);
                }
                hint
            };
            let left = self.expr_expected(left, hint.as_ref())?;
            let left_flow = self.flow(left);
            let right_hint = if shifting {
                None
            } else {
                hint.or_else(|| {
                    if matches!(
                        left_flow.ty,
                        ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool
                    ) {
                        Some(left_flow.ty.clone())
                    } else {
                        None
                    }
                })
            };
            let right = self.expr_expected_when(
                right,
                self.reachable && left_flow.completes,
                right_hint.as_ref(),
            )?;
            let (left, right) = self.scalar_reference_operands(*op, left, right)?;
            let l = self.flow(left);
            let r = self.flow(right);
            let str_comparison = l.ty.is_shared_str()
                && r.ty.is_shared_str()
                && matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                );
            if str_comparison {
                // Builtin `&str` comparison: byte-lexicographic, exactly
                // `<str as PartialOrd>` / `PartialEq` on UTF-8 contents.
                let completes = l.completes && r.completes;
                (
                    ir::ExprKind::Binary {
                        op: *op,
                        left,
                        right,
                    },
                    Flow {
                        ty: ir::Ty::Bool,
                        completes,
                    },
                )
            } else {
                if matches!(l.ty, ir::Ty::Ref { .. }) || matches!(r.ty, ir::Ty::Ref { .. }) {
                    return Err("reference comparisons and arithmetic are unsupported in B1".into());
                }
                if matches!(
                    l.ty,
                    ir::Ty::Tuple(_) | ir::Ty::Struct(_) | ir::Ty::Array { .. }
                ) || matches!(
                    r.ty,
                    ir::Ty::Tuple(_) | ir::Ty::Struct(_) | ir::Ty::Array { .. }
                ) {
                    return Err("aggregate operators and comparisons are unsupported".into());
                }
                let numeric = if matches!(l.ty, ir::Ty::Int(_) | ir::Ty::Float(_)) {
                    l.ty.clone()
                } else if matches!(r.ty, ir::Ty::Int(_) | ir::Ty::Float(_)) {
                    r.ty.clone()
                } else {
                    ir::Ty::Int(ir::IntTy::I32)
                };
                let (operand, result) = match op {
                    BinaryOp::And | BinaryOp::Or => (ir::Ty::Bool, ir::Ty::Bool),
                    BinaryOp::Shl | BinaryOp::Shr => {
                        if l.ty == ir::Ty::Never || r.ty == ir::Ty::Never {
                            return Err("shift operator on semantic Never is unsupported".into());
                        }
                        if !matches!(l.ty, ir::Ty::Int(_)) || !matches!(r.ty, ir::Ty::Int(_)) {
                            return Err("shift operands must both be integers".into());
                        }
                        (l.ty.clone(), l.ty.clone())
                    }
                    BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                        let operand = self.join(&l.ty, &r.ty, "bitwise operands")?;
                        if !matches!(operand, ir::Ty::Int(_) | ir::Ty::Bool) {
                            return Err(
                                "bitwise operands require identical integer types or Bool".into()
                            );
                        }
                        (operand.clone(), operand)
                    }
                    BinaryOp::Eq | BinaryOp::Ne => {
                        (self.join(&l.ty, &r.ty, "equality operands")?, ir::Ty::Bool)
                    }
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => (
                        if l.ty == ir::Ty::Char || r.ty == ir::Ty::Char {
                            ir::Ty::Char
                        } else {
                            numeric.clone()
                        },
                        ir::Ty::Bool,
                    ),
                    _ => (numeric.clone(), numeric),
                };
                self.require(&l, &operand)?;
                if !shifting {
                    self.require(&r, &operand)?;
                }
                // Trait-based scalar operators do not gain a never coercion
                // merely because evaluating an operand cannot complete.
                if !logical && (l.ty == ir::Ty::Never || r.ty == ir::Ty::Never) {
                    return Err("binary operator on never is unsupported".into());
                }
                let completes = l.completes && (logical || r.completes);
                (
                    ir::ExprKind::Binary {
                        op: *op,
                        left,
                        right,
                    },
                    Flow {
                        ty: result,
                        completes,
                    },
                )
            }
        };
        self.finish_checked_expression(source, kind, flow)
    }
}

fn scalar(ty: &ir::Ty) -> bool {
    matches!(
        ty,
        ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool | ir::Ty::Char
    )
}

impl Checker<'_> {
    /// Stock forwarding impls for primitive operators: arithmetic, bitwise and
    /// shift operators accept one shared reference layer on either operand
    /// (`impl Add<&i32> for &i32`, ...). Comparisons accept equal reference
    /// depths on both sides (`impl PartialEq<&B> for &A`). Each accepted layer
    /// is an explicit read through the reference; nothing else is admitted.
    fn scalar_reference_operands(
        &mut self,
        op: BinaryOp,
        left: ir::ExprId,
        right: ir::ExprId,
    ) -> Result<(ir::ExprId, ir::ExprId), String> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return Ok((left, right));
        }
        let comparison = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        if comparison {
            let (mut l, mut r) = (left, right);
            for _ in 0..MAX_INTRINSIC_AUTODEREF {
                let (ir::Ty::Ref { referent: a, .. }, ir::Ty::Ref { referent: b, .. }) =
                    (&self.exprs[l.0].ty, &self.exprs[r.0].ty)
                else {
                    break;
                };
                let (a, b) = (a.as_ref().clone(), b.as_ref().clone());
                l = self.read_through_reference(l, a)?;
                r = self.read_through_reference(r, b)?;
            }
            // Only rewrite when the peeled operands are scalars; any other
            // shape keeps its original operands and existing diagnostics.
            if (l, r) != (left, right) && scalar(&self.exprs[l.0].ty) && scalar(&self.exprs[r.0].ty)
            {
                return Ok((l, r));
            }
            return Ok((left, right));
        }
        let mut peel = |checker: &mut Self, id: ir::ExprId| -> Result<ir::ExprId, String> {
            if let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &checker.exprs[id.0].ty
                && matches!(
                    referent.as_ref(),
                    ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool
                )
            {
                let referent = referent.as_ref().clone();
                return checker.read_through_reference(id, referent);
            }
            Ok(id)
        };
        let l = peel(self, left)?;
        let r = peel(self, right)?;
        Ok((l, r))
    }

    fn read_through_reference(
        &mut self,
        value: ir::ExprId,
        ty: ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let completion = self.exprs[value.0].completion;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            kind: ir::ExprKind::Deref { value },
            completion,
        });
        Ok(id)
    }
}

/// A literal operand beside `&scalar` takes the referent's type, as the
/// forwarding impl `impl Op<T> for &T` dictates. Other hints are unchanged.
fn scalar_referent(ty: ir::Ty) -> ir::Ty {
    match &ty {
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent,
        } if matches!(referent.as_ref(), ir::Ty::Int(_) | ir::Ty::Float(_)) => {
            referent.as_ref().clone()
        }
        _ => ty,
    }
}
