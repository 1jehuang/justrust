//! Ephemeral native ownership proof for graph-authorized physical CFGs.
//! This wrapper is deliberately borrowed and is never a published graph fact.
use super::*;

pub(in crate::compiler) struct PhysicalConcrete<'a> {
    function: Arc<cfg::Function>,
    facts: Arc<ValidatedFacts>,
    analysis: Arc<Analysis>,
    validation: &'a super::super::opaque_physical::PhysicalValidation<'a>,
    _reservation: Arc<super::super::ir::RetainedStorage>,
}
impl<'a> PhysicalConcrete<'a> {
    pub(in crate::compiler) fn function(&self) -> &Arc<cfg::Function> {
        &self.function
    }
    pub(in crate::compiler) fn facts(&self) -> &Arc<ValidatedFacts> {
        &self.facts
    }
    pub(in crate::compiler) fn validate(&self) -> Result<(), String> {
        if !self.analysis.passed() {
            return Err("physical native proof has failed ownership analysis".into());
        }
        self.function
            .validate_physical(&self.facts, self.validation)
    }
}

#[allow(clippy::result_large_err)]
pub(in crate::compiler) fn analyze_physical<'a>(
    function: Arc<cfg::Function>,
    facts: Arc<ValidatedFacts>,
    validation: &'a super::super::opaque_physical::PhysicalValidation<'a>,
) -> Result<PhysicalConcrete<'a>, Analysis> {
    if let Err(message) = function.validate_physical(&facts, validation) {
        return Err(invalid_facts_analysis(&function, &message, facts.budget()));
    }
    let mut effects = None;
    let analysis = analyze_with_context(
        &function,
        Limits::default(),
        FactsCtx::Validated(&facts),
        Some(facts.budget()),
        (Some(&function), Some(&facts)),
        None,
        &mut effects,
        Some(validation),
    );
    if !analysis.passed() {
        return Err(analysis);
    }
    // A missing carrier proof is not accepted merely because the public opaque
    // family has been normalized away. The real physical borrow plan must have
    // completed its declaration and occurrence-bound carrier checks.
    if effects.is_none() {
        return Err(invalid_facts_analysis(
            &function,
            "physical ownership has no complete carrier proof",
            facts.budget(),
        ));
    }
    let (analysis, reservation) = bind_proof(
        analysis,
        &function,
        facts.budget(),
        std::mem::size_of::<PhysicalConcrete<'_>>(),
    )?;
    Ok(PhysicalConcrete {
        function,
        facts,
        analysis,
        validation,
        _reservation: reservation,
    })
}
