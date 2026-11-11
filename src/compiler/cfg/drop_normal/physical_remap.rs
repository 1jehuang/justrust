//! Borrowed reissuance of existing cleanup at its exact original CFG event.
//! This does not insert events or authorize any executable destructor demand.
use super::*;
use crate::compiler::source_replay::CanonicalSourcePublication;
use crate::compiler::traits::FactsPhase;

pub(in crate::compiler) struct PhysicalCleanupReplay<'a> {
    original: &'a Function,
    publication: &'a Arc<CanonicalSourcePublication>,
    closed: &'a ValidatedFacts,
    budget: &'a ir::PlanBudget,
    source_scratch: TraitScratch,
    closed_scratch: TraitScratch,
}

impl ValidatedCleanupCall {
    pub(in crate::compiler) fn physical_owner_replay<'a>(
        original: &'a Function,
        publication: &'a Arc<CanonicalSourcePublication>,
        target: &ir::CallTarget,
        closed: &'a ValidatedFacts,
        budget: &'a ir::PlanBudget,
    ) -> Result<PhysicalCleanupReplay<'a>, String> {
        original
            .validate_source_publication(publication, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        if closed.phase() != FactsPhase::Closed
            || closed.target() != Some(target)
            || publication.facts().declaration() != target.declaration
            || !Arc::ptr_eq(publication.facts().world(), closed.world())
            || !closed.budget().same_authority(budget)
        {
            return Err("physical cleanup changed source, caller, world or budget".into());
        }
        closed
            .validate_source_type_closing_context(
                publication.facts(),
                publication.facts().env(),
                &target.type_args,
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(PhysicalCleanupReplay {
            original,
            publication,
            closed,
            budget,
            source_scratch: TraitScratch::default(),
            closed_scratch: TraitScratch::default(),
        })
    }
}

