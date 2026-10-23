//! Canonical unsafe raw-parts descriptor construction. No element is read.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn slice_data_pointer_expression(&mut self) -> Result<Expr, ParseError> {
        self.ident()?;
        self.expect("!")?;
        self.expect("(")?;
        self.names
            .retain_parser_text(std::mem::size_of::<Expr>())
            .map_err(native_adapter::name_error)?;
        let slice = self.expr(0)?;
        self.eat(",");
        self.expect(")")?;
        Ok(Expr::SliceDataPointer(Box::new(slice)))
    }

    pub(super) fn raw_slice_expression(
        &mut self,
        kind: ir::BorrowKind,
    ) -> Result<Expr, ParseError> {
        self.ident()?;
        self.expect("!")?;
        self.expect("(")?;
        self.names
            .retain_parser_text(2 * std::mem::size_of::<Expr>())
            .map_err(native_adapter::name_error)?;
        let mut arguments = Vec::new();
        arguments.try_reserve_exact(2).map_err(|_| {
            native_adapter::transport_error(self.names.budget().poison_allocation().into())
        })?;
        arguments.push(self.expr(0)?);
        self.expect(",")?;
        arguments.push(self.expr(0)?);
        self.eat(",");
        self.expect(")")?;
        Ok(Expr::RawSlice { kind, arguments })
    }
}

impl type_check::Checker<'_> {
    pub(super) fn slice_data_pointer_hint(&self, slice: &Expr) -> Result<Option<ir::Ty>, String> {
        self.scalar_hint(slice)?
            .map(|ty| ir::slice_data_pointer_ty(&ty))
            .transpose()
    }

    pub(super) fn slice_data_pointer(
        &mut self,
        slice: &Expr,
    ) -> Result<(ir::ExprKind, Flow), String> {
        let slice = self.expr(slice)?;
        let source = self.flow(slice);
        let ty = ir::slice_data_pointer_ty(&source.ty)?;
        let element = source
            .ty
            .mut_slice_element()
            .ok_or("slice data pointer requires mutable slice")?;
        ir::validate_shared_slice_element(element, self.facts(), &self.signatures.plan()?.budget)?;
        Ok((
            ir::ExprKind::SliceDataPointer { slice },
            Flow {
                ty,
                completes: source.completes,
            },
        ))
    }

    pub(super) fn raw_slice_hint(
        &self,
        kind: ir::BorrowKind,
        arguments: &[Expr],
    ) -> Result<Option<ir::Ty>, String> {
        if arguments.len() != 2 {
            return Err("raw slice constructor requires pointer and length".into());
        }
        self.scalar_hint(&arguments[0])?
            .map(|pointer| ir::raw_slice_ty(kind, &pointer))
            .transpose()
    }

    pub(super) fn raw_slice(
        &mut self,
        kind: ir::BorrowKind,
        arguments: &[Expr],
    ) -> Result<(ir::ExprKind, Flow), String> {
        if self.unsafe_depth == 0 {
            return Err("E0133: raw slice construction requires unsafe block".into());
        }
        if arguments.len() != 2 {
            return Err("raw slice constructor requires pointer and length".into());
        }
        self.charge_node(2 * std::mem::size_of::<ir::ExprId>())?;
        let pointer = self.expr(&arguments[0])?;
        let ty = ir::raw_slice_ty(kind, &self.exprs[pointer.0].ty)?;
        let (_, element) = ty.slice_ref_element().ok_or("raw slice result invariant")?;
        ir::validate_shared_slice_element(element, self.facts(), &self.signatures.plan()?.budget)?;
        let length = self.expr_expected(&arguments[1], Some(&ir::Ty::Int(ir::IntTy::Usize)))?;
        self.require(&self.flow(length), &ir::Ty::Int(ir::IntTy::Usize))?;
        let completes = self.flow(pointer).completes && self.flow(length).completes;
        Ok((
            ir::ExprKind::RawSlice {
                kind,
                args: vec![pointer, length],
            },
            Flow { ty, completes },
        ))
    }
}
