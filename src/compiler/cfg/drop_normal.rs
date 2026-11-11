//! Semantic normal-path cleanup, integrated before canonical CFG publication.
use super::*;
mod physical_remap;
use crate::compiler::traits::{NormalDropPlan, ValidatedDestructor, ValidatedFacts};
use std::sync::Arc;

mod abstract_scope;
mod physical;
pub(in crate::compiler) use physical::materialize as materialize_physical_cleanup;

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
        scratch: &mut TraitScratch,
    ) -> Result<(), String> {
        if self.owner.local == function.return_local {
            return Err("cleanup requires an unprotected owning root".into());
        }
        if self.owner.projection.iter().any(|p| {
            matches!(
                p,
                ir::Projection::DynamicIndex(_) | ir::Projection::ConstantIndex(_)
            )
        }) {
            return Err("unsupported: indexed normal Drop replacement".into());
        }
        let owner = ir::projected_ty(
            &function
                .locals
                .get(self.owner.local.0)
                .ok_or("invalid cleanup owner local")?
                .ty,
            &self.owner.projection,
            facts.budget(),
        )?;
        self.destructor
            .validate_with_scratch(&owner, facts, scratch)
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
            | Ty::PhantomData(_)
            | Ty::ManuallyDrop(_)
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
        // Universal unresolved types defer to the demanded closed instance,
        // which is freshly lowered and sealed with concrete cleanup evidence.
        NormalDropPlan::Unresolved
            if facts.phase() == crate::compiler::traits::FactsPhase::Universal =>
        {
            Ok(None)
        }
        NormalDropPlan::Unresolved => Err("unsupported: unresolved normal Drop obligation".into()),
        NormalDropPlan::Destructor(d) => Ok(Some(d)),
    }
}

/// Authenticate each owned pattern move before relying on per-field cleanup.
/// Whole owners move intact. Structural fields use the existing path flags,
/// but no non-Copy move may cross an owner with its own destructor.
fn admit_pattern_moves(
    root: &Ty,
    pattern: &ir::MatchPattern,
    facts: ir::FactsCtx<'_>,
    depth: usize,
) -> Result<(), String> {
    let authority = facts
        .validated()
        .ok_or("Drop pattern requires validated caller facts")?;
    authority.budget().check_depth(depth)?;
    authority.budget().charge_work(1)?;
    let bindings = match pattern {
        ir::MatchPattern::Or { rows, .. } => {
            for row in rows {
                admit_pattern_moves(root, row, facts, depth + 1)?;
            }
            return Ok(());
        }
        ir::MatchPattern::Aggregate { bindings, .. }
        | ir::MatchPattern::EnumVariant { bindings, .. }
        | ir::MatchPattern::Decision { bindings, .. } => bindings,
        _ => return Ok(()),
    };
    for binding in bindings {
        authority.budget().charge_work(1)?;
        let selected = facts.projected_ty(root, &binding.projection, authority.budget())?;
        if binding.borrow.is_some() || facts.must_copy(&selected)? {
            continue;
        }
        for (index, step) in binding.projection.iter().enumerate() {
            authority.budget().charge_work(1)?;
            // Downcast and its field are one canonical projection step. The
            // enum owner was checked before the downcast, and the complete
            // binding projection above already authenticated the field.
            if index > 0 && matches!(binding.projection[index - 1], ir::Projection::Downcast(_)) {
                continue;
            }
            let owner =
                facts.projected_ty(root, &binding.projection[..index], authority.budget())?;
            if descriptor(&owner, facts)?.is_some_and(|d| d.own_target().is_some()) {
                return Err(
                    "E0509: cannot move out of a type which implements the `Drop` trait".into(),
                );
            }
            match step {
                ir::Projection::ConstantIndex(_) | ir::Projection::DynamicIndex(_) => {
                    return Err("unsupported: indexed normal Drop pattern move".into());
                }
                ir::Projection::Deref
                    if !matches!(
                        owner.owned_pointer_pointee(),
                        Some((ir::OwnedPointerKind::Box, _))
                    ) =>
                {
                    // References and raw pointers do not transfer the tracked
                    // root's storage. Their reads remain ordinary loan checks.
                    break;
                }
                _ => {}
            }
        }
    }
    Ok(())
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
        if !enabled {
            // Preserve source statement-temporary events without claiming a
            // destructor for a representation not yet available to this node.
            enabled = abstract_scope::required(ty, authority)?;
        }
    }
    // Every demanded closed body is lowered again before its generated shape
    // is sealed. The instance frontier separately demands each concrete cleanup
    // target, so a universal unresolved type is never a no-drop executable proof.
    // Structural pattern moves use the same typed projections and runtime
    // path flags as ordinary partial moves. Validate every owned prefix,
    // including nested own-Drop boundaries, before publishing this CFG.
    for expression in &source.exprs {
        authority.budget().charge_work(1)?;
        if let ir::ExprKind::Match {
            scrutinee, arms, ..
        } = &expression.kind
            && descriptor(&source.exprs[scrutinee.0].ty, facts)?.is_some()
        {
            for arm in arms {
                admit_pattern_moves(&source.exprs[scrutinee.0].ty, &arm.pattern, facts, 0)?;
            }
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
    /// Replacement of a projected place drops its old value first.
    pub(super) fn drop_place(&mut self, owner: Place, origin: Option<ExprId>) {
        if self.resource_error.is_some() {
            return;
        }
        let result = (|| {
            // Stores through a raw pointer have `ptr::write` semantics in this
            // draft: the library writes into uninitialized heap slots this way.
            // (Rust would drop the old pointee; documented deviation.)
            let budget = self.budget;
            if ir::raw_deref_index_optional(
                &self.result.locals[owner.local.0].ty,
                &owner.projection,
                budget,
            )?
            .is_some()
            {
                return Ok(());
            }
            let ty = ir::projected_ty_optional(
                &self.result.locals[owner.local.0].ty,
                &owner.projection,
                budget,
            )?;
            let Some(destructor) = descriptor(&ty, self.facts)? else {
                return Ok(());
            };
            if owner.projection.iter().any(|p| {
                matches!(
                    p,
                    ir::Projection::DynamicIndex(_) | ir::Projection::ConstantIndex(_)
                )
            }) {
                return Err("unsupported: indexed normal Drop replacement".to_string());
            }
            if !self.allocate(
                1,
                std::mem::size_of::<ValidatedCleanupCall>() + 2 * std::mem::size_of::<usize>(),
            ) {
                return Ok(());
            }
            let next = self.block();
            let cleanup = Arc::new(ValidatedCleanupCall { owner, destructor });
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
