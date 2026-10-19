//! Parent-local closure inference, capture checking and final value construction.
use super::super::type_check::{Checker, Expectation, LocalKey, checked_ty};
use super::check::{CheckedClosure, CheckedClosureBody};
use super::*;
use std::collections::{BTreeMap, HashMap};

impl Checker<'_> {
    /// Resolve syntax and check its body exactly once. No parent expression is
    /// emitted here, so generic argument inference can request this authority
    /// before evaluating the closure's actual environment initializer.
    pub(in crate::compiler::frontend) fn prepare_stored_closure(
        &mut self,
        path: &AstPath,
        expected: Option<&ir::Signature>,
    ) -> Result<Option<Arc<ir::ClosureTy>>, String> {
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
        if let Some(previous) = self
            .checked_closures
            .iter()
            .find(|checked| checked.ty.provider() == provider)
        {
            return Ok(Some(previous.ty.clone()));
        }
        let owner_env = plan.callable_env(owner)?;
        if owner_env.is_generic() {
            return Err("unsupported: stored closure in generic parent body".into());
        }
        let source_key = name_scope::path_key(plan.program.names.as_ref(), path, Namespace::Type)?;
        let by_move = plan
            .program
            .closure_sources
            .get(&source_key)
            .ok_or("closure syntax source unavailable")?
            .by_move;
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
        child.inferred_closure_result = matches!(syntax.result, Type::Infer);
        child.install_lexical_scope(syntax.lexical_scope)?;
        child.capture_parent_scope = self.lexical_scope;
        let receiver = child.temporary("closure self", ir::Ty::Unit, false)?;
        let arguments_ty = if parameter_types.is_empty() {
            ir::Ty::Unit
        } else {
            ir::Ty::Tuple(parameter_types.clone().into())
        };
        let arguments = child.temporary("closure arguments", arguments_ty.clone(), false)?;
        // Exact checked keys only. Shadowing is resolved in the parent, and
        // aliases to one local share one environment slot in canonical order.
        let mut visible = HashMap::<LocalKey, ir::LocalId>::new();
        for scope in &self.scopes {
            visible.extend(scope.iter().map(|(key, local)| (*key, *local)));
        }
        let mut grouped = BTreeMap::<ir::LocalId, Vec<LocalKey>>::new();
        for (key, local) in visible {
            grouped.entry(local).or_default().push(key);
        }
        let mut candidates = Vec::new();
        let mut candidate_indices = HashMap::new();
        for (parent, keys) in grouped {
            let original = self
                .locals
                .get(parent.0)
                .ok_or("capture binding has invalid parent local")?;
            let local = child.temporary(&original.name, original.ty.clone(), original.mutable)?;
            for key in keys {
                child.capture_bindings.insert(key);
                child.scopes[0].insert(key, local);
            }
            candidate_indices.insert(local, candidates.len());
            candidates.push((parent, local));
        }
        child.scopes.push(HashMap::new());
        let mut prefix = Vec::new();
        for (index, ((name, _), ty)) in syntax.params.iter().zip(parameter_types.iter()).enumerate()
        {
            let local = child.bind(name, ty.clone())?;
            child.locals[local.0].mutable = syntax.param_mut.get(index).copied().unwrap_or(false);
            let base = child.for_expr(arguments_ty.clone(), ir::ExprKind::Local(arguments))?;
            let value = child.for_expr(ty.clone(), ir::ExprKind::Field { base, index })?;
            prefix.push(ir::Stmt::Let { local, value });
        }
        let expectation = match result_hint.as_ref() {
            Some(ty) if matches!(syntax.result, Type::Infer) => Expectation::Inferred(ty),
            Some(ty) => Expectation::HasType(ty),
            None => Expectation::None,
        };
        let (mut body, flow) = child.block_with_expectation(&syntax.body, expectation)?;
        let result = result_hint.unwrap_or(flow.ty.clone());
        if !ir::carrier_locations(&result, &plan.budget)?.is_empty() {
            return Err(
                "unsupported: closure reference results require checked capture carrier contracts"
                    .into(),
            );
        }
        child.require(&flow, &result)?;
        prefix.append(&mut body.statements);
        body.statements = prefix;
        let uses = capture_use::analyze(
            &child.exprs,
            &body,
            &child.locals,
            &candidate_indices,
            child.facts(),
        )?;
        let mut sources = Vec::new();
        let mut capture_types = Vec::new();
        let mut projections = Vec::new();
        let mut kind = ir::ClosureKind::Fn;
        for ((parent, local), captures) in candidates.into_iter().zip(uses) {
            for capture in captures {
                let usage = capture.usage;
                kind = match (kind, usage.kind()) {
                    (ir::ClosureKind::FnOnce, _) | (_, ir::ClosureKind::FnOnce) => {
                        ir::ClosureKind::FnOnce
                    }
                    (ir::ClosureKind::FnMut, _) | (_, ir::ClosureKind::FnMut) => {
                        ir::ClosureKind::FnMut
                    }
                    _ => ir::ClosureKind::Fn,
                };
                let mode = if by_move || usage.moved {
                    ir::CaptureKind::Value
                } else if usage.mutable {
                    ir::CaptureKind::Mut
                } else {
                    ir::CaptureKind::Shared
                };
                let original = child
                    .facts()
                    .projected_ty(&child.locals[local.0].ty, &capture.place.projection)?
                    .clone();
                let ty = match mode {
                    ir::CaptureKind::Value => original,
                    ir::CaptureKind::Shared => ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(original),
                    },
                    ir::CaptureKind::Mut => ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(original),
                    },
                };
                projections.push(capture_rewrite::CaptureProjection {
                    local,
                    field: capture_types.len(),
                    prefix: capture.place.projection.clone(),
                    indirect: mode != ir::CaptureKind::Value,
                });
                capture_types.push(ty);
                sources.push(ir::CaptureSource {
                    place: ir::Place {
                        local: parent,
                        projection: capture.place.projection,
                    },
                    kind: mode,
                });
            }
        }
        let closure = Arc::new(
            ir::ClosureTy::new(
                owner,
                Arc::from([]),
                syntax.site,
                provider,
                parameter_types.into(),
                result.clone(),
                capture_types.into(),
                kind,
            )
            .with_sources(sources.into()),
        );
        let mut modes = Vec::new();
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            if !closure.supports(mode) {
                continue;
            }
            let facts = plan
                .trait_world()?
                .closure_callable_facts_for(closure.clone(), mode)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            let signature = facts
                .world()
                .closure_body_signature(&facts)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            let receiver_ty = signature
                .params
                .first()
                .ok_or("closure body lacks self parameter")?
                .clone();
            let mut locals = child.locals.clone();
            locals[receiver.0].ty = receiver_ty.clone();
            let mut exprs = child.exprs.clone();
            let mut body = body.clone();
            capture_rewrite::rewrite(
                &mut exprs,
                &mut body,
                receiver,
                receiver_ty,
                closure.clone(),
                &projections,
                &plan.budget,
            )?;
            let checked = ir::CheckedFunction {
                runtime_sites: ir::RuntimeSites::from_source(child.runtime_sites.clone())?,
                name: format!("closure {:?}:{}:{mode:?}", owner, syntax.site),
                ref_contract: signature.ref_contract,
                params: vec![receiver, arguments],
                result: result.clone(),
                locals,
                exprs,
                call_evidence: child.call_evidence.clone().into(),
                body_substitution: None,
                body,
            };
            modes.push(CheckedClosureBody {
                mode,
                body: Arc::new(checked),
            });
        }
        let primary = modes
            .iter()
            .find(|checked| checked.mode == kind)
            .ok_or("closure primary body unavailable")?
            .body
            .clone();
        self.checked_closures.extend(child.checked_closures);
        self.checked_closures.push(CheckedClosure {
            ty: closure.clone(),
            body: primary,
            modes: modes.into(),
        });
        Ok(Some(closure))
    }

    pub(super) fn construct_stored_closure(
        &mut self,
        closure: Arc<ir::ClosureTy>,
    ) -> Result<ir::ExprId, String> {
        if self.declaration != Some(closure.owner()) {
            return Err("closure value belongs to another parent body".into());
        }
        let original = self
            .checked_closures
            .iter()
            .find(|checked| checked.ty.provider() == closure.provider())
            .ok_or("closure construction has no checked parent-body authorization")?;
        let plan = self.signatures.plan()?;
        if !plan
            .trait_world()?
            .types_equal(
                &[ir::Ty::Closure(original.ty.clone())],
                &[ir::Ty::Closure(closure.clone())],
            )
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
        {
            return Err("closure value changed its checked capture metadata".into());
        }
        let mut values = Vec::new();
        for (source, ty) in closure.sources().iter().zip(closure.captures()) {
            let value = match source.kind {
                ir::CaptureKind::Value => self.place_read_expr(&source.place, ty.clone())?,
                ir::CaptureKind::Shared => {
                    self.borrow_expression(ir::BorrowKind::Shared, source.place.clone())?
                }
                ir::CaptureKind::Mut => {
                    self.borrow_expression(ir::BorrowKind::Mut, source.place.clone())?
                }
            };
            values.push(value);
        }
        self.for_expr(ir::Ty::Closure(closure), ir::ExprKind::Tuple(values))
    }
}
