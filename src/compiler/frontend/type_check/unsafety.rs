//! Unsafe-operation checks (E0133): calls to `unsafe fn` require an enclosing
//! unsafe block or an `unsafe fn` body.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn require_safe_callable(&self, declaration: ir::DeclId) -> Result<(), String> {
        if self.unsafe_depth > 0 {
            return Ok(());
        }
        let plan = self.signatures.plan()?;
        if plan
            .registry
            .source_callable(declaration)
            .is_some_and(|f| f.unsafety)
        {
            return Err(
                "E0133: call to unsafe function is unsafe and requires unsafe block".into(),
            );
        }
        Ok(())
    }
}
