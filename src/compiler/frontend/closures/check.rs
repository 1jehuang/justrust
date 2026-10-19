//! Stored callable values and their immutable parent-body-owned executables.
use super::super::type_check::{Checker, checked_ty};
use super::*;
#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;

#[derive(Clone)]
pub(in crate::compiler) struct CheckedClosure {
    pub ty: Arc<ir::ClosureTy>,
    pub body: Arc<ir::CheckedFunction>,
}

impl PlanCtx {
    pub(in crate::compiler) fn validate_closure_instance(
        &self,
        original: &CheckedClosure,
        target: &ir::CallTarget,
    ) -> Result<(), String> {
        if target.declaration != original.ty.provider() {
            return Err("closure provider changed from its parent-body fact".into());
        }
        let receiver = target
            .type_args
            .first()
            .ok_or("closure instance has no receiver actual")?;
        let environment = match receiver {
            ir::Ty::Ref { referent, .. } => referent.as_ref(),
            other => other,
        };
        if !self
            .trait_world()?
            .types_equal(
                std::slice::from_ref(environment),
                &[ir::Ty::Closure(original.ty.clone())],
            )
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))?
        {
            return Err(
                "closure instance metadata differs from its original parent-body fact".into(),
            );
        }
        self.instance_facts(target)?;
        Ok(())
    }
    pub(in crate::compiler::frontend) fn closure_method_selection(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Option<MethodSelection>, String> {
        let ir::Ty::Closure(closure) = receiver else {
            return Ok(None);
        };
        if self
            .program
            .names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            != "call"
        {
            return Ok(None);
        }
        let kind = closure.kind();
        let world = self.trait_world()?;
        for (index, declaration) in self.registry.trait_ids.iter().enumerate() {
            if world
                .closure_trait_kind(*declaration)
                .map_err(|e| e.into_legacy_diagnostic(&self.budget))?
                != Some(kind)
            {
                continue;
            }
            let item = *self.registry.trait_items[index]
                .first()
                .ok_or("callable trait lacks call item")?;
            let arguments = if closure.params().is_empty() {
                ir::Ty::Unit
            } else {
                ir::Ty::Tuple(closure.params().into())
            };
            let call = ir::TraitCall {
                trait_ref: ir::TraitRef {
                    declaration: *declaration,
                    self_ty: receiver.clone(),
                    args: Arc::from([arguments]),
                },
                item,
                method_args: Arc::from([]),
            };
            let ir::FactsCtx::Validated(facts) = caller else {
                return Err("closure method requires authoritative caller facts".into());
            };
            let selected = ir::TraitCx::resolve_trait_call(world.as_ref(), &call, facts)
                .map_err(|e| e.into_legacy_diagnostic(&self.budget))?;
            let (mode, adjustment) = match kind {
                ir::ClosureKind::Fn => (
                    ReceiverKind::Borrowed {
                        kind: ir::BorrowKind::Shared,
                    },
                    ReceiverAdjustment::SharedAutoref,
                ),
                ir::ClosureKind::FnMut => (
                    ReceiverKind::Borrowed {
                        kind: ir::BorrowKind::Mut,
                    },
                    ReceiverAdjustment::MutAutoref,
                ),
                ir::ClosureKind::FnOnce => (
                    ReceiverKind::Value { mutable: false },
                    ReceiverAdjustment::Identity,
                ),
            };
            return Ok(Some(MethodSelection {
                target: selected.callee().clone(),
                signature: selected.signature().clone(),
                mode,
                depth: 0,
                adjustment,
            }));
        }
        Err("canonical callable trait missing from prelude".into())
    }
    pub(in crate::compiler) fn stored_closure_owner(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Option<(ir::DeclId, u32)>, String> {
        if !matches!(
            self.callable_provider(declaration)?,
            CallableProvider::Source { .. }
        ) {
            return Ok(None);
        }
        let function = self.callable_body(&self.program, declaration)?;
        let key =
            name_scope::path_key(self.program.names.as_ref(), &function.name, Namespace::Type)?;
        let Some(source) = self.program.closure_sources.get(&key) else {
            return Ok(None);
        };
        let Some(syntax) = &source.stored else {
            return Ok(None);
        };
        let parent = source
            .parent
            .as_ref()
            .ok_or("stored closure lacks parent")?;
        let key =
            name_scope::path_key(self.program.names.as_ref(), &parent.name, Namespace::Value)?;
        let owner = *self
            .registry
            .functions
            .get(&key)
            .ok_or("stored closure owner unavailable")?;
        Ok(Some((owner, syntax.site)))
    }
}

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn check_stored_closure(
        &mut self,
        path: &AstPath,
        expected: Option<&ir::Signature>,
    ) -> Result<Option<ir::ExprId>, String> {
        let plan = self.signatures.plan()?;
        let Some(syntax) = plan.stored_closure_syntax(path)? else {
            return Ok(None);
        };
        let syntax = syntax.clone();
        let key = name_scope::path_key(plan.program.names.as_ref(), path, Namespace::Value)?;
        let provider = *plan
            .registry
            .functions
            .get(&key)
            .ok_or("closure adapter unavailable")?;
        let owner = self.declaration.ok_or("closure outside a callable")?;
        if plan.stored_closure_owner(provider)? != Some((owner, syntax.site)) {
            return Err("closure belongs to another parent body".into());
        }
        let owner_env = plan.callable_env(owner)?;
        if owner_env.is_generic() {
            return Err("unsupported: stored closure in generic parent body".into());
        }
        let mut parameter_types = Vec::new();
        for (index, (_, ty)) in syntax.params.iter().enumerate() {
            parameter_types.push(if matches!(ty, Type::Infer) {
                expected
                    .and_then(|signature| signature.params.get(index))
                    .cloned()
                    .ok_or("closure parameter needs an inferred callable signature")?
            } else {
                checked_ty(ty, self.signatures)?
            });
        }
        let result_hint = if matches!(syntax.result, Type::Infer) {
            expected.map(|signature| signature.result.clone())
        } else {
            Some(checked_ty(&syntax.result, self.signatures)?)
        };
        let signatures = plan.environment(Some(provider))?;
        let mut child = Checker::new(
            &signatures,
            result_hint.clone().unwrap_or(ir::Ty::Unit),
            self.module,
        );
        child.declaration = Some(provider);
        child.install_lexical_scope(syntax.lexical_scope)?;
        let env = plan.callable_env(provider)?;
        let receiver = child.temporary(
            "closure self",
            ir::Ty::Param(
                env.ordered_params()
                    .next()
                    .ok_or("closure receiver binder missing")?,
            ),
            false,
        )?;
        let arguments_ty = if parameter_types.is_empty() {
            ir::Ty::Unit
        } else {
            ir::Ty::Tuple(parameter_types.clone().into())
        };
        let arguments = child.temporary("closure arguments", arguments_ty.clone(), false)?;
        let mut prefix = Vec::new();
        for (index, ((name, _), ty)) in syntax.params.iter().zip(parameter_types.iter()).enumerate()
        {
            let local = child.bind(name, ty.clone())?;
            child.locals[local.0].mutable = syntax.param_mut.get(index).copied().unwrap_or(false);
            let base = child.for_expr(arguments_ty.clone(), ir::ExprKind::Local(arguments))?;
            let value = child.for_expr(ty.clone(), ir::ExprKind::Field { base, index })?;
            prefix.push(ir::Stmt::Let { local, value });
        }
        let (mut body, flow) = child.block_expected(&syntax.body, result_hint.as_ref())?;
        let result = result_hint.unwrap_or(flow.ty.clone());
        child.require(&flow, &result)?;
        prefix.append(&mut body.statements);
        body.statements = prefix;
        let closure = Arc::new(ir::ClosureTy::new(
            owner,
            Arc::from([]),
            syntax.site,
            provider,
            parameter_types.into(),
            result.clone(),
            Arc::from([]),
            ir::ClosureKind::Fn,
        ));
        let facts = plan
            .trait_world()?
            .closure_callable_facts(closure.clone())
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
        let signature = facts
            .world()
            .closure_body_signature(&facts)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
        let checked = ir::CheckedFunction {
            name: format!("closure {:?}:{}", owner, syntax.site),
            ref_contract: signature.ref_contract,
            params: vec![receiver, arguments],
            result,
            locals: child.locals,
            exprs: child.exprs,
            call_evidence: child.call_evidence.into(),
            body_substitution: None,
            body,
        };
        self.checked_closures.extend(child.checked_closures);
        self.checked_closures.push(CheckedClosure {
            ty: closure.clone(),
            body: Arc::new(checked),
        });
        self.for_expr(ir::Ty::Closure(closure), ir::ExprKind::Tuple(Vec::new()))
            .map(Some)
    }
}
