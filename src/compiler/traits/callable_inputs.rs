//! Closed callable tuple inputs use the checked closure's exact structural ABI.
use super::*;

impl ValidatedTraitWorld {
    /// The required Fn output projection may acquire reference layers only from
    /// the exact parent-sealed closure adapter selected for this application.
    pub(super) fn closed_closure_argument_result_signature(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<Signature>> {
        self.check_context(context)?;
        let ProofOrigin::Closure { kind } = &selection.0.origin else {
            return Ok(None);
        };
        let Ty::Closure(closure) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result selection lost its closure",
            ));
        };
        let Some(receipt) = closure.result_receipt() else {
            return Ok(None);
        };
        if receipt.argument_origin().is_none() {
            return Ok(None);
        }
        if self.closure_trait_kind(item.trait_decl)? != Some(*kind)
            || item.trait_decl != goal.declaration
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
            || !closure.supports(*kind)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result selection changed exact source authority",
            ));
        }
        if self.trait_header(item.trait_decl)?.items.as_ref() != [item.declaration]
            || self.item_header(item.declaration)?.trait_decl != item.trait_decl
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result requires the exact sole callable trait item",
            ));
        }
        self.validate_closure_metadata(closure, context.env())?;
        if *kind != ir::ClosureKind::FnOnce {
            retain::<Ty>(&self.budget, 1)?;
        }
        let target = self.closure_target(closure, *kind)?;
        let schema = self.callable_schema(target.declaration)?;
        let own = schema.env.scopes().last().ok_or_else(|| {
            FactsError::invalid(&self.budget, "closure result adapter lost its own binder")
        })?;
        let parameter = |index| {
            Ty::Param(ParamId {
                owner: own.binder(),
                index,
            })
        };
        if schema.signature.params.as_slice() != [parameter(0), parameter(1)]
            || schema.signature.result != parameter(2)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result adapter changed its canonical source signature",
            ));
        }
        // Structural replay only: the selected Closure proof already discharged
        // owner predicates. Do not enter trait normalization or call discharge
        // while deriving the declaration-owned reference topology.
        let mut provider = self.instantiate_raw_cached(
            &schema,
            &target.type_args,
            context,
            Some(target.declaration),
            &mut active.instantiations,
        )?;
        provider.signature.ref_contract = self.realize_ref_contract(
            &schema,
            &target.type_args,
            &provider.signature,
            Some(target.declaration),
        )?;
        if goal.args.len() != 1
            || !self.eq_ty(&goal.args[0], &self.closure_arguments(closure)?, 0)?
            || !self.eq_types(&signature.params, &provider.signature.params, 0)?
            || !self.eq_ty(&signature.result, &provider.signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closure result differs from its authenticated adapter ABI",
            ));
        }
        Ok(Some(provider.signature))
    }

    pub(super) fn closed_closure_tuple_input(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        signature: &Signature,
        selection: &ProofEvidence,
        index: usize,
        context: ProofContext<'_>,
    ) -> Result<bool> {
        self.check_context(context)?;
        if index != 1 {
            return Ok(false);
        }
        if let ProofOrigin::FnItem { .. } = &selection.0.origin {
            return self.closed_fn_item_tuple_input(goal, signature, index, context);
        }
        let ProofOrigin::Closure { kind } = &selection.0.origin else {
            return Ok(false);
        };
        let kind = *kind;
        let Ty::Closure(closure) = &goal.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple proof has no checked closure",
            ));
        };
        if self.closure_trait_kind(item.trait_decl)? != Some(kind)
            || item.trait_decl != goal.declaration
            || selection.0.world != self.id
            || selection.0.environment != self.empty.index
            || selection.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&selection.0.goal, &Predicate::Implements(goal.clone()))?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple proof changed its canonical owner or phase",
            ));
        }
        self.validate_closure_metadata(closure, context.env())?;
        let target = self.closure_target(closure, kind)?;
        let mut active = ProofScratch::default();
        let provider = self.discharge_in(&target, context, &mut active)?;
        if !self.eq_types(&signature.params, &provider.signature.params, 0)?
            || !self.eq_ty(&signature.result, &provider.signature.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple differs from its checked adapter ABI",
            ));
        }
        let Some(tuple) = signature.params.get(index) else {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple signature has no argument position",
            ));
        };
        if goal.args.len() != 1
            || !matches!(tuple, Ty::Tuple(_) | Ty::Unit)
            || !closure.supports(kind)
            || !self.eq_ty(tuple, &goal.args[0], 0)?
            || !self.eq_ty(tuple, &self.closure_arguments(closure)?, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "callable tuple differs from its checked closure arguments",
            ));
        }
        // Reference results require their own checked source-origin descriptor.
        // Input admission must not manufacture that separate capability.
        if !charged(
            &self.budget,
            ir::carrier_locations(&signature.result, &self.budget),
        )?
        .is_empty()
            && self
                .closed_closure_argument_result_signature(
                    item,
                    goal,
                    signature,
                    selection,
                    context,
                    &mut active,
                )?
                .is_none()
        {
            return Ok(false);
        }
        for location in charged(&self.budget, ir::carrier_locations(tuple, &self.budget))?.iter() {
            if !matches!(location.path.0.first(), Some(ir::CarrierStep::Field(_)))
                || !matches!(location.channel, ir::CarrierChannel::Fixed(_))
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed callable tuple has a nonstructural or unresolved carrier",
                ));
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend;

    #[test]
    fn projected_argument_result_replays_sealed_adapter_and_tuple_source() {
        use crate::compiler::{
            ClosureBodiesFact, CompilerProvider, Input, Snapshot, TEMPLATE_BODY, keys,
        };
        use crate::engine::Engine;
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(
                "struct Item{name:String}fn main(){let f=|i:&&Item|i.name.as_str();}".into(),
            ),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let signatures = engine.request(keys::SignaturesDebtRoot.key());
        let body = engine.request(keys::item(TEMPLATE_BODY, 0));
        engine.run(1).unwrap();
        let plan = signatures.fact::<frontend::PlanCtx>(0).unwrap();
        let closures = body.fact::<ClosureBodiesFact>(1).unwrap();
        let closure = closures.bodies.first().unwrap().ty.clone();
        let world = plan.trait_world().unwrap();
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            let header = world
                .headers
                .traits
                .iter()
                .find(|header| world.closure_trait_kind(header.declaration).unwrap() == Some(mode))
                .unwrap();
            let item = world.item_header(header.items[0]).unwrap();
            let goal = TraitRef {
                declaration: header.declaration,
                self_ty: Ty::Closure(closure.clone()),
                args: Arc::from([world.closure_arguments(&closure).unwrap()]),
            };
            let context = ProofContext {
                original: None,
                predicates: &world.empty,
                phase: FactsPhase::Closed,
            };
            let mut active = ProofScratch::default();
            let selection = world
                .require_in(&Predicate::Implements(goal.clone()), context, &mut active)
                .unwrap();
            let application = world
                .instantiate_projected_item(item, &goal, &[], context)
                .unwrap();
            let signature = &application.instantiated.signature;
            assert_eq!(signature.ref_contract.results.len(), 1);
            assert!(
                world
                    .closed_closure_tuple_input(item, &goal, signature, &selection, 1, context)
                    .unwrap()
            );
            let facts = world
                .closure_callable_facts_for(closure.clone(), mode)
                .unwrap();
            let callee = Callee::TraitItem(TraitCall {
                item: item.declaration,
                trait_ref: goal.clone(),
                method_args: Arc::from([]),
            });
            let view = facts
                .carrier_contract(SignatureSubject::Call(&callee))
                .unwrap();
            view.validate_for(&facts).unwrap();
            let sources = &view.contract().results[0].sources;
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].param, 1);
            assert_eq!(
                sources[0].location.path.0.as_ref(),
                &[ir::CarrierStep::Field(0)]
            );
            assert_eq!(
                sources[0].location.channel,
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
            );
            for change in 0..3 {
                let mut record = selection.0.as_ref().clone();
                match change {
                    0 => record.environment ^= 1,
                    1 => record.phase = FactsPhase::Universal,
                    _ => {
                        record.origin = ProofOrigin::Closure {
                            kind: if mode == ir::ClosureKind::Fn {
                                ir::ClosureKind::FnOnce
                            } else {
                                ir::ClosureKind::Fn
                            },
                        }
                    }
                }
                assert!(
                    world
                        .closed_closure_argument_result_signature(
                            item,
                            &goal,
                            signature,
                            &ProofEvidence(Arc::new(record)),
                            context,
                            &mut active,
                        )
                        .is_err()
                );
            }
            let mut changed = signature.clone();
            changed.result = Ty::Bool;
            assert!(
                world
                    .closed_closure_argument_result_signature(
                        item,
                        &goal,
                        &changed,
                        &selection,
                        context,
                        &mut active,
                    )
                    .is_err()
            );
            let mut wrong_item = item.clone();
            wrong_item.declaration = closure.provider();
            assert!(
                world
                    .closed_closure_argument_result_signature(
                        &wrong_item,
                        &goal,
                        signature,
                        &selection,
                        context,
                        &mut active,
                    )
                    .is_err()
            );
            let mut forged = view.as_ref().clone();
            let mut results = forged.contract.results.to_vec();
            let mut sources = results[0].sources.to_vec();
            sources[0].param = 0;
            results[0].sources = sources.into();
            forged.contract.results = results.into();
            assert!(forged.validate_for(&facts).is_err());
        }
    }

    fn fixture() -> (Arc<ValidatedTraitWorld>, Arc<ir::ClosureTy>) {
        let mut program = frontend::parse("mod ops {pub trait FnOnce<A>{type Output;fn call_once(self,args:A)->Self::Output;}pub trait Fn<A>:FnOnce<A>{fn call(&self,args:A)->Self::Output;}}use ops::{Fn,FnOnce};fn main(){let f=|x:&i32,flag:bool|if flag{*x}else{0};}").unwrap();
        program.prelude_module = program.modules.parent(program.traits[0].module).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let index = program
            .functions
            .iter()
            .position(|function| {
                program
                    .names
                    .spelling(*function.name.segments().last().unwrap())
                    .unwrap()
                    == "main"
            })
            .unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected checked parent source");
        };
        let closure = publication
            .source()
            .exprs
            .iter()
            .find_map(|expression| match &expression.ty {
                Ty::Closure(closure) => Some(closure.clone()),
                _ => None,
            })
            .unwrap();
        (plan.trait_world().unwrap(), closure)
    }

    #[test]
    fn closed_callable_tuple_replays_exact_proof_and_adapter_identity() {
        let (world, closure) = fixture();
        let header = world
            .headers
            .traits
            .iter()
            .find(|header| {
                world.closure_trait_kind(header.declaration).unwrap() == Some(ir::ClosureKind::Fn)
            })
            .unwrap();
        let item = world.item_header(header.items[0]).unwrap();
        let goal = TraitRef {
            declaration: header.declaration,
            self_ty: Ty::Closure(closure.clone()),
            args: Arc::from([world.closure_arguments(&closure).unwrap()]),
        };
        let context = ProofContext {
            original: None,
            predicates: &world.empty,
            phase: FactsPhase::Closed,
        };
        let selection = world
            .require_in(
                &Predicate::Implements(goal.clone()),
                context,
                &mut ProofScratch::default(),
            )
            .unwrap();
        let application = world
            .instantiate_projected_item(item, &goal, &[], context)
            .unwrap();
        let signature = application.instantiated.signature;
        assert!(
            world
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 1, context)
                .unwrap()
        );
        assert!(
            !world
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 0, context)
                .unwrap()
        );

        let reject = |goal: &TraitRef, signature: &Signature, selection: &ProofEvidence| {
            assert!(
                matches!(world.closed_closure_tuple_input(item, goal, signature, selection, 1, context), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        };
        let mut changed = signature.clone();
        changed.params.swap(0, 1);
        reject(&goal, &changed, &selection);
        let mut changed = signature.clone();
        let Ty::Tuple(fields) = &signature.params[1] else {
            panic!("checked callable has no tuple");
        };
        let mut reversed = fields.to_vec();
        reversed.reverse();
        changed.params[1] = Ty::Tuple(reversed.into());
        reject(&goal, &changed, &selection);
        let mut strengthened = fields.to_vec();
        let Ty::Ref { referent, .. } = &strengthened[0] else {
            panic!("expected reference input");
        };
        strengthened[0] = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: referent.clone(),
        };
        changed.params[1] = Ty::Tuple(strengthened.into());
        reject(&goal, &changed, &selection);
        let mut changed = signature.clone();
        changed.result = Ty::Bool;
        reject(&goal, &changed, &selection);
        for phase in [FactsPhase::Universal, FactsPhase::Closed] {
            let mut record = selection.0.as_ref().clone();
            record.phase = phase;
            record.environment ^= 1;
            reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        }
        let mut record = selection.0.as_ref().clone();
        record.phase = FactsPhase::Universal;
        reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        let mut record = selection.0.as_ref().clone();
        record.origin = ProofOrigin::Closure {
            kind: ir::ClosureKind::FnOnce,
        };
        reject(&goal, &signature, &ProofEvidence(Arc::new(record)));
        let forged = Arc::new(
            ir::ClosureTy::new(
                closure.owner(),
                closure.owner_args().into(),
                closure.site() + 1,
                closure.provider(),
                closure.params().into(),
                closure.result().clone(),
                closure.captures().into(),
                closure.kind(),
            )
            .with_sources(closure.sources().into()),
        );
        let mut changed_goal = goal.clone();
        changed_goal.self_ty = Ty::Closure(forged);
        let mut record = selection.0.as_ref().clone();
        record.goal = Predicate::Implements(changed_goal.clone());
        reject(&changed_goal, &signature, &ProofEvidence(Arc::new(record)));
        let (other, other_closure) = fixture();
        let foreign_context = ProofContext {
            original: None,
            predicates: &other.empty,
            phase: FactsPhase::Closed,
        };
        assert!(
            other
                .closed_closure_tuple_input(item, &goal, &signature, &selection, 1, foreign_context)
                .is_err()
        );
        let mut changed_goal = goal.clone();
        changed_goal.self_ty = Ty::Closure(other_closure);
        let mut record = selection.0.as_ref().clone();
        record.goal = Predicate::Implements(changed_goal.clone());
        reject(&changed_goal, &signature, &ProofEvidence(Arc::new(record)));
    }
}
