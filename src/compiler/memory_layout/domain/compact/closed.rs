//! Compact closed-owner obligations. Body-bearing ClosedCalls is consumed in
//! bind and never retained in an output or a transitive dependency receipt.
pub(in crate::compiler) mod topology;
use super::super::{ContractStep, OpaqueCall, SelectedSite};
use super::{Requirement, Summary, ir, traits};
use std::sync::Arc;

type Seed = traits::alignment_provider::ProviderDomainSeed;
#[derive(Clone)]
struct Method {
    declaration: ir::DeclId,
    item: ir::DeclId,
    prefix: Arc<[ir::Ty]>,
    closed: Requirement,
    own: Requirement,
}
impl Method {
    fn bind(seed: Seed, summary: &Summary, budget: &ir::PlanBudget) -> Result<Self, String> {
        seed.validate()
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        let required = summary.requirement(seed.declaration())?;
        let prefix = seed.prefix();
        let arity = seed.method_arity();
        if prefix.len().checked_add(arity) != Some(required.parameters.len()) {
            return Err("opaque source method changed exact binder prefix/own order".into());
        }
        let mut closed = Requirement::empty(0, budget)?;
        let mut own = Requirement::empty(arity, budget)?;
        for (slot, argument) in prefix.iter().enumerate() {
            budget.charge_work(1)?;
            if required.parameters[slot] {
                super::super::normalize(argument, &[], &mut closed, budget, 0)?;
            }
            if required.raw_pointees[slot] {
                super::super::normalize_raw_pointee(argument, &[], &mut closed, budget)?;
            }
        }
        for slot in 0..arity {
            budget.charge_work(1)?;
            own.parameters[slot] = required.parameters[prefix.len() + slot];
            own.raw_pointees[slot] = required.raw_pointees[prefix.len() + slot];
        }
        closed.unsupported = closed.unsupported.or(required.unsupported);
        own.deferred_dispatch = required.deferred_dispatch;
        budget.retain_bytes(std::mem::size_of_val(prefix))?;
        Ok(Self {
            declaration: seed.declaration(),
            item: seed.item(),
            prefix: Arc::from(prefix),
            closed,
            own,
        })
    }
    fn instantiate(
        &self,
        actuals: &[ir::Ty],
        facts: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CallTarget, String> {
        if actuals.len() != self.own.parameters.len() {
            return Err("opaque source invocation changed own method arity".into());
        }
        let mut required = self.closed.clone();
        for (slot, actual) in actuals.iter().enumerate() {
            budget.charge_work(1)?;
            if self.own.parameters[slot] {
                super::super::normalize(actual, &[], &mut required, budget, 0)?;
            }
            if self.own.raw_pointees[slot] {
                super::super::normalize_raw_pointee(actual, &[], &mut required, budget)?;
            }
        }
        required.admit()?;
        // Remaining dispatch is an edge to the exact closed source owner,
        // never interpreted as an empty method requirement or final success.
        let mut type_args = self.prefix.to_vec();
        type_args.extend_from_slice(actuals);
        budget.retain_bytes(std::mem::size_of_val(type_args.as_slice()))?;
        let target = ir::CallTarget {
            declaration: self.declaration,
            type_args: type_args.into(),
        };
        facts
            .service()
            .discharge_direct(&target, facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        Ok(target)
    }
}
#[derive(Clone)]
enum Bound {
    Concrete {
        public: ir::TraitRef,
        method: Method,
    },
    Forwarded {
        public: ir::TraitRef,
        application: ir::TraitRef,
        item: ir::DeclId,
    },
}
#[derive(Clone)]
struct Pack {
    occurrence: ir::ExprId,
    opaque: Arc<ir::OpaqueTy>,
    methods: Arc<[Bound]>,
}

/// Original occurrence-to-selected-target evidence only, not an instantiated
/// body, source publication, or recursively retained universal proof.
#[derive(Clone)]
pub(in crate::compiler) struct Header {
    target: ir::CallTarget,
    selected: Arc<[(SelectedSite, ir::CallTarget)]>,
    queries: Arc<[(ir::ExprId, ir::Ty)]>,
    virtual_sites: Arc<[ir::ExprId]>,
    opaque_calls: Arc<[OpaqueCall]>,
    packs: Arc<[Pack]>,
}
impl Header {
    pub(in crate::compiler) fn no_continuation(
        target: ir::CallTarget,
        summary: &Summary,
        facts: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        if summary.closed_requirements(&target, facts, budget)? {
            return Err("closed alignment header omitted required continuation".into());
        }
        Ok(Self {
            target,
            selected: Arc::from([]),
            queries: Arc::from([]),
            virtual_sites: Arc::from([]),
            opaque_calls: Arc::from([]),
            packs: Arc::from([]),
        })
    }
    pub(in crate::compiler) fn bind(
        source: super::super::ClosedCalls,
        summaries: Vec<Arc<Summary>>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let mut summaries = summaries.into_iter();
        let mut packs = Vec::new();
        for pack in source.packs.iter() {
            let mut methods = Vec::new();
            for provider in pack.providers.iter() {
                budget.charge_work(1)?;
                methods.push(match provider {
                    traits::alignment_provider::BoundProvider::Concrete { public, seed } => {
                        let summary = summaries
                            .next()
                            .ok_or("opaque source pack omitted provider summary")?;
                        Bound::Concrete {
                            public: public.clone(),
                            method: Method::bind(seed.clone(), &summary, budget)?,
                        }
                    }
                    traits::alignment_provider::BoundProvider::Forwarded {
                        public,
                        application,
                        item,
                    } => Bound::Forwarded {
                        public: public.clone(),
                        application: application.clone(),
                        item: *item,
                    },
                });
            }
            packs.push(Pack {
                occurrence: pack.occurrence,
                opaque: pack.opaque.clone(),
                methods: methods.into(),
            });
        }
        if summaries.next().is_some() {
            return Err("opaque source pack added unrelated provider summary".into());
        }
        let mut selected = Vec::new();
        for (site, target, proof) in source.selected.iter() {
            budget.charge_work(1)?;
            if proof.target() != target {
                return Err("closed alignment target omitted canonical call proof".into());
            }
            selected.push((*site, target.clone()));
        }
        Ok(Self {
            target: source.target,
            selected: selected.into(),
            queries: source.queries,
            virtual_sites: source.virtual_sites,
            opaque_calls: source.opaque_sites,
            packs: packs.into(),
        })
    }
    pub(in crate::compiler) fn target(&self) -> &ir::CallTarget {
        &self.target
    }
    pub(in crate::compiler) fn targets(&self) -> impl Iterator<Item = &ir::CallTarget> {
        self.selected.iter().map(|(_, target)| target)
    }
    pub(in crate::compiler) fn opaque_calls(&self) -> &[OpaqueCall] {
        &self.opaque_calls
    }
    #[cfg(test)]
    pub(in crate::compiler) fn virtual_sites(&self) -> &[ir::ExprId] {
        &self.virtual_sites
    }
    pub(in crate::compiler) fn resolve(
        &self,
        invocation: &OpaqueCall,
        facts: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<ContractStep, String> {
        let expected = crate::compiler::instances::graph_key::encode(&self.target, budget)?;
        let subject = facts
            .target()
            .ok_or("opaque resolution omitted closed source fact subject")?;
        if crate::compiler::instances::graph_key::encode(subject, budget)? != expected {
            return Err("opaque resolution changed closed source fact subject".into());
        }
        if crate::compiler::instances::graph_key::encode(&invocation.owner_target()?, budget)?
            != expected
        {
            return Err("opaque source contract changed canonical owner actuals".into());
        }
        let ir::Ty::Opaque(opaque) = &invocation.call.trait_ref.self_ty else {
            return Err("opaque invocation lost public type identity".into());
        };
        let mut selected = None;
        for pack in self.packs.iter() {
            budget.charge_work(1)?;
            if pack.opaque != *opaque {
                continue;
            }
            for bound in pack.methods.iter() {
                budget.charge_work(1)?;
                let (public, item) = match bound {
                    Bound::Concrete { public, method } => (public, method.item),
                    Bound::Forwarded { public, item, .. } => (public, *item),
                };
                if public != &invocation.call.trait_ref || item != invocation.call.item {
                    continue;
                }
                if selected.is_some() {
                    return Err("opaque source contract has ambiguous provider".into());
                }
                selected =
                    Some(match bound {
                        Bound::Concrete { method, .. } => ContractStep::Source(
                            method.instantiate(&invocation.call.method_args, facts, budget)?,
                        ),
                        Bound::Forwarded {
                            application, item, ..
                        } => {
                            if !matches!(application.self_ty, ir::Ty::Opaque(_)) {
                                return Err(
                                    "forwarded contract lost original public opaque owner".into()
                                );
                            }
                            ContractStep::Forwarded(OpaqueCall {
                                occurrence: pack.occurrence,
                                call: ir::TraitCall {
                                    trait_ref: application.clone(),
                                    item: *item,
                                    method_args: invocation.call.method_args.clone(),
                                },
                            })
                        }
                    });
            }
        }
        selected.ok_or_else(|| "opaque invocation omitted exact source-owned bound contract".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::memory_layout::domain::{
        compact::tests::closed_header, tests::fixture_plan,
    };

    #[test]
    fn alignment_domain_opaque_resolution_keeps_exact_fact_and_owner_coordinates() {
        let (_, owner, budget, mut plan) = fixture_plan(
            "struct V;trait Inspect{fn width(&self)->usize;}impl Inspect for V{fn width(&self)->usize{std::mem::align_of::<u16>()}}fn make<T,U>()->impl Inspect{V}fn main(){let v=make::<u8,u16>();let _n=v.width();}",
            "main",
        );
        let root = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let caller = closed_header(&mut plan, root.clone(), &budget);
        let invocation = caller.opaque_calls().first().unwrap().clone();
        let original = invocation.owner_target().unwrap();
        let words = crate::compiler::instances::graph_key::encode(&original, &budget).unwrap();
        let decoded =
            crate::compiler::instances::graph_key::decode(&words, &plan, &budget).unwrap();
        let producer = closed_header(&mut plan, decoded, &budget);
        let facts = plan.instance_facts(&original).unwrap();
        assert!(producer.resolve(&invocation, &facts, &budget).is_ok());
        let foreign_facts = plan.instance_facts(&root).unwrap();
        assert!(
            producer
                .resolve(&invocation, &foreign_facts, &budget)
                .is_err()
        );
        let mut reordered = original;
        Arc::make_mut(&mut reordered.type_args).swap(0, 1);
        let wrong_facts = plan.instance_facts(&reordered).unwrap();
        assert!(
            producer
                .resolve(&invocation, &wrong_facts, &budget)
                .is_err()
        );
        let mut wrong_owner = producer;
        Arc::make_mut(&mut wrong_owner).target = reordered;
        assert!(
            wrong_owner
                .resolve(&invocation, &wrong_facts, &budget)
                .is_err()
        );
    }
}
