//! Borrows of runtime-indexed places: `&s[i]`, `&mut s[i]`, `&v.data[i]`,
//! `&self.rows[i][j]`, `&(*r)[i]`. Each runtime index is evaluated once in
//! source order into a fresh `usize` local; the borrow then names the place
//! with `DynamicIndex` steps. The backend bounds-checks every runtime step
//! when the borrow address is materialized, and ownership treats a runtime
//! element loan as covering the whole array (no disjointness by index).
use super::*;

enum IndexedBorrowStep<'e> {
    Field(&'e FieldSelector),
    Const(usize),
    Deref,
    Runtime(&'e Expr),
}

impl type_check::Checker<'_> {
    /// Lower `&place` / `&mut place` for a local-rooted place containing a
    /// runtime index. Returns `None` when `value` is not such a place.
    pub(super) fn borrow_indexed_place(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        self.indexed_place_borrow(kind, value, false)
    }

    pub(super) fn format_indexed_place_borrow(
        &mut self,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        self.indexed_place_borrow(ir::BorrowKind::Shared, value, true)
    }

    fn indexed_place_borrow(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
        constants: bool,
    ) -> Result<Option<ir::ExprId>, String> {
        let mut steps = Vec::new();
        let mut cursor = value;
        let root = loop {
            if steps.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            match cursor {
                Expr::Var(path) => {
                    break if path.anchor() == PathAnchor::SelfModule && path.is_empty() {
                        LocalName::Receiver
                    } else if path.anchor() == PathAnchor::Relative
                        && let Some(name) = path.single()
                    {
                        LocalName::Named(name)
                    } else {
                        return Ok(None);
                    };
                }
                Expr::DynamicIndex { base, index } => {
                    steps.push(IndexedBorrowStep::Runtime(index));
                    cursor = base;
                }
                Expr::Index { base, index } => {
                    steps.push(IndexedBorrowStep::Const(*index));
                    cursor = base;
                }
                Expr::Field { base, field } => {
                    steps.push(IndexedBorrowStep::Field(field));
                    cursor = base;
                }
                Expr::Deref { value } => {
                    steps.push(IndexedBorrowStep::Deref);
                    cursor = value;
                }
                _ => return Ok(None),
            }
        };
        if !steps.iter().any(|step| {
            matches!(step, IndexedBorrowStep::Runtime(_))
                || (constants && matches!(step, IndexedBorrowStep::Const(_)))
        }) {
            return Ok(None);
        }
        steps.reverse();
        let root = self.lookup(root)?;
        let mut ty = self.locals[root.0].ty.clone();
        let mut writable = self.locals[root.0].mutable;
        let mut projection = Vec::with_capacity(steps.len() + 2);
        let mut statements = Vec::new();
        let mut completes = true;
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        // Peel one reference (auto-deref for `r.field` / `r[i]`).
        fn peel(
            ty: &mut ir::Ty,
            writable: &mut bool,
            projection: &mut Vec<ir::Projection>,
        ) -> Result<(), String> {
            let ir::Ty::Ref { kind, referent } = ty.clone() else {
                return Err("dereference requires a reference".into());
            };
            *writable = kind == ir::BorrowKind::Mut;
            *ty = referent.as_ref().clone();
            projection.push(ir::Projection::Deref);
            Ok(())
        }
        for step in steps {
            if projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            match step {
                IndexedBorrowStep::Deref => {
                    if matches!(ty, ir::Ty::RawPtr { .. }) {
                        return Err(
                            "unsupported: borrowing runtime-indexed places behind raw pointers"
                                .into(),
                        );
                    }
                    peel(&mut ty, &mut writable, &mut projection)?;
                }
                IndexedBorrowStep::Field(field) => {
                    while matches!(ty, ir::Ty::Ref { .. }) {
                        peel(&mut ty, &mut writable, &mut projection)?;
                    }
                    let (index, field_ty) = self.field_type(&ty, field)?;
                    projection.push(ir::Projection::Field(index));
                    ty = field_ty;
                }
                IndexedBorrowStep::Const(index) => {
                    while matches!(ty, ir::Ty::Ref { .. }) {
                        peel(&mut ty, &mut writable, &mut projection)?;
                    }
                    if let ir::Ty::Slice { element } = &ty {
                        let element = element.as_ref().clone();
                        let literal = Expr::Integer(IntegerLiteral {
                            magnitude: index as u128,
                            suffix: Some(ir::IntTy::Usize),
                        });
                        let value = self.expr(&literal)?;
                        let saved = self.temporary(
                            &format!("$bi{}", self.locals.len()),
                            usize_ty.clone(),
                            false,
                        )?;
                        self.charge_node(std::mem::size_of::<ir::Stmt>())?;
                        statements.push(ir::Stmt::Let {
                            local: saved,
                            value,
                        });
                        projection.push(ir::Projection::DynamicIndex(saved));
                        ty = element;
                    } else {
                        if constants && !matches!(ty, ir::Ty::Array { .. }) {
                            return Ok(None);
                        }
                        ty = self.index_type(&ty, index)?;
                        projection.push(ir::Projection::ConstantIndex(index));
                    }
                }
                IndexedBorrowStep::Runtime(index) => {
                    while matches!(ty, ir::Ty::Ref { .. }) {
                        peel(&mut ty, &mut writable, &mut projection)?;
                    }
                    let element = match &ty {
                        ir::Ty::Array { element, .. } => element.as_ref().clone(),
                        ir::Ty::Slice { element }
                            if matches!(projection.last(), Some(ir::Projection::Deref)) =>
                        {
                            element.as_ref().clone()
                        }
                        _ => return Ok(None),
                    };
                    let index = self.expr_expected_when(
                        index,
                        self.reachable && completes,
                        Some(&usize_ty),
                    )?;
                    let flow = self.flow(index);
                    self.require_index(&flow)?;
                    completes &= flow.completes;
                    let saved = self.temporary(
                        &format!("$bi{}", self.locals.len()),
                        usize_ty.clone(),
                        false,
                    )?;
                    self.charge_node(std::mem::size_of::<ir::Stmt>())?;
                    statements.push(ir::Stmt::Let {
                        local: saved,
                        value: index,
                    });
                    projection.push(ir::Projection::DynamicIndex(saved));
                    ty = element;
                }
            }
        }
        self.accessible(&ty)?;
        if kind == ir::BorrowKind::Mut && !writable {
            return Err(
                "E0596: cannot borrow an element as mutable: the place is not declared mutable and is not behind `&mut`"
                    .into(),
            );
        }
        let borrowed = self.borrow_expression(
            kind,
            ir::Place {
                local: root,
                projection,
            },
        )?;
        let ty = self.exprs[borrowed.0].ty.clone();
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: if completes { ty } else { ir::Ty::Never },
            completion: if completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            },
            kind: ir::ExprKind::Block(ir::Block {
                statements,
                tail: completes.then_some(borrowed),
            }),
        });
        Ok(Some(id))
    }
}
