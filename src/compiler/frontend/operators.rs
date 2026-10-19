//! Overloaded operators. A binary, unary or compound-assignment operator whose
//! operand is not a primitive lowers to a call of the matching prelude
//! `std::ops` / `std::cmp` trait method, selected and proven like any other
//! trait call (`<L as Add<R>>::add(l, r)`, `PartialEq::eq(&l, &r)`, ...).
use super::*;

fn overloadable(ty: &ir::Ty) -> bool {
    matches!(
        ty,
        ir::Ty::Struct(_) | ir::Ty::Enum(_) | ir::Ty::Tuple(_) | ir::Ty::Param(_)
    )
}

fn binary_trait(op: BinaryOp) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match op {
        BinaryOp::Add => ("ops", "Add", "add"),
        BinaryOp::Sub => ("ops", "Sub", "sub"),
        BinaryOp::Mul => ("ops", "Mul", "mul"),
        BinaryOp::Div => ("ops", "Div", "div"),
        BinaryOp::Rem => ("ops", "Rem", "rem"),
        BinaryOp::BitAnd => ("ops", "BitAnd", "bitand"),
        BinaryOp::BitOr => ("ops", "BitOr", "bitor"),
        BinaryOp::BitXor => ("ops", "BitXor", "bitxor"),
        BinaryOp::Shl => ("ops", "Shl", "shl"),
        BinaryOp::Shr => ("ops", "Shr", "shr"),
        BinaryOp::Eq => ("cmp", "PartialEq", "eq"),
        BinaryOp::Ne => ("cmp", "PartialEq", "ne"),
        BinaryOp::Lt => ("cmp", "PartialOrd", "lt"),
        BinaryOp::Le => ("cmp", "PartialOrd", "le"),
        BinaryOp::Gt => ("cmp", "PartialOrd", "gt"),
        BinaryOp::Ge => ("cmp", "PartialOrd", "ge"),
        BinaryOp::And | BinaryOp::Or => return None,
    })
}

fn assign_trait(op: BinaryOp) -> Option<(&'static str, &'static str)> {
    Some(match op {
        BinaryOp::Add => ("AddAssign", "add_assign"),
        BinaryOp::Sub => ("SubAssign", "sub_assign"),
        BinaryOp::Mul => ("MulAssign", "mul_assign"),
        BinaryOp::Div => ("DivAssign", "div_assign"),
        BinaryOp::Rem => ("RemAssign", "rem_assign"),
        BinaryOp::BitAnd => ("BitAndAssign", "bitand_assign"),
        BinaryOp::BitOr => ("BitOrAssign", "bitor_assign"),
        BinaryOp::BitXor => ("BitXorAssign", "bitxor_assign"),
        BinaryOp::Shl => ("ShlAssign", "shl_assign"),
        BinaryOp::Shr => ("ShrAssign", "shr_assign"),
        _ => return None,
    })
}

/// Shared borrow of an operand, as a place borrow when it is one.
fn borrow_operand(kind: ir::BorrowKind, value: &Expr) -> Expr {
    match type_check::static_source_place(value) {
        Ok(place) if type_check::place_expression(value) => Expr::Borrow { kind, place },
        _ => Expr::BorrowValue {
            kind,
            value: Box::new(value.clone()),
        },
    }
}

