//! Overloaded operators. A binary, unary or compound-assignment operator whose
//! operand is not a primitive lowers to a call of the matching prelude
//! `std::ops` / `std::cmp` trait method, selected and proven like any other
//! trait call (`<L as Add<R>>::add(l, r)`, `PartialEq::eq(&l, &r)`, ...).
use super::*;

#[path = "operators/hints.rs"]
mod hints;

fn overloadable(ty: &ir::Ty) -> bool {
    matches!(
        ty,
        ir::Ty::Struct(_) | ir::Ty::Enum(_) | ir::Ty::Tuple(_) | ir::Ty::Param(_)
    )
}

fn constructor_operand(e: &Expr) -> bool {
    let mut base = e;
    while let Expr::Apply { value, .. } = base {
        base = value;
    }
    matches!(
        base,
        Expr::Struct { .. } | Expr::TupleStruct { .. } | Expr::EnumVariant { .. }
    )
}

pub(super) fn binary_trait(op: BinaryOp) -> Option<(&'static str, &'static str, &'static str)> {
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

pub(super) fn assign_trait(op: BinaryOp) -> Option<(&'static str, &'static str)> {
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

pub(super) enum First<'e> {
    Value(&'e Expr),
    Borrow(ir::BorrowKind, &'e Expr),
    Place(ir::BorrowKind, &'e Place),
}

impl type_check::Checker<'_> {
    /// Find the immutable Index signature without evaluating an operand. This
    /// supplies the same element type to formatting and generic inference that
    /// the eventual checked operator call supplies to expression lowering.
    pub(super) fn index_reference_hint(
        &self,
        owner: &ir::Ty,
        index: &Expr,
    ) -> Result<Option<ir::Ty>, String> {
        if !overloadable(owner) {
            return Ok(None);
        }
        let (declaration, name) = self.operator_trait("ops", "Index", "index")?;
        let plan = self.signatures.plan()?;
        let index_ty = self
            .method_receiver_hint(index)?
            .unwrap_or(ir::Ty::Int(ir::IntTy::Usize));
        let candidates = if index_ty == ir::Ty::Int(ir::IntTy::I32) {
            vec![index_ty, ir::Ty::Int(ir::IntTy::Usize)]
        } else {
            vec![index_ty]
        };
        for index_ty in candidates {
            let reference = ir::TraitRef {
                declaration,
                self_ty: owner.clone(),
                args: vec![index_ty].into(),
            };
            if let Some((_, signature, _)) =
                plan.select_trait(&reference, &name, &[], self.facts(), false, &plan.budget)?
            {
                return Ok(Some(signature.result));
            }
        }
        Ok(None)
    }

    /// `Deref::Target` of a nominal owner through the exact canonical impl,
    /// or `None` when no impl is selected. Grants no type authority; the
    /// checked `*x` reselects and discharges the same trait call.
    pub(super) fn deref_target_hint(&self, owner: &ir::Ty) -> Result<Option<ir::Ty>, String> {
        if !overloadable(owner) || matches!(owner, ir::Ty::Tuple(_)) {
            return Ok(None);
        }
        let Ok((declaration, name)) = self.operator_trait("ops", "Deref", "deref") else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let reference = ir::TraitRef {
            declaration,
            self_ty: owner.clone(),
            args: Arc::from([]),
        };
        Ok(
            match plan.select_trait(&reference, &name, &[], self.facts(), false, &plan.budget)? {
                Some((_, signature, _)) => match signature.result {
                    ir::Ty::Ref { referent, .. } => Some(referent.as_ref().clone()),
                    _ => None,
                },
                None => None,
            },
        )
    }

    pub(super) fn static_index_parts(place: &Place) -> Option<(Place, Expr)> {
        let (FieldSelector::ConstantIndex(index), fields) = place.fields.split_last()? else {
            return None;
        };
        Some((
            Place {
                local: place.local,
                fields: fields.to_vec(),
            },
            Expr::Integer(IntegerLiteral {
                magnitude: *index as u128,
                suffix: None,
            }),
        ))
    }

    pub(super) fn static_index_borrow_hint(&self, place: &Place) -> Result<Option<ir::Ty>, String> {
        let Some((base, index)) = Self::static_index_parts(place) else {
            return Ok(None);
        };
        let (_, owner, _) = self.checked_place(&base)?;
        self.index_reference_hint(&owner, &index)
    }

    /// Preserve the reference returned by Index rather than copying its
    /// referent into a borrowed temporary. Mutable borrows use IndexMut.
    pub(super) fn overloaded_index_borrow(
        &mut self,
        expression: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let (kind, value) = match expression {
            Expr::Borrow { kind, place }
                if matches!(place.fields.last(), Some(FieldSelector::Deref)) =>
            {
                // `&*p` / `&mut *p` on a nominal `p` is exactly
                // `Deref::deref(&p)` / `DerefMut::deref_mut(&mut p)`.
                let base = Place {
                    local: place.local,
                    fields: place.fields[..place.fields.len() - 1].to_vec(),
                };
                let (_, owner, _) = self.checked_place(&base)?;
                if !overloadable(&owner) || matches!(owner, ir::Ty::Tuple(_)) {
                    return Ok(None);
                }
                let spec = if *kind == ir::BorrowKind::Mut {
                    ("ops", "DerefMut", "deref_mut")
                } else {
                    ("ops", "Deref", "deref")
                };
                let call =
                    self.operator_call(spec, owner, None, First::Place(*kind, &base), &[])?;
                self.record_operator(expression as *const Expr as usize, call)?;
                return Ok(Some(call));
            }
            Expr::Borrow { kind, place } => {
                let Some((base, index)) = Self::static_index_parts(place) else {
                    return Ok(None);
                };
                let (_, owner, _) = self.checked_place(&base)?;
                if !overloadable(&owner) {
                    return Ok(None);
                }
                let spec = if *kind == ir::BorrowKind::Mut {
                    ("ops", "IndexMut", "index_mut")
                } else {
                    ("ops", "Index", "index")
                };
                let call = self.operator_call(
                    spec,
                    owner,
                    Some(ir::Ty::Int(ir::IntTy::Usize)),
                    First::Place(*kind, &base),
                    &[index],
                )?;
                self.record_operator(expression as *const Expr as usize, call)?;
                return Ok(Some(call));
            }
            Expr::BorrowValue { kind, value } => (*kind, value.as_ref()),
            _ => return Ok(None),
        };
        self.overloaded_index_reference(kind, value, expression as *const Expr as usize)
    }

    pub(super) fn overloaded_index_reference(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
        pointer: usize,
    ) -> Result<Option<ir::ExprId>, String> {
        let (base, literal, index) = match value {
            Expr::DynamicIndex { base, index, .. } => (base.as_ref(), None, Some(index.as_ref())),
            Expr::Index { base, index, .. } => (
                base.as_ref(),
                Some(Expr::Integer(IntegerLiteral {
                    magnitude: *index as u128,
                    suffix: None,
                })),
                None,
            ),
            _ => return Ok(None),
        };
        let index = index.or(literal.as_ref()).unwrap();
        let Some(owner) = self.method_receiver_hint(base)? else {
            return Ok(None);
        };
        if !overloadable(&owner) {
            return Ok(None);
        }
        let index_ty = self
            .method_receiver_hint(index)?
            .unwrap_or(ir::Ty::Int(ir::IntTy::Usize));
        let spec = if kind == ir::BorrowKind::Mut {
            ("ops", "IndexMut", "index_mut")
        } else {
            ("ops", "Index", "index")
        };
        let call = self.operator_call(
            spec,
            owner,
            Some(index_ty),
            First::Borrow(kind, base),
            std::slice::from_ref(index),
        )?;
        self.record_operator(pointer, call)?;
        Ok(Some(call))
    }

    pub(super) fn overloaded_index_assignment(
        &mut self,
        statement: &Stmt,
        statements: &mut Vec<ir::TypedStmt>,
    ) -> Result<Option<(ir::TypedStmt, bool)>, String> {
        let (base, literal, index, op, value) = match statement {
            Stmt::AssignIndex {
                array,
                index,
                value,
                ..
            } => (array.clone(), None, Some(index), None, value),
            Stmt::AssignIndexOp {
                array,
                index,
                op,
                value,
                ..
            } => (array.clone(), None, Some(index), Some(*op), value),
            Stmt::AssignPlace { place, value, .. } => {
                let Some((base, index)) = Self::static_index_parts(place) else {
                    return Ok(None);
                };
                (base, Some(index), None, None, value)
            }
            Stmt::AssignOp {
                place, op, value, ..
            } => {
                let Some((base, index)) = Self::static_index_parts(place) else {
                    return Ok(None);
                };
                (base, Some(index), None, Some(*op), value)
            }
            _ => return Ok(None),
        };
        let index = index.or(literal.as_ref()).unwrap();
        let (_, mut owner, mut writable) = self.checked_place(&base)?;
        // Stock index autoderef: `r[i] = x` with `r: &mut Vec<T>` assigns
        // through `IndexMut` on `*r`.
        let mut base = base;
        if let ir::Ty::Ref { kind, referent } = &owner
            && overloadable(referent)
            && !matches!(referent.as_ref(), ir::Ty::Tuple(_))
        {
            writable = *kind == ir::BorrowKind::Mut;
            owner = referent.as_ref().clone();
            base.fields.push(FieldSelector::Deref);
        }
        if !overloadable(&owner) {
            return Ok(None);
        }
        if !writable {
            return Err("E0596: cannot mutably index an immutable place".into());
        }
        let reference = self
            .index_reference_hint(&owner, index)?
            .ok_or("no Index implementation for indexed assignment")?;
        let ir::Ty::Ref { referent, .. } = reference else {
            return Err("Index must return a reference".into());
        };
        // Stock evaluates RHS before the indexed place. Save it once before
        // asking IndexMut for the actual destination address.
        let (rhs, rhs_flow) = if let Some(op) = op {
            self.compound_value(op, &referent, value)?
        } else {
            let rhs = self.expr_expected(value, Some(&referent))?;
            let flow = self.flow(rhs);
            self.require(&flow, &referent)?;
            (rhs, flow)
        };
        let rhs_ty = self.exprs[rhs.0].ty.clone();
        let saved = self.temporary(
            "$index_rhs",
            if rhs_ty == ir::Ty::Never {
                referent.as_ref().clone()
            } else {
                rhs_ty.clone()
            },
            false,
        )?;
        self.charge_node(
            std::mem::size_of::<ir::TypedStmt>() * 2 + std::mem::size_of::<ir::TypedExpr>(),
        )?;
        statements.push(ir::TypedStmt::Let {
            local: saved,
            value: rhs,
        });
        let idx_ty = self
            .method_receiver_hint(index)?
            .unwrap_or(ir::Ty::Int(ir::IntTy::Usize));
        let call = self.operator_call(
            ("ops", "IndexMut", "index_mut"),
            owner,
            Some(idx_ty),
            First::Place(ir::BorrowKind::Mut, &base),
            std::slice::from_ref(index),
        )?;
        self.record_operator(statement as *const Stmt as usize | 1, call)?;
        let completes = rhs_flow.completes && self.flow(call).completes;
        let carrier = self.temporary("$index_carrier", self.exprs[call.0].ty.clone(), false)?;
        statements.push(ir::TypedStmt::Let {
            local: carrier,
            value: call,
        });
        let loaded = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: rhs_ty,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Local(saved),
        });
        let place = ir::Place {
            local: carrier,
            projection: vec![ir::Projection::Deref],
        };
        let result = if let Some(op) = op {
            ir::TypedStmt::AssignOp {
                place,
                op,
                value: loaded,
            }
        } else {
            ir::TypedStmt::AssignPlace {
                place,
                value: loaded,
            }
        };
        Ok(Some((result, completes)))
    }

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

    pub(super) fn operator_call(
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
        let rhs_ty = if arity == 2 {
            if let Some(rhs) = rest.last() {
                self.unsuffixed_operator_rhs_hint(&self_ty, declaration, &ident, module, rhs)?
                    .or(rhs_ty)
            } else {
                rhs_ty
            }
        } else {
            rhs_ty
        };
        // A genuine numeric literal cannot have a nominal Self type. Keep its
        // final numeric fallback distinct from speculative nonliteral hints.
        let candidates: Vec<Vec<ir::Ty>> = match (arity, rhs_ty) {
            (1, _) => vec![Vec::new()],
            // An unsuffixed integer index variable infers from the selected
            // impl (usually `usize`); its hint is only the i32 fallback.
            (2, Some(rhs))
                if matches!(name, "Index" | "IndexMut") && rhs == ir::Ty::Int(ir::IntTy::I32) =>
            {
                vec![vec![rhs], vec![ir::Ty::Int(ir::IntTy::Usize)]]
            }
            (2, Some(rhs)) => vec![vec![rhs]],
            (2, None) => {
                let fallback = match rest.last() {
                    Some(Expr::Float(_)) => ir::Ty::Float(ir::FloatTy::F64),
                    _ => ir::Ty::Int(ir::IntTy::I32),
                };
                if matches!(rest.last(), Some(Expr::Integer(_) | Expr::Float(_))) {
                    vec![vec![fallback]]
                } else {
                    vec![vec![self_ty.clone()], vec![fallback]]
                }
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
                return self.finish_checked_call_mode(
                    target,
                    ident,
                    signature,
                    rest,
                    Arc::from([]),
                    (receiver, module == "cmp"),
                );
            }
        }
        Err(format!(
            "E0369: no implementation of std::{module}::{name} for the operand type"
        ))
    }

    /// `v[N]` through a reference (`v: &[T]`, `&Vec<T>`) is the same
    /// bounds-checked runtime place as `v[Nusize]`. Static constant-index
    /// places are array-only, so such operator operands take the runtime path.
    fn index_through_reference(&self, value: &Expr) -> Result<Option<Expr>, String> {
        let Expr::Index { base, index, site } = value else {
            return Ok(None);
        };
        let Ok(place) = type_check::static_source_place(base) else {
            return Ok(None);
        };
        if !place.fields.is_empty() {
            return Ok(None);
        }
        let Ok(local) = self.lookup(place.local) else {
            return Ok(None);
        };
        if !matches!(self.locals[local.0].ty, ir::Ty::Ref { .. }) {
            return Ok(None);
        }
        Ok(Some(Expr::DynamicIndex {
            base: base.clone(),
            index: Box::new(Expr::Integer(IntegerLiteral {
                magnitude: *index as u128,
                suffix: Some(ir::IntTy::Usize),
            })),
            site: *site,
        }))
    }

    /// Overloaded binary/unary operator, or `None` for primitive operands.
    pub(super) fn overloaded_operator(&mut self, e: &Expr) -> Result<Option<ir::ExprId>, String> {
        match e {
            Expr::Binary {
                op, left, right, ..
            } => {
                let Some(spec) = binary_trait(*op) else {
                    return Ok(None);
                };
                let Some(lhs) = self.method_receiver_hint(left)? else {
                    return Ok(None);
                };
                if !overloadable(&lhs) {
                    return Ok(None);
                }
                // A constructor operand (`x == W { a: 1 }`, `x == Some(1)`)
                // takes its generic arguments from the selected impl, as in
                // stock. Typing it standalone would apply literal fallback
                // (`W<i32>`) before the trait candidate is tried.
                let rhs = if constructor_operand(right) {
                    None
                } else {
                    self.method_receiver_hint(right)?
                };
                let left_runtime = self.index_through_reference(left)?;
                let right_runtime = self.index_through_reference(right)?;
                let left = left_runtime.as_ref().unwrap_or(left.as_ref());
                let right = right_runtime.as_ref().unwrap_or(right.as_ref());
                let id = if spec.0 == "cmp" {
                    let rest = std::slice::from_ref(right);
                    let first = First::Borrow(ir::BorrowKind::Shared, left);
                    self.operator_call(spec, lhs, rhs, first, rest)?
                } else {
                    let rest = std::slice::from_ref(right);
                    self.operator_call(spec, lhs, rhs, First::Value(left), rest)?
                };
                self.record_operator(e as *const Expr as usize, id)?;
                Ok(Some(id))
            }
            Expr::Unary { op, value, .. } => {
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
            Expr::DynamicIndex { base, index, .. } => {
                let Some(mut ty) = self.method_receiver_hint(base)? else {
                    return Ok(None);
                };
                // Stock index autoderef: `r[i]` with `r: &Vec<T>` / `&mut Vec<T>`
                // indexes `*r` through the nominal's `Index` impl.
                let explicit;
                let mut base: &Expr = base;
                if let ir::Ty::Ref { referent, .. } = &ty
                    && overloadable(referent)
                    && !matches!(referent.as_ref(), ir::Ty::Tuple(_))
                {
                    let referent = referent.as_ref().clone();
                    explicit = Expr::Deref {
                        value: Box::new(base.clone()),
                    };
                    base = &explicit;
                    ty = referent;
                }
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
            Expr::Index { base, index, .. } => {
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
