//! Unsafe-operation checks (E0133): calls to `unsafe fn` require an enclosing
//! unsafe block or an `unsafe fn` body.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn require_safe_callable(&self, declaration: ir::DeclId) -> Result<(), String> {
        let plan = self.signatures.plan()?;
        for operation in [
            tr::CatalogItem::ManuallyDropNew,
            tr::CatalogItem::ManuallyDropIntoInner,
            tr::CatalogItem::ManuallyDropDrop,
            tr::CatalogItem::CellNew,
            tr::CatalogItem::CellGet,
            tr::CatalogItem::CellSet,
        ] {
            plan.budget.charge_work(1)?;
            if declaration == plan.catalog.id(operation) {
                return Err("unsupported: canonical ManuallyDrop operations require original dedicated source evidence".into());
            }
        }
        if self.unsafe_depth > 0 {
            return Ok(());
        }
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
