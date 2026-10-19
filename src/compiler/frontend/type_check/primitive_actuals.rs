//! Fail closed when implicit trait actual inference would rely on coercion.
//! Explicit UFCS already supplies the trait arguments and keeps ordinary call
//! coercions. Primitive associated syntax does not supply those arguments.
use super::*;

impl Checker<'_> {
    pub(super) fn primitive_associated_actuals(
        &self,
        source: &Expr,
        owner: &ir::Ty,
        call: ir::ExprId,
    ) -> Result<(), String> {
        if !matches!(source, Expr::AssociatedCall { .. })
            || !matches!(
                owner,
                ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Char | ir::Ty::Bool | ir::Ty::Str
            )
        {
            return Ok(());
        }
        let ir::ExprKind::Call {
            args, signature, ..
        } = &self.exprs[call.0].kind
        else {
            return Err("primitive associated call lost checked call shape".into());
        };
        let facts = self.facts();
        let authority = facts
            .validated()
            .ok_or("primitive associated call requires validated facts")?;
        for (actual, formal) in args.iter().zip(signature.params.iter()) {
            if !authority
                .world()
                .types_equal(
                    std::slice::from_ref(&self.exprs[actual.0].ty),
                    std::slice::from_ref(formal),
                )
                .map_err(|error| error.into_legacy_diagnostic(authority.budget()))?
            {
                return Err("unsupported: inferred primitive associated trait arguments requiring coercion; use explicit UFCS trait arguments".into());
            }
        }
        Ok(())
    }
}
