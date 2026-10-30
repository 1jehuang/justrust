//! Compact closed-owner obligations. Body-bearing ClosedCalls is consumed in
//! bind and never retained in an output or a transitive dependency receipt.
pub(in crate::compiler) mod topology;
use super::super::{ContractStep, OpaqueCall, SelectedSite};
use super::{Requirement, Summary, ir, traits};
use std::sync::Arc;

type Seed = traits::alignment_provider::ProviderDomainSeed;
#[derive(Clone)]
struct MethodReceipt {
    declaration: ir::DeclId,
    item: ir::DeclId,
    prefix: Arc<[ir::Ty]>,
    closed: Requirement,
    own: Requirement,
}

#[derive(Clone)]
struct Method {
    declaration: ir::DeclId,
    item: ir::DeclId,
    prefix: Arc<[ir::Ty]>,
    closed: Requirement,
    own: Requirement,
    receipt: Arc<MethodReceipt>,
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
            if let Some(limit) = required.semantic_limits[slot] {
                super::super::normalize_semantic_size(argument, &[], limit, &mut closed, budget)?;
            }
            if let Some(limit) = required.physical_limits[slot] {
                super::super::normalize_physical_size(argument, &[], limit, &mut closed, budget)?;
            }
            if required.metadata_pointees[slot] {
                super::super::normalize_metadata_pointee(argument, &[], &mut closed, budget)?;
            }
            if required.thin_pointees[slot] {
                super::super::normalize_thin_pointee(argument, &[], &mut closed, budget)?;
            }
            if required.pointer_strides[slot] {
                super::super::normalize_pointer_stride(argument, &[], &mut closed, budget, 0)?;
            }
            if required.mut_ref_pointees[slot] {
                super::super::normalize_mut_ref_pointee(argument, &[], &mut closed, budget)?;
            }
        }
        for slot in 0..arity {
            budget.charge_work(1)?;
            own.parameters[slot] = required.parameters[prefix.len() + slot];
            own.raw_pointees[slot] = required.raw_pointees[prefix.len() + slot];
            own.semantic_limits[slot] = required.semantic_limits[prefix.len() + slot];
            own.physical_limits[slot] = required.physical_limits[prefix.len() + slot];
            own.thin_pointees[slot] = required.thin_pointees[prefix.len() + slot];
            own.metadata_pointees[slot] = required.metadata_pointees[prefix.len() + slot];
            own.pointer_strides[slot] = required.pointer_strides[prefix.len() + slot];
            own.mut_ref_pointees[slot] = required.mut_ref_pointees[prefix.len() + slot];
        }
        closed.unsupported = closed.unsupported.or(required.unsupported);
        own.deferred_dispatch = required.deferred_dispatch;
        budget.retain_bytes(std::mem::size_of_val(prefix) + 2 * std::mem::size_of::<usize>())?;
        budget.retain_bytes(
            std::mem::size_of::<MethodReceipt>()
                + 2 * std::mem::size_of::<usize>()
                + arity
                    .checked_mul(6 + 2 * std::mem::size_of::<Option<usize>>())
                    .ok_or("opaque method receipt arity overflow")?,
        )?;
        let prefix: Arc<[ir::Ty]> = Arc::from(prefix);
        let receipt = Arc::new(MethodReceipt {
            declaration: seed.declaration(),
            item: seed.item(),
            prefix: prefix.clone(),
            closed: closed.clone(),
            own: own.clone(),
        });
        Ok(Self {
            declaration: seed.declaration(),
            item: seed.item(),
            prefix,
            closed,
            own,
            receipt,
        })
    }
    fn validate_receipt(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(1 + self.own.parameters.len().saturating_mul(8))?;
        if self.declaration != self.receipt.declaration
            || self.item != self.receipt.item
            || self.closed != self.receipt.closed
            || self.own != self.receipt.own
            || self.prefix.len() != self.receipt.prefix.len()
        {
            return Err("opaque method changed its immutable source binder receipt".into());
        }
        for (actual, original) in self.prefix.iter().zip(self.receipt.prefix.iter()) {
            if !ir::identical_type(actual, original, budget)? {
                return Err("opaque method changed its exact source prefix substitution".into());
            }
        }
        Ok(())
    }
    fn instantiate(
        &self,
        actuals: &[ir::Ty],
        facts: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CallTarget, String> {
        self.validate_receipt(budget)?;
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
            if let Some(limit) = self.own.semantic_limits[slot] {
                super::super::normalize_semantic_size(actual, &[], limit, &mut required, budget)?;
            }
            if let Some(limit) = self.own.physical_limits[slot] {
                super::super::normalize_physical_size(actual, &[], limit, &mut required, budget)?;
            }
            if self.own.metadata_pointees[slot] {
                super::super::normalize_metadata_pointee(actual, &[], &mut required, budget)?;
            }
            if self.own.thin_pointees[slot] {
                super::super::normalize_thin_pointee(actual, &[], &mut required, budget)?;
            }
            if self.own.pointer_strides[slot] {
                super::super::normalize_pointer_stride(actual, &[], &mut required, budget, 0)?;
            }
            if self.own.mut_ref_pointees[slot] {
                super::super::normalize_mut_ref_pointee(actual, &[], &mut required, budget)?;
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
        method: Arc<Method>,
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
                            method: {
                                budget.retain_bytes(
                                    std::mem::size_of::<Method>()
                                        + 2 * std::mem::size_of::<usize>(),
                                )?;
                                Arc::new(Method::bind(seed.clone(), &summary, budget)?)
                            },
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
    fn semantic_size_method_receipt_rejects_binder_and_requirement_mutation() {
        let original = "struct W<T,V>(T,V);trait Inspect{fn width<Q>(&self)->usize;}impl<T,V> Inspect for W<T,V>{fn width<Q>(&self)->usize{let _p=std::ptr::null::<Q>().wrapping_add(1usize);let _a=std::ptr::null::<[Q;3]>().wrapping_add(1usize);std::mem::size_of::<[T;3]>()+std::mem::size_of::<[Q;2]>()}}fn make<T,V>(a:T,b:V)->impl Inspect{W(a,b)}fn main(){let _n=make(1u16,2u32).width::<u8>();}";
        let (_, owner, budget, mut plan) = fixture_plan(original, "main");
        let root = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([]),
        };
        let caller = closed_header(&mut plan, root, &budget);
        let invocation = caller.opaque_calls().first().unwrap().clone();
        let target = invocation.owner_target().unwrap();
        let producer = closed_header(&mut plan, target.clone(), &budget);
        let method = producer
            .packs
            .iter()
            .flat_map(|pack| pack.methods.iter())
            .find_map(|bound| match bound {
                Bound::Concrete { method, .. } => Some(method.as_ref().clone()),
                Bound::Forwarded { .. } => None,
            })
            .expect("genuine concrete source method");
        assert_eq!(
            method.prefix.as_ref(),
            &[ir::Ty::Int(ir::IntTy::U16), ir::Ty::Int(ir::IntTy::U32)]
        );
        assert_eq!(
            method.own.semantic_limits,
            [Some(super::super::super::SEMANTIC_SIZE_LIMIT / 2)]
        );
        assert_eq!(
            method.own.physical_limits,
            [None],
            "deferred trait-source own={:?}",
            method.own
        );
        assert!(
            method.own.deferred_dispatch,
            "generic pointer trait calls require exact closed replay"
        );
        let facts = plan.instance_facts(&target).unwrap();
        let actuals = [ir::Ty::Int(ir::IntTy::U8)];
        method.validate_receipt(&budget).unwrap();
        method.instantiate(&actuals, &facts, &budget).unwrap();
        for field in 0..8 {
            let mut changed = method.clone();
            match field {
                0 => changed.own.semantic_limits[0] = None,
                1 => changed.own.thin_pointees[0] = true,
                2 => changed.own.raw_pointees[0] = true,
                3 => changed.own.mut_ref_pointees[0] = true,
                4 => changed.own.pointer_strides[0] = !method.own.pointer_strides[0],
                5 => changed.own.physical_limits[0] = Some(0),
                6 => changed.own.metadata_pointees[0] = true,
                _ => changed.own.parameters[0] = true,
            }
            assert!(
                changed.instantiate(&actuals, &facts, &budget).is_err(),
                "forged field {field}"
            );
        }
        let mut bound = method.clone();
        bound.own.semantic_limits[0] = method.own.semantic_limits[0].map(|limit| limit + 1);
        assert!(bound.instantiate(&actuals, &facts, &budget).is_err());
        let mut arity = method.clone();
        arity.own.parameters.push(false);
        assert!(arity.instantiate(&actuals, &facts, &budget).is_err());
        let mut prefix = method.clone();
        Arc::make_mut(&mut prefix.prefix).swap(0, 1);
        assert!(prefix.instantiate(&actuals, &facts, &budget).is_err());
        let mut prefix_arity = method.clone();
        prefix_arity.prefix = Arc::from([ir::Ty::Int(ir::IntTy::U16)]);
        assert!(prefix_arity.instantiate(&actuals, &facts, &budget).is_err());
        let mut closed = method.clone();
        closed.closed.unsupported = Some("unsupported: forged closed prefix domain");
        assert!(closed.instantiate(&actuals, &facts, &budget).is_err());
        let mut declaration = method.clone();
        declaration.declaration = target.declaration;
        assert!(declaration.instantiate(&actuals, &facts, &budget).is_err());
        let mut item = method.clone();
        assert_ne!(item.item, item.declaration);
        item.item = item.declaration;
        assert!(item.instantiate(&actuals, &facts, &budget).is_err());
        let limited = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(method.validate_receipt(&limited).is_err());
    }

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
