//! Borrowed canonical owner substitution before opaque representation mapping.
//! This context stays within OBJECT. Only immutable mapped types leave it.
use super::*;

pub(super) struct Specialization<'a> {
    publication: &'a Arc<source_replay::CanonicalSourcePublication>,
    target: &'a ir::CallTarget,
    context: frontend::PlanCtx,
    closed: Arc<traits::ValidatedFacts>,
    budget: &'a ir::PlanBudget,
    engine: crate::engine::EngineAuthority,
    // Reuse only within this authenticated publication/target substitution.
    // This scratch never escapes the OBJECT operation or becomes a fact.
    types: Vec<(ir::Ty, ir::Ty)>,
    // Original publication and closed instance have distinct proof subjects.
    // Keep their node-local caches separate rather than resetting per call.
    original_scratch: traits::TraitScratch,
    closed_scratch: traits::TraitScratch,
}

impl<'a> Specialization<'a> {
    pub(super) fn authenticate_context(
        &self,
        publication: &Arc<source_replay::CanonicalSourcePublication>,
        context: &frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        budget.charge_work(1).map_err(Error::new)?;
        if !Arc::ptr_eq(publication, self.publication)
            || !budget.same_authority(self.budget)
            || !Arc::ptr_eq(
                &context.trait_world().map_err(Error::new)?,
                self.closed.world(),
            )
        {
            return Err(Error::new(
                "opaque specialization changed exact publication, world or budget",
            ));
        }
        Ok(())
    }

