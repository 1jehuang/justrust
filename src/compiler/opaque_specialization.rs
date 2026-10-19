//! Borrowed canonical owner substitution before opaque representation mapping.
//! This context stays within OBJECT. Only immutable mapped types leave it.
use super::*;

pub(super) struct Specialization<'a> {
    publication: &'a Arc<source_replay::CanonicalSourcePublication>,
    target: &'a ir::CallTarget,
    context: frontend::PlanCtx,
    closed: Arc<traits::ValidatedFacts>,
    budget: &'a ir::PlanBudget,
}

impl<'a> Specialization<'a> {
    pub(super) fn new(
        publication: &'a Arc<source_replay::CanonicalSourcePublication>,
        target: &'a ir::CallTarget,
        context: &frontend::PlanCtx,
        budget: &'a ir::PlanBudget,
    ) -> Result<Self, Error> {
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|error| source_fact_error(error, budget))?;
        let closed = context.instance_facts(target).map_err(Error::new)?;
        if closed.phase() != traits::FactsPhase::Closed
            || publication.facts().declaration() != target.declaration
            || closed.target() != Some(target)
            || !Arc::ptr_eq(publication.facts().world(), closed.world())
            || !closed.budget().same_authority(budget)
            || publication.facts().env().total_arity() != target.type_args.len()
        {
            return Err(Error::new(
                "opaque specialization changed its original owner, world or ordered actuals",
            ));
        }
        Ok(Self {
            publication,
            target,
            context: context.clone(),
            closed,
            budget,
        })
    }

    pub(super) fn ty(&mut self, original: &ir::Ty) -> Result<ir::Ty, Error> {
        ir::substitute_ty(
            original,
            self.publication.facts().env(),
            &self.target.type_args,
            ir::FactsCtx::from_validated(&self.closed),
            &mut self.context,
            self.budget,
        )
        .map_err(Error::new)
    }
}
