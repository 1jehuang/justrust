//! Function items used as values (`f` in `apply(f, x)`). The value is the
//! zero-sized `Ty::FnItem` of the exact resolved declaration and its ordered
//! actuals, never an eta-expanded closure. A generic free function's actuals
//! are solved per occurrence by the body solver.
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
        let arity = plan.param_env(declaration)?.arity();
        if arity == 0 {
            return Ok(Some(ir::Ty::FnItem(std::sync::Arc::new(ir::CallTarget {
                declaration,
                type_args: std::sync::Arc::from([]),
            }))));
        }
        // `id` as a value: the body solver publishes the ordered actuals it
        // solved for this exact occurrence. The trait world still proves the
        // item's own predicates for them when the value is used.
        match self.generic_fn_item_actuals(path, arity) {
            Some(type_args) => Ok(Some(ir::Ty::FnItem(std::sync::Arc::new(ir::CallTarget {
                declaration,
                type_args,
            })))),
            None => Err(
                "E0282: type annotations needed: cannot infer the generic arguments of this function value"
                    .into(),
            ),
        }
    }

    /// The generic free function a nonlocal value path names, if any.
    pub(in crate::compiler::frontend) fn generic_fn_item_declaration(
        &self,
        path: &AstPath,
    ) -> Result<Option<ir::DeclId>, String> {
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
        Ok((plan.param_env(declaration)?.arity() > 0).then_some(declaration))
    }

    /// Solved ordered actuals of a generic function value occurrence. The
    /// solver publishes them as one tuple keyed by the occurrence's address,
    /// beside the inferred owners of associated function values.
    fn generic_fn_item_actuals(
        &self,
        path: &AstPath,
        arity: usize,
    ) -> Option<std::sync::Arc<[ir::Ty]>> {
        match self
            .associated_owner_hints
            .get(&(path as *const AstPath as usize))?
        {
            ir::Ty::Tuple(actuals) if actuals.len() == arity => Some(actuals.clone()),
            _ => None,
        }
    }

    /// Like `fn_item_value`, but None while a generic item has no solved
    /// actuals yet (hint positions must not diagnose).
    pub(super) fn fn_item_value_hint(&self, path: &AstPath) -> Result<Option<ir::Ty>, String> {
        if let Some(declaration) = self.generic_fn_item_declaration(path)? {
            let arity = self.signatures.plan()?.param_env(declaration)?.arity();
            if self.generic_fn_item_actuals(path, arity).is_none() {
                return Ok(None);
            }
        }
        self.fn_item_value(path)
    }
}

impl Checker<'_> {
    /// True for `Vec::new` as a value: the owner was written without its
    /// generic arguments, which the body solver infers.
    pub(super) fn fn_item_owner_inferred(&self, value: &AssociatedConstUse) -> bool {
        value.trait_path.is_none()
            && matches!(value.owner, Type::Apply(_))
            && partial_annotation::has_nested_infer(&value.owner)
    }

    /// The function item type when it is already determined. None while an
    /// inferred owner has no solved hint yet.
    pub(super) fn associated_fn_item_hint(
        &self,
        value: &AssociatedConstUse,
    ) -> Result<Option<ir::Ty>, String> {
        if self.fn_item_owner_inferred(value)
            && !self
                .associated_owner_hints
                .contains_key(&(&value.owner as *const Type as usize))
        {
            return Ok(None);
        }
        self.associated_fn_item(value).map(Some)
    }

    /// `Owner::name` as a value: the exact associated function selected for the
    /// checked owner type, as a zero-sized function item. Method generics are
    /// never inferred here.
    pub(super) fn associated_fn_item(&self, value: &AssociatedConstUse) -> Result<ir::Ty, String> {
        if value.trait_path.is_some() {
            return Err("unsupported: trait-qualified associated function values".into());
        }
        let plan = self.signatures.plan()?;
        let owner = if self.fn_item_owner_inferred(value) {
            self.associated_owner_hints
                .get(&(&value.owner as *const Type as usize))
                .cloned()
                .ok_or(
                    "E0282: type annotations needed: cannot infer the owner type of this function value",
                )?
        } else {
            checked_local_ty(&value.owner, self.signatures)?
        };
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
        self.selected_fn_item(callee)
    }

    /// The function item of a selected associated callee. `i32::max`,
    /// `str::parse::<i32>`: a trait item selected for a closed owner resolves
    /// to its exact implementation or provided default body, never a dispatch.
    fn selected_fn_item(&self, callee: ir::Callee) -> Result<ir::Ty, String> {
        let plan = self.signatures.plan()?;
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

impl Checker<'_> {
    /// `id::<u8>`, `str::parse::<i32>` as values: a function path applied to
    /// explicit generic arguments. None when the application is not a
    /// function value (calls and constructors keep their own paths).
    pub(in crate::compiler::frontend) fn applied_fn_item(
        &self,
        path: &GenericPath,
        value: &Expr,
    ) -> Result<Option<ir::Ty>, String> {
        let plan = self.signatures.plan()?;
        match value {
            Expr::AssociatedFn(function) => {
                if function.trait_path.is_some() {
                    return Err("unsupported: trait-qualified associated function values".into());
                }
                if self.fn_item_owner_inferred(function) {
                    return Err(
                        "unsupported: method generic arguments on a function value whose owner arguments are inferred"
                            .into(),
                    );
                }
                let written = &path
                    .segments
                    .last()
                    .ok_or("associated function value lost its member segment")?
                    .args;
                self.infer_storage(written.len())?;
                let mut actuals = Vec::new();
                for argument in written {
                    actuals.push(checked_local_ty(argument, self.signatures)?);
                }
                let owner = checked_local_ty(&function.owner, self.signatures)?;
                let (callee, _, _) = plan.select_associated_with_scratch(
                    &owner,
                    &function.name,
                    &actuals,
                    self.facts(),
                    &self.module,
                    false,
                    &plan.budget,
                    &self.trait_scratch,
                )?;
                self.selected_fn_item(callee).map(Some)
            }
            Expr::Var(name) => {
                if is_local_shaped_path(name) || plan.stored_closure_syntax(name)?.is_some() {
                    return Ok(None);
                }
                let Some(declaration) = plan
                    .registry
                    .functions
                    .get(&self.path_key(name, Namespace::Value)?)
                    .copied()
                else {
                    return Ok(None);
                };
                let (_, actuals) = plan.application(
                    path,
                    Namespace::Value,
                    &self.signatures.params,
                    self.facts(),
                    &plan.budget,
                )?;
                let anonymous = plan
                    .registry
                    .source_callable(declaration)
                    .map_or(0, |function| function.impl_params);
                let arity = plan.param_env(declaration)?.arity();
                if anonymous != 0 {
                    return Err(
                        "unsupported: explicit generic arguments on a function value with `impl Trait` parameters"
                            .into(),
                    );
                }
                if actuals.len() != arity {
                    return Err(format!(
                        "E0107: function takes {arity} generic arguments but {} were supplied",
                        actuals.len()
                    ));
                }
                Ok(Some(ir::Ty::FnItem(std::sync::Arc::new(ir::CallTarget {
                    declaration,
                    type_args: actuals.into(),
                }))))
            }
            _ => Ok(None),
        }
    }
}
