//! Mutable slices `&mut [T]` (MUTABLE-SLICES.md). The carrier uses the same
//! private two-word (data, len) descriptor as `&[T]`, but it is not Copy:
//! reads (`s[i]`, `s.len()`) go through a shared reborrow
//! `&*s` of the local-rooted carrier place, never a move of the carrier, and
//! writes (`s[i] = v`, `s[i] op= v`) name the element place
//! `s.*[DynamicIndex($i)]` so the backend bounds-checks it against the
//! descriptor length when the address is formed.
use super::*;

impl type_check::Checker<'_> {
    /// Obtain a range receiver through ordinary borrow checking. The range
    /// descriptor never invents storage authority or proves disjointness.
    pub(super) fn range_slice_base(
        &mut self,
        kind: ir::BorrowKind,
        base: &Expr,
    ) -> Result<ir::ExprId, String> {
        if let Expr::RangeIndex {
            base,
            start,
            end,
            inclusive,
        } = base
        {
            return self.subslice_with_kind(
                kind,
                base,
                start.as_deref(),
                end.as_deref(),
                *inclusive,
            );
        }
        let hint = self.scalar_hint(base)?;
        if let Some(view) = self.vec_range_view(kind, base, hint.as_ref())? {
            return Ok(view);
        }
        if matches!(hint, Some(ir::Ty::Array { .. })) {
            let place = type_check::static_source_place(base).map_err(|_| {
                "unsupported: range indexing of a non-place array value".to_string()
            })?;
            return self.expr(&Expr::Borrow { kind, place });
        }
        if let Some(ir::Ty::Ref {
            kind: source_kind,
            referent,
        }) = &hint
            && matches!(
                referent.as_ref(),
                ir::Ty::Slice { .. } | ir::Ty::Array { .. }
            )
        {
            if kind == ir::BorrowKind::Mut && *source_kind != ir::BorrowKind::Mut {
                return Err("E0596: cannot mutably borrow a shared range receiver".into());
            }
            if *source_kind == ir::BorrowKind::Mut {
                let source = type_check::static_source_place(base).map_err(|_| {
                    "unsupported: range reborrow requires a source place".to_string()
                })?;
                let (mut place, _, _) = self.checked_place(&source)?;
                if place.projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                    return Err("place projection nesting exceeds limit128".into());
                }
                place.projection.push(ir::Projection::Deref);
                return self.borrow_expression(kind, place);
            }
        }
        self.expr(base)
    }
    /// `v[a..b]` on a prelude `Vec<T>` (or a reference to one) ranges over the
    /// ordinary source view `v.as_slice()` / `v.as_mut_slice()`, as stock's
    /// `Index<Range<usize>>` derefs to `[T]`. The view call is checked like
    /// any written method call, so receiver borrowing rules are unchanged.
    fn vec_range_view(
        &mut self,
        kind: ir::BorrowKind,
        base: &Expr,
        hint: Option<&ir::Ty>,
    ) -> Result<Option<ir::ExprId>, String> {
        let mut ty = hint;
        while let Some(ir::Ty::Ref { referent, .. }) = ty {
            ty = Some(referent.as_ref());
        }
        let Some(ir::Ty::Struct(value)) = ty else {
            return Ok(None);
        };
        if !matches!(base, Expr::Var(_) | Expr::Field { .. }) {
            return Ok(None);
        }
        if !self.is_prelude_vec_struct(value) {
            return Ok(None);
        }
        let method = match kind {
            ir::BorrowKind::Shared => "as_slice",
            ir::BorrowKind::Mut => "as_mut_slice",
        };
        let Some(name) = self.prelude_inherent_method(value.def().source_decl, method)? else {
            return Err(format!(
                "unsupported: prelude Vec::{method} is not available"
            ));
        };
        let call = Expr::MethodCall {
            receiver: Box::new(base.clone()),
            name,
            type_args: Vec::new(),
            args: Vec::new(),
        };
        let id = self.expr_with_expectation_inner(&call, type_check::Expectation::None)?;
        self.record_operator(base as *const Expr as usize, id)?;
        Ok(Some(id))
    }

    /// The prelude's own `Vec<T>` nominal (by canonical name; diagnostics-only
    /// names are not identity, but the prelude module path is reserved).
    pub(super) fn is_prelude_vec_struct(&self, value: &ir::StructTy) -> bool {
        let std = crate::compiler::source::source_prelude::PRELUDE_MODULE;
        value.def().name == format!("crate::{std}::vec::Vec")
    }

    /// Source name of the inherent method `method` declared on `nominal`.
    fn prelude_inherent_method(
        &self,
        nominal: ir::DeclId,
        method: &str,
    ) -> Result<Option<AstIdent>, String> {
        let plan = self.signatures.plan()?;
        let names = plan.program.names.as_ref();
        for id in &plan.registry.function_ids {
            let info = plan.registry.declaration(*id)?;
            let (Some(owner), DeclItemName::Source(name)) = (&info.path.associated, info.path.item)
            else {
                continue;
            };
            if owner.trait_decl.is_none()
                && owner.nominal == nominal
                && names
                    .spelling(name)
                    .map_err(native_adapter::name_diagnostic)?
                    == method
            {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    pub(super) fn mut_slice_constant_assignment(
        &mut self,
        place: &Place,
        op: Option<BinaryOp>,
        value: &Expr,
    ) -> Result<Option<(ir::TypedStmt, bool)>, String> {
        let Some((FieldSelector::ConstantIndex(index), prefix)) = place.fields.split_last() else {
            return Ok(None);
        };
        self.charge_node(std::mem::size_of_val(prefix))?;
        let array = Place {
            local: place.local,
            fields: prefix.to_vec(),
        };
        let index = Expr::Integer(IntegerLiteral {
            magnitude: *index as u128,
            suffix: Some(ir::IntTy::Usize),
        });
        self.mut_slice_index_assignment(&array, &index, op, value)
    }

    /// Static place of type `&mut [T]` that `expr` names (`s`, `self.buf`,
    /// `*r` for `r: &mut &mut [T]` is not admitted). Never evaluates.
    fn mut_slice_place(&self, expr: &Expr) -> Result<Option<(ir::Place, ir::Ty)>, String> {
        if !self
            .scalar_hint(expr)?
            .is_some_and(|ty| ty.mut_slice_element().is_some())
        {
            return Ok(None);
        }
        if !matches!(expr, Expr::Var(_) | Expr::Field { .. }) {
            return Ok(None);
        }
        let Ok(source) = type_check::static_source_place(expr) else {
            return Ok(None);
        };
        let (place, ty, _) = self.checked_place(&source)?;
        if ty.mut_slice_element().is_none() {
            return Ok(None);
        }
        Ok(Some((place, ty)))
    }

    /// `&*s` for a local-rooted `&mut [T]` place: a shared `&[T]` view that
    /// leaves the mutable carrier in place (stock reads through `&mut [T]`
    /// never move it). `None` when `expr` is not such a place.
    pub(super) fn shared_slice_view(&mut self, expr: &Expr) -> Result<Option<ir::ExprId>, String> {
        let Some((mut place, _)) = self.mut_slice_place(expr)? else {
            return Ok(None);
        };
        if place.projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        place.projection.push(ir::Projection::Deref);
        self.borrow_expression(ir::BorrowKind::Shared, place)
            .map(Some)
    }

    /// `s[i] = v` / `s[i] op= v` where `s` names a local-rooted `&mut [T]`
    /// place. Stock order: value, then index, then the bounds check and the
    /// write. `None` when the array operand is not such a place.
    pub(super) fn mut_slice_index_assignment(
        &mut self,
        array: &Place,
        index: &Expr,
        op: Option<BinaryOp>,
        value: &Expr,
    ) -> Result<Option<(ir::TypedStmt, bool)>, String> {
        let (mut place, ty, _) = self.checked_place(array)?;
        let Some(element) = ty.mut_slice_element() else {
            if ty.slice_ref_element().is_some() {
                return Err(
                    "E0594: cannot assign to an element of a `&` slice reference; it is not writable"
                        .into(),
                );
            }
            return Ok(None);
        };
        let element = element.clone();
        ir::validate_shared_slice_element(&element, self.facts(), &self.signatures.plan()?.budget)?;
        if !self.facts().must_copy(&element)? {
            return Err("unsupported: mutable slice assignment of non-Copy elements".into());
        }
        if place.projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("place projection nesting exceeds limit128".into());
        }
        place.projection.push(ir::Projection::Deref);
        let (value, flow) = match op {
            Some(op) => self.compound_value(op, &element, value)?,
            None => {
                let value = self.expr_expected(value, Some(&element))?;
                let flow = self.flow(value);
                self.require(&flow, &element)?;
                (value, flow)
            }
        };
        let index = self.expr_expected_when(
            index,
            self.reachable && flow.completes,
            Some(&ir::Ty::Int(ir::IntTy::Usize)),
        )?;
        let index_flow = self.flow(index);
        self.require_index(&index_flow)?;
        let completes = flow.completes && index_flow.completes;
        Ok(Some((
            match op {
                Some(op) => ir::TypedStmt::AssignIndexOp {
                    array: place,
                    index,
                    op,
                    value,
                },
                None => ir::TypedStmt::AssignIndex {
                    array: place,
                    index,
                    value,
                },
            },
            completes,
        )))
    }
}
