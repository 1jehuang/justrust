//! Immutable authority for the executable canonical thread panic-state recipe.
use super::{DeclId, DeclKind, FactsCtx, PlanBudget, ReceiverMode, SignatureSubject, Ty};
use crate::compiler::module_identity::CatalogModule;
use crate::compiler::traits::{
    CallableProvider, CatalogItem, DeclOrigin, RecipeDescriptor, ValidatedFacts,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeQueryKind {
    ThreadPanicking,
}

/// This value describes a state read, not a pure or constant expression.
/// Private fields prevent raw IR from minting canonical recipe authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeQuery {
    owner: DeclId,
    kind: RuntimeQueryKind,
}

impl RuntimeQuery {
    pub(in crate::compiler) fn issue(facts: &ValidatedFacts) -> Result<Self, String> {
        let query = Self {
            owner: facts.declaration(),
            kind: RuntimeQueryKind::ThreadPanicking,
        };
        query.validate_owner(facts, facts.budget())?;
        Ok(query)
    }

    pub fn kind(&self) -> RuntimeQueryKind {
        self.kind
    }

    pub(in crate::compiler) fn validate(
        &self,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        let facts = facts
            .validated()
            .ok_or("runtime query requires validated canonical recipe facts")?;
        self.validate_owner(facts, budget)
    }

