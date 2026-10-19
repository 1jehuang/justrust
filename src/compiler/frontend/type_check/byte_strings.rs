//! Reuse the existing checked array-to-slice transport for literal array reads.
//! No bytes or lifetimes are inferred from spelling at this boundary.
use super::*;

impl Checker<'_> {
    pub(super) fn array_reference_as_slice(
        &mut self,
        value: ir::ExprId,
    ) -> Result<ir::ExprId, String> {
        let Some((element, _)) = self.exprs[value.0].ty.shared_array_element_len() else {
            return Ok(value);
        };
        let target = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Slice {
                element: Arc::new(element.clone()),
            }),
        };
        self.coerce_shared_array_to_slice(value, &target)
    }
}
