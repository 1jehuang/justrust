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
        let mut unsafety = plan
            .registry
            .source_callable(declaration)
            .is_some_and(|f| f.unsafety);
        if !unsafety {
            if let Ok(info) = plan.registry.declaration(declaration)
                && info.kind == ir::DeclKind::TraitItem
            {
                let owner = info.owner.ok_or("trait callable missing canonical owner")?;
                let owner_info = plan.registry.declaration(owner)?;
                if owner_info.kind != ir::DeclKind::Trait {
                    return Err("trait callable owner is not a canonical trait".into());
                }
                // Catalog trait items have no source declaration to inspect.
                // source_trait is a dyn-source lookup and rejects those owners.
                if owner_info.source_index != usize::MAX {
                    let source = plan.registry.source_trait(owner)?;
                    let item = source
                        .items
                        .get(info.source_index)
                        .ok_or("trait callable missing canonical source item")?;
                    unsafety = item.signature.unsafety;
                }
            }
        }
        if unsafety {
            return Err(
                "E0133: call to unsafe function is unsafe and requires unsafe block".into(),
            );
        }
        Ok(())
    }
}
