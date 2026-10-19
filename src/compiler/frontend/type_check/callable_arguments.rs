//! Prepare inline closure declarations before generic argument evidence is read.
//!
//! Preparation checks a child body but emits no parent expression, so receiver
//! and argument evaluation remain in their original source order. Never cross a
//! block or pattern binder here: those closures require that scope's checker.
use super::*;

impl Checker<'_> {
    pub(super) fn callable_value_hint(&self, path: &AstPath) -> Result<Option<ir::Ty>, String> {
        if self
            .signatures
            .plan()?
            .stored_closure_syntax(path)?
            .is_some()
        {
            return self.prepared_closure_type(path);
        }
        Ok(self
            .lookup_optional(path)?
            .map(|local| self.locals[local.0].ty.clone()))
    }

    pub(super) fn prepared_closure_type(&self, path: &AstPath) -> Result<Option<ir::Ty>, String> {
        let plan = self.signatures.plan()?;
        let Some(syntax) = plan.stored_closure_syntax(path)? else {
            return Ok(None);
        };
        let key = self.path_key(path, Namespace::Value)?;
        let provider = *plan
            .registry
            .functions
            .get(&key)
            .ok_or("closure adapter unavailable")?;
        let owner = self.declaration.ok_or("closure outside a callable")?;
        if plan.stored_closure_owner(provider)? != Some((owner, syntax.site)) {
            return Err("inline closure belongs to another parent body".into());
        }
        Ok(self
            .checked_closures
            .iter()
            .find(|checked| {
                checked.ty.provider() == provider
                    && checked.ty.owner() == owner
                    && checked.ty.site() == syntax.site
            })
            .map(|checked| ir::Ty::Closure(checked.ty.clone())))
    }

    pub(super) fn prepare_callable_arguments(
        &mut self,
        expression: &Expr,
        depth: usize,
    ) -> Result<(), String> {
        self.infer_work(depth)?;
        match expression {
            Expr::Var(path) => {
                // The closure checker authenticates source owner, site, final
                // parameter/result types and captures before publishing metadata.
                let key = self.path_key(path, Namespace::Value)?;
                let hint = self.closure_signature_hints.get(&key).cloned();
                if self.prepare_stored_closure(path, hint.as_ref())?.is_some() {
                    self.inference_evidence_cache.borrow_mut().clear();
                }
            }
            Expr::Call { args, .. }
            | Expr::AssociatedCall { args, .. }
            | Expr::TraitCall { args, .. }
            | Expr::Tuple(args)
            | Expr::Array(args)
            | Expr::TupleStruct { fields: args, .. } => {
                for argument in args {
                    self.prepare_callable_arguments(argument, depth + 1)?;
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.prepare_callable_arguments(receiver, depth + 1)?;
                for argument in args {
                    self.prepare_callable_arguments(argument, depth + 1)?;
                }
            }
            Expr::Struct { fields, .. } => {
                for (_, field) in fields {
                    self.prepare_callable_arguments(field, depth + 1)?;
                }
            }
            Expr::EnumVariant { fields, .. } => {
                for (_, field) in fields {
                    self.prepare_callable_arguments(field, depth + 1)?;
                }
            }
            Expr::Apply { value, .. }
            | Expr::Cast { value, .. }
            | Expr::BorrowValue { value, .. }
            | Expr::Deref { value }
            | Expr::Repeat { value, .. }
            | Expr::Unary { value, .. } => self.prepare_callable_arguments(value, depth + 1)?,
            Expr::Field { base, .. } | Expr::Index { base, .. } => {
                self.prepare_callable_arguments(base, depth + 1)?;
            }
            // Branch/block bodies and pattern bindings are deliberately not
            // visited until their own lexical environment is active.
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_callable_evidence_rejects_foreign_owner_and_argument_signature() {
        let program =
            parse("fn first(){let f=|x:i32|x+1;}fn second(){let f=|x:i32|x+2;}fn main(){}")
                .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let sites: Vec<_> = (0..plan.callable_count())
            .filter_map(|index| {
                let provider = plan.callable_decl(index).ok()?;
                plan.stored_closure_owner(provider)
                    .ok()
                    .flatten()
                    .map(|(owner, _)| (provider, owner))
            })
            .collect();
        assert_eq!(sites.len(), 2);
        let (provider, owner) = sites[0];
        let path = plan.callable_body(&program, provider).unwrap().name.clone();
        let signatures = plan.environment(Some(owner)).unwrap();
        let module = plan.callable_body(&program, owner).unwrap().module;
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, module);
        checker.declaration = Some(owner);
        checker
            .prepare_callable_arguments(&Expr::Var(path.clone()), 0)
            .unwrap();
        let Some(ir::Ty::Closure(closure)) = checker.callable_value_hint(&path).unwrap() else {
            panic!("prepared declaration did not supply canonical closure evidence");
        };
        let checked = checker
            .checked_closures
            .iter()
            .find(|body| body.ty.provider() == provider)
            .unwrap();
        let target = ir::CallTarget {
            declaration: provider,
            type_args: Arc::from([
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Closure(closure.clone())),
                },
                ir::Ty::Tuple(Arc::from([ir::Ty::Bool])),
                closure.result().clone(),
            ]),
        };
        assert!(plan.validate_closure_instance(checked, &target).is_err());
        let foreign = plan
            .callable_body(&program, sites[1].0)
            .unwrap()
            .name
            .clone();
        assert!(
            checker
                .prepare_callable_arguments(&Expr::Var(foreign.clone()), 0)
                .is_err()
        );
        assert!(checker.callable_value_hint(&foreign).is_err());
    }
}
