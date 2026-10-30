//! Canonical source ingress for the target alignment observation.
//! The public function stays an ordinary source call. Only its authenticated
//! embedded body may contain the private typed operation, which remains
//! symbolic until ordinary generic substitution supplies its queried type.
use super::*;

impl Parser<'_, '_> {
    #[inline(never)]
    pub(super) fn type_alignment_expression(&mut self) -> Result<Expr, ParseError> {
        self.ident()?;
        self.expect("!")?;
        self.expect("(")?;
        let queried = self.ty()?;
        self.eat(",");
        self.expect(")")?;
        Ok(Expr::TypeAlignment(queried))
    }
}

impl type_check::Checker<'_> {
    pub(super) fn type_alignment(
        &self,
        queried: &Type,
    ) -> Result<(ir::ExprKind, type_check::Flow), String> {
        let plan = self
            .signatures
            .plan
            .as_ref()
            .ok_or("target alignment requires canonical source planning facts")?;
        let facts = self
            .facts()
            .validated()
            .ok_or("target alignment requires validated caller facts")?;
        let declaration = plan.declaration(facts.declaration())?;
        let root = plan
            .program
            .prelude_module
            .ok_or("target alignment requires the embedded prelude")?;
        if plan
            .program
            .modules
            .parent(declaration.path.module)
            .map_err(module_diagnostic)?
            != Some(root)
        {
            return Err(
                "private target alignment operation outside its canonical source body".into(),
            );
        }
        let module = plan
            .program
            .modules
            .name(declaration.path.module)
            .map_err(module_diagnostic)?
            .ok_or("target alignment declaration has no module name")?;
        let DeclItemName::Source(item) = declaration.path.item else {
            return Err("target alignment requires a source declaration".into());
        };
        let kind = match plan
            .program
            .names
            .spelling(item)
            .map_err(native_adapter::name_diagnostic)?
        {
            "align_of" => ir::TypeQueryKind::Alignment,
            "size_of" => ir::TypeQueryKind::SemanticSize,
            "element_stride" => ir::TypeQueryKind::PrivateElementStride,
            "pointer_stride" => ir::TypeQueryKind::SupportedPointerStride,
            "storage_stride" => ir::TypeQueryKind::PrivateStorageStride,
            "storage_alignment" => ir::TypeQueryKind::PrivateStorageAlignment,
            "packed_reserve" => ir::TypeQueryKind::PackedByteReserveUnit,
            _ => return Err("private type query outside its canonical source body".into()),
        };
        if plan
            .program
            .names
            .spelling(module)
            .map_err(native_adapter::name_diagnostic)?
            != "mem"
            || declaration.owner.is_some()
        {
            return Err(
                "private target alignment operation outside its canonical source body".into(),
            );
        }
        let queried = type_check::checked_local_ty(queried, self.signatures)?;
        let query = if kind == ir::TypeQueryKind::PackedByteReserveUnit {
            let authority = plan.packed_reserve_authority(facts.declaration())?;
            ir::TypeAlignmentQuery::issue_packed(queried, facts, authority)?
        } else if kind == ir::TypeQueryKind::Alignment {
            ir::TypeAlignmentQuery::issue(queried, facts)?
        } else {
            ir::TypeAlignmentQuery::issue_kind(queried, facts, kind)?
        };
        super::super::memory_layout::query_value(&query, self.facts(), &plan.budget)?;
        Ok((
            ir::ExprKind::TypeAlignment(query),
            type_check::Flow::value(ir::Ty::Int(ir::IntTy::Usize)),
        ))
    }
}
