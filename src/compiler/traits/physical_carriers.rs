//! Physical carrier rows have a separate authority mode. These views are local
//! to physical OBJECT ownership and are never accepted by ordinary facts APIs.
use super::*;

#[cfg(test)]
fn same_result_subject(
    left: &parameter_equality::EqualityResultSubject,
    right: &parameter_equality::EqualityResultSubject,
) -> bool {
    use parameter_equality::EqualityResultSubject;
    match (left, right) {
        (
            EqualityResultSubject::Body {
                declaration: a,
                arguments: aa,
            },
            EqualityResultSubject::Body {
                declaration: b,
                arguments: ba,
            },
        ) => a == b && aa == ba,
        (EqualityResultSubject::Direct(a), EqualityResultSubject::Direct(b)) => a == b,
        (EqualityResultSubject::Required(a), EqualityResultSubject::Required(b)) => a == b,
        _ => false,
    }
}

impl ValidatedFacts {
    /// Pair exact, independently authenticated closed nominal applications for
    /// physical carrier coordinates. This grants no source equality shortcut.
    pub(in crate::compiler) fn physical_carrier_nominals(
        &self,
        original: &Ty,
        physical: &Ty,
        engine: crate::engine::EngineAuthority,
        budget: &PlanBudget,
    ) -> Result<Vec<ir::NominalCarrierMapping>> {
        if self.phase() != FactsPhase::Closed {
            return Err(FactsError::invalid(
                budget,
                "physical carrier nominal mapping requires closed facts",
            ));
        }
        self.validate_ty_metadata_with_budget(original, budget)?;
        self.validate_ty_metadata_with_budget(physical, budget)?;
        if !self.world.types_equal(
            std::slice::from_ref(original),
            std::slice::from_ref(physical),
        )? {
            return Err(FactsError::invalid(
                budget,
                "physical carrier nominal pair changed canonical type",
            ));
        }
        super::super::nominal_realization::substitution::replay_closed_type_mapping_with(
            None,
            engine,
            original,
            physical,
            self,
            budget,
            &mut TraitScratch::default(),
        )
        .map_err(|error| FactsError::from_message(budget, &error))?;
        let mut output = Vec::new();
        self.world
            .collect_nominal_mappings(original, physical, &mut output, 0)?;
        Ok(output)
    }
}

impl CarrierContractView {
    /// Declaration-owned instantiated signature, before physical representation mapping.
    pub(in crate::compiler) fn physical_source_signature(&self) -> &Signature {
        &self.result_signature
    }
    pub(in crate::compiler) fn from_physical_boundary(
        original: &Self,
        identity: super::super::opaque_physical::PhysicalCarrierIdentity,
        signature: Signature,
        contract: ir::CarrierContract,
        source_regions: Arc<ir::SourceRegionEnv>,
        components: Arc<[ir::CarrierComponentMapping]>,
        nominals: Arc<[ir::NominalCarrierMapping]>,
    ) -> Self {
        let mut view = original.clone();
        view.physical = Some(identity);
        view.contract = contract;
        view.source_regions = source_regions;
        view.components = components;
        view.nominals = nominals;
        view.fixed = signature.ref_contract.clone();
        view.result_signature = signature;
        // The borrowed physical boundary replays the original view before
        // constructing this one. Its source-signature equality result cannot
        // authorize the different physical signature and is not retained.
        view.equality_results = None;
        // The borrowed boundary expands only graph-authenticated opaque
        // coordinates. Named IDs, outlives edges, and region constraints are
        // unchanged, while each family component retains its exact identity.
        view
    }

    #[cfg(test)]
    pub(in crate::compiler) fn matches_physical_boundary(
        &self,
        expected: &Self,
        facts: &ValidatedFacts,
    ) -> std::result::Result<(), String> {
        facts
            .world
            .check_facts(facts, false)
            .map_err(|error| error.to_string())?;
        if self.physical.is_none()
            || self.physical != expected.physical
            || self.world != expected.world
            || self.registry != expected.registry
            || self.environment != expected.environment
            || self.phase != expected.phase
            || self.declaration != expected.declaration
            || self.arguments != expected.arguments
            || !self.budget.same_authority(facts.budget())
            || self.contract != expected.contract
            || self.original_contract != expected.original_contract
            || self.origin != expected.origin
            || self.fixed != expected.fixed
            || self.regions != expected.regions
            || self.source_regions != expected.source_regions
            || self.components != expected.components
            || self.nominals != expected.nominals
            || self.result_signature != expected.result_signature
            || self.projected_subject != expected.projected_subject
            || !same_result_subject(&self.result_subject, &expected.result_subject)
            || self.projected.is_some()
            || self.closure_result.is_some()
            || self.equality_required.is_some()
            || self.equality_results.is_some()
            || !facts
                .world
                .same_env(&self.original_env, &expected.original_env)
                .map_err(|error| error.to_string())?
        {
            return Err("physical carrier view differs from exact original and graph-authorized realization".into());
        }
        Ok(())
    }
}
