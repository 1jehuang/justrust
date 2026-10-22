//! Stored callable values and their immutable parent-body-owned executables.
use super::super::type_check::Checker;
use super::*;
#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;

#[derive(Clone)]
pub(in crate::compiler) struct CheckedClosureBody {
    pub mode: ir::ClosureKind,
    pub body: Arc<ir::CheckedFunction>,
}
#[derive(Clone)]
pub(in crate::compiler) struct CheckedClosure {
    pub ty: Arc<ir::ClosureTy>,
    pub body: Arc<ir::CheckedFunction>,
    pub modes: Arc<[CheckedClosureBody]>,
    pub(super) authority: Arc<super::publication::CheckedClosureAuthority>,
}
impl CheckedClosure {
    pub(in crate::compiler) fn body_for(
        &self,
        mode: ir::ClosureKind,
    ) -> Result<Arc<ir::CheckedFunction>, String> {
        if !self.ty.supports(mode) {
            return Err("closure does not support this receiver capability".into());
        }
        if mode == self.ty.kind() {
            return Ok(self.body.clone());
        }
        self.modes
            .iter()
            .find(|body| body.mode == mode)
            .map(|body| body.body.clone())
            .ok_or_else(|| "closure capability has no final checked body".into())
    }
}

impl PlanCtx {
    fn bounded_closure_method_selection(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Option<MethodSelection>, String> {
        if self
            .program
            .names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            != "call"
        {
            return Ok(None);
        }
        let world = self.trait_world()?;
        // Only the resolver's canonical callable-syntax member authorizes this
        // adaptation, not an unrelated user method with the same spelling.
        let mut syntax_member = false;
        for (index, declaration) in self.registry.trait_ids.iter().enumerate() {
            if world
                .closure_trait_kind(*declaration)
                .map_err(|e| e.into_legacy_diagnostic(&self.budget))?
                == Some(ir::ClosureKind::Fn)
            {
                if let Some(item) = self.registry.trait_items[index].first()
                    && let DeclItemName::Source(member) = self.declaration(*item)?.path.item
                {
                    let names = self.program.names.as_ref();
                    syntax_member =
                        super::super::super::semantic_name::occurrence_index(names, member)
                            .map_err(native_adapter::name_diagnostic)?
                            == super::super::super::semantic_name::occurrence_index(names, *name)
                                .map_err(native_adapter::name_diagnostic)?;
                }
                break;
            }
        }
        if !syntax_member {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Ok(None);
        };
        let opaque_bounds = self.opaque_callable_bound_candidates(receiver, caller)?;
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            for predicate in facts
                .predicates()
                .predicates()
                .iter()
                .chain(opaque_bounds.iter())
            {
                let ir::Predicate::Implements(bound) = predicate else {
                    continue;
                };
                if world
                    .closure_trait_kind(bound.declaration)
                    .map_err(|e| e.into_legacy_diagnostic(&self.budget))?
                    != Some(mode)
                    || !world
                        .types_equal(
                            std::slice::from_ref(receiver),
                            std::slice::from_ref(&bound.self_ty),
                        )
                        .map_err(|e| e.into_legacy_diagnostic(&self.budget))?
                {
                    continue;
                }
                let index = self
                    .registry
                    .trait_ids
                    .iter()
                    .position(|id| *id == bound.declaration)
                    .ok_or("callable bound trait missing")?;
                let item = *self.registry.trait_items[index]
                    .first()
                    .ok_or("callable bound item missing")?;
                let DeclItemName::Source(member) = self.declaration(item)?.path.item else {
                    return Err("callable bound item has no source identity".into());
                };
                let Some((target, signature, receiver_mode)) =
                    self.select_trait(bound, &member, &[], caller, true, &self.budget)?
                else {
                    continue;
                };
                let adjustment = match mode {
                    ir::ClosureKind::Fn => ReceiverAdjustment::SharedAutoref,
                    ir::ClosureKind::FnMut => ReceiverAdjustment::MutAutoref,
                    ir::ClosureKind::FnOnce => ReceiverAdjustment::Identity,
                };
                return Ok(Some(MethodSelection {
                    target,
                    signature,
                    original_signature: None,
                    mode: receiver_mode,
                    depth: 0,
                    adjustment,
                }));
            }
        }
        Ok(None)
    }
    pub(in crate::compiler) fn closure_instance_mode(
        &self,
        target: &ir::CallTarget,
    ) -> Result<ir::ClosureKind, String> {
        let caller = self.callable_env(target.declaration)?;
        let (_, mode) = self
            .trait_world()?
            .closure_adapter_environment(target, &caller)
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))?;
        Ok(mode)
    }
    pub(in crate::compiler) fn validate_closure_instance(
        &self,
        original: &CheckedClosure,
        target: &ir::CallTarget,
    ) -> Result<(), String> {
        original.authority.validate(self, original)?;
        if target.declaration != original.ty.provider() {
            return Err("closure provider changed from its parent-body fact".into());
        }
        let caller = self.callable_env(target.declaration)?;
        let mode = self
            .trait_world()?
            .validate_closure_instance_substitution(&original.ty, target, &caller)
            .map_err(|e| e.into_legacy_diagnostic(&self.budget))?;
        original.body_for(mode)?;
        self.instance_facts(target)?;
        Ok(())
    }
    pub(in crate::compiler::frontend) fn closure_method_selection(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Option<MethodSelection>, String> {
        let (kind, params) = match receiver {
            ir::Ty::Closure(closure) => (closure.kind(), closure.params().to_vec()),
            // Direct invocation of a function item value needs the untupling
            // call form (FNITEM slice 3); never approximate it.
            ir::Ty::FnItem(_) => {
                return Err(
                    "unsupported: calling a function item value directly requires the untupling call form"
                        .into(),
                );
            }
            _ => return self.bounded_closure_method_selection(receiver, name, caller),
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
            let arguments = if params.is_empty() {
                ir::Ty::Unit
            } else {
                ir::Ty::Tuple(params.clone().into())
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
                original_signature: None,
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
        let owner = self.registry.closure_source_owner(source)?;
        Ok(Some((owner, syntax.site)))
    }
}

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn check_stored_closure(
        &mut self,
        path: &AstPath,
        expected: Option<&ir::Signature>,
    ) -> Result<Option<ir::ExprId>, String> {
        let Some(closure) = self.prepare_stored_closure(path, expected)? else {
            return Ok(None);
        };
        self.construct_stored_closure(closure).map(Some)
    }
}
