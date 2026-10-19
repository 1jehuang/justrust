//! Per-selected-initializer qualification. Values remain separate lazy demands.
use super::*;

#[derive(Debug)]
pub(super) struct QualifiedMemberUse {
    pub(super) authorization: Arc<frontend::AuthorizedAssociatedConstant>,
    pub(super) checked: Arc<AssociatedConstantCheckedFact>,
    dependencies: Arc<[Arc<QualifiedMemberUse>]>,
}
impl QualifiedMemberUse {
    pub(super) fn new(
        authorization: Arc<frontend::AuthorizedAssociatedConstant>,
        checked: Arc<AssociatedConstantCheckedFact>,
        dependencies: Vec<Arc<Self>>,
        plan: &frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<Self, Error> {
        budget
            .retain_bytes(
                std::mem::size_of::<Self>() + dependencies.len() * std::mem::size_of::<Arc<Self>>(),
            )
            .map_err(Error::new)?;
        let proof = Self {
            authorization,
            checked,
            dependencies: dependencies.into(),
        };
        proof.validate(
            plan,
            proof.authorization.caller(),
            proof.authorization.path(),
            budget,
        )?;
        Ok(proof)
    }
    pub(super) fn validate(
        &self,
        plan: &frontend::PlanCtx,
        caller: &ir::CallTarget,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        if self.authorization.caller() != caller || self.authorization.path() != path {
            return Err(Error::new(
                "associated constant qualification has foreign original caller/site",
            ));
        }
        self.authorization
            .validate(plan, budget)
            .map_err(Error::new)?;
        self.checked
            .validate(plan, self.authorization.selected().declaration(), budget)?;
        let body = self.checked.body.as_ref().ok_or_else(|| {
            Error::new("qualified associated constant has no original initializer")
        })?;
        let mut dependencies = self.dependencies.iter();
        for expression in body.source().expressions.iter() {
            budget.charge_work(1).map_err(Error::new)?;
            let const_source::SyntaxKind::MemberInput(input) = &expression.kind else {
                continue;
            };
            input.validate(plan, budget).map_err(Error::new)?;
            let dependency = dependencies.next().ok_or_else(|| {
                Error::new("associated constant qualification lacks an original member input")
            })?;
            let expected = initializer_dependency_path(path, input.path(), budget)?;
            if dependency.authorization.caller() != caller
                || dependency.authorization.path() != expected
            {
                return Err(Error::new(
                    "associated constant qualification changed original dependency order/site",
                ));
            }
            // Each child is itself a sealed graph qualification. Recheck its
            // exact declaration and source selection, without synchronously
            // traversing unrelated descendants or demanding any scalar value.
            dependency
                .authorization
                .validate(plan, budget)
                .map_err(Error::new)?;
            dependency.checked.validate(
                plan,
                dependency.authorization.selected().declaration(),
                budget,
            )?;
        }
        if dependencies.next().is_some() {
            return Err(Error::new(
                "associated constant qualification has extra member inputs",
            ));
        }
        Ok(())
    }
}
