//! Closed callable tuple inputs use the checked closure's exact structural ABI.
use super::*;

impl ValidatedTraitWorld {
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
        let provider = self.discharge_in(&target, context)?;
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
            predicates: &world.empty,
            phase: FactsPhase::Closed,
        };
        let selection = world
            .require_in(
                &Predicate::Implements(goal.clone()),
                context,
                &mut Vec::new(),
            )
            .unwrap();
        let application = world
            .instantiate_projected_item(item, &goal, context)
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