enum First<'e> {
    Value(&'e Expr),
    Borrow(ir::BorrowKind, &'e Expr),
    Place(ir::BorrowKind, &'e Place),
}

impl type_check::Checker<'_> {
    /// Prelude trait `std::<module>::<name>` and its method `method`.
    pub(super) fn operator_trait(
        &self,
        module: &str,
        name: &str,
        method: &str,
    ) -> Result<(ir::DeclId, AstIdent), String> {
        let plan = self.signatures.plan()?;
        let names = plan.program.names.as_ref();
        let suffix = format!("::{module}::{name}");
        for id in &plan.registry.trait_ids {
            let info = plan.registry.declaration(*id)?;
            if !info.name.ends_with(&suffix) || !plan.program.in_prelude(info.path.module) {
                continue;
            }
            let source = plan.registry.source_trait(*id)?;
            for item in &source.items {
                let spelling = names
                    .spelling(item.signature.name)
                    .map_err(native_adapter::name_diagnostic)?;
                if spelling == method {
                    return Ok((*id, item.signature.name));
                }
            }
        }
        Err(format!(
            "unsupported: operator trait std::{module}::{name} is not available"
        ))
    }

    fn operator_call(
        &mut self,
        (module, name, method): (&str, &str, &str),
        self_ty: ir::Ty,
        rhs_ty: Option<ir::Ty>,
        first: First<'_>,
        rest: &[Expr],
    ) -> Result<ir::ExprId, String> {
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("operator call requires registry")?;
        let (declaration, ident) = self.operator_trait(module, name, method)?;
        let arity = plan.registry.declaration(declaration)?.param_env.arity();
        // Untyped right operands (integer/float literals) try `Self`, then the
        // literal's fallback type, like stock inference with one candidate.
        let candidates: Vec<Vec<ir::Ty>> = match (arity, rhs_ty) {
            (1, _) => vec![Vec::new()],
            // An unsuffixed integer index variable infers from the selected
            // impl (usually `usize`); its hint is only the i32 fallback.
            (2, Some(rhs)) if name == "Index" && rhs == ir::Ty::Int(ir::IntTy::I32) => {
                vec![vec![rhs], vec![ir::Ty::Int(ir::IntTy::Usize)]]
            }
            (2, Some(rhs)) => vec![vec![rhs]],
            (2, None) => {
                let fallback = match rest.last() {
                    Some(Expr::Float(_)) => ir::Ty::Float(ir::FloatTy::F64),
                    _ => ir::Ty::Int(ir::IntTy::I32),
                };
                vec![vec![self_ty.clone()], vec![fallback]]
            }
            _ => return Err("operator trait arity mismatch".into()),
        };
        for trait_args in candidates {
            let trait_ref = ir::TraitRef {
                declaration,
                self_ty: self_ty.clone(),
                args: trait_args.into(),
            };
            if let Some((target, signature, _)) =
                plan.select_trait(&trait_ref, &ident, &[], self.facts(), false, &plan.budget)?
            {
                // The first operand is checked as an already-evaluated receiver
                // so every source operand keeps its original occurrence.
                let first = match first {
                    First::Value(value) => self.expr_expected(value, signature.params.first())?,
                    First::Borrow(kind, value) => {
                        if let Ok(place) = type_check::static_source_place(value)
                            && type_check::place_expression(value)
                        {
                            let (place, _, _) = self.checked_place(&place)?;
                            self.borrow_expression(kind, place)?
                        } else {
                            self.borrow_value(kind, value, None)?
                        }
                    }
                    First::Place(kind, place) => {
                        let (place, _, _) = self.checked_place(place)?;
                        self.borrow_expression(kind, place)?
                    }
                };
                let receiver = Some((first, None));
                return self.finish_checked_call(
                    target,
                    ident,
                    signature,
                    rest,
                    Arc::from([]),
                    receiver,
                );
            }
        }
        Err(format!(
            "E0369: no implementation of std::{module}::{name} for the operand type"
        ))
    }

    /// Overloaded binary/unary operator, or `None` for primitive operands.
    pub(super) fn overloaded_operator(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        match e {
            Expr::Binary { op, left, right } => {
                let Some(spec) = binary_trait(*op) else {
                    return Ok(None);
                };
                let Some(lhs) = self.method_receiver_hint(left)? else {
                    return Ok(None);
                };
                if !overloadable(&lhs) {
                    return Ok(None);
                }
                let rhs = self.method_receiver_hint(right)?;
                let id = if spec.0 == "cmp" {
                    let rest = [borrow_operand(ir::BorrowKind::Shared, right)];
                    let first = First::Borrow(ir::BorrowKind::Shared, left);
                    self.operator_call(spec, lhs, rhs, first, &rest)?
                } else {
                    let rest = std::slice::from_ref(right.as_ref());
                    self.operator_call(spec, lhs, rhs, First::Value(left), rest)?
                };
                self.record_operator(e as *const Expr as usize, id)?;
                Ok(Some(id))
            }
            Expr::Unary { op, value } => {
                let Some(ty) = self.method_receiver_hint(value)? else {
                    return Ok(None);
                };
                if !overloadable(&ty) {
                    return Ok(None);
                }
                let spec = match op {
                    UnaryOp::Neg => ("ops", "Neg", "neg"),
                    UnaryOp::Not => ("ops", "Not", "not"),
                };
                let id = self.operator_call(spec, ty, None, First::Value(value), &[])?;
                self.record_operator(e as *const Expr as usize, id)?;
                Ok(Some(id))
            }
            Expr::Deref { value } => {
                let Some(ty) = self.method_receiver_hint(value)? else {
                    return Ok(None);
                };
                if !overloadable(&ty) {
                    return Ok(None);
                }
                let first = First::Borrow(ir::BorrowKind::Shared, value);
                let call = self.operator_call(("ops", "Deref", "deref"), ty, None, first, &[])?;
                self.record_operator(e as *const Expr as usize, call)?;
                self.deref_overloaded_result(call).map(Some)
            }
            Expr::DynamicIndex { base, index } => {
                let Some(ty) = self.method_receiver_hint(base)? else {
                    return Ok(None);
                };
                if !overloadable(&ty) {
                    return Ok(None);
                }
                let idx = self
                    .method_receiver_hint(index)?
                    .unwrap_or(ir::Ty::Int(ir::IntTy::Usize));
                let first = First::Borrow(ir::BorrowKind::Shared, base);
                let rest = std::slice::from_ref(index.as_ref());
                let call =
                    self.operator_call(("ops", "Index", "index"), ty, Some(idx), first, rest)?;
                self.record_operator(e as *const Expr as usize, call)?;
                self.deref_overloaded_result(call).map(Some)
            }
            Expr::Index { base, index } => {
                let Some(ty) = self.method_receiver_hint(base)? else {
                    return Ok(None);
                };
                if !overloadable(&ty) || matches!(ty, ir::Ty::Tuple(_)) {
                    return Ok(None);
                }
                let literal = Expr::Integer(IntegerLiteral {
                    magnitude: *index as u128,
                    suffix: None,
                });
                let first = First::Borrow(ir::BorrowKind::Shared, base);
                let call = self.operator_call(
                    ("ops", "Index", "index"),
                    ty,
                    Some(ir::Ty::Int(ir::IntTy::Usize)),
                    first,
                    std::slice::from_ref(&literal),
                )?;
                self.record_operator(e as *const Expr as usize, call)?;
                self.deref_overloaded_result(call).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// `*call` for an overloaded place operator returning `&Target`.
    fn deref_overloaded_result(&mut self, call: ir::ExprId) -> Result<ir::ExprId, String> {
        let ir::Ty::Ref { referent, .. } = &self.exprs[call.0].ty else {
            return Err("overloaded place operator must return a reference".into());
        };
        let ty = referent.as_ref().clone();
        let completion = self.exprs[call.0].completion;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            kind: ir::ExprKind::Deref { value: call },
            completion,
        });
        Ok(id)
    }

    /// `place OP= value` on a non-primitive place: `OpAssign::op_assign(&mut place, value)`.
    pub(super) fn overloaded_assign(
        &mut self,
        statement: &Stmt,
        place: &Place,
        op: BinaryOp,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let (_, ty, _) = self.checked_place(place)?;
        if !overloadable(&ty) {
            return Ok(None);
        }
        let Some((name, method)) = assign_trait(op) else {
            return Ok(None);
        };
        let rhs = self.method_receiver_hint(value)?;
        let first = First::Place(ir::BorrowKind::Mut, place);
        let id = self.operator_call(
            ("ops", name, method),
            ty,
            rhs,
            first,
            std::slice::from_ref(value),
        )?;
        self.record_operator(statement as *const Stmt as usize | 1, id)?;
        Ok(Some(id))
    }

    pub(super) fn record_operator(&self, pointer: usize, id: ir::ExprId) -> Result<(), String> {
        if let Some(cap) = self.canonical {
            cap.record_call_at(pointer, id, &self.exprs[id.0].kind)?;
        }
        Ok(())
    }
}
