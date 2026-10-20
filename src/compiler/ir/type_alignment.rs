//! Immutable authority for one canonical source alignment query.
use super::{DeclId, FactsCtx, ParamId, PlanBudget, Ty};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeQueryKind {
    Alignment,
    PrivateStorageStride,
    /// Exact original packed reserve helper only; never private u8 stride.
    PackedByteReserveUnit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAlignmentQuery {
    kind: TypeQueryKind,
    owner: DeclId,
    parameter: Option<ParamId>,
    queried: Ty,
    packed: Option<super::super::frontend::PackedReserveAuthority>,
}

impl TypeAlignmentQuery {
    // Only the authenticated embedded source checker issues this witness.
    pub(in crate::compiler) fn issue(
        queried: Ty,
        facts: &crate::compiler::traits::ValidatedFacts,
    ) -> Result<Self, String> {
        Self::issue_kind(queried, facts, TypeQueryKind::Alignment)
    }

    pub(in crate::compiler) fn issue_kind(
        queried: Ty,
        facts: &crate::compiler::traits::ValidatedFacts,
        kind: TypeQueryKind,
    ) -> Result<Self, String> {
        if kind == TypeQueryKind::PackedByteReserveUnit {
            return Err(
                "packed byte query requires operation-specific original source authority".into(),
            );
        }
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
            kind,
            owner: facts.declaration(),
            parameter: Some(parameter),
            queried,
            packed: None,
        })
    }

    pub(in crate::compiler) fn issue_packed(
        queried: Ty,
        facts: &crate::compiler::traits::ValidatedFacts,
        authority: super::super::frontend::PackedReserveAuthority,
    ) -> Result<Self, String> {
        if queried != Ty::Int(super::IntTy::U8) {
            return Err("packed byte query requires its original exact nongeneric u8 body".into());
        }
        facts
            .replay_packed_reserve_authority(&authority)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            kind: TypeQueryKind::PackedByteReserveUnit,
            owner: facts.declaration(),
            parameter: None,
            queried,
            packed: Some(authority),
        })
    }

    pub fn is_packed_byte_reserve(&self) -> bool {
        self.kind == TypeQueryKind::PackedByteReserveUnit
    }

    pub fn is_private_storage_stride(&self) -> bool {
        self.kind == TypeQueryKind::PrivateStorageStride
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
        Ok(self.kind == other.kind
            && self.owner == other.owner
            && self.parameter == other.parameter
            && self.packed == other.packed
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
        let expected = if self.kind == TypeQueryKind::PackedByteReserveUnit {
            let authority = self
                .packed
                .as_ref()
                .ok_or("packed query lost its original operation authority")?;
            if self.parameter.is_some() {
                return Err("packed query changed canonical parameter channel".into());
            }
            facts
                .replay_packed_reserve_authority(authority)
                .map_err(|error| error.to_string())?;
            Ty::Int(super::IntTy::U8)
        } else if self.packed.is_some() {
            return Err(
                "packed query operation authority transplanted to another query kind".into(),
            );
        } else if let Some(target) = facts.target() {
            if self.parameter.is_none() {
                return Err("alignment query lost its canonical parameter".into());
            }
            if target.type_args.len() != 1 {
                return Err("alignment query closed argument arity mismatch".into());
            }
            target.type_args[0].clone()
        } else {
            let mut parameters = facts.env().ordered_params();
            if parameters.next() != self.parameter || parameters.next().is_some() {
                return Err("alignment query canonical parameter identity mismatch".into());
            }
            Ty::Param(
                self.parameter
                    .ok_or("alignment query canonical parameter missing")?,
            )
        };
        if !super::identical_type(&self.queried, &expected, budget)? {
            return Err("alignment queried type differs from its exact source substitution".into());
        }
        FactsCtx::Validated(facts).validate_ty_with_budget(&self.queried, budget)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "packed_query_tests.rs"]
mod packed_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{frontend, ir};
    use std::sync::Arc;

    #[test]
    fn private_stride_query_rejects_kind_owner_and_type_tampering() {
        let program =
            frontend::parse("fn f<T>()->usize{0usize} fn g<T>()->usize{0usize} fn main(){}")
                .unwrap();
        let budget = PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let other = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        let parameter = facts.env().ordered_params().next().unwrap();
        // Internal issuance tests the sealed replay relation. Source ingress
        // separately requires the original embedded mem::storage_stride body.
        let query = TypeAlignmentQuery::issue_kind(
            Ty::Param(parameter),
            &facts,
            TypeQueryKind::PrivateStorageStride,
        )
        .unwrap();
        query
            .validate(FactsCtx::Validated(&facts), &budget)
            .unwrap();
        assert!(query.same_query(&query.clone(), &budget).unwrap());
        assert!(
            query
                .validate(FactsCtx::Validated(&other), &budget)
                .is_err()
        );
        let mut changed = query.clone();
        changed.kind = TypeQueryKind::Alignment;
        assert!(!query.same_query(&changed, &budget).unwrap());
        changed = query.clone();
        changed.queried = Ty::Int(ir::IntTy::U64);
        assert!(!query.same_query(&changed, &budget).unwrap());
        assert!(
            changed
                .validate(FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
    }

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
        forged_parameter.parameter = other.env().ordered_params().next();
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
