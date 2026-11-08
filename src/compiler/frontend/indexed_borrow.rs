//! Borrows of runtime-indexed places: `&s[i]`, `&mut s[i]`, `&v.data[i]`,
//! `&self.rows[i][j]`, `&(*r)[i]`. Each runtime index is evaluated once in
//! source order into a fresh `usize` local; the borrow then names the place
//! with `DynamicIndex` steps. The backend bounds-checks every runtime step
//! when the borrow address is materialized, and ownership treats a runtime
//! element loan as covering the whole array (no disjointness by index).
use super::*;

enum IndexedBorrowStep<'e> {
    Field(&'e FieldSelector),
    Const(usize, &'e Expr),
    Deref,
    Runtime(&'e Expr, &'e Expr),
}

impl type_check::Checker<'_> {
    /// An owned aggregate rvalue has fresh storage, unlike a place reached
    /// through a reference. Retain the complete owner, then borrow its checked
    /// projection. In particular, do not copy a projected reference referent.
    pub(super) fn borrow_owned_projection(
        &mut self,
        kind: ir::BorrowKind,
        source: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let mut steps = Vec::new();
        let mut root = source;
        loop {
            if steps.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            match root {
                Expr::Field { base, field } => {
                    steps.push(IndexedBorrowStep::Field(field));
                    root = base;
                }
                Expr::Index { base, index, .. } => {
                    steps.push(IndexedBorrowStep::Const(*index, root));
                    root = base;
                }
                Expr::DynamicIndex { base, index, .. } => {
                    steps.push(IndexedBorrowStep::Runtime(index, root));
                    root = base;
                }
                _ => break,
            }
        }
        if steps.is_empty() || type_check::place_expression(root) {
            return Ok(None);
        }
        let Some(mut ty) = self.method_receiver_hint(root)? else {
            return Ok(None);
        };
        steps.reverse();
        for step in &steps {
            // Autoderef requires the original reference's storage/provenance.
            // Leave it to the returned-reference and indexed-place protocols.
            if matches!(ty, ir::Ty::Ref { .. } | ir::Ty::RawPtr { .. }) {
                return Ok(None);
            }
            match step {
                IndexedBorrowStep::Field(field) => {
                    let (_, field_ty) = self.field_type(&ty, field)?;
                    ty = field_ty;
                }
                IndexedBorrowStep::Const(index, _) => {
                    if !matches!(ty, ir::Ty::Array { .. }) {
                        return Ok(None);
                    }
                    ty = self.index_type(&ty, *index)?;
                }
                IndexedBorrowStep::Runtime(..) => {
                    let ir::Ty::Array { element, .. } = ty else {
                        return Ok(None);
                    };
                    ty = element.as_ref().clone();
                }
                _ => unreachable!(),
            }
        }
        let value = self.expr(root)?;
        let flow = self.flow(value);
        if !flow.completes {
            return Ok(Some(value));
        }
        let local = self.temporary("$projected_owner", flow.ty, true)?;
        let extended = self
            .extending_borrows
            .contains(&(source as *const Expr as usize));
        // Destructors in extending positions need the owning-let protocol,
        // rather than a statement-scoped assignment.
        if extended && !self.trivially_dropped(&self.locals[local.0].ty)? {
            return Err("unsupported: destructor projected temporary lifetime extension".into());
        }
        self.register_borrow_temporary(local, extended, None)?;
        let mut projection = Vec::new();
        let mut statements = vec![ir::TypedStmt::Assign { local, value }];
        let mut ty = self.locals[local.0].ty.clone();
        for step in steps {
            self.charge_node(std::mem::size_of::<ir::Projection>())?;
            match step {
                IndexedBorrowStep::Field(field) => {
                    let (index, field_ty) = self.field_type(&ty, field)?;
                    projection.push(ir::Projection::Field(index));
                    ty = field_ty;
                }
                IndexedBorrowStep::Const(index, _) => {
                    ty = self.index_type(&ty, index)?;
                    projection.push(ir::Projection::ConstantIndex(index));
                }
                IndexedBorrowStep::Runtime(index, original) => {
                    let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
                    let index = self.expr_expected(index, Some(&usize_ty))?;
                    self.require_index(&self.flow(index))?;
                    if !self.flow(index).completes {
                        statements.push(ir::TypedStmt::Expr(index));
                        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
                        let id = ir::ExprId(self.exprs.len());
                        self.exprs.push(ir::TypedExpr {
                            ty: ir::Ty::Never,
                            completion: ir::Completion::Never,
                            kind: ir::ExprKind::Block(ir::TypedBlock {
                                statements,
                                tail: None,
                            }),
                        });
                        return Ok(Some(id));
                    }
                    let saved = self.temporary("$projected_index", usize_ty.clone(), false)?;
                    self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
                    statements.push(ir::TypedStmt::Let {
                        local: saved,
                        value: index,
                    });
                    let checked = self.for_expr(usize_ty, ir::ExprKind::Local(saved))?;
                    self.index_bounds_probe(
                        &ir::Place {
                            local,
                            projection: projection.clone(),
                        },
                        checked,
                        original,
                        &mut statements,
                    )?;
                    let ir::Ty::Array { element, .. } = ty else {
                        return Err("owned projection array type changed".into());
                    };
                    ty = element.as_ref().clone();
                    projection.push(ir::Projection::DynamicIndex(saved));
                }
                IndexedBorrowStep::Deref => unreachable!(),
            }
        }
        let borrowed = self.borrow_expression(kind, ir::Place { local, projection })?;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: self.exprs[borrowed.0].ty.clone(),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements,
                tail: Some(borrowed),
            }),
        });
        Ok(Some(id))
    }

    /// Lower `&place` / `&mut place` for an indexed local-rooted place or a
    /// dereferenced call result. Returns `None` for other place shapes.
    pub(super) fn borrow_indexed_place(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        if let Expr::Deref { value } = value
            && type_check::genuine_call(value)
        {
            return self.borrow_call_referent(kind, value).map(Some);
        }
        self.indexed_place_borrow(kind, value, true)
    }

    /// `&v[0].f` with an overloaded `Index` (or `&call().f` through a returned
    /// reference): the reference the call returns is held in a temporary and
    /// the borrow names `(*tmp).f`, autodereferencing reference layers, so it
    /// aliases the original storage and keeps its loan provenance.
    pub(super) fn borrow_call_rooted_field(
        &mut self,
        kind: ir::BorrowKind,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        if let Some(reference) = self.mutable_slice_field_borrow(kind, value)? {
            return Ok(Some(reference));
        }
        let mut fields = Vec::new();
        let mut root = value;
        while let Expr::Field { base, field } = root {
            if fields.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            fields.push(field);
            root = base;
        }
        // A local owner's non-stored field is also a returned-reference
        // place: obtain its ordinary checked Deref call without moving the
        // owner, then use exactly the same projection/loan protocol below.
        let local_owner = if !fields.is_empty() && matches!(root, Expr::Var(_)) {
            match self.method_receiver_hint(root)? {
                Some(hint) => {
                    let owner = match &hint {
                        ir::Ty::Ref { referent, .. } => referent.as_ref(),
                        ty => ty,
                    };
                    if let ir::Ty::Struct(nominal) = owner
                        && !self.has_member(nominal.def().source_decl, fields[fields.len() - 1])?
                    {
                        Some(hint)
                    } else {
                        None
                    }
                }
                None => None,
            }
        } else {
            None
        };
        if fields.is_empty()
            || !(matches!(root, Expr::Index { .. } | Expr::DynamicIndex { .. })
                || type_check::genuine_call(root)
                || local_owner.is_some())
        {
            return Ok(None);
        }
        if matches!(root, Expr::Index { .. })
            && type_check::static_source_place(root)
                .is_ok_and(|place| self.checked_place(&place).is_ok())
        {
            return Ok(None);
        }
        // An owned call result is not a returned-reference place. Decline
        // before lowering it: the caller's owned-field fallback must remain
        // the only evaluation and the only original call correspondence.
        if type_check::genuine_call(root)
            && self
                .method_receiver_hint(root)?
                .is_some_and(|ty| !matches!(ty, ir::Ty::Ref { .. } | ir::Ty::Never))
        {
            return Ok(None);
        }
        let indexed = if matches!(root, Expr::Index { .. } | Expr::DynamicIndex { .. }) {
            self.overloaded_index_reference(kind, root, root as *const Expr as usize)?
        } else {
            None
        };
        let evaluated = if let Some(owner) = local_owner {
            let Some(carrier) = self.receiver_overloaded_deref(root, &owner, kind)? else {
                return Ok(None);
            };
            self.deref_overloaded_result(carrier)?
        } else if let Some(carrier) = indexed {
            self.deref_overloaded_result(carrier)?
        } else {
            self.expr(root)?
        };
        if self.exprs[evaluated.0].completion == ir::Completion::Never {
            return Ok(Some(evaluated));
        }
        // `v[0][1].f`: builtin constant array steps on the returned reference.
        let mut outer = evaluated;
        let mut array_steps = Vec::new();
        while let ir::ExprKind::Index { base, index } = self.exprs[outer.0].kind {
            if array_steps.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                return Err("place projection nesting exceeds limit128".into());
            }
            array_steps.push(index);
            outer = base;
        }
        let carrier = match self.exprs[outer.0].kind {
            ir::ExprKind::Deref { value }
                if matches!(self.exprs[value.0].kind, ir::ExprKind::Call { .. }) =>
            {
                value
            }
            _ if array_steps.is_empty()
                && type_check::genuine_call(root)
                && matches!(self.exprs[evaluated.0].ty, ir::Ty::Ref { .. }) =>
            {
                evaluated
            }
            _ => return Ok(None),
        };
        let carrier_ty = self.exprs[carrier.0].ty.clone();
        let mut ty = carrier_ty.clone();
        let mut projection = Vec::new();
        if !array_steps.is_empty() {
            let ir::Ty::Ref {
                kind: layer,
                referent,
            } = &ty
            else {
                return Ok(None);
            };
            if kind == ir::BorrowKind::Mut && *layer == ir::BorrowKind::Shared {
                return Err("cannot borrow as mutable through a shared reference".into());
            }
            projection.push(ir::Projection::Deref);
            ty = referent.as_ref().clone();
            for index in array_steps.into_iter().rev() {
                ty = self.index_type(&ty, index)?;
                projection.push(ir::Projection::ConstantIndex(index));
            }
        }
        for field in fields.into_iter().rev() {
            for _ in 0..ir::MAX_PATTERN_PROJECTION_DEPTH {
                let ir::Ty::Ref {
                    kind: layer,
                    referent,
                } = &ty
                else {
                    break;
                };
                if kind == ir::BorrowKind::Mut && *layer == ir::BorrowKind::Shared {
                    return Err("cannot borrow as mutable through a shared reference".into());
                }
                projection.push(ir::Projection::Deref);
                ty = referent.as_ref().clone();
            }
            let (index, field_ty) = self.field_type(&ty, field)?;
            projection.push(ir::Projection::Field(index));
            ty = field_ty;
        }
        let local = self.temporary("$reborrow_call", carrier_ty, true)?;
        self.charge_node(std::mem::size_of::<ir::LocalId>() + projection.len() * 16)?;
        self.register_borrow_temporary(local, false, None)?;
        let borrowed = self.borrow_expression(kind, ir::Place { local, projection })?;
        let ty = self.exprs[borrowed.0].ty.clone();
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign {
                    local,
                    value: carrier,
                }],
                tail: Some(borrowed),
            }),
        });
        Ok(Some(id))
    }

    /// `&*call()` reborrows the returned reference. Storing the referent would
    /// copy or move it and lose both aliasing and its original loan provenance.
    fn borrow_call_referent(
        &mut self,
        kind: ir::BorrowKind,
        source: &Expr,
    ) -> Result<ir::ExprId, String> {
        let value = self.expr(source)?;
        let ty = self.exprs[value.0].ty.clone();
        if self.exprs[value.0].completion == ir::Completion::Never {
            return Ok(value);
        }
        let ir::Ty::Ref { kind: original, .. } = &ty else {
            return Err("unsupported: reborrowing a call result requires a reference".into());
        };
        if kind == ir::BorrowKind::Mut && *original == ir::BorrowKind::Shared {
            return Err("cannot borrow as mutable through a shared reference".into());
        }
        let local = self.temporary("$reborrow_call", ty, true)?;
        self.charge_node(std::mem::size_of::<ir::LocalId>())?;
        self.register_borrow_temporary(local, false, None)?;
        let borrowed = self.borrow_expression(
            kind,
            ir::Place {
                local,
                projection: vec![ir::Projection::Deref],
            },
        )?;
        let ty = self.exprs[borrowed.0].ty.clone();
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 64)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements: vec![ir::TypedStmt::Assign { local, value }],
                tail: Some(borrowed),
            }),
        });
        Ok(id)
    }

    pub(super) fn format_indexed_place_borrow(
        &mut self,
        value: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        if let Some(reference) = self.indexed_place_borrow(ir::BorrowKind::Shared, value, true)? {
            return Ok(Some(reference));
        }
        let Expr::DynamicIndex { base, .. } = value else {
            return Ok(None);
        };
        if !matches!(base.as_ref(), Expr::Field { .. })
            || !matches!(self.method_receiver_hint(base)?, Some(ir::Ty::Array { .. }))
        {
            return Ok(None);
        }
        let Some(carrier) = self.borrow_call_rooted_field(ir::BorrowKind::Shared, base)? else {
            return Ok(None);
        };
        self.borrow_checked_array_reference(ir::BorrowKind::Shared, carrier, value, false)
            .map(Some)
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
                        receiver_authority::local(path)
                    } else if path.anchor() == PathAnchor::Relative
                        && let Some(name) = path.single()
                    {
                        LocalName::Named(name)
                    } else {
                        return Ok(None);
                    };
                }
                Expr::DynamicIndex { base, index, .. } => {
                    steps.push(IndexedBorrowStep::Runtime(index, cursor));
                    cursor = base;
                }
                Expr::Index { base, index, .. } => {
                    steps.push(IndexedBorrowStep::Const(*index, cursor));
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
            matches!(step, IndexedBorrowStep::Runtime(..))
                || (constants && matches!(step, IndexedBorrowStep::Const(..)))
        }) {
            return Ok(None);
        }
        steps.reverse();
        let root = self.lookup(root)?;
        // Decide builtin eligibility before lowering any runtime index. A
        // non-stored nominal field belongs to canonical field lowering, not
        // to this builtin place-borrow fast path.
        let mut eligibility = self.locals[root.0].ty.clone();
        let mut eligibility_depth = 0;
        for step in &steps {
            self.signatures.plan()?.budget.charge_work(1)?;
            if matches!(step, IndexedBorrowStep::Deref) {
                if eligibility_depth >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                    return Err("place projection nesting exceeds limit128".into());
                }
                let ir::Ty::Ref { referent, .. } = &eligibility else {
                    return Ok(None);
                };
                eligibility = referent.as_ref().clone();
                eligibility_depth += 1;
            } else {
                while let ir::Ty::Ref { referent, .. } = &eligibility {
                    if eligibility_depth >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("place projection nesting exceeds limit128".into());
                    }
                    eligibility = referent.as_ref().clone();
                    eligibility_depth += 1;
                }
                eligibility = match step {
                    IndexedBorrowStep::Field(field) => {
                        if let ir::Ty::Struct(nominal) = &eligibility
                            && !self.explicit_field_stored(nominal, field)?
                        {
                            return Ok(None);
                        }
                        self.field_type(&eligibility, field)?.1
                    }
                    IndexedBorrowStep::Const(index, _) => match &eligibility {
                        ir::Ty::Array { .. } => self.index_type(&eligibility, *index)?,
                        ir::Ty::Slice { element } => element.as_ref().clone(),
                        _ => return Ok(None),
                    },
                    IndexedBorrowStep::Runtime(..) => match &eligibility {
                        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                            element.as_ref().clone()
                        }
                        _ => return Ok(None),
                    },
                    IndexedBorrowStep::Deref => unreachable!(),
                };
            }
        }
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
                IndexedBorrowStep::Const(index, original) => {
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
                        self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
                        statements.push(ir::TypedStmt::Let {
                            local: saved,
                            value,
                        });
                        let checked_index =
                            self.for_expr(usize_ty.clone(), ir::ExprKind::Local(saved))?;
                        self.index_bounds_probe(
                            &ir::Place {
                                local: root,
                                projection: projection.clone(),
                            },
                            checked_index,
                            original,
                            &mut statements,
                        )?;
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
                IndexedBorrowStep::Runtime(index, original) => {
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
                    self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
                    statements.push(ir::TypedStmt::Let {
                        local: saved,
                        value: index,
                    });
                    let checked_index =
                        self.for_expr(usize_ty.clone(), ir::ExprKind::Local(saved))?;
                    self.index_bounds_probe(
                        &ir::Place {
                            local: root,
                            projection: projection.clone(),
                        },
                        checked_index,
                        original,
                        &mut statements,
                    )?;
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
            kind: ir::ExprKind::Block(ir::TypedBlock {
                statements,
                tail: completes.then_some(borrowed),
            }),
        });
        Ok(Some(id))
    }
}
