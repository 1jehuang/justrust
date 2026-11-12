//! Open application frontiers are pending only during nonexecuting inference.
use super::*;

type Candidates = crate::compiler::token_session::BudgetVec<(ir::TraitRef, ir::DeclId)>;

pub(in crate::compiler::frontend) struct InferredMethodCandidates {
    pub(in crate::compiler::frontend) applications: Candidates,
    pub(in crate::compiler::frontend) unproved_expected: bool,
    pub(in crate::compiler::frontend) pending: bool,
}

impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler::frontend) fn method_trait_candidates_expected(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        budget: &ir::PlanBudget,
        expected: Option<(&ir::Ty, ReceiverKind)>,
    ) -> Result<(Candidates, bool), String> {
        // Ordinary checked selection remains strict. An open application is
        // still an explicit diagnostic, never a lower-priority fallback.
        self.method_trait_candidates_discovery(owner, name, caller, module, budget, expected, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler::frontend) fn method_trait_candidates_inferred(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        budget: &ir::PlanBudget,
        expected: Option<(&ir::Ty, ReceiverKind)>,
    ) -> Result<InferredMethodCandidates, String> {
        let mut pending = false;
        let (applications, unproved_expected) = self.method_trait_candidates_discovery(
            owner,
            name,
            caller,
            module,
            budget,
            expected,
            Some(&mut pending),
        )?;
        Ok(InferredMethodCandidates {
            applications,
            unproved_expected,
            pending,
        })
    }
}
