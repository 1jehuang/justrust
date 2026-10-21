//! Closed-root cleanup demand, derived from canonical source rather than CFG descriptors.
use super::{InstanceBodyFact, cfg, ir, source_replay, traits};

/// A per-root graph output. No mutable compiler authority is retained here.
pub(super) struct RootCleanupDemand {
    owner: ir::CallTarget,
    // Target clones share immutable, already admitted type-argument storage from
    // canonical facts. The reservation below pays for the outer target slots,
    // not a second copy of those Arc-backed arguments.
    targets: Vec<ir::CallTarget>,
    _storage: ir::RetainedStorage,
}

impl RootCleanupDemand {
    pub(super) fn derive(
        owner: &ir::CallTarget,
        body: &InstanceBodyFact,
        publication: &source_replay::CanonicalSourcePublication,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        if !owner.type_args.is_empty()
            || body.facts.phase() != traits::FactsPhase::Closed
            || body.facts.target() != Some(owner)
        {
            return Err("closed-root cleanup demand changed its owner".into());
        }
        publication
            .validate_source(&body.checked, &body.facts, budget)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        let candidates = || {
            body.checked
                .locals
                .iter()
                .map(|local| &local.ty)
                .chain(body.checked.exprs.iter().map(|expr| &expr.ty))
                .chain(std::iter::once(&body.checked.result))
                .filter(|ty| {
                    !matches!(
                        ty,
                        ir::Ty::Int(_)
                            | ir::Ty::Float(_)
                            | ir::Ty::Bool
                            | ir::Ty::Char
                            | ir::Ty::Unit
                            | ir::Ty::Never
                            | ir::Ty::Ref { .. }
                            | ir::Ty::RawPtr { .. }
                    )
                })
        };
        let source_slots = body
            .checked
            .locals
            .len()
            .checked_add(body.checked.exprs.len())
            .and_then(|n| n.checked_add(1))
            .ok_or("cleanup source walk overflow")?;
        budget.charge_work(source_slots)?;
        let count = candidates().count();
        let bytes = Self::storage_bytes(count)?;
        let reservation = budget.reserve_bytes(bytes)?;
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(count)
            .map_err(|_| "cleanup demand allocation failed")?;
        if targets.capacity() > count {
            return Err("cleanup demand allocation exceeded admitted capacity".into());
        }
        if body.facts.has_normal_drop_impls() {
            budget.charge_work(source_slots)?;
            for ty in candidates() {
                match body
                    .facts
                    .normal_drop_plan(ty)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                {
                    traits::NormalDropPlan::Destructor(destructor) => {
                        for target in destructor.targets() {
                            budget.charge_work(targets.len())?;
                            if !targets.contains(&target) {
                                targets
                                    .try_reserve(1)
                                    .map_err(|_| "cleanup demand allocation failed")?;
                                targets.push(target);
                            }
                        }
                    }
                    traits::NormalDropPlan::Trivial => {}
                    traits::NormalDropPlan::Unresolved => {
                        return Err("unsupported: unresolved closed Drop demand".into());
                    }
                }
            }
        }
        Ok(Self {
            owner: owner.clone(),
            targets,
            _storage: reservation.retain()?,
        })
    }

    fn storage_bytes(count: usize) -> Result<usize, String> {
        count
            .checked_mul(std::mem::size_of::<ir::CallTarget>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| "cleanup demand storage overflow".into())
    }

