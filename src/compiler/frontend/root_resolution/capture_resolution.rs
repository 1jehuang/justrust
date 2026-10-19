//! Source-site capture resolution. All scratch dies with the RESOLVE task.
//! The resulting descriptors are immutable parsed graph output.
use super::*;

impl Resolver<'_> {
    pub(super) fn reject_mutable_capture(
        &self,
        name: LocalName,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<(), String> {
        if let LocalName::Named(ident) = name {
            let path = self.single_path(ident)?;
            if !self.local_contains(&path, locals)? {
                return Err("unsupported: mutable closure captures are not implemented".into());
            }
        }
        Ok(())
    }
    pub(super) fn begin_closure_function(&mut self, function: &Function) -> Result<(), String> {
        self.current_function_name = Some(function.name.clone());
        self.active_closure = None;
        if self.anonymous_closure_body {
            let key = self.key(&function.name)?;
            self.alias_budget.charge_work(1)?;
            self.active_closure = self.program.closure_sources.get(&key).cloned();
            if self.active_closure.is_none() {
                return Err("anonymous closure lacks its original source site".into());
            }
        }
        Ok(())
    }

    pub(super) fn finish_closure_function(
        &mut self,
        function: &mut Function,
    ) -> Result<(), String> {
        let Some(source) = self.active_closure.take() else {
            return Ok(());
        };
        if !source.captures.is_empty() {
            self.charge_entries::<(BindingName, Type)>(source.captures.len())?;
            function
                .params
                .try_reserve(source.captures.len())
                .map_err(|_| "closure environment parameter allocation")?;
            function.params.splice(
                0..0,
                source.captures.iter().map(|capture| {
                    (
                        BindingName::Named(capture.environment),
                        Type::Ref {
                            kind: ir::BorrowKind::Shared,
                            region: None,
                            referent: Box::new(capture.ty.clone()),
                        },
                    )
                }),
            );
            function
                .param_mut
                .try_reserve(source.captures.len())
                .map_err(|_| "closure environment mutability allocation")?;
            function
                .param_mut
                .splice(0..0, source.captures.iter().map(|_| false));
        }
        self.charge_entries::<closures::ClosureSource>(1)?;
        self.resolved_closures
            .try_reserve(1)
            .map_err(|_| "resolved closure source allocation")?;
        let key = self.key(&source.name)?;
        if !source.captures.is_empty() {
            let parent = source
                .parent
                .as_ref()
                .ok_or("capturing closure lacks parent")?;
            let owner = self.key(&parent.name)?;
            self.charge_entries::<PathKey>(1)?;
            self.resolved_closure_owners
                .try_reserve(1)
                .map_err(|_| "closure owner index allocation")?;
            let children = self.resolved_closure_owners.entry(owner).or_default();
            children
                .try_reserve(1)
                .map_err(|_| "closure owner sites allocation")?;
            children.push(key.clone());
        }
        if self.resolved_closures.insert(key, source).is_some() {
            return Err("duplicate resolved closure source".into());
        }
        Ok(())
    }

    /// Only original explicit scalar parent parameters are admitted. A shadowed
    /// local is checked again at the construction site, never guessed by text.
    pub(super) fn capture_place(&mut self, path: &AstPath) -> Result<Place, String> {
        let ident = path
            .single()
            .ok_or("unsupported: nonlocal closure capture path")?;
        let key = self.ident_key(ident, LookupClass::Local)?;
        let source = self
            .active_closure
            .as_ref()
            .ok_or("unsupported: closure has no capture source")?;
        if source.by_move {
            return Err("unsupported: move capture environments are not implemented".into());
        }
        let parent = source
            .parent
            .as_ref()
            .ok_or("unsupported: closure requires an explicit free-function parent")?;
        let mut matched = None;
        for (parameter, (name, ty)) in parent.params.iter().enumerate() {
            self.alias_budget.charge_work(1)?;
            if let BindingName::Named(name) = name
                && self.ident_key(*name, LookupClass::Local)? == key
            {
                if !matches!(
                    ty,
                    Type::Unit | Type::Bool | Type::Char | Type::Int(_) | Type::Float(_)
                ) {
                    return Err(
                        "unsupported: closure captures currently require scalar parent parameters"
                            .into(),
                    );
                }
                matched = Some((parameter, *name, ty.clone()));
                break;
            }
        }
        let (parameter, name, ty) = matched.ok_or(
            "unsupported: closure captures of inferred or local bindings are not implemented",
        )?;
        let existing = source
            .captures
            .iter()
            .position(|capture| capture.parameter == parameter);
        let environment = if let Some(slot) = existing {
            source.captures[slot].environment
        } else {
            self.charge_entries::<closures::SourceCapture>(1)?;
            let source = self
                .active_closure
                .as_mut()
                .ok_or("missing active closure source")?;
            let environment = *source
                .environments
                .get(parameter)
                .ok_or("capture field binder outside original source")?;
            source
                .captures
                .try_reserve(1)
                .map_err(|_| "closure capture allocation")?;
            source.captures.push(closures::SourceCapture {
                parameter,
                name,
                environment,
                ty,
            });
            environment
        };
        Ok(Place {
            local: LocalName::Named(environment),
            fields: vec![FieldSelector::Deref],
        })
    }

    pub(super) fn capture_read(&mut self, path: &AstPath) -> Result<Expr, String> {
        let place = self.capture_place(path)?;
        let LocalName::Named(environment) = place.local else {
            return Err("invalid closure environment root".into());
        };
        Ok(Expr::Deref {
            value: Box::new(Expr::Var(self.single_path(environment)?)),
        })
    }

    pub(super) fn closure_arguments(
        &mut self,
        name: &AstPath,
        args: &mut Vec<Expr>,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<(), String> {
        let key = self.key(name)?;
        self.alias_budget.charge_work(1)?;
        let source = self
            .resolved_closures
            .get(&key)
            .cloned()
            .ok_or("anonymous call has no resolved original closure body")?;
        if source.captures.is_empty() {
            return Ok(());
        }
        if self.lexical_function.is_some() {
            return Err(
                "unsupported: captured closures with source macro scopes are not implemented"
                    .into(),
            );
        }
        let parent = source
            .parent
            .as_ref()
            .ok_or("capturing closure has no original parent")?;
        let current = self
            .current_function_name
            .as_ref()
            .ok_or("capturing closure outside a free function")?;
        if self.key(current)? != self.key(&parent.name)? {
            return Err("closure environment belongs to another source owner".into());
        }
        let mut fields = Vec::new();
        self.charge_entries::<Expr>(source.captures.len())?;
        fields
            .try_reserve_exact(source.captures.len())
            .map_err(|_| "closure environment allocation")?;
        for capture in &source.captures {
            let path = self.single_path(capture.name)?;
            let key = self
                .local_path(&path)?
                .ok_or("unsupported: capture has no lexical binding")?;
            let mut owner_scope = None;
            for (index, scope) in locals.iter().enumerate().rev() {
                self.alias_budget.charge_work(1)?;
                if scope.contains(&key) {
                    owner_scope = Some(index);
                    break;
                }
            }
            if owner_scope != Some(0) {
                return Err("unsupported: shadowed or non-parameter closure capture".into());
            }
            fields.push(Expr::Borrow {
                kind: ir::BorrowKind::Shared,
                place: Place {
                    local: LocalName::Named(capture.name),
                    fields: Vec::new(),
                },
            });
        }
        self.charge_entries::<Expr>(fields.len())?;
        args.try_reserve(fields.len())
            .map_err(|_| "closure environment argument allocation")?;
        args.splice(0..0, fields);
        Ok(())
    }
}
