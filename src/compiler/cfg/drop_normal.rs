//! Semantic normal-path cleanup, integrated before canonical CFG publication.
use super::*;
use crate::compiler::traits::{NormalDropPlan, ValidatedDestructor, ValidatedFacts};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCleanupCall {
    owner: Place,
    destructor: Arc<ValidatedDestructor>,
}
impl ValidatedCleanupCall {
    pub fn owner(&self) -> &Place {
        &self.owner
    }
    pub fn destructor(&self) -> &Arc<ValidatedDestructor> {
        &self.destructor
    }
    pub(super) fn validate(
        &self,
        function: &Function,
        facts: &ValidatedFacts,
    ) -> Result<(), String> {
        if !self.owner.projection.is_empty() || self.owner.local == function.return_local {
            return Err("cleanup requires an unprotected whole owning root".into());
        }
        self.destructor
            .validate(function.place_ty(&self.owner)?, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))
    }
}

pub(super) fn descriptor(
    ty: &Ty,
    facts: ir::FactsCtx<'_>,
) -> Result<Option<Arc<ValidatedDestructor>>, String> {
    let Some(facts) = facts.validated() else {
        return Ok(None);
    };
    if !facts.has_normal_drop_impls() {
        return Ok(None);
    }
    // The checked source arena already validates these local/expression types.
    // Their outer form cannot own Drop, even if a pointee is an owning type.
    if matches!(
        ty,
        Ty::Int(_)
            | Ty::Float(_)
            | Ty::Bool
            | Ty::Char
            | Ty::Unit
            | Ty::Never
            | Ty::Ref { .. }
            | Ty::RawPtr { .. }
    ) {
        return Ok(None);
    }
    match facts
        .normal_drop_plan(ty)
        .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
    {
        NormalDropPlan::Trivial => Ok(None),
        // A universal template may defer cleanup only because every closed
        // instantiation is admitted again and Drop-carrying generics rejected.
        NormalDropPlan::Unresolved
            if facts.phase() == crate::compiler::traits::FactsPhase::Universal =>
        {
            Ok(None)
        }
        NormalDropPlan::Unresolved => Err("unsupported: unresolved normal Drop obligation".into()),
        NormalDropPlan::Destructor(d) => Ok(Some(d)),
    }
}

/// This whole-arena check includes dead syntax. References do not own their
/// referents: ordinary loan analysis checks their uses against the canonical
/// cleanup's exclusive receiver access. No separate borrowed-owner exemption
/// is needed for either source helpers or destructor bodies.
pub(super) fn admit(source: &ir::CheckedFunction, facts: ir::FactsCtx<'_>) -> Result<bool, String> {
    let Some(authority) = facts.validated() else {
        return Ok(false);
    };
    // Explicit destructor calls are invalid even without concrete impls.
    for expression in &source.exprs {
        authority.budget().charge_work(1)?;
        if let ir::ExprKind::Call { target, .. } = &expression.kind
            && authority
                .is_canonical_drop_call(target)
                .map_err(|e| e.into_legacy_diagnostic(authority.budget()))?
        {
            return Err("explicit canonical destructor calls are not allowed".into());
        }
    }
    if !authority.has_normal_drop_impls() {
        return Ok(false);
    }
    let mut enabled = false;
    for ty in source
        .locals
        .iter()
        .map(|l| &l.ty)
        .chain(source.exprs.iter().map(|e| &e.ty))
        .chain(std::iter::once(&source.result))
    {
        authority.budget().charge_work(1)?;
        enabled |= descriptor(ty, facts)?.is_some();
    }
    if enabled && (authority.env().is_generic() || source.body_substitution.is_some()) {
        return Err("unsupported: generic function carrying a normal Drop owner".into());
    }
    for expression in &source.exprs {
        authority.budget().charge_work(1)?;
        if let ir::ExprKind::Match { scrutinee, .. } = &expression.kind
            && descriptor(&source.exprs[scrutinee.0].ty, facts)?.is_some()
        {
            return Err("unsupported: pattern matching a normal Drop owner".into());
        }
    }
    Ok(enabled)
}

impl Lowerer<'_> {
    /// End ownership, then end physical storage. A replacement uses drop_value
    /// alone, so the still-live destination can receive the already-owned RHS.
    pub(super) fn drop_value(&mut self, local: LocalId, origin: Option<ExprId>) {
        if self.resource_error.is_some() {
            return;
        }
        let result = (|| {
            let Some(destructor) = descriptor(&self.result.locals[local.0].ty, self.facts)? else {
                return Ok(());
            };
            if local == self.result.return_local {
                return Ok(());
            }
            if !self.allocate(
                1,
                std::mem::size_of::<ValidatedCleanupCall>() + 2 * std::mem::size_of::<usize>(),
            ) {
                return Ok(());
            }
            let next = self.block();
            let cleanup = Arc::new(ValidatedCleanupCall {
                owner: place(local),
                destructor,
            });
            self.terminate(Terminator::Drop { cleanup, next }, origin)?;
            self.current = next;
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.resource_error = Some(error);
        }
    }
    pub(super) fn end_storage(&mut self, local: LocalId, origin: Option<ExprId>) {
        self.drop_value(local, origin);
        self.statement(Statement::StorageDead(local), origin);
    }
    pub(super) fn live_binding(&mut self, local: LocalId, origin: Option<ExprId>) {
        self.statement(Statement::StorageLive(local), origin);
        let index = self.scopes.len() - if self.drop_enabled { 2 } else { 1 };
        self.scopes[index].push(local);
    }
}

#[cfg(test)]
mod tests;