    fn validate_owner(&self, facts: &ValidatedFacts, budget: &PlanBudget) -> Result<(), String> {
        budget.charge_work(1)?;
        let catalog = facts.world().canonical_catalog();
        let item = CatalogItem::ThreadPanicking;
        let declaration = catalog.id(item);
        let entry = catalog
            .declaration(declaration)
            .ok_or("runtime query lost canonical catalog declaration")?;
        let schema = entry
            .schema
            .as_ref()
            .ok_or("runtime query lost callable schema")?;
        let contract = &schema.signature.ref_contract;
        if facts.phase() == crate::compiler::traits::FactsPhase::Schema
            || !budget.same_authority(facts.budget())
            || self.owner != declaration
            || facts.declaration() != declaration
            || facts.registry_snapshot() != catalog.snapshot()
            || entry.declaration != declaration
            || entry.origin
                != (DeclOrigin::CoreCatalog {
                    version: catalog.version(),
                    item,
                })
            || entry.callable_provider
                != Some(CallableProvider::Recipe {
                    descriptor: RecipeDescriptor::ThreadPanicking,
                })
            || entry.info.declaration != declaration
            || entry.info.kind != DeclKind::Function
            || entry.info.owner.is_some()
            || entry.info.receiver != ReceiverMode::None
            || entry.info.module != catalog.modules().catalog_module(CatalogModule::Thread)
            || entry.info.own_env.binder().declaration != declaration
            || entry.info.own_env.arity() != 0
            || entry.info.env.total_arity() != 0
            || schema.env.total_arity() != 0
            || facts.env().total_arity() != 0
            || !schema.predicates.is_empty()
            || !schema.signature.params.is_empty()
            || schema.signature.result != Ty::Bool
            || contract.region_count != 0
            || !contract.input_regions.is_empty()
            || !contract.results.is_empty()
            || !contract.outlives.is_empty()
            || contract.static_regions != 0
            || facts
                .target()
                .is_some_and(|target| !target.type_args.is_empty())
        {
            return Err(
                "runtime query requires its exact canonical thread::panicking recipe".into(),
            );
        }
        facts
            .validate_signature(SignatureSubject::Body, &schema.signature)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ir::{CallTarget, RefContract, Signature, SnapshotId};
    use crate::compiler::traits::test_support;
    use std::sync::Arc;

    fn modules() -> Arc<crate::compiler::module_identity::ModuleArena> {
        let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
        crate::compiler::module_identity::ModuleArenaBuilder::for_catalog(
            budget.clone(),
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap()
    }

    #[test]
    fn runtime_query_requires_exact_canonical_recipe_and_rejects_transplants() {
        let budget = PlanBudget::default();
        let world = test_support::world(modules(), &[], &budget).unwrap();
        let catalog = world.canonical_catalog();
        let declaration = catalog.id(CatalogItem::ThreadPanicking);
        let facts = world.callable_facts(declaration).unwrap();
        let query = RuntimeQuery::issue(&facts).unwrap();
        assert_eq!(query.kind(), RuntimeQueryKind::ThreadPanicking);
        assert_eq!(query, query.clone()); // Lawful structural proof equality, not CSE eligibility.
        query
            .validate(FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let target = CallTarget {
            declaration,
            type_args: Arc::from([]),
        };
        let closed = world.instance_facts(&target).unwrap();
        query
            .validate(FactsCtx::Validated(&closed), &budget)
            .unwrap();
        assert!(query.validate(FactsCtx::Concrete, &budget).is_err());
        assert!(
            query
                .validate(FactsCtx::Universal(facts.env()), &budget)
                .is_err()
        );
        assert!(
            query
                .validate(FactsCtx::Validated(&facts), &PlanBudget::default())
                .is_err()
        );

        let mut forged = query.clone();
        forged.owner = catalog.id(CatalogItem::CellGet);
        assert!(
            forged
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
        forged.owner = DeclId {
            snapshot: SnapshotId::standalone().unwrap(),
            ..declaration
        };
        assert!(
            forged
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
        let signature = Signature {
            params: Vec::new(),
            result: Ty::Bool,
            ref_contract: Arc::new(RefContract::default()),
        };
        let user = test_support::facts_for_signature(modules(), &signature, &[], &budget).unwrap();
        assert!(RuntimeQuery::issue(&user).is_err());
        assert!(query.validate(FactsCtx::Validated(&user), &budget).is_err());
        forged.owner = user.declaration();
        assert!(
            forged
                .validate(FactsCtx::Validated(&user), &budget)
                .is_err()
        );

        let other_world = test_support::world(modules(), &[], &budget).unwrap();
        let other_facts = other_world
            .callable_facts(
                other_world
                    .canonical_catalog()
                    .id(CatalogItem::ThreadPanicking),
            )
            .unwrap();
        assert!(
            query
                .validate(FactsCtx::Validated(&other_facts), &budget)
                .is_err()
        );
    }

    #[test]
    fn forged_runtime_query_ir_and_cfg_require_exact_recipe_facts_and_bool_shape() {
        use crate::compiler::{cfg, ir};
        let budget = PlanBudget::default();
        let world = test_support::world(modules(), &[], &budget).unwrap();
        let target = CallTarget {
            declaration: world.canonical_catalog().id(CatalogItem::ThreadPanicking),
            type_args: Arc::from([]),
        };
        let facts = world.instance_facts(&target).unwrap();
        let query = RuntimeQuery::issue(&facts).unwrap();
        let mut body = ir::CheckedFunction {
            runtime_sites: Default::default(),
            call_evidence: Arc::from([]),
            body_substitution: None,
            ref_contract: Arc::new(RefContract::default()),
            name: "panicking".into(),
            params: Vec::new(),
            result: Ty::Bool,
            locals: Vec::new(),
            exprs: vec![ir::TypedExpr {
                ty: Ty::Bool,
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::RuntimeQuery(query),
            }],
            body: ir::TypedBlock {
                statements: Vec::new(),
                tail: Some(ir::ExprId(0)),
            },
        };
        body.validate_with_facts(FactsCtx::Validated(&facts))
            .unwrap();
        let lowered = cfg::lower_with_facts(&body, &facts, &budget).unwrap();
        lowered
            .validate_with_facts(FactsCtx::Validated(&facts))
            .unwrap();
        assert!(body.validate_with_facts(FactsCtx::Concrete).is_err());
        assert!(lowered.validate_with_facts(FactsCtx::Concrete).is_err());
        let signature = Signature {
            params: Vec::new(),
            result: Ty::Bool,
            ref_contract: Arc::new(RefContract::default()),
        };
        let user = test_support::facts_for_signature(modules(), &signature, &[], &budget).unwrap();
        assert!(
            body.validate_with_facts(FactsCtx::Validated(&user))
                .is_err()
        );
        assert!(
            lowered
                .validate_with_facts(FactsCtx::Validated(&user))
                .is_err()
        );
        body.exprs[0].ty = Ty::Unit;
        assert!(
            body.validate_with_facts(FactsCtx::Validated(&facts))
                .is_err()
        );
        body.exprs[0].ty = Ty::Bool;
        body.exprs[0].completion = ir::Completion::Never;
        assert!(
            body.validate_with_facts(FactsCtx::Validated(&facts))
                .is_err()
        );
    }

    #[test]
    fn runtime_query_validation_is_metered() {
        let budget = PlanBudget::default();
        let world = test_support::world(modules(), &[], &budget).unwrap();
        let facts = world
            .callable_facts(world.canonical_catalog().id(CatalogItem::ThreadPanicking))
            .unwrap();
        let query = RuntimeQuery::issue(&facts).unwrap();
        let used = budget.usage_for_tests(|usage| usage.work);
        budget.charge_work(budget.limits().work - used).unwrap();
        assert!(
            query
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
    }
}