impl PhysicalCleanupReplay<'_> {
    pub(in crate::compiler) fn remap(
        &mut self,
        block: usize,
        source: &Arc<ValidatedCleanupCall>,
        locals: &[ir::LocalDecl],
        return_local: LocalId,
    ) -> Result<Arc<ValidatedCleanupCall>, String> {
        self.budget.charge_work(1)?;
        let original = self
            .original
            .blocks
            .get(block)
            .ok_or("physical cleanup block is outside original CFG")?;
        let Terminator::Drop { cleanup, .. } = &original.terminator.value else {
            return Err("physical cleanup requires an original Drop event".into());
        };
        if !Arc::ptr_eq(source, cleanup)
            || locals.len() != self.original.locals.len()
            || return_local != self.original.return_local
        {
            return Err(
                "physical cleanup changed exact original descriptor or local inventory".into(),
            );
        }
        source.validate(
            self.original,
            self.publication.facts(),
            &mut self.source_scratch,
        )?;
        let owner = &source.owner;
        let ty = ir::projected_ty(
            &locals
                .get(owner.local.0)
                .ok_or("physical cleanup owner local is absent")?
                .ty,
            &owner.projection,
            self.budget,
        )?;
        if owner.local == return_local {
            return Err("physical cleanup cannot destroy protected return storage".into());
        }
        let NormalDropPlan::Destructor(destructor) = self
            .closed
            .normal_drop_plan_with_scratch(&ty, &mut self.closed_scratch)
            .map_err(|e| e.into_legacy_diagnostic(self.budget))?
        else {
            return Err("unsupported: mapped existing cleanup has no canonical destructor".into());
        };
        destructor
            .validate_with_scratch(&ty, self.closed, &mut self.closed_scratch)
            .map_err(|e| e.into_legacy_diagnostic(self.budget))?;
        let bytes = owner
            .projection
            .len()
            .checked_mul(std::mem::size_of::<ir::Projection>())
            .and_then(|n| {
                n.checked_add(
                    std::mem::size_of::<ValidatedCleanupCall>() + 2 * std::mem::size_of::<usize>(),
                )
            })
            .ok_or("physical cleanup descriptor allocation overflow")?;
        self.budget.charge_nodes(1)?;
        self.budget.retain_bytes(bytes)?;
        Ok(Arc::new(ValidatedCleanupCall {
            owner: owner.clone(),
            destructor,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{
        CompilerProvider, Facts, Input, Snapshot, TEMPLATE_BODY, TemplateBodyFact, keys,
        opaque_specialization, source,
    };
    use crate::{Context, Engine, Error, Key, Provider, TaskFuture};

    #[test]
    fn physical_existing_cleanup_replays_exact_source_owner_and_facts() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }
            fn start(&self, cx: Context, requested: Key) -> TaskFuture {
                if requested.kind != 9027 {
                    return self.0.start(cx, requested);
                }
                let provider = self.0.clone();
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let find = |name| {
                        (0..types.callable_count()).find(|i| {
                        types.callable_decl(*i).ok().and_then(|id| types.persistent_decl_path(id).ok())
                            .is_some_and(|p| matches!(p.item(), crate::compiler::frontend::PersistentDeclItem::Name(n) if n == name))
                    }).unwrap()
                    };
                    let index = find("probe");
                    let other = find("other");
                    let template: Arc<TemplateBodyFact> =
                        facts.get(keys::item(TEMPLATE_BODY, index)).await?;
                    let publication = template.publication.as_ref().unwrap();
                    let target = ir::CallTarget {
                        declaration: types.callable_decl(index).map_err(Error::new)?,
                        type_args: Arc::from([]),
                    };
                    let budget = &provider.0.budget;
                    let original = crate::compiler::cfg::lower_source_template(publication, budget)
                        .map_err(|e| Error::new(e.to_string()))?;
                    let closed = types.instance_facts(&target).map_err(Error::new)?;
                    let mut replay = ValidatedCleanupCall::physical_owner_replay(
                        original.function(),
                        publication,
                        &target,
                        &closed,
                        budget,
                    )
                    .map_err(Error::new)?;
                    let mut specialization = opaque_specialization::Specialization::new(
                        publication,
                        &target,
                        &types,
                        budget,
                        cx.authority(),
                    )?;
                    let mut locals = original.function().locals.clone();
                    for local in &mut locals {
                        local.ty = specialization.ty(&local.ty)?;
                    }
                    let events = original
                        .function()
                        .blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(i, block)| {
                            if let Terminator::Drop { cleanup, .. } = &block.terminator.value {
                                Some((i, cleanup.clone()))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    assert!(events.len() >= 2, "actual Box and Token cleanup events");
                    let mut changed_allocation = false;
                    for (block, source) in &events {
                        let physical = replay
                            .remap(*block, source, &locals, original.function().return_local)
                            .map_err(Error::new)?;
                        let ty = ir::projected_ty(
                            &locals[source.owner.local.0].ty,
                            &source.owner.projection,
                            budget,
                        )
                        .map_err(Error::new)?;
                        physical
                            .destructor
                            .validate(&ty, &closed)
                            .map_err(|e| crate::compiler::source_fact_error(e, budget))?;
                        changed_allocation |= !ir::identical_type(
                            source.destructor.owner(),
                            physical.destructor.owner(),
                            budget,
                        )
                        .map_err(Error::new)?;
                        assert!(
                            replay
                                .remap(
                                    usize::MAX,
                                    source,
                                    &locals,
                                    original.function().return_local
                                )
                                .is_err()
                        );
                        assert!(
                            replay
                                .remap(
                                    *block,
                                    &Arc::new(source.as_ref().clone()),
                                    &locals,
                                    original.function().return_local
                                )
                                .is_err()
                        );
                        let mut forged = source.as_ref().clone();
                        forged.owner.local = original.function().return_local;
                        assert!(
                            replay
                                .remap(
                                    *block,
                                    &Arc::new(forged),
                                    &locals,
                                    original.function().return_local
                                )
                                .is_err()
                        );
                        let mut changed = locals.clone();
                        changed[source.owner.local.0].ty = Ty::Bool;
                        assert!(
                            replay
                                .remap(*block, source, &changed, original.function().return_local)
                                .is_err()
                        );
                    }
                    assert!(
                        changed_allocation,
                        "real closure Box owner allocation is reminted by source closing"
                    );
                    let (block, source) = &events[0];
                    let other_descriptor = events
                        .iter()
                        .find(|(_, event)| event.destructor.glue() != source.destructor.glue())
                        .expect("distinct canonical Box/Token glue");
                    let forged = Arc::new(ValidatedCleanupCall {
                        owner: source.owner.clone(),
                        destructor: other_descriptor.1.destructor.clone(),
                    });
                    assert!(
                        replay
                            .remap(*block, &forged, &locals, original.function().return_local)
                            .is_err(),
                        "source target/glue substitution must not authorize another descriptor"
                    );
                    let non_drop = original
                        .function()
                        .blocks
                        .iter()
                        .position(|b| !matches!(b.terminator.value, Terminator::Drop { .. }))
                        .unwrap();
                    assert!(
                        replay
                            .remap(non_drop, source, &locals, original.function().return_local)
                            .is_err(),
                        "no new cleanup event may be invented"
                    );
                    let foreign_target = ir::CallTarget {
                        declaration: types.callable_decl(other).map_err(Error::new)?,
                        type_args: Arc::from([]),
                    };
                    let foreign = types.instance_facts(&foreign_target).map_err(Error::new)?;
                    assert!(
                        ValidatedCleanupCall::physical_owner_replay(
                            original.function(),
                            publication,
                            &target,
                            &foreign,
                            budget
                        )
                        .is_err()
                    );
                    let wrong_args = ir::CallTarget {
                        declaration: target.declaration,
                        type_args: Arc::from([Ty::Int(ir::IntTy::U16)]),
                    };
                    assert!(
                        ValidatedCleanupCall::physical_owner_replay(
                            original.function(),
                            publication,
                            &wrong_args,
                            &closed,
                            budget
                        )
                        .is_err()
                    );
                    cx.publish_fact(0, ())
                })
            }
        }
        for workers in [1, 8] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("struct Token{n:i32}impl Drop for Token{fn drop(&mut self){}}fn counter()->impl FnMut()->i32{let mut n=0;move||{n+=1;n}}fn probe(){let mut c=counter();c();let n=41;let b=Box::new(move||n+1);let token=Token{n:7};let f=b.as_ref();println!(\"{} {}\",f(),token.n);}fn other(){}fn main(){}".into()),
                source_options: source::SourceOptions::default(), budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(keys::test_key(9027, vec![]));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }
}
