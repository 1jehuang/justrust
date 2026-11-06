//! Source-owned replay of `&owner.field` through one canonical Deref call.
use super::*;

impl Replay<'_> {
    pub(super) fn projected_borrow(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        block: &ir::TypedBlock,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        let Expr::Borrow { kind, place } = source else {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow lost source syntax",
            ));
        };
        let Some(first) = place.fields.first() else {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow lost source fields",
            ));
        };
        if !matches!(first, FieldSelector::Named(_) | FieldSelector::Tuple(_)) {
            return Err(FactsError::unsupported(
                self.budget,
                "projected borrow requires a stored field after one Deref",
            ));
        }
        let (owner_local, owner) = self.operand(node, depth + 1)?;
        let ir::Ty::Struct(nominal) = &owner else {
            return Err(FactsError::unsupported(
                self.budget,
                "projected borrow requires an owned nominal local",
            ));
        };
        let context = self
            .original
            .plan
            .callable_context(self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
        if owner.owned_box_pointee().is_some()
            || checker
                .explicit_field_stored(nominal, first)
                .map_err(|e| source_error(self.budget, &e))?
        {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow bypassed original stored-field precedence",
            ));
        }
        let ([ir::TypedStmt::Assign { local, value }], Some(tail)) =
            (block.statements.as_slice(), block.tail)
        else {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow carrier block changed",
            ));
        };
        // The operator type comes solely from the original lexical owner and
        // exact authored STEP1 capsule. Its entire canonical signature, callee,
        // reference contract, operand, evidence and actual regions are proved
        // by the shared coercion replay, not inferred from this checked Block.
        let carrier = self.explicit_borrow_coercion(source, node, *value, depth + 1)?;
        let (projection, referent) = checker
            .explicit_field_borrow_projection(*kind, carrier.clone(), &place.fields)
            .map_err(|e| source_error(self.budget, &e))?;
        sr::work(self.budget, self.original.binding_mapping.len())?;
        sr::work(self.budget, self.original.scoped_initializers.len())?;
        if *local == owner_local
            || self
                .original
                .binding_mapping
                .iter()
                .any(|(_, bound)| bound == local)
            || self
                .original
                .scoped_initializers
                .iter()
                .any(|(_, bound, _)| bound == local)
        {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow carrier aliases an authored local",
            ));
        }
        let temporary = self.checked.locals.get(local.0).ok_or_else(|| {
            FactsError::invalid(self.budget, "projected borrow carrier local missing")
        })?;
        let borrowed = self
            .checked
            .exprs
            .get(tail.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "projected borrow tail missing"))?;
        let call =
            self.checked.exprs.get(value.0).ok_or_else(|| {
                FactsError::invalid(self.budget, "projected borrow operator missing")
            })?;
        let wrapper =
            self.checked.exprs.get(id.0).ok_or_else(|| {
                FactsError::invalid(self.budget, "projected borrow block missing")
            })?;
        let result = self.reference(*kind, referent)?;
        if !matches!(&borrowed.kind,
            ir::ExprKind::Borrow { kind: actual_kind, place: actual_place, timing: ir::BorrowTiming::Immediate }
                if actual_kind == kind && actual_place.local == *local && actual_place.projection == projection)
            || temporary.mutable
            || !self.equal(&temporary.ty, &carrier)?
            || !self.equal(&borrowed.ty, &result)?
            || !self.equal(&wrapper.ty, &result)?
            || [wrapper.completion, borrowed.completion, call.completion]
                .iter()
                .any(|completion| *completion != ir::Completion::Returns)
        {
            return Err(FactsError::invalid(
                self.budget,
                "projected borrow carrier, source projection or immediate reference changed",
            ));
        }
        Ok(result)
    }
}
