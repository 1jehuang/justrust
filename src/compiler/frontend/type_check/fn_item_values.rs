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

impl Checker<'_> {
    /// `Owner::name` as a value: the exact associated function selected for the
    /// checked owner type, as a zero-sized function item. Method generics are
    /// never inferred here.
    pub(super) fn associated_fn_item(&self, value: &AssociatedConstUse) -> Result<ir::Ty, String> {
        if value.trait_path.is_some() {
            return Err("unsupported: trait-qualified associated function values".into());
        }
        let plan = self.signatures.plan()?;
        let owner = checked_local_ty(&value.owner, self.signatures)?;
        let (callee, _, _) = plan.select_associated_with_scratch(
            &owner,
            &value.name,
            &[],
            self.facts(),
            &self.module,
            false,
            &plan.budget,
            &self.trait_scratch,
        )?;
        // `i32::max`: a trait item selected for a closed owner resolves to its
        // exact implementation or provided default body, never a dispatch.
        let target = match callee {
            ir::Callee::Direct(target) => target,
            ir::Callee::TraitItem(call) => {
                let ir::FactsCtx::Validated(facts) = self.facts() else {
                    return Err("associated function value requires validated facts".into());
                };
                match facts
                    .service()
                    .resolve_trait_call(&call, facts)
                    .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
                    .callee()
                {
                    ir::Callee::Direct(target) => target.clone(),
                    _ => {
                        return Err(
                            "unsupported: associated function value through an open trait bound"
                                .into(),
                        );
                    }
                }
            }
            _ => {
                return Err("unsupported: trait-dispatched associated function values".into());
            }
        };
        Ok(ir::Ty::FnItem(std::sync::Arc::new(target)))
    }
}
