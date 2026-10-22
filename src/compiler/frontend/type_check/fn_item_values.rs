//! Function items used as values (`f` in `apply(f, x)`). The value is the
//! zero-sized `Ty::FnItem` of the exact resolved declaration and its ordered
//! actuals, never an eta-expanded closure. Generic items need explicit or
//! inferred actuals and are not admitted in this slice.
use super::*;

impl Checker<'_> {
    /// Exact function item type for a value path that names a function and no
    /// local or closure binding. Locals shadow items, as in stock Rust.
    pub(super) fn fn_item_value(&self, path: &AstPath) -> Result<Option<ir::Ty>, String> {
        if is_local_shaped_path(path) {
            return Ok(None);
        }
        let plan = self.signatures.plan()?;
        if plan.stored_closure_syntax(path)?.is_some() {
            return Ok(None);
        }
        let Some(declaration) = plan
            .registry
            .functions
            .get(&self.path_key(path, Namespace::Value)?)
            .copied()
        else {
            return Ok(None);
        };
        if plan.param_env(declaration)?.arity() > 0 {
            return Err(
                "unsupported: generic function items as values require explicit type arguments"
                    .into(),
            );
        }
        Ok(Some(ir::Ty::FnItem(std::sync::Arc::new(ir::CallTarget {
            declaration,
            type_args: std::sync::Arc::from([]),
        }))))
    }
}