    fn type_buffer(&self, count: usize) -> Result<Vec<ir::Ty>, Error> {
        let resource = |message| {
            source_fact_error(
                traits::FactsError::resource(self.budget, message),
                self.budget,
            )
        };
        let bytes = count
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or_else(|| resource("opaque replay allocation overflow"))?;
        self.budget.retain_bytes(bytes).map_err(Error::new)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| resource("opaque replay allocation failed"))?;
        Ok(values)
    }

    pub(super) fn new(
        publication: &'a Arc<source_replay::CanonicalSourcePublication>,
        target: &'a ir::CallTarget,
        context: &frontend::PlanCtx,
        budget: &'a ir::PlanBudget,
        engine: crate::engine::EngineAuthority,
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
            engine,
            types: Vec::new(),
            original_scratch: traits::TraitScratch::default(),
            closed_scratch: traits::TraitScratch::default(),
        })
    }

    pub(super) fn ty(&mut self, original: &ir::Ty) -> Result<ir::Ty, Error> {
        for (source, mapped) in &self.types {
            if ir::identical_type(source, original, self.budget).map_err(Error::new)? {
                return Ok(mapped.clone());
            }
        }
        let mapped = nominal_realization::substitution::substitute_ty(
            original,
            self.publication.facts().env(),
            &self.target.type_args,
            ir::FactsCtx::from_validated(&self.closed),
            &mut self.context,
            self.budget,
            Some(self.engine),
        )
        .map_err(Error::new)?;
        self.budget
            .retain_bytes(std::mem::size_of::<(ir::Ty, ir::Ty)>())
            .map_err(Error::new)?;
        self.types.try_reserve_exact(1).map_err(|_| {
            source_fact_error(
                traits::FactsError::resource(
                    self.budget,
                    "opaque substitution reuse allocation failed",
                ),
                self.budget,
            )
        })?;
        self.types.push((original.clone(), mapped.clone()));
        Ok(mapped)
    }

    /// Authenticate the original occurrence before substituting its exact
    /// caller arguments. This returns declaration-owned public metadata only,
    /// never hidden representation or permission to emit a physical call.
    pub(super) fn call(
        &mut self,
        occurrence: ir::ExprId,
    ) -> Result<(ir::Callee, ir::Signature), Error> {
        let publication = self.publication.clone();
        let expression = publication
            .source()
            .exprs
            .get(occurrence.0)
            .ok_or_else(|| Error::new("opaque specialization call occurrence is absent"))?;
        let ir::ExprKind::Call {
            target, signature, ..
        } = &expression.kind
        else {
            return Err(Error::new("opaque specialization occurrence is not a call"));
        };
        ir::validate_callee_scratch(
            target,
            signature,
            ir::FactsCtx::from_validated(self.publication.facts()),
            &mut self.original_scratch,
        )
        .map_err(Error::new)?;
        let (target, canonical) = match target {
            ir::Callee::Direct(target) => {
                let mut arguments = self.type_buffer(target.type_args.len())?;
                for argument in target.type_args.iter() {
                    arguments.push(self.ty(argument)?);
                }
                let target = ir::CallTarget {
                    declaration: target.declaration,
                    type_args: arguments.into(),
                };
                let proof = self
                    .closed
                    .service()
                    .discharge_direct_with(&target, &self.closed, &mut self.closed_scratch)
                    .map_err(|error| source_fact_error(error, self.budget))?;
                let mut params = self.type_buffer(proof.signature().params.len())?;
                params.extend(proof.signature().params.iter().cloned());
                let signature = ir::Signature {
                    ref_contract: proof.signature().ref_contract.clone(),
                    params,
                    result: proof.signature().result.clone(),
                };
                (ir::Callee::Direct(target), signature)
            }
            ir::Callee::TraitItem(call) => {
                let self_ty = self.ty(&call.trait_ref.self_ty)?;
                let mut arguments = self.type_buffer(call.trait_ref.args.len())?;
                let mut method_args = self.type_buffer(call.method_args.len())?;
                for argument in call.trait_ref.args.iter() {
                    arguments.push(self.ty(argument)?);
                }
                for argument in call.method_args.iter() {
                    method_args.push(self.ty(argument)?);
                }
                let call = ir::TraitCall {
                    trait_ref: ir::TraitRef {
                        declaration: call.trait_ref.declaration,
                        self_ty,
                        args: arguments.into(),
                    },
                    item: call.item,
                    method_args: method_args.into(),
                };
                let signature = self
                    .closed
                    .service()
                    .instantiate_trait_signature_with(&call, &self.closed, &mut self.closed_scratch)
                    .map_err(|error| source_fact_error(error, self.budget))?;
                (ir::Callee::TraitItem(call), signature)
            }
            ir::Callee::Virtual(_) => {
                return Err(Error::new(
                    "unsupported: virtual opaque specialization call",
                ));
            }
            ir::Callee::Untupled(_) => {
                return Err(Error::new(
                    "unsupported: function item dispatch in opaque specialization",
                ));
            }
        };
        let mut parameters = self.type_buffer(signature.params.len())?;
        for parameter in &signature.params {
            parameters.push(self.ty(parameter)?);
        }
        let result = self.ty(&signature.result)?;
        let world = self.closed.world();
        if !world
            .types_equal(&parameters, &canonical.params)
            .map_err(|error| source_fact_error(error, self.budget))?
            || !world
                .types_equal(
                    std::slice::from_ref(&result),
                    std::slice::from_ref(&canonical.result),
                )
                .map_err(|error| source_fact_error(error, self.budget))?
        {
            return Err(Error::new(
                "opaque call specialization differs from its exact declaration signature",
            ));
        }
        // Preserve the exact typed nominal reads carried by the source call's
        // substituted ABI. Declaration metadata may use equivalent source-slot
        // identities, which cannot replace graph identities in CFG operands.
        let mapped = ir::Signature {
            params: parameters,
            result,
            ref_contract: canonical.ref_contract.clone(),
        };
        nominal_realization::substitution::replay_closed_signature_with(
            self.engine,
            &target,
            &canonical,
            &mapped,
            ir::FactsCtx::from_validated(&self.closed),
            self.budget,
            &mut self.closed_scratch,
        )
        .map_err(Error::new)?;
        Ok((target, mapped))
    }
}
