//! Borrowed carrier-boundary realization for an already replayed physical CFG.
//! This is not a declaration carrier view and cannot be passed to ordinary
//! ownership as one. The original declaration contract remains available and
//! every realized boundary retains its borrowed normalization authority.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler) struct PhysicalCarrierIdentity {
    publication: source_replay::OriginalBodyId,
    owner: ir::CallTarget,
    occurrence: Option<ir::ExprId>,
    signature: ir::Signature,
    contract: ir::CarrierContract,
}

pub(in crate::compiler) struct PhysicalCarrierBoundary<'a> {
    validation: &'a PhysicalValidation<'a>,
    function: &'a cfg::Function,
    facts: &'a traits::ValidatedFacts,
    original: Arc<traits::CarrierContractView>,
    occurrence: Option<ir::ExprId>,
    signature: ir::Signature,
    contract: ir::CarrierContract,
}

impl PhysicalValidation<'_> {
    pub(in crate::compiler) fn validate_carrier_view(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        occurrence: Option<ir::ExprId>,
        view: &traits::CarrierContractView,
    ) -> Result<(), String> {
        let expected = if let Some(site) = occurrence {
            let index = function
                .blocks
                .iter()
                .position(|block| block.terminator.origin == Some(site))
                .ok_or("physical carrier view lost its original call occurrence")?;
            self.call_carriers(function, facts, cfg::BlockId(index))?
                .realize()?
        } else {
            self.body_carriers(function, facts)?.realize()?
        };
        view.matches_physical_boundary(&expected, facts)
    }

    pub(in crate::compiler) fn original_call_evidence(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<Arc<ir::ValidatedCallEvidence>, String> {
        self.replay_call_arguments(function, facts, block)?;
        let site = function.blocks[block.0]
            .terminator
            .origin
            .ok_or("physical call lost source occurrence")?;
        self.original
            .call_evidence
            .iter()
            .find(|evidence| evidence.call() == site)
            .cloned()
            .ok_or_else(|| "physical call lacks canonical original argument evidence".into())
    }

    pub(in crate::compiler) fn body_carriers<'a>(
        &'a self,
        function: &'a cfg::Function,
        facts: &'a traits::ValidatedFacts,
    ) -> Result<PhysicalCarrierBoundary<'a>, String> {
        self.validate_cfg(function, facts)?;
        let signature = ir::Signature {
            params: function
                .params
                .iter()
                .map(|id| function.locals[id.0].ty.clone())
                .collect(),
            result: function.result.clone(),
            ref_contract: function.ref_contract.clone(),
        };
        let original = facts
            .carrier_contract(ir::SignatureSubject::Body)
            .map_err(|error| {
                error
                    .into_diagnostic(self.budget)
                    .unwrap_or_else(|error| error.to_string())
            })?;
        original.validate_for(facts).map_err(|error| {
            error
                .into_diagnostic(self.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        // The first physical body slice has no boundary carriers. A public
        // opaque family is not presumed empty: validate_cfg above replays the
        // exact defining publication and graph-read representation records.
        require_empty_boundary(&signature, self.budget)?;
        Ok(PhysicalCarrierBoundary {
            validation: self,
            function,
            facts,
            original,
            occurrence: None,
            signature,
            contract: ir::CarrierContract::default(),
        })
    }

    pub(in crate::compiler) fn call_carriers<'a>(
        &'a self,
        function: &'a cfg::Function,
        facts: &'a traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<PhysicalCarrierBoundary<'a>, String> {
        self.validate_cfg(function, facts)?;
        let block = function
            .blocks
            .get(block.0)
            .ok_or("physical carrier call block is absent")?;
        let cfg::Terminator::Call {
            target, signature, ..
        } = &block.terminator.value
        else {
            return Err("physical carrier boundary does not name a call".into());
        };
        self.validate_call(block.terminator.origin, target, signature)?;
        let original = facts
            .carrier_contract(ir::SignatureSubject::Call(target))
            .map_err(|error| {
                error
                    .into_diagnostic(self.budget)
                    .unwrap_or_else(|error| error.to_string())
            })?;
        original.validate_for(facts).map_err(|error| {
            error
                .into_diagnostic(self.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        let site = block
            .terminator
            .origin
            .ok_or("physical carrier call has no source occurrence")?;
        let contract =
            if self.proof.direct_calls.iter().any(|call| {
                call.occurrence == site && call.public_signature != call.physical_signature
            }) {
                // An ABI-changing direct call is admitted only when its complete
                // independently normalized input and output carrier sets are empty.
                require_empty_boundary(signature, self.budget)?;
                ir::CarrierContract::default()
            } else {
                // The selected physical trait implementation has its own genuine
                // declaration-owned carrier contract, not the abstract opaque one.
                // Its signature must match that declaration before keeping rows.
                ir::validate_callee(target, signature, ir::FactsCtx::from_validated(facts))?;
                original.contract().clone()
            };
        Ok(PhysicalCarrierBoundary {
            validation: self,
            function,
            facts,
            original,
            occurrence: Some(site),
            signature: signature.clone(),
            contract,
        })
    }

    /// Recover exact original argument evidence only after the full CFG and
    /// occurrence-specific physical callee mapping have replayed. This never
    /// relabels the original universal capsule as closed evidence.
    pub(in crate::compiler) fn replay_call_arguments(
        &self,
        function: &cfg::Function,
        facts: &traits::ValidatedFacts,
        block: cfg::BlockId,
    ) -> Result<(), String> {
        self.validate_cfg(function, facts)?;
        let block = function
            .blocks
            .get(block.0)
            .ok_or("physical evidence call block is absent")?;
        let cfg::Terminator::Call {
            target,
            signature,
            args,
            ..
        } = &block.terminator.value
        else {
            return Err("physical evidence does not name a call".into());
        };
        self.validate_call(block.terminator.origin, target, signature)?;
        let site = block
            .terminator
            .origin
            .ok_or("physical evidence lost source occurrence")?;
        let evidence = self
            .original
            .call_evidence
            .iter()
            .find(|evidence| evidence.call() == site)
            .ok_or("physical call lacks original validated argument evidence")?;
        if !Arc::ptr_eq(evidence.facts(), self.publication.facts()) {
            return Err("physical call argument evidence belongs to another facts context".into());
        }
        let expression = self
            .publication
            .source()
            .exprs
            .get(site.0)
            .ok_or("physical call source occurrence is absent")?;
        let ir::ExprKind::Call {
            args: original_args,
            ..
        } = &expression.kind
        else {
            return Err("physical argument evidence source is not a call".into());
        };
        if evidence.trace().arguments.len() != args.len()
            || original_args.len() != args.len()
            || evidence
                .trace()
                .arguments
                .iter()
                .enumerate()
                .any(|(index, row)| row.argument != index || row.value != original_args[index])
        {
            return Err("physical call original argument trace changed".into());
        }
        Ok(())
    }
}

impl PhysicalCarrierBoundary<'_> {
    pub(in crate::compiler) fn realize(&self) -> Result<Arc<traits::CarrierContractView>, String> {
        self.validate()?;
        if self.original.source_region_env().region_count != 0
            || !self.original.region_schema().constraints.is_empty()
        {
            return Err(
                "unsupported: physical carrier realization has declared lifetime constraints"
                    .into(),
            );
        }
        let identity = PhysicalCarrierIdentity {
            publication: self.validation.publication.identity().clone(),
            owner: self.validation.proof.target.clone(),
            occurrence: self.occurrence,
            signature: self.signature.clone(),
            contract: self.contract.clone(),
        };
        Ok(Arc::new(
            traits::CarrierContractView::from_physical_boundary(
                &self.original,
                identity,
                self.signature.clone(),
                self.contract.clone(),
            ),
        ))
    }
    pub(in crate::compiler) fn validate(&self) -> Result<(), String> {
        self.validation.validate_cfg(self.function, self.facts)?;
        self.original.validate_for(self.facts).map_err(|error| {
            error
                .into_diagnostic(self.validation.budget)
                .unwrap_or_else(|error| error.to_string())
        })?;
        if let Some(site) = self.occurrence {
            let block = self
                .function
                .blocks
                .iter()
                .find(|block| block.terminator.origin == Some(site))
                .ok_or("physical carrier boundary occurrence disappeared")?;
            let cfg::Terminator::Call {
                target, signature, ..
            } = &block.terminator.value
            else {
                return Err("physical carrier boundary occurrence changed kind".into());
            };
            self.validation
                .validate_call(Some(site), target, signature)?;
            if signature != &self.signature {
                return Err("physical carrier boundary signature changed".into());
            }
        } else {
            require_empty_boundary(&self.signature, self.validation.budget)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::compiler) fn contract(&self) -> &ir::CarrierContract {
        &self.contract
    }
    #[cfg(test)]
    pub(in crate::compiler) fn original(&self) -> &traits::CarrierContractView {
        &self.original
    }
}

fn require_empty_boundary(
    signature: &ir::Signature,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for ty in signature
        .params
        .iter()
        .chain(std::iter::once(&signature.result))
    {
        if !ir::carrier_locations(ty, budget)?.is_empty() {
            return Err(
                "unsupported: ABI-changing physical boundary has reference carriers".into(),
            );
        }
    }
    signature
        .ref_contract
        .validate(&signature.params, &signature.result)?;
    Ok(())
}
