//! Immutable authority for one canonical source alignment query.
use super::{DeclId, FactsCtx, ParamId, PlanBudget, Ty};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAlignmentQuery {
    owner: DeclId,
    parameter: ParamId,
    queried: Ty,
}

impl TypeAlignmentQuery {
    // Only the authenticated embedded source checker issues this witness.
    pub(in crate::compiler) fn issue(
        queried: Ty,
        facts: &crate::compiler::traits::ValidatedFacts,
    ) -> Result<Self, String> {
        let mut parameters = facts.env().ordered_params();
        let parameter = parameters
            .next()
            .ok_or("alignment query requires one type parameter")?;
        if parameters.next().is_some()
            || parameter.owner.declaration != facts.declaration()
            || queried != Ty::Param(parameter)
            || facts.target().is_some()
        {
            return Err(
                "alignment query must retain its exact canonical rigid type parameter".into(),
            );
        }
        Ok(Self {
            owner: facts.declaration(),
            parameter,
            queried,
        })
    }

    pub fn queried_type(&self) -> &Ty {
        &self.queried
    }

    pub(super) fn substitute(&mut self, queried: Ty) {
        self.queried = queried;
    }

    pub(in crate::compiler) fn same_query(
        &self,
        other: &Self,
        budget: &PlanBudget,
    ) -> Result<bool, String> {
        budget.charge_work(1)?;
        Ok(self.owner == other.owner
            && self.parameter == other.parameter
            && super::identical_type(&self.queried, &other.queried, budget)?)
    }

    pub(in crate::compiler) fn validate(
        &self,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        let facts = facts
            .validated()
            .ok_or("alignment query requires validated source facts")?;
        if facts.declaration() != self.owner || facts.registry_snapshot() != self.owner.snapshot {
            return Err("alignment query has a foreign canonical source owner".into());
        }
        let expected = if let Some(target) = facts.target() {
            if target.type_args.len() != 1 {
                return Err("alignment query closed argument arity mismatch".into());
            }
            target.type_args[0].clone()
        } else {
            let mut parameters = facts.env().ordered_params();
            if parameters.next() != Some(self.parameter) || parameters.next().is_some() {
                return Err("alignment query canonical parameter identity mismatch".into());
            }
            Ty::Param(self.parameter)
        };
        if !super::identical_type(&self.queried, &expected, budget)? {
            return Err("alignment queried type differs from its exact source substitution".into());
        }
        FactsCtx::Validated(facts).validate_ty_with_budget(&self.queried, budget)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{frontend, ir};
    use std::sync::Arc;

    #[test]
    fn alignment_query_rejects_foreign_owner_and_changed_queried_type() {
        let program =
            frontend::parse("fn f<T>()->usize{0usize} fn g<T>()->usize{0usize} fn main(){}")
                .unwrap();
        let budget = PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let facts = plan.callable_facts(declaration).unwrap();
        let parameter = facts.env().ordered_params().next().unwrap();
        let query = TypeAlignmentQuery::issue(Ty::Param(parameter), &facts).unwrap();
        query
            .validate(FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let mut forged = query.clone();
        forged.queried = Ty::Int(ir::IntTy::U8);
        assert!(
            forged
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
        let other = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        let mut forged_parameter = query.clone();
        forged_parameter.parameter = other.env().ordered_params().next().unwrap();
        assert!(
            forged_parameter
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
        assert!(TypeAlignmentQuery::issue(Ty::Int(ir::IntTy::U8), &facts).is_err());
        assert!(
            query
                .validate(FactsCtx::Validated(&other), &budget)
                .is_err()
        );
        let target = ir::CallTarget {
            declaration,
            type_args: Arc::from([Ty::Int(ir::IntTy::U8)]),
        };
        let closed = facts.world().instance_facts(&target).unwrap();
        let mut instantiated = query;
        instantiated.substitute(Ty::Int(ir::IntTy::U8));
        instantiated
            .validate(FactsCtx::Validated(&closed), &budget)
            .unwrap();
        instantiated.queried = Ty::Int(ir::IntTy::U128);
        assert!(
            instantiated
                .validate(FactsCtx::Validated(&closed), &budget)
                .is_err()
        );
    }
}
