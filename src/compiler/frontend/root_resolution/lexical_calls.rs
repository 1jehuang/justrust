//! Classify source calls by genuine lexical identity before module lookup.
use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_lexical_call(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<bool, String> {
        let Expr::Call { name, .. } = expr else {
            return Ok(false);
        };
        let key = self.local_path(name)?;
        let mut found = false;
        if let Some(key) = key {
            for scope in locals.iter().rev() {
                self.alias_budget.charge_work(1)?;
                if scope.contains(&key) {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            if matches!(key, Some(LocalKey::Anchored(_))) {
                return Err("lexical callable binding is absent from its source scope".into());
            }
            if name.anchor() == PathAnchor::Relative && name.single().is_some() {
                // A child's unresolved occurrence does not establish absence
                // of an outer-function capture. Do not silently rebind it to
                // a same-spelled item until explicit cross-owner scope
                // occurrences exist. Qualified item calls remain unaffected.
                for scope in locals.iter() {
                    for local in scope {
                        self.alias_budget.charge_work(1)?;
                        let LocalKey::Anchored(binding) = local else {
                            continue;
                        };
                        let arena = self
                            .program
                            .scopes
                            .as_ref()
                            .ok_or("lexical callable binding has no source scope arena")?;
                        let owner = arena.binding_function(*binding).map_err(scope_diagnostic)?;
                        if self.lexical_function != Some(owner) {
                            return Err("unsupported: unqualified closure call requires its cross-function lexical capture occurrence".into());
                        }
                    }
                }
            }
            // Recorded unresolved in the same function, or an unbound name in
            // a non-macro body, is a genuine source item candidate. A closure
            // body cannot see its parent's locals here: record the name so
            // the parent call site rejects it when a parent local shadows it.
            if self.anonymous_closure_body
                && name.anchor() == PathAnchor::Relative
                && let Some(ident) = name.single()
                && let Some(source) = self.active_closure.as_mut()
            {
                self.alias_budget.charge_work(1)?;
                source
                    .item_calls
                    .try_reserve(1)
                    .map_err(|_| "closure item call allocation")?;
                source.item_calls.push(ident);
            }
            return Ok(false);
        }
        let Expr::Call { name, args } = expr else {
            return Err("lexical callable source changed during classification".into());
        };
        let method = self.callable_call_ident()?;
        let receiver = Expr::Var(name.clone());
        let args = std::mem::take(args);
        let arguments = if args.is_empty() {
            Expr::Block(Block {
                statements: Vec::new(),
                tail: None,
            })
        } else {
            Expr::Tuple(args)
        };
        *expr = Expr::MethodCall {
            receiver: Box::new(receiver),
            name: method,
            type_args: Vec::new(),
            args: vec![arguments],
        };
        self.expr(expr, module, locals)?;
        Ok(true)
    }
}