    pub(super) fn validate(
        &self,
        owner: &ir::CallTarget,
        function: &cfg::Function,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        if owner != &self.owner || !owner.type_args.is_empty() {
            return Err("cleanup demand belongs to a different closed root".into());
        }
        for block in &function.blocks {
            budget.charge_work(1)?;
            if let cfg::Terminator::Drop { cleanup, .. } = &block.terminator.value {
                for target in cleanup.destructor().targets() {
                    budget.charge_work(self.targets.len())?;
                    if !self.targets.contains(&target) {
                        return Err("cleanup target lacks owner-local canonical demand".into());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend;
    use std::sync::Arc;

    fn fixture_for(
        source: &str,
    ) -> (
        ir::CallTarget,
        InstanceBodyFact,
        Arc<source_replay::CanonicalSourcePublication>,
        ir::PlanBudget,
        cfg::Function,
    ) {
        let program = frontend::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = context.function_decl(0).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            context.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected Source publication")
        };
        let owner = publication.facts().target().unwrap().clone();
        let body = InstanceBodyFact {
            facts: publication.facts().clone(),
            checked: publication.source().clone(),
        };
        let function = cfg::lower_source_closed(&publication, &budget).unwrap();
        (owner, body, publication, budget, function)
    }

    fn fixture() -> (
        ir::CallTarget,
        InstanceBodyFact,
        Arc<source_replay::CanonicalSourcePublication>,
        ir::PlanBudget,
        cfg::Function,
    ) {
        fixture_for(
            "fn f(){let d=D{v:1};}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){}",
        )
    }

    #[test]
    fn storage_size_rejects_overflow_before_allocation() {
        assert_eq!(
            RootCleanupDemand::storage_bytes(0).unwrap(),
            std::mem::size_of::<RootCleanupDemand>()
        );
        assert!(RootCleanupDemand::storage_bytes(usize::MAX).is_err());
    }

    #[test]
    fn empty_and_duplicate_candidates_keep_exact_outer_storage() {
        for (source, expected) in [
            ("fn f(){}fn main(){}", 0),
            (
                "fn f(){let a=D{v:1};let b=D{v:2};}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){}",
                1,
            ),
        ] {
            let (owner, body, publication, budget, function) = fixture_for(source);
            let demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
            assert_eq!(demand.targets.len(), expected);
            assert_eq!(
                demand._storage.bytes(),
                RootCleanupDemand::storage_bytes(demand.targets.capacity()).unwrap()
            );
            demand.validate(&owner, &function, &budget).unwrap();
        }
    }

    #[test]
    fn closed_root_preserves_distinct_generic_destructor_arguments() {
        let (owner, body, publication, budget, function) = fixture_for(
            "fn f(){let mut x=1i32;let mut y=2u8;let a=D::<i32>{v:&mut x as *mut i32};let b=D::<u8>{v:&mut y as *mut u8};}struct D<T>{v:*mut T}impl<T> Drop for D<T>{fn drop(&mut self){}}fn main(){}",
        );
        let demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
        assert_eq!(demand.targets.len(), 2);
        assert_eq!(demand.targets[0].declaration, demand.targets[1].declaration);
        assert_ne!(demand.targets[0].type_args, demand.targets[1].type_args);
        assert!(
            demand
                .targets
                .iter()
                .all(|target| target.type_args.len() == 1)
        );
        demand.validate(&owner, &function, &budget).unwrap();
    }

    #[test]
    fn exhausted_work_and_bytes_cannot_publish_demand() {
        let (owner, body, publication, budget, _) = fixture();
        let remaining = budget.limits().work - budget.usage_snapshot().work;
        budget.charge_work(remaining).unwrap();
        assert!(RootCleanupDemand::derive(&owner, &body, &publication, &budget).is_err());
        assert_eq!(budget.usage_snapshot().work, budget.limits().work);

        let (owner, body, publication, budget, _) = fixture();
        let remaining = budget.limits().bytes - budget.usage_snapshot().bytes;
        budget.retain_bytes(remaining).unwrap();
        assert!(RootCleanupDemand::derive(&owner, &body, &publication, &budget).is_err());
        assert_eq!(budget.usage_snapshot().bytes, budget.limits().bytes);
    }

    #[test]
    fn root_cleanup_demand_is_canonical_and_rejects_missing_target() {
        let (owner, body, publication, budget, function) = fixture();
        let mut demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
        assert!(!demand.targets.is_empty());
        demand.validate(&owner, &function, &budget).unwrap();
        demand.targets.clear();
        assert!(
            demand
                .validate(&owner, &function, &budget)
                .unwrap_err()
                .contains("owner-local canonical demand")
        );
    }

    #[test]
    fn root_cleanup_demand_rejects_foreign_publication_and_owner() {
        let (owner, body, publication, budget, function) = fixture();
        let (foreign_owner, _, foreign_publication, _, _) = fixture();
        assert!(RootCleanupDemand::derive(&owner, &body, &foreign_publication, &budget).is_err());
        assert!(RootCleanupDemand::derive(&foreign_owner, &body, &publication, &budget).is_err());
        let demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
        assert!(demand.validate(&foreign_owner, &function, &budget).is_err());
    }

    #[test]
    fn root_cleanup_demand_rejects_swapped_targets_within_one_snapshot() {
        let program = frontend::parse("fn first(){let a=A{v:1};}fn second(){let b=B{v:2};}struct A{v:i32}struct B{v:i32}impl Drop for A{fn drop(&mut self){}}impl Drop for B{fn drop(&mut self){}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let publish = |index| {
            let declaration = context.function_decl(index).unwrap();
            let frontend::PublishedCallableBody::Source(publication) =
                context.publish_callable_body(declaration, &budget).unwrap()
            else {
                panic!("expected Source publication")
            };
            let owner = publication.facts().target().unwrap().clone();
            let body = InstanceBodyFact {
                facts: publication.facts().clone(),
                checked: publication.source().clone(),
            };
            let demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
            let function = cfg::lower_source_closed(&publication, &budget).unwrap();
            (owner, demand, function)
        };
        let (first, first_demand, _) = publish(0);
        let (second, mut second_demand, function) = publish(1);
        assert_eq!(first.declaration.snapshot, second.declaration.snapshot);
        assert!(first_demand.validate(&second, &function, &budget).is_err());
        second_demand.targets = first_demand.targets;
        assert!(second_demand.validate(&second, &function, &budget).is_err());
    }

    #[test]
    fn root_cleanup_demand_rejects_type_arguments_and_changed_source() {
        let (owner, mut body, publication, budget, function) = fixture();
        let mut changed_owner = owner.clone();
        changed_owner.type_args = Arc::from([ir::Ty::Int(ir::IntTy::I32)]);
        assert!(RootCleanupDemand::derive(&changed_owner, &body, &publication, &budget).is_err());
        let demand = RootCleanupDemand::derive(&owner, &body, &publication, &budget).unwrap();
        assert!(demand.validate(&changed_owner, &function, &budget).is_err());
        body.checked = Arc::new((*body.checked).clone());
        assert!(RootCleanupDemand::derive(&owner, &body, &publication, &budget).is_err());
    }
}

impl crate::engine::Fact for RootCleanupDemand {}
