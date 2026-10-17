//! Reachable-CFG storage and definite-initialization analysis.
//!
//! Includes bounded B1 local-reference checking, not general Rust borrowing or
//! Drop elaboration. Private `Move` operands consume even Copy-typed places so that
//! move-path analysis can be tested before non-Copy syntax is supported.
use super::borrow::{
    self, Access, CanonicalPlace, CanonicalRoot, Phase, Problem, Target, TargetPrecision,
};
use super::cfg::{self, BlockId, Operand, Rvalue, Statement, Terminator};
#[cfg(test)]
use super::ir::ReferencePosition;
use super::ir::{CheckedFunction, ExprId, FactsCtx, GenericEnv, LocalId, Place, Projection, Ty};
use super::traits::{FactsError, FactsPhase, ValidatedFacts};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct AnalysisStats {
    pub blocks_total: usize,
    pub blocks_reachable: usize,
    pub edges_reachable: usize,
    pub locals: usize,
    pub move_paths: usize,
    pub worklist_pops: usize,
    pub state_changes: usize,
    /// Operand evaluations in the final diagnostic pass, including constants.
    pub operands_read: usize,
    pub moves: usize,
    pub writes: usize,
    pub storage_events: usize,
    pub diagnostics_suppressed: usize,
    /// Downcast authorizations in final diagnostic replay, including singleton proofs.
    pub variant_proof_checks: usize,
    /// Reachable SwitchVariant true edges, independent of worklist iteration order.
    pub variant_refinements: usize,
    /// Known descendant proofs invalidated in final diagnostic replay.
    pub variant_proofs_killed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidCfg(String),
    StorageAlreadyLive,
    StorageNotDefinitelyLive,
    ReadNotDefinitelyInitialized,
    VariantNotDefinitelySelected,
    WriteIntoUninitializedAggregate,
    ImmutableReassignment,
    BorrowConflict(String),
    InvalidReference(String),
    UnsupportedBorrowing(String),
    ResourceLimit,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: ErrorKind,
    pub block: BlockId,
    /// None identifies the block terminator.
    pub statement: Option<usize>,
    pub origin: Option<ExprId>,
    pub place: Option<Place>,
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub stats: AnalysisStats,
    pub diagnostics: Box<[Diagnostic]>,
    /// False means resource exhaustion or truncated diagnostics, never success.
    pub complete: bool,
}
impl Analysis {
    pub fn passed(&self) -> bool {
        self.complete && self.diagnostics.is_empty()
    }
}

/// Successful universal analysis of the exact checked source and its lowering.
/// There is deliberately no constructor accepting a caller-provided proof.
#[derive(Clone, Debug)]
pub struct VerifiedTemplateCfg {
    template: cfg::TemplateCfg,
    analysis: Arc<Analysis>,
    carrier_effects: Option<Arc<CarrierEffectProof>>,
    _reservation: Arc<super::ir::PlanReservation>,
}
impl VerifiedTemplateCfg {
    pub fn source(&self) -> &Arc<CheckedFunction> {
        self.template.source()
    }
    pub fn facts(&self) -> &Arc<ValidatedFacts> {
        self.template.facts()
    }
    pub fn env(&self) -> &Arc<GenericEnv> {
        self.facts().env()
    }
    pub fn function(&self) -> &Arc<cfg::Function> {
        self.template.function()
    }
    pub fn analysis(&self) -> &Arc<Analysis> {
        &self.analysis
    }
    pub fn budget(&self) -> &super::ir::PlanBudget {
        self.facts().budget()
    }
    pub(super) fn carrier_effects(&self) -> Option<&Arc<CarrierEffectProof>> {
        self.carrier_effects.as_ref()
    }
}

/// A successful SAME-Plan family/effect proof. It deliberately retains the
/// original immutable inputs, not the wrapper which owns this proof.
#[derive(Clone, Debug)]
pub(super) struct CarrierEffectProof {
    source: Arc<CheckedFunction>,
    function: Arc<cfg::Function>,
    facts: Arc<ValidatedFacts>,
    analysis: Arc<Analysis>,
    effects: Arc<borrow::CarrierEffects>,
}

/// Published only by the root's acyclic universal-effects join. Absence is
/// conservative declaration authority, never a successful family proof.
#[derive(Clone, Debug)]
pub(super) struct CarrierEffectTable {
    templates: Arc<[VerifiedTemplateCfg]>,
    budget: super::ir::PlanBudget,
}
impl CarrierEffectTable {
    pub(super) fn lookup(
        &self,
        target: &super::ir::Callee,
        caller: &cfg::Function,
        facts: &ValidatedFacts,
        view: &super::traits::CarrierContractView,
    ) -> Result<Option<Arc<borrow::CarrierEffects>>, super::traits::FactsError> {
        let budget = facts.budget();
        if !budget.same_authority(&self.budget) || facts.phase() != FactsPhase::Closed {
            return Err(super::traits::FactsError::invalid(
                budget,
                "effect lookup changed closed planning authority",
            ));
        }
        caller
            .validate_provenance(facts, budget)
            .map_err(|message| super::traits::FactsError::from_message(budget, &message))?;
        view.validate_for(facts)?;
        for template in self.templates.iter() {
            budget
                .charge_work(1)
                .map_err(|message| super::traits::FactsError::resource(budget, &message))?;
            if !Arc::ptr_eq(template.facts().world(), facts.world()) {
                return Err(super::traits::FactsError::invalid(
                    budget,
                    "effect lookup mixed canonical worlds",
                ));
            }
        }
        let super::ir::Callee::Direct(target) = target else {
            return Ok(None);
        };
        if target.declaration != view.declaration()
            || !facts
                .world()
                .types_equal(&target.type_args, view.arguments())?
        {
            return Err(super::traits::FactsError::invalid(
                budget,
                "effect lookup selected a different canonical declaration",
            ));
        }
        for template in self.templates.iter() {
            budget
                .charge_work(1)
                .map_err(|message| super::traits::FactsError::resource(budget, &message))?;
            if template.facts().declaration() != target.declaration {
                continue;
            }
            if !Arc::ptr_eq(template.env(), view.original_env()) {
                return Err(super::traits::FactsError::invalid(
                    budget,
                    "effect lookup changed original binder environment",
                ));
            }
            let Some(proof) = template.carrier_effects() else {
                return Ok(None);
            };
            if !Arc::ptr_eq(&proof.source, template.source())
                || !Arc::ptr_eq(&proof.function, template.function())
                || !Arc::ptr_eq(&proof.analysis, template.analysis())
                || !Arc::ptr_eq(&proof.facts, template.facts())
                || !proof.analysis.passed()
                || !template.budget().same_authority(budget)
            {
                return Err(super::traits::FactsError::invalid(
                    budget,
                    "effect lookup lost exact universal body proof",
                ));
            }
            return Ok(Some(proof.effects.clone()));
        }
        Ok(None)
    }
    pub(super) fn from_verified(
        templates: &[VerifiedTemplateCfg],
        budget: &super::ir::PlanBudget,
    ) -> Result<Arc<Self>, String> {
        // Both the table and its retained slice have independent Arc control
        // blocks, including the empty-table case. The caller's source slice
        // remains live throughout this allocation.
        budget.charge_nodes(2)?;
        budget.charge_work(templates.len())?;
        let bytes = templates
            .len()
            .checked_mul(std::mem::size_of::<VerifiedTemplateCfg>())
            .and_then(|bytes| {
                bytes.checked_add(std::mem::size_of::<Self>() + 4 * std::mem::size_of::<usize>())
            })
            .ok_or("carrier effect table size overflow")?;
        budget.retain_bytes(bytes)?;
        for (index, template) in templates.iter().enumerate() {
            if !template.budget().same_authority(budget)
                || template.facts().phase() != FactsPhase::Universal
                || !template.analysis().passed()
            {
                return Err(
                    "carrier effect table contains foreign or unsuccessful template".into(),
                );
            }
            budget.charge_work(index)?;
            for previous in &templates[..index] {
                if !Arc::ptr_eq(previous.facts().world(), template.facts().world()) {
                    return Err("carrier effect table mixes canonical worlds".into());
                }
                if previous.facts().declaration() == template.facts().declaration()
                    && (!Arc::ptr_eq(previous.source(), template.source())
                        || !Arc::ptr_eq(previous.function(), template.function())
                        || !Arc::ptr_eq(previous.analysis(), template.analysis())
                        || !Arc::ptr_eq(previous.facts(), template.facts())
                        || !match (previous.carrier_effects(), template.carrier_effects()) {
                            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                            (None, None) => true,
                            _ => false,
                        })
                {
                    return Err(
                        "carrier effect table has conflicting provider proof identities".into(),
                    );
                }
            }
            if let Some(proof) = template.carrier_effects()
                && (!Arc::ptr_eq(&proof.source, template.source())
                    || !Arc::ptr_eq(&proof.function, template.function())
                    || !Arc::ptr_eq(&proof.facts, template.facts())
                    || !Arc::ptr_eq(&proof.analysis, template.analysis()))
            {
                return Err("carrier effect proof changed its exact verified inputs".into());
            }
        }
        // Universal analysis assumes declaration envelopes, never another
        // body's summary. Once every universal proof exists, independently
        // check new ordinary reference-actual calls even in unused/dead source
        // metadata. A reference-free-only callee proof is not sufficient.
        for caller in templates {
            for expression in &caller.source().exprs {
                budget.charge_work(1)?;
                let super::ir::ExprKind::Call { target, .. } = &expression.kind else {
                    continue;
                };
                let actuals = match target {
                    super::ir::Callee::Direct(target) => target.type_args.as_ref(),
                    super::ir::Callee::TraitItem(call) => call.method_args.as_ref(),
                };
                budget.charge_work(actuals.len())?;
                if !actuals.iter().any(|ty| matches!(ty, Ty::Ref { .. })) {
                    continue;
                }
                let super::ir::Callee::Direct(target) = target else {
                    return Err(
                        "unsupported abstract method reference-actual body admission".into(),
                    );
                };
                budget.charge_work(templates.len())?;
                let callee = templates
                    .iter()
                    .find(|candidate| candidate.facts().declaration() == target.declaration)
                    .ok_or("reference-actual call has no canonical universal body")?;
                if callee.carrier_effects().is_none() {
                    return Err("unsupported reference-actual call requires its callee's complete universal family proof".into());
                }
            }
        }
        Ok(Arc::new(Self {
            templates: Arc::from(templates),
            budget: budget.clone(),
        }))
    }
}

/// Only this successful, exact closed-CFG proof permits native work.
#[derive(Clone, Debug)]
pub struct ConcreteCfg {
    closed: cfg::ClosedCfg,
    analysis: Arc<Analysis>,
    _reservation: Arc<super::ir::PlanReservation>,
}
impl ConcreteCfg {
    pub fn facts(&self) -> &Arc<ValidatedFacts> {
        self.closed.facts()
    }
    pub fn function(&self) -> &Arc<cfg::Function> {
        self.closed.function()
    }
    pub fn analysis(&self) -> &Arc<Analysis> {
        &self.analysis
    }
    pub fn budget(&self) -> &super::ir::PlanBudget {
        self.facts().budget()
    }
}

/// Derive the compatibility boundary from the original checked source, including
/// dead arena metadata. Demanded instance arguments cannot opt a body into the
/// old reference-free-only fallback.
fn type_requires_carrier_proof(
    ty: &Ty,
    budget: &super::ir::PlanBudget,
) -> Result<bool, FactsError> {
    let classify = |message: String| FactsError::from_message(budget, &message);
    let mut cursor = ty;
    let mut depth = 0usize;
    let mut mutable = false;
    while let Ty::Ref { kind, referent } = cursor {
        budget.check_depth(depth).map_err(classify)?;
        budget.charge_work(1).map_err(classify)?;
        depth = depth
            .checked_add(1)
            .ok_or_else(|| FactsError::resource(budget, "original carrier depth overflow"))?;
        mutable |= *kind == super::ir::BorrowKind::Mut;
        cursor = referent;
    }
    let locations = super::ir::carrier_locations(ty, budget).map_err(classify)?;
    budget.charge_work(locations.len()).map_err(classify)?;
    Ok(depth > 2
        || (depth > 1 && mutable)
        || locations.iter().any(|location| {
            matches!(location.channel, super::ir::CarrierChannel::Fixed(_))
                && !location.path.0.is_empty()
        }))
}

fn source_requires_carrier_proof(
    source: &CheckedFunction,
    budget: &super::ir::PlanBudget,
) -> Result<bool, FactsError> {
    let classify = |message: String| FactsError::from_message(budget, &message);
    let mut required = false;
    for expression in &source.exprs {
        budget.charge_work(1).map_err(classify)?;
        if let super::ir::ExprKind::Call {
            target, signature, ..
        } = &expression.kind
        {
            let actuals = match target {
                super::ir::Callee::Direct(target) => target.type_args.as_ref(),
                super::ir::Callee::TraitItem(call) => call.method_args.as_ref(),
            };
            budget.charge_work(actuals.len()).map_err(classify)?;
            required |= actuals.iter().any(|ty| matches!(ty, Ty::Ref { .. }));
            for ty in signature
                .params
                .iter()
                .chain(std::iter::once(&signature.result))
            {
                required |= type_requires_carrier_proof(ty, budget)?;
            }
        }
    }
    for ty in std::iter::once(&source.result)
        .chain(source.locals.iter().map(|local| &local.ty))
        .chain(source.exprs.iter().map(|expression| &expression.ty))
    {
        required |= type_requires_carrier_proof(ty, budget)?;
    }
    Ok(required)
}

fn closed_requires_carrier_proof(closed: &cfg::ClosedCfg) -> Result<bool, FactsError> {
    if let Some(proof) = closed.body_substitution() {
        return source_requires_carrier_proof(proof.template().source(), closed.budget());
    }
    if let Some(evidence) = closed.call_evidence().first() {
        return source_requires_carrier_proof(evidence.source(), closed.budget());
    }
    // No substitution or source call capsules: this is an original nongeneric
    // body. Its complete typed local arena is unchanged by specialization.
    let mut required = false;
    for ty in std::iter::once(&closed.function().result)
        .chain(closed.function().locals.iter().map(|local| &local.ty))
    {
        required |= type_requires_carrier_proof(ty, closed.budget())?;
    }
    Ok(required)
}

// Failed verification deliberately returns the full existing report, not a
// lossy error summary or a second allocation on the failure path.
#[allow(clippy::result_large_err)]
pub fn verify_template(template: cfg::TemplateCfg) -> Result<VerifiedTemplateCfg, Analysis> {
    if template.facts().phase() != FactsPhase::Universal {
        return Err(invalid_facts_analysis(
            template.function(),
            "template ownership requires universal facts",
            template.facts().budget(),
        ));
    }
    let mut candidate_effects = None;
    let analysis = analyze_with_context(
        template.function(),
        Limits::default(),
        FactsCtx::Validated(template.facts()),
        Some(template.budget()),
        (Some(template.function()), Some(template.facts())),
        None,
        &mut candidate_effects,
    );
    if !analysis.passed() {
        return Err(analysis);
    }
    match source_requires_carrier_proof(template.source(), template.budget()) {
        Ok(true) if candidate_effects.is_none() => {
            return Err(facts_error_analysis(
                template.function(),
                FactsError::unsupported(
                    template.budget(),
                    "unsupported original carrier body requires a complete universal family proof",
                ),
                template.budget(),
            ));
        }
        Err(error) => {
            return Err(facts_error_analysis(
                template.function(),
                error,
                template.budget(),
            ));
        }
        _ => {}
    }
    if candidate_effects.is_some() {
        let proof_storage =
            std::mem::size_of::<CarrierEffectProof>() + 2 * std::mem::size_of::<usize>();
        if let Err(message) = template
            .budget()
            .charge_nodes(1)
            .and_then(|()| template.budget().retain_bytes(proof_storage))
        {
            return Err(facts_error_analysis(
                template.function(),
                FactsError::resource(template.budget(), &message),
                template.budget(),
            ));
        }
    }
    let (analysis, reservation) = bind_proof(
        analysis,
        template.function(),
        template.budget(),
        std::mem::size_of::<VerifiedTemplateCfg>(),
    )?;
    // The candidate is produced by the SAME Plan, but only a complete,
    // successful analysis above may turn it into executable family authority.
    // Retain original immutable inputs, never this owning template wrapper.
    let carrier_effects = candidate_effects.map(|effects| {
        Arc::new(CarrierEffectProof {
            source: template.source().clone(),
            function: template.function().clone(),
            facts: template.facts().clone(),
            analysis: analysis.clone(),
            effects,
        })
    });
    Ok(VerifiedTemplateCfg {
        template,
        analysis,
        carrier_effects,
        _reservation: reservation,
    })
}

#[allow(clippy::result_large_err)]
pub(super) fn analyze_closed_with_effects(
    closed: cfg::ClosedCfg,
    effects: &Arc<CarrierEffectTable>,
) -> Result<ConcreteCfg, Analysis> {
    if !closed.budget().same_authority(&effects.budget) {
        return Err(invalid_facts_analysis(
            closed.function(),
            "carrier effects changed planning authority",
            closed.budget(),
        ));
    }
    // The table is not permission to omit the ordinary closed analysis. The
    // SAME Plan consumes only exact, occurrence-bound entries during transfer.
    analyze_closed_context(closed, Some(effects))
}

#[allow(clippy::result_large_err)]
pub fn analyze_closed(closed: cfg::ClosedCfg) -> Result<ConcreteCfg, Analysis> {
    analyze_closed_context(closed, None)
}
#[allow(clippy::result_large_err)]
fn analyze_closed_context(
    closed: cfg::ClosedCfg,
    effects: Option<&Arc<CarrierEffectTable>>,
) -> Result<ConcreteCfg, Analysis> {
    if closed.facts().phase() != FactsPhase::Closed {
        return Err(invalid_facts_analysis(
            closed.function(),
            "concrete ownership requires closed facts",
            closed.facts().budget(),
        ));
    }
    let mut candidate_effects = None;
    let analysis = analyze_with_context(
        closed.function(),
        Limits::default(),
        FactsCtx::Validated(closed.facts()),
        Some(closed.budget()),
        (Some(closed.function()), Some(closed.facts())),
        effects,
        &mut candidate_effects,
    );
    if !analysis.passed() {
        return Err(analysis);
    }
    match closed_requires_carrier_proof(&closed) {
        Ok(true) if candidate_effects.is_none() => {
            return Err(facts_error_analysis(
                closed.function(),
                FactsError::unsupported(
                    closed.budget(),
                    "unsupported original carrier body requires a complete closed family proof",
                ),
                closed.budget(),
            ));
        }
        Err(error) => {
            return Err(facts_error_analysis(
                closed.function(),
                error,
                closed.budget(),
            ));
        }
        _ => {}
    }
    let (analysis, reservation) = bind_proof(
        analysis,
        closed.function(),
        closed.budget(),
        std::mem::size_of::<ConcreteCfg>(),
    )?;
    Ok(ConcreteCfg {
        closed,
        analysis,
        _reservation: reservation,
    })
}

fn invalid_facts_analysis(
    function: &cfg::Function,
    message: &str,
    budget: &super::ir::PlanBudget,
) -> Analysis {
    let error = super::traits::FactsError::invalid(budget, message);
    facts_error_analysis(function, error, budget)
}

fn facts_error_analysis(
    function: &cfg::Function,
    error: FactsError,
    budget: &super::ir::PlanBudget,
) -> Analysis {
    let category = error.kind();
    let kind = if error.is_resource() {
        ErrorKind::ResourceLimit
    } else {
        match error.into_diagnostic(budget) {
            Ok(message) if category == super::traits::FactsErrorKind::Unsupported => {
                ErrorKind::UnsupportedBorrowing(message)
            }
            Ok(message) => ErrorKind::InvalidCfg(message),
            Err(_) => ErrorKind::ResourceLimit,
        }
    };
    let complete = kind != ErrorKind::ResourceLimit;
    Analysis {
        stats: AnalysisStats {
            blocks_total: function.blocks.len(),
            locals: function.locals.len(),
            ..AnalysisStats::default()
        },
        diagnostics: vec![Diagnostic {
            kind,
            block: function.entry,
            statement: None,
            origin: None,
            place: None,
        }]
        .into_boxed_slice(),
        complete,
    }
}

#[allow(clippy::result_large_err)]
fn bind_proof(
    mut analysis: Analysis,
    function: &cfg::Function,
    budget: &super::ir::PlanBudget,
    wrapper_bytes: usize,
) -> Result<(Arc<Analysis>, Arc<super::ir::PlanReservation>), Analysis> {
    let reservation = budget.charge_nodes(1).and_then(|()| {
        budget.reserve_bytes(
            wrapper_bytes
                + std::mem::size_of::<Analysis>()
                + std::mem::size_of::<super::ir::PlanReservation>()
                + 128,
        )
    });
    match reservation {
        Ok(reservation) => Ok((Arc::new(analysis), Arc::new(reservation))),
        Err(_) => {
            analysis.complete = false;
            analysis.diagnostics = vec![Diagnostic {
                kind: ErrorKind::ResourceLimit,
                block: function.entry,
                statement: None,
                origin: None,
                place: None,
            }]
            .into_boxed_slice();
            Err(analysis)
        }
    }
}

#[derive(Clone, Copy)]
struct Limits {
    blocks: usize,
    locals: usize,
    paths: usize,
    operations: usize,
    state_bytes: usize,
    work: usize,
    diagnostics: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            blocks: 16_384,
            locals: 65_536,
            paths: 131_072,
            operations: 1_000_000,
            state_bytes: 128 * 1024 * 1024,
            work: 32 * 1024 * 1024,
            diagnostics: 256,
        }
    }
}
/// Data from the same structural recipe events that emit checked IR. Counts are
/// conservative upper bounds, not callable facts, evidence, or Copy permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RecipePlanCounts {
    pub binders: usize,
    pub expressions: usize,
    pub locals: usize,
    pub blocks: usize,
    pub points: usize,
    pub carrier_slots: usize,
    pub loan_sites: usize,
    pub formal_origins: usize,
    pub call_result_origins: usize,
    pub path_nodes: usize,
    pub variant_offsets: usize,
    pub operations: usize,
    pub origin_memberships: usize,
    pub parent_memberships: usize,
    pub lifetime_dependency_memberships: usize,
    pub target_memberships: usize,
    /// Allocation attempts include candidates later deduplicated by the solver.
    pub target_allocations: usize,
    pub target_projection_steps: usize,
    pub target_authority_entries: usize,
    /// Formal targets use two overhead words, derived targets use three.
    pub target_overhead_words: usize,
    pub ir_retained_bytes: usize,
    pub cfg_retained_bytes: usize,
    /// Additional construction scratch/relocation coexisting with retained IR.
    pub construction_peak_bytes: usize,
    pub verification_scratch_bytes: usize,
    pub work_upper_bound: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RecipeFootprint {
    pub origins: usize,
    pub memberships: usize,
    pub ownership_bytes: usize,
    pub borrow_bytes: usize,
    pub path_bytes: usize,
    pub state_bytes: usize,
    pub peak_bytes: usize,
    pub work: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecipeLimit {
    Overflow,
    Bound {
        resource: &'static str,
        required: usize,
        limit: usize,
    },
}
impl RecipeLimit {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Overflow => "recipe footprint count arithmetic overflow",
            Self::Bound { resource, .. } => resource,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecipeAdmission {
    Available(RecipeFootprint),
    IntrinsicLimit(RecipeLimit),
}

fn ownership_state_bytes(locals: usize, paths: usize, blocks: usize) -> Result<usize, Exhausted> {
    let per_state = locals
        .div_ceil(64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(paths.div_ceil(64).checked_mul(3)?))
        .and_then(|n| n.checked_add(paths))
        .ok_or(Exhausted)?
        .checked_mul(8)
        .and_then(|n| n.checked_add(std::mem::size_of::<State>()))
        .ok_or(Exhausted)?;
    per_state
        .checked_mul(blocks.checked_add(3).ok_or(Exhausted)?)
        .and_then(|n| n.checked_add(paths.div_ceil(64).checked_mul(8)?))
        .and_then(|n| n.checked_add(std::mem::size_of::<Bits>()))
        .ok_or(Exhausted)
}

fn paths_storage_bytes(locals: usize, nodes: usize, variants: usize) -> Result<usize, Exhausted> {
    nodes
        .checked_mul(4 * std::mem::size_of::<PathNode>() + 4 * std::mem::size_of::<usize>())
        .and_then(|n| n.checked_add(variants.checked_mul(4 * std::mem::size_of::<usize>())?))
        .and_then(|n| {
            n.checked_add(locals.checked_mul(
                4 * std::mem::size_of::<usize>() + std::mem::size_of::<[Option<usize>; 2]>(),
            )?)
        })
        .ok_or(Exhausted)
}

/// Absolute descriptor admission is independent of the ledger's remaining
/// capacity. Charging this calculation can still exhaust that SAME ledger, which
/// is an error for the whole plan, never a silently unavailable descriptor.
pub(super) fn preflight_recipe_counts(
    counts: &RecipePlanCounts,
    budget: &super::ir::PlanBudget,
) -> Result<RecipeAdmission, FactsError> {
    budget.charge_work(128).map_err(|_| {
        FactsError::resource(budget, "recipe footprint preflight resource limit exceeded")
    })?;
    let limits = Limits::default();
    let plan = super::ir::PlanLimits::default();
    let bound = |resource, required, limit| {
        RecipeAdmission::IntrinsicLimit(RecipeLimit::Bound {
            resource,
            required,
            limit,
        })
    };
    for (name, required, limit) in [
        ("recipe binder limit exceeded", counts.binders, plan.params),
        (
            "recipe expression limit exceeded",
            counts.expressions,
            plan.nodes,
        ),
        ("recipe local limit exceeded", counts.locals, limits.locals),
        ("recipe block limit exceeded", counts.blocks, limits.blocks),
        (
            "recipe move-path limit exceeded",
            counts.path_nodes,
            limits.paths,
        ),
        (
            "recipe operation limit exceeded",
            counts.operations,
            limits.operations,
        ),
        (
            "recipe work limit exceeded",
            counts.work_upper_bound,
            plan.work.min(limits.work),
        ),
    ] {
        if required > limit {
            return Ok(bound(name, required, limit));
        }
    }
    if counts.points < counts.blocks {
        return Err(FactsError::invalid(
            budget,
            "recipe count model omits terminator points",
        ));
    }
    let calculated = (|| -> Result<RecipeFootprint, Exhausted> {
        let origins = counts
            .loan_sites
            .checked_add(counts.formal_origins)
            .and_then(|n| n.checked_add(counts.call_result_origins))
            .ok_or(Exhausted)?;
        let memberships = counts
            .origin_memberships
            .checked_add(counts.parent_memberships)
            .and_then(|n| n.checked_add(counts.lifetime_dependency_memberships))
            .and_then(|n| n.checked_add(counts.target_memberships))
            .ok_or(Exhausted)?;
        let ownership_bytes =
            ownership_state_bytes(counts.locals, counts.path_nodes, counts.blocks)?;
        let mut borrow_bytes = if origins != 0 || counts.carrier_slots != 0 {
            borrow::plan_storage_bytes(
                counts.locals,
                counts.blocks,
                counts.points,
                counts.carrier_slots,
                origins,
            )?
            .checked_add(borrow::target_payload_bytes(
                counts.target_allocations,
                counts.target_projection_steps,
                counts.target_authority_entries,
                counts.target_overhead_words,
            )?)
            .ok_or(Exhausted)?
        } else {
            0
        };
        // The same dynamic occurrence graph kernel pays for the retained body
        // graph and the simultaneously live rigid/occurrence copies per call.
        // Recipe events bound each call by the original carrier and binder rows.
        let region_nodes = counts
            .formal_origins
            .checked_add(counts.binders)
            .and_then(|n| n.checked_add(2))
            .ok_or(Exhausted)?;
        let region_copies = counts
            .call_result_origins
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(Exhausted)?;
        borrow_bytes = borrow_bytes
            .checked_add(
                borrow::region_storage_bytes(region_nodes)?
                    .checked_mul(region_copies)
                    .ok_or(Exhausted)?,
            )
            .ok_or(Exhausted)?;
        let path_bytes =
            paths_storage_bytes(counts.locals, counts.path_nodes, counts.variant_offsets)?;
        let state_bytes = ownership_bytes.checked_add(borrow_bytes).ok_or(Exhausted)?;
        let retained = counts
            .ir_retained_bytes
            .checked_add(counts.cfg_retained_bytes)
            .ok_or(Exhausted)?;
        let analysis_peak = path_bytes.checked_add(state_bytes).ok_or(Exhausted)?;
        let peak_bytes = retained
            .checked_add(
                analysis_peak
                    .max(counts.verification_scratch_bytes)
                    .max(counts.construction_peak_bytes),
            )
            .ok_or(Exhausted)?;
        let work = counts.work_upper_bound.max(borrow::plan_initial_work(
            counts.blocks,
            counts.points,
            counts.carrier_slots,
            origins,
        )?);
        Ok(RecipeFootprint {
            origins,
            memberships,
            ownership_bytes,
            borrow_bytes,
            path_bytes,
            state_bytes,
            peak_bytes,
            work,
        })
    })();
    let Ok(footprint) = calculated else {
        return Ok(RecipeAdmission::IntrinsicLimit(RecipeLimit::Overflow));
    };
    for (name, required, limit) in [
        (
            "recipe combined-origin limit exceeded",
            footprint.origins,
            borrow::MAX_LOANS,
        ),
        (
            "recipe membership limit exceeded",
            footprint.memberships,
            borrow::MAX_MEMBERSHIPS,
        ),
        (
            "recipe ownership/borrow state limit exceeded",
            footprint.state_bytes,
            limits.state_bytes,
        ),
        (
            "recipe retained peak byte limit exceeded",
            footprint.peak_bytes,
            plan.bytes,
        ),
        (
            "recipe work limit exceeded",
            footprint.work,
            plan.work.min(limits.work),
        ),
    ] {
        if required > limit {
            return Ok(bound(name, required, limit));
        }
    }
    Ok(RecipeAdmission::Available(footprint))
}

#[derive(Debug)]
pub(super) struct Exhausted;
pub(super) struct Budget {
    remaining: usize,
    work_limit: usize,
    plan: Option<super::ir::PlanBudget>,
    reservations: Vec<super::ir::PlanReservation>,
    peak_scratch: usize,
    reserved_bytes: usize,
    state_limit: usize,
}
impl Budget {
    #[cfg(test)]
    fn new(remaining: usize) -> Self {
        Self {
            remaining,
            work_limit: remaining,
            plan: None,
            reservations: Vec::new(),
            peak_scratch: 0,
            reserved_bytes: 0,
            state_limit: 128 * 1024 * 1024,
        }
    }
    fn for_plan(remaining: usize, plan: Option<&super::ir::PlanBudget>) -> Self {
        Self {
            remaining,
            work_limit: remaining,
            plan: plan.cloned(),
            reservations: Vec::new(),
            peak_scratch: 0,
            reserved_bytes: 0,
            state_limit: 128 * 1024 * 1024,
        }
    }
    #[track_caller]
    fn exhausted(
        &self,
        kind: super::ir::ResourceBudgetKind,
        requested: usize,
        used: usize,
        limit: usize,
    ) -> Exhausted {
        if let Some(plan) = &self.plan {
            plan.record_resource_failure(super::ir::ResourceBudgetFailure::new(
                kind, requested, used, limit,
            ));
        }
        Exhausted
    }
    #[track_caller]
    pub(super) fn spend(&mut self, amount: usize) -> Result<(), Exhausted> {
        let Some(remaining) = self.remaining.checked_sub(amount) else {
            return Err(self.exhausted(
                super::ir::ResourceBudgetKind::OwnershipWork,
                amount,
                self.work_limit - self.remaining,
                self.work_limit,
            ));
        };
        if let Some(plan) = &self.plan {
            plan.charge_work(amount).map_err(|_| Exhausted)?;
        }
        self.remaining = remaining;
        Ok(())
    }
    pub(super) fn plan_budget(&self) -> Option<&super::ir::PlanBudget> {
        self.plan.as_ref()
    }
    pub(super) fn reserved_bytes(&self) -> usize {
        self.reserved_bytes
    }
    #[track_caller]
    pub(super) fn reserve_bytes(&mut self, amount: usize) -> Result<(), Exhausted> {
        let growth =
            if self.plan.is_some() && self.reservations.len() == self.reservations.capacity() {
                Some(
                    self.reservations
                        .capacity()
                        .checked_mul(2)
                        .ok_or(Exhausted)?
                        .max(4),
                )
            } else {
                None
            };
        let overhead = match growth {
            Some(capacity) => capacity
                .checked_add(self.reservations.capacity())
                .and_then(|words| {
                    words.checked_mul(std::mem::size_of::<super::ir::PlanReservation>())
                })
                .ok_or(Exhausted)?,
            None => 0,
        };
        let Some(amount) = amount.checked_add(overhead) else {
            return Err(self.exhausted(
                super::ir::ResourceBudgetKind::OwnershipReservationStorage,
                overhead,
                amount,
                self.state_limit,
            ));
        };
        let Some(next) = self.reserved_bytes.checked_add(amount) else {
            return Err(self.exhausted(
                super::ir::ResourceBudgetKind::OwnershipStateBytes,
                amount,
                self.reserved_bytes,
                self.state_limit,
            ));
        };
        if next > self.state_limit {
            return Err(self.exhausted(
                super::ir::ResourceBudgetKind::OwnershipStateBytes,
                amount,
                self.reserved_bytes,
                self.state_limit,
            ));
        }
        if growth.is_some() {
            self.spend(self.reservations.len().checked_add(1).ok_or(Exhausted)?)?;
            self.nodes(1)?;
        }
        if let Some(plan) = &self.plan {
            let reservation = plan.reserve_bytes(amount).map_err(|_| Exhausted)?;
            if let Some(capacity) = growth {
                self.reservations
                    .try_reserve_exact(
                        capacity
                            .checked_sub(self.reservations.len())
                            .ok_or(Exhausted)?,
                    )
                    .map_err(|_| Exhausted)?;
            }
            self.reservations.push(reservation);
        }
        self.reserved_bytes = next;
        Ok(())
    }
    #[track_caller]
    pub(super) fn nodes(&mut self, count: usize) -> Result<(), Exhausted> {
        if let Some(plan) = &self.plan {
            plan.charge_nodes(count).map_err(|_| Exhausted)?;
        }
        Ok(())
    }
    #[track_caller]
    fn scratch(&mut self, amount: usize) -> Result<Option<super::ir::PlanReservation>, Exhausted> {
        // Retain scratch accounting conservatively for this analysis. This also
        // covers overlapping reservations without a second untracked allocator.
        self.reserve_bytes(amount)?;
        self.peak_scratch = self.peak_scratch.max(amount);
        Ok(None)
    }
}

/// Bound type traversal before calling the full structural verifier, including
/// repeated shared aggregate types that could otherwise expand exponentially.
#[cfg(test)]
fn preflight(f: &cfg::Function, limits: Limits) -> Result<(), Exhausted> {
    preflight_with_plan(f, limits, None, FactsCtx::Concrete).map(|_| ())
}
fn preflight_with_plan(
    f: &cfg::Function,
    limits: Limits,
    plan: Option<&super::ir::PlanBudget>,
    facts: FactsCtx<'_>,
) -> Result<usize, Exhausted> {
    fn contract_scratch(
        type_work: usize,
        arity: usize,
        loan_sites: usize,
        max_bytes: usize,
        contract: &super::ir::RefContract,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
    ) -> Result<Option<super::ir::PlanReservation>, Exhausted> {
        let mut candidate_count = 0usize;
        let mut envelope_count = 0usize;
        let mut steps = 0usize;
        let result_limit = if matches!(facts, FactsCtx::Validated(_)) {
            4096
        } else {
            2
        };
        if contract.results.len() > result_limit || contract.input_regions.len() > 4096 {
            return Err(Exhausted);
        }
        budget.spend(contract.results.len() + contract.input_regions.len() + 1)?;
        for result in contract.results.iter() {
            let (candidates, envelopes): (&[super::ir::InputPath], &[super::ir::InputEnvelope]) =
                match &result.returns {
                    super::ir::ReturnContract::NoReference => (&[], &[]),
                    super::ir::ReturnContract::SharedInputPaths(paths)
                    | super::ir::ReturnContract::MutInputPaths(paths) => (paths, &[]),
                    super::ir::ReturnContract::SharedInputEnvelopes(inputs)
                    | super::ir::ReturnContract::MutInputEnvelopes(inputs) => (&[], inputs),
                };
            candidate_count = candidate_count
                .checked_add(candidates.len())
                .ok_or(Exhausted)?;
            envelope_count = envelope_count
                .checked_add(envelopes.len())
                .ok_or(Exhausted)?;
            if candidate_count
                .checked_add(envelope_count)
                .ok_or(Exhausted)?
                > 4096
            {
                return Err(Exhausted);
            }
            budget.spend(candidates.len() + envelopes.len() + 1)?;
            for candidate in candidates {
                if candidate.projection.len() > 128 {
                    return Err(Exhausted);
                }
                steps = steps
                    .checked_add(candidate.projection.len())
                    .ok_or(Exhausted)?;
                if steps > 131072 {
                    return Err(Exhausted);
                }
            }
        }
        budget.spend(steps)?;
        let provided_bytes = candidate_count
            .checked_mul(std::mem::size_of::<super::ir::InputPath>())
            .and_then(|n| n.checked_add(steps.checked_mul(std::mem::size_of::<Projection>())?))
            .and_then(|n| {
                n.checked_add(
                    envelope_count.checked_mul(std::mem::size_of::<super::ir::InputEnvelope>())?,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    contract
                        .input_regions
                        .len()
                        .checked_mul(std::mem::size_of::<super::ir::InputRegion>())?,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    contract
                        .results
                        .len()
                        .checked_mul(std::mem::size_of::<super::ir::ResultReference>())?,
                )
            })
            .ok_or(Exhausted)?;
        let generated_bytes = if contract.results.is_empty() {
            0
        } else {
            // Expanded preceding type visits bound occurrence count. Reserve
            // candidate Arc/Vec growth and full path copies, independently of
            // the supplied list (which may be forged small or empty).
            type_work
                .min(4096)
                .checked_mul(std::mem::size_of::<super::ir::InputPath>())
                .and_then(|n| {
                    n.checked_add(
                        type_work
                            .checked_mul(128)?
                            .min(131072)
                            .checked_mul(std::mem::size_of::<Projection>())?,
                    )
                })
                .and_then(|n| n.checked_mul(4))
                .ok_or(Exhausted)?
        };
        // A structural type visit can retain a comparison pair, depth, Vec
        // growth and hash-table slot. The expanded type walk also bounds
        // occurrence enumeration, independently of helper DAG equality caches.
        let bytes = type_work
            .checked_mul(128)
            .and_then(|n| n.checked_add(arity.checked_mul(std::mem::size_of::<Ty>())?))
            .and_then(|n| n.checked_add(loan_sites.checked_mul(std::mem::size_of::<usize>())?))
            .and_then(|n| n.checked_add(provided_bytes))
            .and_then(|n| n.checked_add(generated_bytes))
            .and_then(|n| n.checked_add(4096))
            .ok_or(Exhausted)?;
        if bytes > max_bytes {
            return Err(Exhausted);
        }
        budget.scratch(bytes)
    }
    fn charge_contract(
        result: Result<usize, String>,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        match result {
            Ok(work) => {
                budget.spend(work)?;
                Ok(true)
            }
            // Shared helpers use explicit limit diagnostics for every work,
            // depth, candidate and region cap. Never turn exhaustion into a
            // complete semantic rejection by replaying it through validate().
            Err(message) if message.contains("limit") || message.contains("exhaust") => {
                Err(Exhausted)
            }
            Err(_) => Ok(false),
        }
    }
    if f.blocks.len() > limits.blocks
        || f.locals.len() > limits.locals
        || f.params.len() > limits.locals
        || f.loan_sites.len() > super::ir::MAX_LOAN_SITES
    {
        return Err(Exhausted);
    }
    let mut budget = Budget::for_plan(limits.operations, plan);
    budget.spend(f.params.len())?;
    budget.spend(f.blocks.len())?;
    let mut call_origins = 0usize;
    for block in &f.blocks {
        if let Terminator::Call { signature, .. } = &block.terminator.value {
            let mut ty = &signature.result;
            for _ in 0..2 {
                budget.spend(1)?;
                let Ty::Ref { referent, .. } = ty else { break };
                call_origins = call_origins.checked_add(1).ok_or(Exhausted)?;
                ty = referent;
            }
        }
    }
    let mut formal_origins = 0usize;
    for local in &f.params {
        let Some(local) = f.locals.get(local.0) else {
            continue;
        };
        let mut ty = &local.ty;
        for _ in 0..2 {
            budget.spend(1)?;
            let Ty::Ref { referent, .. } = ty else { break };
            formal_origins = formal_origins.checked_add(1).ok_or(Exhausted)?;
            ty = referent;
        }
    }
    if f.blocks.len() > limits.blocks
        || f.locals.len() > limits.locals
        || f.loan_sites
            .len()
            .checked_add(formal_origins)
            .and_then(|n| n.checked_add(call_origins))
            .ok_or(Exhausted)?
            > super::ir::MAX_LOAN_SITES
    {
        return Err(Exhausted);
    }
    fn place_type(f: &cfg::Function, place: &Place, budget: &mut Budget) -> Result<(), Exhausted> {
        budget.spend(place.projection.len() + 1)?;
        // Invalid references are diagnosed by the verifier after this bounded
        // preflight, rather than indexing untrusted input here.
        let Some(local) = f.locals.get(place.local.0) else {
            return Ok(());
        };
        let mut ty = &local.ty;
        let mut projection = place.projection.iter();
        while let Some(step) = projection.next() {
            ty = match step {
                Projection::Field(index) => match ty.field_ty(*index) {
                    Some(field) => field,
                    None => return Ok(()),
                },
                Projection::Deref => {
                    let Ty::Ref { referent, .. } = ty else {
                        return Ok(());
                    };
                    referent
                }
                Projection::ConstantIndex(index) => {
                    let Ty::Array { element, len } = ty else {
                        return Ok(());
                    };
                    if *index >= *len {
                        return Ok(());
                    }
                    element
                }
                Projection::Downcast(variant) => {
                    let Ty::Enum(enumeration) = ty else {
                        return Ok(());
                    };
                    if enumeration.def().id != variant.owner {
                        return Ok(());
                    }
                    let Some(def) = enumeration.def().variants.get(variant.index) else {
                        return Ok(());
                    };
                    let Some(Projection::Field(index)) = projection.next() else {
                        return Ok(());
                    };
                    let Some(field) = def.fields.get(*index) else {
                        return Ok(());
                    };
                    &field.ty
                }
            };
        }
        preflight_ty(ty, 0, budget)
    }
    let before_types = budget.remaining;
    for local in &f.locals {
        preflight_ty(&local.ty, 0, &mut budget)?;
    }
    // The contract helper sees formal occurrences, not distinct local slots.
    // Malformed duplicate formal indices are diagnosed later, but must not
    // undercount generated candidates or scratch before that validation.
    for param in &f.params {
        if let Some(local) = f.locals.get(param.0) {
            preflight_ty(&local.ty, 0, &mut budget)?;
        }
    }
    preflight_ty(&f.result, 0, &mut budget)?;
    let own_scratch = contract_scratch(
        before_types - budget.remaining,
        f.params.len(),
        f.loan_sites.len(),
        limits.state_bytes,
        &f.ref_contract,
        facts,
        &mut budget,
    )?;
    let mut param_types = Vec::with_capacity(f.params.len());
    for param in &f.params {
        let Some(local) = f.locals.get(param.0) else {
            return Ok(budget.peak_scratch);
        };
        param_types.push(local.ty.clone());
    }
    let own_valid = if let FactsCtx::Validated(authority) = facts {
        match authority.validate_signature(
            super::ir::SignatureSubject::Body,
            &super::ir::Signature {
                params: param_types,
                result: f.result.clone(),
                ref_contract: f.ref_contract.clone(),
            },
        ) {
            Ok(()) => true,
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(_) => false,
        }
    } else {
        let valid = charge_contract(
            f.ref_contract.validation_work(&param_types, &f.result),
            &mut budget,
        )?;
        drop(param_types);
        valid
    };
    if !own_valid {
        return Ok(budget.peak_scratch); // Full verifier owns malformed-contract diagnostics.
    }
    drop(own_scratch);
    for site in &f.loan_sites {
        place_type(f, &site.place, &mut budget)?;
    }
    let operand = |op: &Operand, budget: &mut Budget| -> Result<(), Exhausted> {
        budget.spend(1)?;
        if let Operand::Copy(p) | Operand::Move(p) = op {
            place_type(f, p, budget)?;
        }
        if let Operand::Constant(cfg::Constant::Enum { ty, .. }) = op {
            preflight_ty(&Ty::Enum(ty.clone()), 0, budget)?;
        }
        Ok(())
    };
    for block in &f.blocks {
        budget.spend(1)?;
        for item in &block.statements {
            budget.spend(1)?;
            if let Statement::Inspect(place) = &item.value {
                place_type(f, place, &mut budget)?;
            }
            match &item.value {
                Statement::IndexInspect { array, index } => {
                    place_type(f, array, &mut budget)?;
                    operand(index, &mut budget)?;
                }
                Statement::IndexAssign {
                    array,
                    index,
                    value,
                }
                | Statement::IndexAssignOp {
                    array,
                    index,
                    value,
                    ..
                } => {
                    // The closed compound operator descriptor has constant cost.
                    if matches!(item.value, Statement::IndexAssignOp { .. }) {
                        budget.spend(1)?;
                    }
                    place_type(f, array, &mut budget)?;
                    operand(value, &mut budget)?;
                    operand(index, &mut budget)?;
                }
                _ => {}
            }
            if let Statement::Assign { destination, value } = &item.value {
                place_type(f, destination, &mut budget)?;
                match value {
                    Rvalue::Borrow(_) => budget.spend(1)?,
                    Rvalue::Discriminant(place) => place_type(f, place, &mut budget)?,
                    Rvalue::IndexCopy { array, index } => {
                        place_type(f, array, &mut budget)?;
                        operand(index, &mut budget)?;
                    }
                    Rvalue::IntCast { value, to } => {
                        preflight_ty(&Ty::Int(*to), 0, &mut budget)?;
                        operand(value, &mut budget)?;
                    }
                    Rvalue::FloatCast { value, to } => {
                        preflight_ty(&to.ty(), 0, &mut budget)?;
                        operand(value, &mut budget)?;
                    }
                    Rvalue::Use(op)
                    | Rvalue::Unary { value: op, .. }
                    | Rvalue::ArrayToSlice { reference: op }
                    | Rvalue::SliceLen { slice: op } => operand(op, &mut budget)?,
                    Rvalue::SliceIndex { slice, index } => {
                        operand(slice, &mut budget)?;
                        operand(index, &mut budget)?;
                    }
                    Rvalue::Repeat { ty, value } => {
                        preflight_ty(ty, 0, &mut budget)?;
                        operand(value, &mut budget)?;
                    }
                    Rvalue::Binary { left, right, .. } => {
                        operand(left, &mut budget)?;
                        operand(right, &mut budget)?;
                    }
                    Rvalue::Aggregate {
                        ty: value_ty,
                        fields,
                    } => {
                        preflight_ty(value_ty, 0, &mut budget)?;
                        for field in fields {
                            operand(field, &mut budget)?;
                        }
                    }
                    Rvalue::EnumVariant { ty, fields, .. } => {
                        preflight_ty(&Ty::Enum(ty.clone()), 0, &mut budget)?;
                        for field in fields {
                            operand(field, &mut budget)?;
                        }
                    }
                    Rvalue::Println(_) => {}
                }
            }
        }
        match &block.terminator.value {
            Terminator::Switch { condition, .. } => operand(condition, &mut budget)?,
            Terminator::SwitchVariant { place, .. } => place_type(f, place, &mut budget)?,
            Terminator::Call {
                target,
                signature,
                args,
                destination,
                ..
            } => {
                let before_types = budget.remaining;
                match target {
                    super::ir::Callee::Direct(target) => {
                        budget.spend(target.type_args.len() + 1)?;
                        for argument in target.type_args.iter() {
                            preflight_ty(argument, 0, &mut budget)?;
                        }
                    }
                    super::ir::Callee::TraitItem(call) => {
                        budget.spend(call.trait_ref.args.len() + call.method_args.len() + 1)?;
                        preflight_ty(&call.trait_ref.self_ty, 0, &mut budget)?;
                        for argument in call.trait_ref.args.iter().chain(call.method_args.iter()) {
                            preflight_ty(argument, 0, &mut budget)?;
                        }
                    }
                }
                for param in &signature.params {
                    preflight_ty(param, 0, &mut budget)?;
                }
                preflight_ty(&signature.result, 0, &mut budget)?;
                let _call_scratch = contract_scratch(
                    before_types - budget.remaining,
                    signature.params.len(),
                    f.loan_sites.len(),
                    limits.state_bytes,
                    &signature.ref_contract,
                    facts,
                    &mut budget,
                )?;
                let call_valid = if let FactsCtx::Validated(authority) = facts {
                    match authority
                        .validate_signature(super::ir::SignatureSubject::Call(target), signature)
                    {
                        Ok(()) => true,
                        Err(error) if error.is_resource() => return Err(Exhausted),
                        Err(_) => false,
                    }
                } else {
                    charge_contract(
                        signature
                            .ref_contract
                            .validation_work(&signature.params, &signature.result),
                        &mut budget,
                    )?
                };
                if !call_valid {
                    return Ok(budget.peak_scratch);
                }
                for arg in args {
                    operand(arg, &mut budget)?;
                }
                place_type(f, destination, &mut budget)?;
            }
            _ => {}
        }
    }
    Ok(budget.peak_scratch)
}
fn preflight_ty(ty: &Ty, depth: usize, budget: &mut Budget) -> Result<(), Exhausted> {
    // Recursive coercion verification can compare each enclosing subtree
    // before descending. Weight by depth to conservatively cover that repeated
    // comparison work, not only the type's leaf count.
    budget.spend(depth + 1)?;
    if depth > 128 {
        return Err(Exhausted);
    }
    if let Ty::Ref { referent, .. } = ty {
        preflight_ty(referent, depth + 1, budget)?;
    }
    if let Ty::Array { element, len } = ty {
        if *len > super::ir::MAX_ARRAY_LEN {
            return Err(Exhausted);
        }
        // Zero arrays still validate their element type and Copy semantics.
        // Nonempty arrays expand before allocation, bounding repeated shared
        // types as well as their distinct element move paths.
        for _ in 0..(*len).max(1) {
            preflight_ty(element, depth + 1, budget)?;
        }
    }
    if let Ty::Enum(ty) = ty {
        for argument in ty.def().type_args.iter() {
            preflight_ty(argument, depth + 1, budget)?;
        }
        budget.spend(ty.def().variants.len())?;
        for variant in ty.def().variants.iter() {
            for field in variant.fields.iter() {
                preflight_ty(&field.ty, depth + 1, budget)?;
            }
        }
    }
    if let Ty::Struct(ty) = ty {
        for argument in ty.def().type_args.iter() {
            preflight_ty(argument, depth + 1, budget)?;
        }
    }
    if let Some(count) = ty.field_count() {
        for index in 0..count {
            preflight_ty(ty.field_ty(index).ok_or(Exhausted)?, depth + 1, budget)?;
        }
    }
    Ok(())
}

/// A borrowed ownership shape, never a native layout or a concrete witness.
/// Repeated DAG occurrences are expanded by Paths, not deduplicated by type.
enum MoveShape<'a> {
    Scalar,
    OpaqueParam,
    Never,
    Product { ty: &'a Ty, fields: usize },
    Array { element: &'a Ty, len: usize },
    Sum(&'a super::ir::EnumDef),
}
impl<'a> MoveShape<'a> {
    fn of(ty: &'a Ty, facts: FactsCtx<'_>, budget: &mut Budget) -> Result<Self, Exhausted> {
        budget.spend(1)?;
        if matches!(facts, FactsCtx::Validated(authority) if authority.phase() == FactsPhase::Schema)
        {
            return Err(Exhausted);
        }
        Ok(match ty {
            Ty::Param(param) => {
                let env = match facts {
                    FactsCtx::Universal(env) => env,
                    FactsCtx::Validated(authority)
                        if authority.phase() == FactsPhase::Universal =>
                    {
                        authority.env()
                    }
                    _ => return Err(Exhausted),
                };
                budget.spend(env.scopes().len())?;
                if !env.contains(*param) {
                    return Err(Exhausted);
                }
                Self::OpaqueParam
            }
            Ty::Never => Self::Never,
            Ty::Tuple(fields) => Self::Product {
                ty,
                fields: fields.len(),
            },
            Ty::Struct(definition) => Self::Product {
                ty,
                fields: definition.def().fields.len(),
            },
            Ty::Array { element, len } => Self::Array { element, len: *len },
            Ty::Enum(definition) => Self::Sum(definition.def()),
            // Exactly shared `&[u8]` is a Copy reference carrier. Its referent
            // is one opaque unsized leaf: u8 elements carry no references and
            // no slice place, move or mutable element access exists.
            Ty::Slice { element } if **element == Ty::Int(super::ir::IntTy::U8) => Self::Scalar,
            Ty::Ref { .. } if ty.shared_byte_slice() => Self::Scalar,
            Ty::Slice { .. } => return Err(Exhausted),
            Ty::Ref { referent, .. } if referent.contains_slice() => return Err(Exhausted),
            Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Unit | Ty::Ref { .. } => Self::Scalar,
        })
    }
}

struct PathNode {
    parent: Option<usize>,
    children: Vec<usize>,
    /// Enum variants select disjoint groups in `children`. Empty for non-enums.
    variant_offsets: Vec<usize>,
    /// Preorder interval [self, end) covers this place's entire subtree.
    end: usize,
}
struct Paths {
    roots: Vec<usize>,
    external: Vec<Vec<(super::ir::CarrierLocation, usize)>>,
    nodes: Vec<PathNode>,
}
impl Paths {
    #[cfg(test)]
    fn new(f: &cfg::Function, limit: usize) -> Result<Self, Exhausted> {
        Self::with_facts(
            f,
            limit,
            FactsCtx::Concrete,
            &mut Budget::new(32 * 1024 * 1024),
        )
    }
    fn with_facts(
        f: &cfg::Function,
        limit: usize,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
    ) -> Result<Self, Exhausted> {
        fn count(
            ty: &Ty,
            facts: FactsCtx<'_>,
            budget: &mut Budget,
            limit: usize,
            depth: usize,
            counts: &mut (usize, usize),
        ) -> Result<(), Exhausted> {
            if depth > 128 || counts.0 >= limit {
                return Err(Exhausted);
            }
            counts.0 += 1;
            match MoveShape::of(ty, facts, budget)? {
                MoveShape::Product { ty, fields } => {
                    for index in 0..fields {
                        count(
                            ty.field_ty(index).ok_or(Exhausted)?,
                            facts,
                            budget,
                            limit,
                            depth + 1,
                            counts,
                        )?;
                    }
                }
                MoveShape::Array { element, len } => {
                    if len > super::ir::MAX_ARRAY_LEN || len > limit - counts.0 {
                        return Err(Exhausted);
                    }
                    for _ in 0..len {
                        count(element, facts, budget, limit, depth + 1, counts)?;
                    }
                }
                MoveShape::Sum(definition) => {
                    counts.1 = counts
                        .1
                        .checked_add(definition.variants.len())
                        .ok_or(Exhausted)?;
                    for variant in definition.variants.iter() {
                        budget.spend(1)?;
                        for field in variant.fields.iter() {
                            count(&field.ty, facts, budget, limit, depth + 1, counts)?;
                        }
                    }
                }
                MoveShape::Scalar | MoveShape::OpaqueParam | MoveShape::Never => {}
            }
            Ok(())
        }
        let mut counts = (0, 0);
        for local in &f.locals {
            count(&local.ty, facts, budget, limit, 0, &mut counts)?;
        }
        let ledger = match facts {
            FactsCtx::Validated(facts) => facts.budget().clone(),
            _ => budget
                .plan
                .clone()
                .unwrap_or_else(|| super::ir::PlanBudget::new(Default::default())),
        };
        for local in &f.params {
            for location in super::ir::carrier_locations(&f.locals[local.0].ty, &ledger)
                .map_err(|_| Exhausted)?
                .iter()
            {
                if let Ty::Ref { referent, .. } =
                    super::ir::carrier_at(&f.locals[local.0].ty, location, &ledger)
                        .map_err(|_| Exhausted)?
                {
                    count(referent, facts, budget, limit, 0, &mut counts)?;
                }
            }
        }
        let (node_count, variant_count) = counts;
        budget.nodes(node_count)?;
        // Small Vec allocations start at four slots, so a one-field product
        // needs more than a two-times logical-edge allowance.
        let bytes = paths_storage_bytes(f.locals.len(), node_count, variant_count)?;
        budget.reserve_bytes(bytes)?;
        fn add(
            paths: &mut Paths,
            ty: &Ty,
            parent: Option<usize>,
            limit: usize,
            facts: FactsCtx<'_>,
            budget: &mut Budget,
        ) -> Result<usize, Exhausted> {
            if paths.nodes.len() >= limit {
                return Err(Exhausted);
            }
            let id = paths.nodes.len();
            paths.nodes.push(PathNode {
                parent,
                children: Vec::new(),
                variant_offsets: Vec::new(),
                end: 0,
            });
            match MoveShape::of(ty, facts, budget)? {
                MoveShape::Product { ty, fields } => {
                    for index in 0..fields {
                        let child = add(
                            paths,
                            ty.field_ty(index).ok_or(Exhausted)?,
                            Some(id),
                            limit,
                            facts,
                            budget,
                        )?;
                        paths.nodes[id].children.push(child);
                    }
                }
                MoveShape::Array { element, len } => {
                    if len > super::ir::MAX_ARRAY_LEN || len > limit - paths.nodes.len() {
                        return Err(Exhausted);
                    }
                    for _ in 0..len {
                        let child = add(paths, element, Some(id), limit, facts, budget)?;
                        paths.nodes[id].children.push(child);
                    }
                }
                MoveShape::Sum(enumeration) => {
                    for variant in enumeration.variants.iter() {
                        budget.spend(1)?;
                        let offset = paths.nodes[id].children.len();
                        paths.nodes[id].variant_offsets.push(offset);
                        for field in variant.fields.iter() {
                            let child = add(paths, &field.ty, Some(id), limit, facts, budget)?;
                            paths.nodes[id].children.push(child);
                        }
                    }
                }
                MoveShape::Scalar | MoveShape::OpaqueParam | MoveShape::Never => {}
            }
            paths.nodes[id].end = paths.nodes.len();
            Ok(id)
        }
        let mut result = Self {
            roots: Vec::new(),
            external: vec![vec![]; f.locals.len()],
            nodes: Vec::new(),
        };
        for local in &f.locals {
            let root = add(&mut result, &local.ty, None, limit, facts, budget)?;
            result.roots.push(root);
        }
        for local in &f.params {
            for location in super::ir::carrier_locations(&f.locals[local.0].ty, &ledger)
                .map_err(|_| Exhausted)?
                .iter()
            {
                if let Ty::Ref { referent, .. } =
                    super::ir::carrier_at(&f.locals[local.0].ty, location, &ledger)
                        .map_err(|_| Exhausted)?
                {
                    let root = add(&mut result, referent, None, limit, facts, budget)?;
                    budget.reserve_bytes(4 * size_of::<(super::ir::CarrierLocation, usize)>())?;
                    result.external[local.0].push((location.clone(), root));
                }
            }
        }
        Ok(result)
    }
    fn resolve(&self, place: &Place) -> usize {
        self.resolve_projection(self.roots[place.local.0], &place.projection)
    }
    fn canonical_root(&self, root: &CanonicalRoot) -> usize {
        match root {
            CanonicalRoot::Owned(local) => self.roots[local.0],
            CanonicalRoot::ParameterReferent(local, position) => {
                self.external[local.0]
                    .iter()
                    .find(|(location, _)| location == position)
                    .expect("formal referent path")
                    .1
            }
        }
    }
    fn resolve_canonical(&self, place: &CanonicalPlace) -> usize {
        self.resolve_projection(self.canonical_root(&place.root), &place.projection)
    }
    fn resolve_projection(&self, mut node: usize, path: &[Projection]) -> usize {
        let mut projection = path.iter();
        while let Some(step) = projection.next() {
            let index = match step {
                Projection::Deref => {
                    unreachable!("deref must be resolved through canonical loan provenance")
                }
                Projection::Field(index) | Projection::ConstantIndex(index) => *index,
                Projection::Downcast(variant) => {
                    let Some(Projection::Field(index)) = projection.next() else {
                        unreachable!("validated downcast has a field")
                    };
                    self.nodes[node].variant_offsets[variant.index] + index
                }
            };
            node = self.nodes[node].children[index];
        }
        node
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bits(Vec<u64>);
impl Bits {
    fn new(count: usize) -> Self {
        Self(vec![0; count.div_ceil(64)])
    }
    fn get(&self, bit: usize) -> bool {
        self.0[bit / 64] & (1 << (bit % 64)) != 0
    }
    fn set(&mut self, bit: usize, value: bool) {
        let mask = 1 << (bit % 64);
        if value {
            self.0[bit / 64] |= mask;
        } else {
            self.0[bit / 64] &= !mask;
        }
    }
    fn range(
        &mut self,
        start: usize,
        end: usize,
        value: bool,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        if start == end {
            return Ok(());
        }
        for (index, word) in self
            .0
            .iter_mut()
            .enumerate()
            .take(end.div_ceil(64))
            .skip(start / 64)
        {
            budget.spend(1)?;
            let low = start.saturating_sub(index * 64);
            let high = (end - index * 64).min(64);
            let mask = (u64::MAX >> (64 - (high - low))) << low;
            if value {
                *word |= mask;
            } else {
                *word &= !mask;
            }
        }
        Ok(())
    }
    fn all(&self, start: usize, end: usize, budget: &mut Budget) -> Result<bool, Exhausted> {
        if start == end {
            return Ok(true);
        }
        for (index, word) in self
            .0
            .iter()
            .enumerate()
            .take(end.div_ceil(64))
            .skip(start / 64)
        {
            budget.spend(1)?;
            let low = start.saturating_sub(index * 64);
            let high = (end - index * 64).min(64);
            let mask = (u64::MAX >> (64 - (high - low))) << low;
            if *word & mask != mask {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn merge(
        &mut self,
        other: &Self,
        intersection: bool,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        budget.spend(self.0.len())?;
        let mut changed = false;
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            let merged = if intersection { *a & *b } else { *a | *b };
            changed |= *a != merged;
            *a = merged;
        }
        Ok(changed)
    }
}
#[derive(Clone)]
struct State {
    must_live: Bits,
    may_live: Bits,
    must_init: Bits,
    may_init: Bits,
    ever_written: Bits,
    /// Variant index or usize::MAX (unknown), indexed by actual enum path.
    /// Owner identity is fixed by the structurally verified path type.
    selected: Vec<usize>,
}
impl State {
    fn new(locals: usize, paths: usize) -> Self {
        Self {
            must_live: Bits::new(locals),
            may_live: Bits::new(locals),
            must_init: Bits::new(paths),
            may_init: Bits::new(paths),
            ever_written: Bits::new(paths),
            selected: vec![usize::MAX; paths],
        }
    }
    fn words(&self) -> usize {
        self.must_live.0.len() * 2 + self.must_init.0.len() * 3 + self.selected.len()
    }
    fn merge(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        let mut changed = self.must_live.merge(&other.must_live, true, budget)?;
        changed |= self.may_live.merge(&other.may_live, false, budget)?;
        changed |= self.must_init.merge(&other.must_init, true, budget)?;
        changed |= self.may_init.merge(&other.may_init, false, budget)?;
        changed |= self
            .ever_written
            .merge(&other.ever_written, false, budget)?;
        budget.spend(self.selected.len())?;
        for (ours, theirs) in self.selected.iter_mut().zip(&other.selected) {
            if *ours != *theirs && *ours != usize::MAX {
                *ours = usize::MAX;
                changed = true;
            }
        }
        Ok(changed)
    }
}

struct Transfer<'a> {
    facts: FactsCtx<'a>,
    f: &'a cfg::Function,
    paths: &'a Paths,
    budget: &'a mut Budget,
    report: Option<&'a mut Report>,
    block: BlockId,
    statement: Option<usize>,
    origin: Option<ExprId>,
    diagnostic_limit: usize,
    loans: Option<&'a borrow::Plan>,
    phase: Phase,
}
struct Report {
    stats: AnalysisStats,
    diagnostics: Vec<Diagnostic>,
    complete: bool,
}
impl Transfer<'_> {
    fn problem(&mut self, problem: Problem, place: Place) {
        let kind = match problem {
            Problem::Conflict(message) => ErrorKind::BorrowConflict(message),
            Problem::Invalid(message) => ErrorKind::InvalidReference(message),
            Problem::Unsupported(message) => ErrorKind::UnsupportedBorrowing(message),
        };
        self.diagnostic(kind, place);
    }
    fn targets(&mut self, place: &Place) -> Result<Vec<Target>, Exhausted> {
        if let Some(loans) = self.loans {
            match loans.resolve(place, self.block, self.statement, self.phase, self.budget)? {
                Ok(targets) => Ok(targets),
                Err(problem) => {
                    self.problem(problem, place.clone());
                    Ok(vec![])
                }
            }
        } else {
            Ok(vec![Target {
                place: CanonicalPlace::owned(place),
                precision: TargetPrecision::Exact,
                authority: vec![],
            }])
        }
    }
    fn check_access(&mut self, target: &Target, access: Access) -> Result<(), Exhausted> {
        if let Some(loans) = self.loans
            && let Some(problem) = loans.check(
                target,
                access,
                self.block,
                self.statement,
                self.phase,
                self.budget,
            )?
        {
            self.problem(problem, target.place.diagnostic());
        }
        Ok(())
    }
    fn carrier_read(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        for (index, projection) in place.projection.iter().enumerate() {
            if *projection != Projection::Deref {
                continue;
            }
            self.budget.spend(index + 1)?;
            let carrier = Place {
                local: place.local,
                projection: place.projection[..index].to_vec(),
            };
            for target in self.targets(&carrier)? {
                self.check_access(&target, Access::Read)?;
                self.read_canonical(state, &target.place)?;
            }
        }
        Ok(())
    }
    fn mut_authority(&mut self, place: &Place) {
        for (index, projection) in place.projection.iter().enumerate() {
            if *projection != Projection::Deref {
                continue;
            }
            if !matches!(
                super::ir::projected_ty(
                    &self.f.locals[place.local.0].ty,
                    &place.projection[..index]
                ),
                Ok(Ty::Ref {
                    kind: super::ir::BorrowKind::Mut,
                    ..
                })
            ) {
                self.problem(
                    Problem::Invalid(
                        "mutation requires mutable authority through every reference carrier"
                            .into(),
                    ),
                    place.clone(),
                );
                return;
            }
        }
    }
    fn read_mode(
        &mut self,
        state: &mut State,
        place: &Place,
        access: Access,
    ) -> Result<(), Exhausted> {
        if self.loans.is_none() {
            return self.read_raw(state, place);
        }
        self.carrier_read(state, place)?;
        for target in self.targets(place)? {
            self.check_access(&target, access)?;
            self.read_canonical(state, &target.place)?;
        }
        Ok(())
    }
    fn read(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.read_mode(state, place, Access::Read)
    }
    fn write(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        if self.loans.is_none() {
            return self.write_raw(state, place);
        }
        let deref = place.projection.contains(&Projection::Deref);
        self.carrier_read(state, place)?;
        if deref {
            self.mut_authority(place);
        }
        if deref {
            self.budget.spend(1)?;
            if let Some(report) = &mut self.report {
                report.stats.writes += 1;
            }
        }
        for target in self.targets(place)? {
            self.check_access(&target, Access::Write)?;
            if deref {
                // Every alternative must already be initialized. A may-alias
                // write must never initialize every possible referent.
                self.read_canonical(state, &target.place)?;
                let node = self.paths.resolve_canonical(&target.place);
                self.kill_proofs(state, node)?;
                self.mark_written(state, node)?;
            } else {
                self.write_raw(state, place)?;
            }
        }
        Ok(())
    }
    fn authorize(&mut self, state: &State, place: &Place) -> Result<bool, Exhausted> {
        self.authorize_canonical(state, &CanonicalPlace::owned(place))
    }
    fn authorize_canonical(
        &mut self,
        state: &State,
        place: &CanonicalPlace,
    ) -> Result<bool, Exhausted> {
        // Cover this traversal and the subsequent resolve traversals for reads,
        // moves and writes. Structural typing was checked for every CFG block.
        self.budget
            .spend(place.projection.len().checked_mul(3).ok_or(Exhausted)? + 1)?;
        let mut node = self.paths.canonical_root(&place.root);
        let mut projection = place.projection.iter();
        while let Some(step) = projection.next() {
            let index = match step {
                Projection::Deref => {
                    unreachable!("deref must be canonicalized before authorization")
                }
                Projection::Field(index) | Projection::ConstantIndex(index) => *index,
                Projection::Downcast(variant) => {
                    if let Some(report) = &mut self.report {
                        report.stats.variant_proof_checks += 1;
                    }
                    if self.paths.nodes[node].variant_offsets.len() != 1
                        && state.selected[node] != variant.index
                    {
                        self.diagnostic(
                            ErrorKind::VariantNotDefinitelySelected,
                            place.diagnostic(),
                        );
                        return Ok(false);
                    }
                    let Some(Projection::Field(index)) = projection.next() else {
                        unreachable!("validated downcast has a field")
                    };
                    self.paths.nodes[node].variant_offsets[variant.index] + index
                }
            };
            node = self.paths.nodes[node].children[index];
        }
        Ok(true)
    }
    fn diagnostic(&mut self, kind: ErrorKind, place: Place) {
        if let Some(report) = &mut self.report {
            if report.diagnostics.len() < self.diagnostic_limit {
                report.diagnostics.push(Diagnostic {
                    kind,
                    block: self.block,
                    statement: self.statement,
                    origin: self.origin,
                    place: Some(place),
                });
            } else {
                report.stats.diagnostics_suppressed += 1;
                report.complete = false;
            }
        }
    }
    fn ancestors(&mut self, state: &State, node: usize) -> Result<bool, Exhausted> {
        let mut parent = self.paths.nodes[node].parent;
        while let Some(node) = parent {
            self.budget.spend(1)?;
            if !state.must_init.get(node) {
                return Ok(false);
            }
            parent = self.paths.nodes[node].parent;
        }
        Ok(true)
    }
    fn kill_proofs(&mut self, state: &mut State, node: usize) -> Result<(), Exhausted> {
        let end = self.paths.nodes[node].end;
        self.budget.spend(end - node)?;
        for selected in &mut state.selected[node..end] {
            if *selected != usize::MAX {
                if let Some(report) = &mut self.report {
                    report.stats.variant_proofs_killed += 1;
                }
                *selected = usize::MAX;
            }
        }
        Ok(())
    }
    fn initialize(&mut self, state: &mut State, node: usize, value: bool) -> Result<(), Exhausted> {
        self.kill_proofs(state, node)?;
        let end = self.paths.nodes[node].end;
        state.must_init.range(node, end, value, self.budget)?;
        state.may_init.range(node, end, value, self.budget)
    }
    fn read_raw(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.read_canonical(state, &CanonicalPlace::owned(place))
    }
    fn read_canonical(
        &mut self,
        state: &mut State,
        place: &CanonicalPlace,
    ) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if !self.authorize_canonical(state, place)? {
            return Ok(());
        }
        if matches!(place.root, CanonicalRoot::Owned(local) if !state.must_live.get(local.0)) {
            self.diagnostic(ErrorKind::StorageNotDefinitelyLive, place.diagnostic());
        } else {
            let node = self.paths.resolve_canonical(place);
            if !self.ancestors(state, node)?
                || !state
                    .must_init
                    .all(node, self.paths.nodes[node].end, self.budget)?
            {
                self.diagnostic(ErrorKind::ReadNotDefinitelyInitialized, place.diagnostic());
            }
        }
        Ok(())
    }
    fn copy_permission(&mut self, place: &Place) -> Result<Option<bool>, Exhausted> {
        let ty = self.f.place_ty(place).expect("structurally verified place");
        preflight_ty(ty, 0, self.budget)?;
        if let FactsCtx::Validated(authority) = self.facts {
            return match authority.must_copy(ty) {
                Ok(permission) => Ok(Some(permission)),
                Err(error) if error.is_resource() => Err(Exhausted),
                Err(error) => {
                    let message = error.into_legacy_diagnostic(authority.budget());
                    self.diagnostic(ErrorKind::InvalidCfg(message), place.clone());
                    Ok(None)
                }
            };
        }
        // Structural-only bootstrap and fixture contexts cannot mint a native
        // capability. Production always queries the exact retained authority.
        self.facts.must_copy(ty).map(Some).map_err(|_| Exhausted)
    }
    fn deny_copy(&mut self, place: &Place, message: &'static str) -> Result<(), Exhausted> {
        let message = if let FactsCtx::Validated(authority) = self.facts {
            let error = super::traits::FactsError::invalid(authority.budget(), message);
            if error.is_resource() {
                return Err(Exhausted);
            }
            error.into_legacy_diagnostic(authority.budget())
        } else {
            message.into()
        };
        self.diagnostic(ErrorKind::InvalidCfg(message), place.clone());
        Ok(())
    }
    fn operand(&mut self, state: &mut State, op: &Operand) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.operands_read += 1;
        }
        if let Operand::Copy(place) = op {
            match self.copy_permission(place)? {
                Some(true) => {}
                Some(false) => {
                    self.deny_copy(place, "Copy operand requires Copy permission")?;
                    return Ok(());
                }
                None => return Ok(()),
            }
        }
        if let Operand::Move(place) = op {
            self.budget.spend(place.projection.len() + 1)?;
            if place
                .projection
                .iter()
                .any(|p| matches!(p, Projection::ConstantIndex(_)))
            {
                // The CFG verifier also rejects this, including in dead blocks.
                // Keep transfer fail-closed independently, and meter repeated
                // element Copy checks rather than recursively walking for free.
                match self.copy_permission(place)? {
                    Some(true) => {}
                    Some(false) => {
                        self.deny_copy(
                            place,
                            "non-Copy move through array indexing is unsupported",
                        )?;
                        return Ok(());
                    }
                    None => return Ok(()),
                }
            }
        }
        if let Operand::Copy(place) | Operand::Move(place) = op {
            if matches!(op, Operand::Move(_)) {
                if matches!(
                    self.f.locals[place.local.0].ty,
                    Ty::Ref {
                        kind: super::ir::BorrowKind::Mut,
                        ..
                    }
                ) && place.projection.is_empty()
                    && let Some(loans) = self.loans
                    && let Some(problem) = loans.check_handle_move(
                        place.local,
                        self.block,
                        self.statement,
                        self.budget,
                    )?
                {
                    self.problem(problem, place.clone());
                }
                if place.projection.contains(&Projection::Deref) {
                    self.mut_authority(place);
                }
                self.read_mode(state, place, Access::Move)?;
            } else {
                self.read(state, place)?;
            }
            if matches!(op, Operand::Move(_)) {
                if let Some(report) = &mut self.report {
                    report.stats.moves += 1;
                }
                for target in self.targets(place)? {
                    self.initialize(state, self.paths.resolve_canonical(&target.place), false)?;
                }
            }
        }
        Ok(())
    }
    /// Returning a value inspects initialization, not executable Copy permission.
    /// In particular opaque parameters and non-Copy nominals may be returned.
    fn inspect_return(&mut self, state: &mut State) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.operands_read += 1;
        }
        self.read(state, &root_place(self.f.return_local))
    }
    fn write_raw(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.writes += 1;
        }
        self.authorize(state, place)?;
        let node = self.paths.resolve(place);
        let root = self.paths.roots[place.local.0];
        if !state.must_live.get(place.local.0) {
            self.diagnostic(ErrorKind::StorageNotDefinitelyLive, place.clone());
        } else if !self.ancestors(state, node)? {
            self.diagnostic(ErrorKind::WriteIntoUninitializedAggregate, place.clone());
        } else if !self.f.locals[place.local.0].mutable && state.ever_written.get(root) {
            self.diagnostic(ErrorKind::ImmutableReassignment, place.clone());
        }
        // Continue a deterministic transfer even after a diagnostic. No failed
        // analysis is a permission to emit, and this limits cascading errors.
        self.initialize(state, node, true)?;
        self.mark_written(state, node)
    }
    fn mark_written(&mut self, state: &mut State, node: usize) -> Result<(), Exhausted> {
        state
            .ever_written
            .range(node, self.paths.nodes[node].end, true, self.budget)?;
        let mut parent = self.paths.nodes[node].parent;
        while let Some(node) = parent {
            self.budget.spend(1)?;
            state.ever_written.set(node, true);
            parent = self.paths.nodes[node].parent;
        }
        Ok(())
    }
    fn index_write(&mut self, state: &mut State, array: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.writes += 1;
        }
        // An unknown slot can only replace an element of an already ready
        // array. Never initialize a guessed child or resurrect missing bits.
        self.carrier_read(state, array)?;
        let deref = array.projection.contains(&Projection::Deref);
        if deref {
            self.mut_authority(array);
        }
        for target in self.targets(array)? {
            self.check_access(&target, Access::Write)?;
            self.read_canonical(state, &target.place)?;
            let node = self.paths.resolve_canonical(&target.place);
            if !deref
                && !self.f.locals[array.local.0].mutable
                && state.ever_written.get(self.paths.roots[array.local.0])
            {
                self.diagnostic(ErrorKind::ImmutableReassignment, target.place.diagnostic());
            }
            self.kill_proofs(state, node)?;
            self.mark_written(state, node)?;
        }
        Ok(())
    }
    fn rvalue(&mut self, state: &mut State, value: &Rvalue) -> Result<(), Exhausted> {
        match value {
            Rvalue::Borrow(id) => {
                let site = &self.f.loan_sites[id.0];
                let source = site.place.clone();
                let kind = site.kind;
                let timing = site.timing;
                self.carrier_read(state, &source)?;
                if kind == super::ir::BorrowKind::Mut {
                    if source.projection.first() == Some(&Projection::Deref) {
                        self.mut_authority(&source);
                    } else if !self.f.locals[source.local.0].mutable {
                        self.diagnostic(ErrorKind::ImmutableReassignment, source.clone());
                    }
                }
                let targets = if let Some(loans) = self.loans {
                    loans.issued(*id, self.budget)?
                } else {
                    vec![]
                };
                if targets.is_empty() {
                    self.problem(
                        Problem::Invalid("borrow issuance has no canonical provenance".into()),
                        source,
                    );
                }
                for target in targets {
                    self.check_access(
                        &target,
                        if kind == super::ir::BorrowKind::Mut {
                            if matches!(timing, cfg::LoanTiming::TwoPhase { .. }) {
                                Access::ReserveMut
                            } else {
                                Access::MutBorrow
                            }
                        } else {
                            Access::SharedBorrow
                        },
                    )?;
                    self.read_canonical(state, &target.place)?;
                }
            }
            // Reading a tag does not copy or move the non-Copy enum, but does
            // require the entire enum, including active payloads, initialized.
            Rvalue::Discriminant(place) => self.read(state, place)?,
            Rvalue::IndexCopy { array, index } => {
                self.operand(state, index)?;
                self.read(state, array)?;
            }
            Rvalue::Use(op)
            | Rvalue::Unary { value: op, .. }
            | Rvalue::IntCast { value: op, .. }
            | Rvalue::FloatCast { value: op, .. } => self.operand(state, op)?,
            // Unsizing copies the shared reference once. Its loan and origins
            // are the operand's; length metadata grants no storage authority.
            Rvalue::ArrayToSlice { reference } => self.operand(state, reference)?,
            // Length reads only the slice handle, never the referent storage.
            Rvalue::SliceLen { slice } => self.operand(state, slice)?,
            // Receiver snapshot, index snapshot, then a live shared read of the
            // referent through the handle's issued origins.
            Rvalue::SliceIndex { slice, index } => {
                self.operand(state, slice)?;
                self.operand(state, index)?;
                match slice {
                    Operand::Copy(handle) if handle.projection.is_empty() => {
                        let mut referent = handle.clone();
                        referent.projection.push(Projection::Deref);
                        self.read(state, &referent)?;
                    }
                    _ => self.problem(
                        Problem::Invalid("slice index requires a whole-local shared handle".into()),
                        match slice {
                            Operand::Copy(place) | Operand::Move(place) => place.clone(),
                            Operand::Constant(_) => root_place(self.f.return_local),
                        },
                    ),
                }
            }
            // Repetition evaluates once even for a zero-length destination.
            Rvalue::Repeat { value, .. } => self.operand(state, value)?,
            Rvalue::Binary { left, right, .. } => {
                self.operand(state, left)?;
                self.operand(state, right)?;
            }
            Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } => {
                for field in fields {
                    self.operand(state, field)?;
                }
            }
            Rvalue::Println(_) => {}
        }
        Ok(())
    }
    fn block(&mut self, state: &mut State) -> Result<(), Exhausted> {
        for (index, item) in self.f.blocks[self.block.0].statements.iter().enumerate() {
            self.budget.spend(1)?;
            self.statement = Some(index);
            self.origin = item.origin;
            self.phase = Phase::Read;
            match &item.value {
                Statement::Inspect(place) => self.read(state, place)?,
                Statement::IndexInspect { array, index } => {
                    self.operand(state, index)?;
                    self.read(state, array)?;
                }
                Statement::IndexAssign {
                    array,
                    index,
                    value,
                }
                | Statement::IndexAssignOp {
                    array,
                    index,
                    value,
                    ..
                } => {
                    // Lowering has already snapshotted RHS and index effects.
                    // Preserve that owning-value-before-index transfer order.
                    self.operand(state, value)?;
                    self.operand(state, index)?;
                    if matches!(item.value, Statement::IndexAssignOp { .. }) {
                        // Compound assignment reads the current element after
                        // both snapshots. Unknown indices conservatively read
                        // the whole array before applying the existing write.
                        self.read(state, array)?;
                    }
                    self.phase = Phase::Write;
                    self.index_write(state, array)?;
                }
                Statement::StorageLive(local) => {
                    self.check_access(
                        &Target {
                            precision: TargetPrecision::Exact,
                            place: CanonicalPlace::owned(&root_place(*local)),
                            authority: vec![],
                        },
                        Access::StorageEnd,
                    )?;
                    if let Some(report) = &mut self.report {
                        report.stats.storage_events += 1;
                    }
                    if state.may_live.get(local.0) {
                        self.diagnostic(ErrorKind::StorageAlreadyLive, root_place(*local));
                    }
                    state.must_live.set(local.0, true);
                    state.may_live.set(local.0, true);
                    let root = self.paths.roots[local.0];
                    self.initialize(state, root, false)?;
                    state.ever_written.range(
                        root,
                        self.paths.nodes[root].end,
                        false,
                        self.budget,
                    )?;
                }
                Statement::StorageDead(local) => {
                    self.check_access(
                        &Target {
                            precision: TargetPrecision::Exact,
                            place: CanonicalPlace::owned(&root_place(*local)),
                            authority: vec![],
                        },
                        Access::StorageEnd,
                    )?;
                    if let Some(report) = &mut self.report {
                        report.stats.storage_events += 1;
                    }
                    if !state.must_live.get(local.0) {
                        self.diagnostic(ErrorKind::StorageNotDefinitelyLive, root_place(*local));
                    }
                    state.must_live.set(local.0, false);
                    state.may_live.set(local.0, false);
                    self.initialize(state, self.paths.roots[local.0], false)?;
                }
                Statement::Assign { destination, value } => {
                    self.rvalue(state, value)?;
                    self.phase = Phase::Write;
                    self.write(state, destination)?;
                }
            }
        }
        self.statement = None;
        self.phase = Phase::Read;
        self.origin = self.f.blocks[self.block.0].terminator.origin;
        match &self.f.blocks[self.block.0].terminator.value {
            Terminator::Switch { condition, .. } => self.operand(state, condition)?,
            Terminator::SwitchVariant { place, .. } => {
                self.budget.spend(place.projection.len() + 1)?;
                let node = self.paths.resolve(place);
                if self.paths.nodes[node].variant_offsets.len() == 1 {
                    // A singleton pattern does not inspect storage or a tag.
                    // Payload bindings still perform their normal full reads.
                    self.authorize(state, place)?;
                } else {
                    self.read(state, place)?;
                }
            }
            Terminator::Call {
                args,
                destination,
                signature,
                ..
            } => {
                // All arguments have already been evaluated into snapshots.
                // Activate only this call's exact reservations, before moves or
                // mutable effects. Other activating loans are never exempted.
                if let Some(loans) = self.loans {
                    self.budget.spend(loans.activations(self.block).len())?;
                    for &id in loans.activations(self.block) {
                        for target in loans.issued(id, self.budget)? {
                            self.check_access(&target, Access::ActivateMut(id))?;
                        }
                    }
                }
                // Snapshot effects before consuming reference handles. A bounded
                // node bitmap deduplicates alias targets instead of retaining
                // an argument-count product of cloned canonical target vectors.
                let mut mutations = signature
                    .params
                    .iter()
                    .any(|ty| matches!(ty, Ty::Ref { .. }))
                    .then(|| Bits::new(self.paths.nodes.len()));
                if let Some(mutations) = &mutations {
                    self.budget.spend(mutations.0.len())?;
                }
                for (arg, ty) in args.iter().zip(&signature.params) {
                    if let Ty::Ref { kind, .. } = ty {
                        let (Operand::Copy(carrier) | Operand::Move(carrier)) = arg else {
                            unreachable!("validated reference argument")
                        };
                        let mut deref = carrier.clone();
                        deref.projection.push(Projection::Deref);
                        self.carrier_read(state, &deref)?;
                        for target in self.targets(&deref)? {
                            self.check_access(
                                &target,
                                if *kind == super::ir::BorrowKind::Mut {
                                    Access::MutBorrow
                                } else {
                                    Access::SharedBorrow
                                },
                            )?;
                            self.read_canonical(state, &target.place)?;
                            if *kind == super::ir::BorrowKind::Mut {
                                mutations
                                    .as_mut()
                                    .expect("reference call effects")
                                    .set(self.paths.resolve_canonical(&target.place), true);
                            }
                        }
                    }
                }
                if let Some(loans) = self.loans {
                    // Effect destinations were resolved from the shared ENTRY
                    // snapshot. A singleton address or RefMut type alone is not
                    // access permission: live children and parent authority are
                    // checked by the same transfer gate as source assignments.
                    for target in loans.call_write_targets(self.block, self.budget)? {
                        self.check_access(&target, Access::Write)?;
                        self.read_canonical(state, &target.place)?;
                    }
                }
                for arg in args {
                    self.operand(state, arg)?;
                }
                if let Some(mutations) = mutations {
                    self.budget.spend(self.paths.nodes.len())?;
                    for node in 0..self.paths.nodes.len() {
                        if mutations.get(node) {
                            self.kill_proofs(state, node)?;
                        }
                    }
                }
                self.phase = Phase::Write;
                self.write(state, destination)?;
            }
            Terminator::Return => {
                let mutable_reference = matches!(
                    self.f.result,
                    Ty::Ref {
                        kind: super::ir::BorrowKind::Mut,
                        ..
                    }
                );
                if !mutable_reference {
                    self.inspect_return(state)?;
                }
                if let Some(loans) = self.loans
                    && let Some(problem) =
                        loans.check_return(self.f, self.block, self.facts, self.budget)?
                {
                    self.problem(problem, root_place(self.f.return_local));
                }
                if mutable_reference {
                    self.operand(state, &Operand::Move(root_place(self.f.return_local)))?;
                }
            }
            Terminator::Goto(_) | Terminator::Unreachable => {}
        }
        Ok(())
    }
}
fn root_place(local: LocalId) -> Place {
    Place {
        local,
        projection: Vec::new(),
    }
}
fn successors(terminator: &Terminator) -> [Option<BlockId>; 2] {
    match terminator {
        Terminator::Goto(next) | Terminator::Call { next, .. } => [Some(*next), None],
        Terminator::Switch {
            then_block,
            else_block,
            ..
        }
        | Terminator::SwitchVariant {
            then_block,
            else_block,
            ..
        } => [Some(*then_block), Some(*else_block)],
        Terminator::Return | Terminator::Unreachable => [None, None],
    }
}

/// Validate every CFG block, then solve reachable states before diagnosing.
/// Structural errors in dead blocks are errors, but dead reads/moves are not.
/// Results are private transfer semantics, not a claim of Rust borrow checking.
pub fn analyze(function: &cfg::Function) -> Analysis {
    analyze_with_limits(function, Limits::default())
}

fn analyze_with_limits(f: &cfg::Function, limits: Limits) -> Analysis {
    analyze_with_facts(f, limits, FactsCtx::Concrete, None)
}
fn analyze_with_facts(
    f: &cfg::Function,
    limits: Limits,
    facts: FactsCtx<'_>,
    plan: Option<&super::ir::PlanBudget>,
) -> Analysis {
    analyze_with_context(f, limits, facts, plan, (None, None), None, &mut None)
}
fn analyze_with_context(
    f: &cfg::Function,
    limits: Limits,
    facts: FactsCtx<'_>,
    plan: Option<&super::ir::PlanBudget>,
    exact: (Option<&Arc<cfg::Function>>, Option<&Arc<ValidatedFacts>>),
    effect_table: Option<&Arc<CarrierEffectTable>>,
    effect_output: &mut Option<Arc<borrow::CarrierEffects>>,
) -> Analysis {
    let (exact_function, exact_facts) = exact;
    if let FactsCtx::Validated(authority) = facts {
        if authority.phase() == FactsPhase::Schema {
            return invalid_facts_analysis(
                f,
                "schema facts cannot analyze executable ownership",
                authority.budget(),
            );
        }
        if plan.is_none_or(|budget| !authority.budget().same_authority(budget)) {
            return invalid_facts_analysis(
                f,
                "ownership facts changed planning budget authority",
                authority.budget(),
            );
        }
    }
    let mut report = Report {
        stats: AnalysisStats {
            blocks_total: f.blocks.len(),
            locals: f.locals.len(),
            ..AnalysisStats::default()
        },
        diagnostics: Vec::new(),
        complete: true,
    };
    let result = (|| -> Result<(), Exhausted> {
        let scratch = preflight_with_plan(f, limits, plan, facts)?;
        let mut budget = Budget::for_plan(limits.work, plan);
        budget.state_limit = limits.state_bytes;
        let verification_scratch = budget.scratch(scratch)?;
        if let Err(error) = f.validate_with_facts(facts) {
            let resource = plan.map_or_else(
                || {
                    ["limit", "exhaust", "resource", "overflow", "poison"]
                        .iter()
                        .any(|term| error.contains(term))
                },
                |plan| super::traits::FactsError::from_message(plan, &error).is_resource(),
            );
            if resource {
                return Err(Exhausted);
            }
            report.diagnostics.push(Diagnostic {
                kind: ErrorKind::InvalidCfg(error),
                block: f.entry,
                statement: None,
                origin: None,
                place: None,
            });
            return Ok(());
        }
        drop(verification_scratch);
        let paths = Paths::with_facts(f, limits.paths, facts, &mut budget)?;
        report.stats.move_paths = paths.nodes.len();
        // Account for dense retained entry states and transient states as well
        // as Vec headers. No allocation proportional to blocks*paths precedes
        // this checked bound.
        let bytes = ownership_state_bytes(f.locals.len(), paths.nodes.len(), f.blocks.len())?;
        if bytes > limits.state_bytes {
            return Err(Exhausted);
        }
        budget.reserve_bytes(bytes)?;
        let mut seed = State::new(f.locals.len(), paths.nodes.len());
        let loans = if let (Some(function), Some(authority)) = (exact_function, exact_facts) {
            Some(borrow::Plan::build_validated(
                function,
                authority,
                effect_table,
                &mut budget,
                limits.state_bytes - bytes,
            )?)
        } else {
            Some(borrow::Plan::build_with_facts(
                f,
                facts,
                &mut budget,
                limits.state_bytes - bytes,
            )?)
        };
        *effect_output = loans.as_ref().and_then(|loans| loans.effects().cloned());
        if let Some(loans) = &loans {
            if bytes.checked_add(loans.bytes).ok_or(Exhausted)? > limits.state_bytes {
                return Err(Exhausted);
            }
            for (block, statement, problem) in &loans.problems {
                if report.diagnostics.len() >= limits.diagnostics {
                    return Err(Exhausted);
                }
                let kind = match problem {
                    Problem::Conflict(message) => ErrorKind::BorrowConflict(message.clone()),
                    Problem::Invalid(message) => ErrorKind::InvalidReference(message.clone()),
                    Problem::Unsupported(message) => {
                        ErrorKind::UnsupportedBorrowing(message.clone())
                    }
                };
                report.diagnostics.push(Diagnostic {
                    kind,
                    block: *block,
                    statement: *statement,
                    origin: None,
                    place: None,
                });
            }
        }
        for local in &f.params {
            seed.must_live.set(local.0, true);
            seed.may_live.set(local.0, true);
            let root = paths.roots[local.0];
            seed.must_init
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            seed.may_init
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            seed.ever_written
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            for (_, root) in &paths.external[local.0] {
                let root = *root;
                seed.must_init
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
                seed.may_init
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
                seed.ever_written
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
            }
        }
        let mut entries: Vec<Option<State>> = (0..f.blocks.len()).map(|_| None).collect();
        entries[f.entry.0] = Some(seed);
        report.stats.blocks_reachable = 1;
        let mut queue = VecDeque::from([f.entry]);
        let mut queued = vec![false; f.blocks.len()];
        queued[f.entry.0] = true;
        while let Some(block) = queue.pop_front() {
            queued[block.0] = false;
            report.stats.worklist_pops += 1;
            budget.spend(1)?;
            let input = entries[block.0].as_ref().expect("queued reachable block");
            budget.spend(input.words())?;
            let mut output = input.clone();
            Transfer {
                facts,
                f,
                paths: &paths,
                budget: &mut budget,
                report: None,
                block,
                statement: None,
                origin: None,
                diagnostic_limit: limits.diagnostics,
                loans: loans.as_ref(),
                phase: Phase::Read,
            }
            .block(&mut output)?;
            for target in successors(&f.blocks[block.0].terminator.value)
                .into_iter()
                .enumerate()
            {
                let (edge, Some(target)) = target else {
                    continue;
                };
                let previous = if edge == 0 {
                    if let Terminator::SwitchVariant { place, variant, .. } =
                        &f.blocks[block.0].terminator.value
                    {
                        budget.spend(place.projection.len() + 1)?;
                        let node = paths.resolve(place);
                        let old = output.selected[node];
                        output.selected[node] = variant.index;
                        Some((node, old))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let changed = if let Some(entry) = &mut entries[target.0] {
                    entry.merge(&output, &mut budget)?
                } else {
                    budget.spend(output.words())?;
                    entries[target.0] = Some(output.clone());
                    report.stats.blocks_reachable += 1;
                    true
                };
                if let Some((node, old)) = previous {
                    output.selected[node] = old;
                }
                if changed {
                    report.stats.state_changes += 1;
                    if !queued[target.0] {
                        queue.push_back(target);
                        queued[target.0] = true;
                    }
                }
            }
        }
        // Diagnostic order and semantic counters do not depend on queue order.
        for (index, entry) in entries.iter().enumerate() {
            if let Some(entry) = entry {
                report.stats.edges_reachable += successors(&f.blocks[index].terminator.value)
                    .iter()
                    .flatten()
                    .count();
                budget.spend(entry.words())?;
                let mut state = entry.clone();
                Transfer {
                    facts,
                    f,
                    paths: &paths,
                    budget: &mut budget,
                    report: Some(&mut report),
                    block: BlockId(index),
                    statement: None,
                    origin: None,
                    diagnostic_limit: limits.diagnostics,
                    loans: loans.as_ref(),
                    phase: Phase::Read,
                }
                .block(&mut state)?;
                if matches!(
                    f.blocks[index].terminator.value,
                    Terminator::SwitchVariant { .. }
                ) {
                    report.stats.variant_refinements += 1;
                }
            }
        }
        Ok(())
    })();
    if result.is_err() || !report.complete {
        report.complete = false;
        if report.diagnostics.len() >= limits.diagnostics && !report.diagnostics.is_empty() {
            report.diagnostics.pop();
            report.stats.diagnostics_suppressed += 1;
        }
        report.diagnostics.push(Diagnostic {
            kind: ErrorKind::ResourceLimit,
            block: f.entry,
            statement: None,
            origin: None,
            place: None,
        });
    }
    Analysis {
        stats: report.stats,
        diagnostics: report.diagnostics.into_boxed_slice(),
        complete: report.complete,
    }
}

#[cfg(test)]
mod tests {

    // Retain the real checked source-name owner alongside its module arena.
    fn fixture_modules() -> (
        std::sync::Arc<super::super::semantic_name::NameStore>,
        std::sync::Arc<super::super::module_identity::ModuleArena>,
    ) {
        use super::super::expansion_budget::ExpansionBudget;
        use super::super::module_identity::ModuleArenaBuilder;
        use super::super::semantic_name::NameStoreBuilder;
        use super::super::token::Edition;
        use super::super::token_session::{SourceSnapshot, TokenSession};
        let budget = ExpansionBudget::default();
        let sources =
            SourceSnapshot::from_text(std::path::Path::new("module-fixture.rs"), "", &budget)
                .unwrap();
        let session = TokenSession::new(sources, budget).unwrap();
        let names = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let modules = ModuleArenaBuilder::for_source(&names)
            .unwrap()
            .freeze()
            .unwrap();
        (names.freeze().unwrap(), modules)
    }
    #[test]
    fn budget_telemetry_local_work_rejection_preserves_plan_and_callsite() {
        use super::super::ir::{PlanBudget, ResourceBudgetKind};
        let plan = PlanBudget::default();
        let mut budget = Budget::for_plan(3, Some(&plan));
        budget.spend(2).unwrap();
        let line = line!() + 1;
        assert!(budget.spend(2).is_err());
        let first = plan.first_resource_failure().unwrap();
        assert_eq!(first.kind, ResourceBudgetKind::OwnershipWork);
        assert_eq!((first.requested, first.used, first.limit), (2, 2, 3));
        assert_eq!((first.file, first.line), (file!(), line));
        assert_eq!(budget.remaining, 1);
        assert_eq!(plan.usage_snapshot().work, 2);
        assert!(budget.spend(usize::MAX).is_err());
        assert_eq!(plan.first_resource_failure(), Some(first));
    }

    #[test]
    fn budget_telemetry_local_storage_rejection_counts_and_releases_reservations() {
        use super::super::ir::{PlanBudget, PlanReservation, ResourceBudgetKind};
        let plan = PlanBudget::default();
        let mut budget = Budget::for_plan(100, Some(&plan));
        let overhead = 4 * std::mem::size_of::<PlanReservation>();
        budget.state_limit = overhead + 9;
        budget.reserve_bytes(1).unwrap();
        let before = plan.usage_snapshot();
        assert_eq!(before.reservations.reservations, 1);
        assert_eq!(before.reservations.current_bytes, overhead + 1);
        let line = line!() + 1;
        assert!(budget.scratch(10).is_err());
        let failure = plan.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::OwnershipStateBytes);
        assert_eq!(
            (failure.requested, failure.used, failure.limit),
            (10, overhead + 1, overhead + 9)
        );
        assert_eq!((failure.file, failure.line), (file!(), line));
        assert_eq!(plan.usage_snapshot().reservations, before.reservations);
        assert_eq!(budget.reserved_bytes(), overhead + 1);
        drop(budget);
        let after = plan.usage_snapshot();
        assert_eq!(after.reservations.current_bytes, 0);
        assert_eq!(after.reservations.current_reservations, 0);
        assert_eq!(after.reservations.cumulative_bytes, overhead + 1);
        assert_eq!(after.first_failure, Some(failure));
    }

    #[test]
    fn budget_telemetry_delegated_plan_rejection_keeps_original_kind_and_site() {
        use super::super::ir::{PlanBudget, PlanLimits, ResourceBudgetKind};
        let plan = PlanBudget::new(PlanLimits {
            work: 1,
            ..PlanLimits::default()
        });
        let mut budget = Budget::for_plan(10, Some(&plan));
        let line = line!() + 1;
        assert!(budget.spend(2).is_err());
        let failure = plan.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Work);
        assert_eq!((failure.requested, failure.used, failure.limit), (2, 0, 1));
        assert_eq!((failure.file, failure.line), (file!(), line));
        assert_eq!(budget.remaining, 10);

        let plan = PlanBudget::new(PlanLimits {
            nodes: 0,
            ..PlanLimits::default()
        });
        let mut budget = Budget::for_plan(10, Some(&plan));
        let line = line!() + 1;
        assert!(budget.nodes(1).is_err());
        let failure = plan.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Nodes);
        assert_eq!((failure.file, failure.line), (file!(), line));
    }

    #[test]
    fn budget_telemetry_storage_arithmetic_overflow_has_exact_operands() {
        use super::super::ir::{PlanBudget, PlanReservation, ResourceBudgetKind};
        let plan = PlanBudget::default();
        let mut budget = Budget::for_plan(10, Some(&plan));
        let line = line!() + 1;
        assert!(budget.reserve_bytes(usize::MAX).is_err());
        let failure = plan.first_resource_failure().unwrap();
        assert_eq!(
            failure.kind,
            ResourceBudgetKind::OwnershipReservationStorage
        );
        assert_eq!(
            failure.requested,
            4 * std::mem::size_of::<PlanReservation>()
        );
        assert_eq!(failure.used, usize::MAX);
        assert_eq!(failure.limit, budget.state_limit);
        assert!(failure.arithmetic_overflow);
        assert_eq!((failure.file, failure.line), (file!(), line));
        assert_eq!(budget.reserved_bytes(), 0);
        assert_eq!(plan.usage_snapshot().reservations.reservations, 0);
    }

    fn input_reference(param: usize) -> super::super::ir::InputReference {
        super::super::ir::InputReference {
            param,
            position: super::ReferencePosition { depth: 0 },
        }
    }
    fn first_return(
        contract: &mut std::sync::Arc<super::super::ir::RefContract>,
    ) -> &mut super::super::ir::ResultReference {
        &mut std::sync::Arc::make_mut(&mut std::sync::Arc::make_mut(contract).results)[0]
    }
    use super::super::ir::{
        DeclId, EnumDef, EnumTy, FieldDef, IntTy, IntValue, LocalDecl, NominalAppId, SnapshotId,
        StructDef, StructTy, VariantDef, VariantId, VariantShape,
    };
    use super::super::traits::test_support::source_decl;
    use super::*;
    use cfg::{BasicBlock, Constant, Located};

    #[test]
    fn recipe_preflight_shared_kernels_include_retained_and_attempted_targets() {
        let counts = RecipePlanCounts {
            binders: 1,
            expressions: 12,
            locals: 8,
            blocks: 3,
            points: 16,
            carrier_slots: 4,
            loan_sites: 2,
            formal_origins: 2,
            call_result_origins: 1,
            path_nodes: 11,
            variant_offsets: 2,
            operations: 20,
            origin_memberships: 7,
            parent_memberships: 3,
            lifetime_dependency_memberships: 2,
            target_memberships: 5,
            target_allocations: 6,
            target_projection_steps: 4,
            target_authority_entries: 5,
            target_overhead_words: 16,
            ir_retained_bytes: 1000,
            cfg_retained_bytes: 2000,
            construction_peak_bytes: 4000,
            verification_scratch_bytes: 5000,
            work_upper_bound: 1000,
        };
        let budget = super::super::ir::PlanBudget::default();
        let RecipeAdmission::Available(footprint) =
            preflight_recipe_counts(&counts, &budget).unwrap()
        else {
            panic!("bounded count fixture must be available")
        };
        assert_eq!(footprint.origins, 5);
        assert_eq!(footprint.memberships, 17);
        assert_eq!(
            footprint.ownership_bytes,
            ownership_state_bytes(8, 11, 3).unwrap()
        );
        assert_eq!(footprint.path_bytes, paths_storage_bytes(8, 11, 2).unwrap());
        assert_eq!(
            footprint.borrow_bytes,
            borrow::plan_storage_bytes(8, 3, 16, 4, 5).unwrap()
                + borrow::target_payload_bytes(6, 4, 5, 16).unwrap()
                + borrow::region_storage_bytes(counts.formal_origins + counts.binders + 2).unwrap()
                    * (2 * counts.call_result_origins + 1)
        );
        assert_eq!(
            footprint.state_bytes,
            footprint.ownership_bytes + footprint.borrow_bytes
        );
        assert_eq!(
            footprint.peak_bytes,
            3000 + (footprint.path_bytes + footprint.state_bytes).max(5000)
        );
        let mut additional_attempt = counts;
        additional_attempt.target_allocations += 1;
        let RecipeAdmission::Available(larger) =
            preflight_recipe_counts(&additional_attempt, &budget).unwrap()
        else {
            panic!("one additional target attempt remains bounded")
        };
        assert_eq!(
            larger.borrow_bytes - footprint.borrow_bytes,
            borrow::target_payload_bytes(1, 0, 0, 0).unwrap()
        );
        additional_attempt.construction_peak_bytes = larger.peak_bytes + 1000;
        let RecipeAdmission::Available(construction) =
            preflight_recipe_counts(&additional_attempt, &budget).unwrap()
        else {
            panic!("retained metadata plus construction scratch remains bounded")
        };
        assert_eq!(construction.peak_bytes, 4000 + larger.peak_bytes);
    }

    #[test]
    fn recipe_preflight_absolute_limits_count_all_origins_memberships_and_peaks() {
        let base = RecipePlanCounts {
            locals: 1,
            blocks: 1,
            points: 1,
            path_nodes: 1,
            ..Default::default()
        };
        let budget = super::super::ir::PlanBudget::default();
        let mut array = base;
        array.loan_sites = 4096;
        array.formal_origins = 1;
        assert!(matches!(
            preflight_recipe_counts(&array, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(RecipeLimit::Bound {
                required: 4097,
                limit: 4096,
                ..
            })
        ));
        let mut tuple = base;
        tuple.binders = 33;
        assert!(matches!(
            preflight_recipe_counts(&tuple, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(RecipeLimit::Bound {
                required: 33,
                limit: 32,
                ..
            })
        ));
        let mut memberships = base;
        memberships.origin_memberships = 250_000;
        memberships.parent_memberships = 250_000;
        memberships.lifetime_dependency_memberships = 250_000;
        memberships.target_memberships = 250_001;
        assert!(matches!(
            preflight_recipe_counts(&memberships, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(RecipeLimit::Bound {
                required: 1_000_001,
                limit: 1_000_000,
                ..
            })
        ));
        let mut peak = base;
        peak.ir_retained_bytes = super::super::ir::PlanLimits::default().bytes;
        assert!(matches!(
            preflight_recipe_counts(&peak, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(_)
        ));
        let mut overflow = base;
        overflow.target_allocations = usize::MAX;
        overflow.loan_sites = 1;
        assert_eq!(
            preflight_recipe_counts(&overflow, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(RecipeLimit::Overflow)
        );
        let mut dense = base;
        dense.locals = 65_536;
        dense.blocks = 16_384;
        dense.points = 16_384;
        dense.path_nodes = 131_072;
        assert!(matches!(
            preflight_recipe_counts(&dense, &budget).unwrap(),
            RecipeAdmission::IntrinsicLimit(_)
        ));
    }

    #[test]
    fn recipe_preflight_ledger_exhaustion_is_error_not_descriptor_unavailability() {
        use super::super::ir::{PlanBudget, PlanLimits};
        let counts = RecipePlanCounts {
            locals: 1,
            blocks: 1,
            points: 1,
            path_nodes: 1,
            ..Default::default()
        };
        let small = PlanBudget::new(PlanLimits {
            bytes: 1,
            ..Default::default()
        });
        assert!(matches!(
            preflight_recipe_counts(&counts, &small).unwrap(),
            RecipeAdmission::Available(_)
        ));
        // Admission did not reserve the unbuilt recipe's footprint.
        let reservation = small.reserve_bytes(1).unwrap();
        assert!(matches!(
            preflight_recipe_counts(&counts, &small).unwrap(),
            RecipeAdmission::Available(_)
        ));
        drop(reservation);
        let exhausted = PlanBudget::new(PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(
            preflight_recipe_counts(&counts, &exhausted)
                .unwrap_err()
                .is_resource()
        );
        let mut intrinsically_large = counts;
        intrinsically_large.binders = 33;
        assert!(
            preflight_recipe_counts(&intrinsically_large, &exhausted)
                .unwrap_err()
                .is_resource()
        );
        assert!(matches!(
            preflight_recipe_counts(&intrinsically_large, &PlanBudget::default()).unwrap(),
            RecipeAdmission::IntrinsicLimit(_)
        ));
        let malformed = RecipePlanCounts {
            blocks: 1,
            ..Default::default()
        };
        assert_eq!(
            preflight_recipe_counts(&malformed, &PlanBudget::default())
                .unwrap_err()
                .kind(),
            super::super::traits::FactsErrorKind::InvalidMetadata
        );
    }

    fn borrowed_source(source: &str) -> cfg::Function {
        let program = super::super::frontend::parse(&format!("{source} fn main(){{}}")).unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        let checked =
            super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                .unwrap();
        cfg::lower(&checked).unwrap()
    }

    fn fixture_world(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        env: Arc<GenericEnv>,
        signature: super::super::ir::Signature,
        budget: &super::super::ir::PlanBudget,
    ) -> Result<(DeclId, Arc<super::super::traits::ValidatedTraitWorld>), String> {
        use super::super::traits::{CallableHeader, GenericSchema};
        let declaration = env.scopes().last().unwrap().binder().declaration;
        let header = CallableHeader {
            declaration,
            schema: GenericSchema {
                env,
                signature,
                predicates: Arc::from([]),
            },
        };
        let world = super::super::traits::test_support::world(modules.clone(), &[header], budget)
            .map_err(|error| error.to_string())?;
        Ok((declaration, world))
    }

    fn lower_fixture_template(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        source: Arc<CheckedFunction>,
        env: Arc<GenericEnv>,
        budget: &super::super::ir::PlanBudget,
    ) -> Result<cfg::TemplateCfg, String> {
        let signature = super::super::ir::Signature {
            params: source
                .params
                .iter()
                .map(|id| source.locals[id.0].ty.clone())
                .collect(),
            result: source.result.clone(),
            ref_contract: source.ref_contract.clone(),
        };
        let (declaration, world) = fixture_world(modules, env, signature, budget)?;
        let facts = world
            .callable_facts(declaration)
            .map_err(|error| error.to_string())?;
        cfg::lower_template(source, facts, budget)
    }

    fn close_fixture(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        function: Arc<cfg::Function>,
        budget: &super::super::ir::PlanBudget,
    ) -> Result<cfg::ClosedCfg, String> {
        use super::super::ir::{BinderId, CallTarget, ParamEnv, Signature};
        let declaration =
            source_decl(SnapshotId::allocate()?, 0, budget).map_err(|error| error.to_string())?;
        let scope = Arc::new(ParamEnv::new(BinderId { declaration }, 0)?);
        let env = Arc::new(GenericEnv::new(&[scope], budget)?);
        let signature = Signature {
            params: function
                .params
                .iter()
                .map(|id| function.locals[id.0].ty.clone())
                .collect(),
            result: function.result.clone(),
            ref_contract: function.ref_contract.clone(),
        };
        let (_, world) = fixture_world(modules, env, signature, budget)?;
        let facts = world
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([]),
            })
            .map_err(|error| error.to_string())?;
        cfg::close(function, facts, budget)
    }

    fn trait_callable_fixture(
        source: &str,
        name: &str,
        budget: &super::super::ir::PlanBudget,
    ) -> (Arc<CheckedFunction>, Arc<ValidatedFacts>) {
        use super::super::frontend;
        let program = frontend::parse(source).unwrap();
        let plan = frontend::PlanCtx::from_program(&program, budget).unwrap();
        let declaration = (0..plan.callable_count())
            .map(|index| plan.callable_decl(index).unwrap())
            .filter(|declaration| {
                matches!(
                    plan.callable_provider(*declaration).unwrap(),
                    frontend::CallableProvider::Source { .. }
                )
            })
            .find(|declaration| {
                let path = &plan.callable_body(&program, *declaration).unwrap().name;
                path.anchor() == super::super::semantic_path::PathAnchor::Crate
                    && path.len() == 1
                    && program.names.spelling(path.segments()[0]).unwrap() == name
            })
            .unwrap();
        let checked = frontend::type_callable_with_plan(&program, declaration, &plan, budget)
            .unwrap_or_else(|error| panic!("{name} in {source}: {error}"));
        let facts = plan.callable_facts(declaration).unwrap();
        // Facts must retain their immutable world and structural adapter after
        // both the source program and the frontend planning handle are dropped.
        (Arc::new(checked), facts)
    }

    fn required_item(source: &CheckedFunction) -> DeclId {
        source
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                super::super::ir::ExprKind::Call {
                    target: super::super::ir::Callee::TraitItem(call),
                    ..
                } => Some(call.item),
                _ => None,
            })
            .unwrap()
    }

    fn reference_template(source: &str, name: &str) -> cfg::TemplateCfg {
        let budget = super::super::ir::PlanBudget::default();
        let (checked, facts) = trait_callable_fixture(source, name, &budget);
        cfg::lower_template(checked, facts, &budget).unwrap()
    }

    fn carrier_reference_template(source: &str) -> cfg::TemplateCfg {
        reference_template(&format!("{source} fn main(){{}}"), "f")
    }

    #[test]
    fn generic_carriers_universal_families_owned_writes_and_deep_results() {
        for source in [
            "fn f<T>(x:T)->T{x}",
            "fn f<T>(d:&mut T,x:T){*d=x;}",
            "fn f<T>(d:&mut T,x:T,y:T)->T{*d=x;y}",
            "fn f<'a,'b,'c>(p:&'a &'b &'c i32)->&'a &'b &'c i32{p}",
            "fn f<'a,'b>(d:&mut &'a mut &'b mut i32,x:&'a mut &'b mut i32){*d=x;}",
        ] {
            let proof = verify_template(carrier_reference_template(source))
                .unwrap_or_else(|analysis| panic!("{source}: {:?}", analysis.diagnostics));
            assert!(proof.analysis().passed());
            assert!(proof.carrier_effects().is_some(), "{source}");
        }
    }

    #[test]
    fn generic_carriers_rigid_extent_dead_calls_and_mutable_parent_lifetimes() {
        for source in [
            "fn id<T>(x:T)->T{x} fn f<'a>(){let x=1;id::<&'a i32>(&x);}",
            "fn f<'a,'b>(p:&'a mut &'b mut i32)->&'b mut i32{*p}",
        ] {
            let program =
                super::super::frontend::parse(&format!("{source} fn main(){{}}")).unwrap();
            let error = super::super::frontend::check(&program)
                .expect_err("invalid carrier source must fail semantic checking");
            assert!(
                !error.contains("limit") && !error.contains("exhaust"),
                "{source}: {error}"
            );
        }
        let source = "fn g<T>(d:&mut T,x:T,y:T)->T{*d=x;y} fn f<'a>(d:&mut &'a mut i32,x:&'a mut i32,y:&'a mut i32){return;g::<&'a mut i32>(d,x,y);}";
        let analysis = verify_template(carrier_reference_template(source)).unwrap_err();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.kind, ErrorKind::UnsupportedBorrowing(_))),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn generic_carriers_exact_capsules_and_combined_resource_denial() {
        let template = carrier_reference_template(
            "fn id<T>(x:T)->T{x} fn f<'a>(x:&'a i32)->&'a i32{id::<&'a i32>(x)}",
        );
        let mut forged = template.function().as_ref().clone();
        forged.lowering_proof = None;
        let analysis = analyze_with_facts(
            &forged,
            Limits::default(),
            FactsCtx::Validated(template.facts()),
            Some(template.budget()),
        );
        assert!(!analysis.passed());
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.kind, ErrorKind::InvalidCfg(_)))
        );
        let ledger = super::super::ir::PlanBudget::default();
        let mut budget = Budget::for_plan(usize::MAX, Some(&ledger));
        budget.state_limit = 63;
        assert!(budget.reserve_bytes(64).is_err());
        let mut budget = Budget::for_plan(usize::MAX, Some(&ledger));
        budget.state_limit = 4096;
        budget.reserve_bytes(3000).unwrap();
        assert!(budget.scratch(1097).is_err());
        let mut budget = Budget::for_plan(usize::MAX, Some(&ledger));
        for _ in 0..17 {
            budget.reserve_bytes(0).unwrap();
        }
        assert!(
            budget.reserved_bytes()
                >= budget.reservations.capacity()
                    * std::mem::size_of::<super::super::ir::PlanReservation>()
        );
        let exhausted = super::super::ir::PlanBudget::new(super::super::ir::PlanLimits {
            bytes: 0,
            ..super::super::ir::PlanLimits::default()
        });
        assert!(CarrierEffectTable::from_verified(&[], &exhausted).is_err());
    }

    #[test]
    fn nested_references_positioned_identity_inner_load_and_generic_calls() {
        for source in [
            "fn f<'a,'b>(p:&'a &'b i32)->&'b i32{*p}",
            "fn f<'a,'b>(p:&'a &'b i32)->&'a &'b i32{p}",
            "fn f<'a,'b>(p:&'a &'b i32)->&'b i32{inner(p)} fn inner<'a,'b>(p:&'a &'b i32)->&'b i32{*p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let f = borrowed_source("fn f<'a,'b>(p:&'a &'b i32)->&'a &'b i32{p}");
        let paths = Paths::new(&f, Limits::default().paths).unwrap();
        assert_ne!(
            paths.external[f.params[0].0][0],
            paths.external[f.params[0].0][1]
        );
        let mut budget = Budget::new(Limits::default().work);
        let plan = borrow::Plan::build(&f, &mut budget, Limits::default().state_bytes).unwrap();
        let block = BlockId(
            f.blocks
                .iter()
                .position(|b| matches!(b.terminator.value, Terminator::Return))
                .unwrap(),
        );
        for depth in 0..2 {
            let place = Place {
                local: f.return_local,
                projection: vec![Projection::Deref; depth + 1],
            };
            let targets = plan
                .resolve(&place, block, None, Phase::Read, &mut budget)
                .unwrap()
                .unwrap();
            assert!(!targets.is_empty());
            assert!(targets.iter().all(|t| t.place.root
                == CanonicalRoot::ParameterReferent(
                    f.params[0],
                    borrow::fixed_location(ReferencePosition { depth: depth as u8 })
                )));
        }
        let source = "fn inner<'a,'b,T>(p:&'a &'b T)->&'b T{*p} fn f<'a,'b,T>(p:&'a &'b T)->&'b T{inner(p)} fn main(){}";
        let template = reference_template(source, "f");
        assert!(verify_template(template).is_ok());
    }

    #[test]
    fn nested_references_snapshots_last_use_and_original_referent_conflicts() {
        for source in [
            "fn f()->i32{let mut x=1;let r:&i32;{let q=&x;let rr=&q;r=*rr;}let n=*r;x=2;n+x}",
            "fn f()->i32{let x=1;let y=2;let mut r=&x;let rr=&r;let s=*rr;r=&y;*s+*r}",
            "fn f(b:bool)->i32{let x=1;let y=2;let a=&x;let c=&y;let rr=if b{&a}else{&c};let s=*rr;*s}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        for source in [
            "fn f()->i32{let mut x=1;let r=&x;let rr=&r;let s=*rr;x=2;*s}",
            "fn f()->i32{let x=1;let y=2;let mut r=&x;let rr=&r;r=&y;**rr}",
            "fn f()->i32{let mut x=1;let r=&x;let rr=&r;let s=*rr;let m=&mut x;*m=2;*s}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete, "{source}: {:?}", result.diagnostics);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn nested_references_forged_dead_positions_and_dense_budget_fail_closed() {
        let f = borrowed_source("fn f<'a,'b>(p:&'a &'b i32)->&'a &'b i32{p}");
        for input in [false, true] {
            let mut forged = f.clone();
            let contract = Arc::make_mut(&mut forged.ref_contract);
            if input {
                Arc::make_mut(&mut contract.input_regions)[1]
                    .input
                    .position
                    .depth = 0;
            } else {
                Arc::make_mut(&mut contract.results)[1].position.depth = 0;
            }
            let result = analyze(&forged);
            assert!(result.complete);
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
        }
        let mut dead = borrowed_source(
            "fn f<'a,'b>(p:&'a &'b i32)->&'b i32{inner(p)} fn inner<'a,'b>(p:&'a &'b i32)->&'b i32{*p}",
        );
        let mut terminator = dead
            .blocks
            .iter()
            .find(|b| matches!(b.terminator.value, Terminator::Call { .. }))
            .unwrap()
            .terminator
            .clone();
        let Terminator::Call { signature, .. } = &mut terminator.value else {
            unreachable!()
        };
        first_return(&mut signature.ref_contract).position.depth = 1;
        dead.blocks.push(BasicBlock {
            statements: vec![],
            terminator,
        });
        let result = analyze(&dead);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
        let mut budget = Budget::new(Limits::default().work);
        assert!(borrow::Plan::build(&f, &mut budget, 0).is_err());
        let mut budget = Budget::new(Limits::default().work);
        let plan = borrow::Plan::build(&f, &mut budget, Limits::default().state_bytes).unwrap();
        let mut budget = Budget::new(Limits::default().work);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        let template =
            reference_template("fn f<'a,'b>(p:&'a &'b i32)->&'b i32{*p} fn main(){}", "f");
        let proof = verify_template(template).unwrap();
        let foreign = super::super::ir::PlanBudget::default();
        let result = analyze_with_facts(
            proof.function(),
            Limits::default(),
            FactsCtx::Validated(proof.facts()),
            Some(&foreign),
        );
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn generic_references_two_phase_activation_explicit_borrows_and_loop_reset() {
        for source in [
            "struct Cell{n:i32} impl Cell{fn read(&self)->i32{self.n} fn bump(&mut self,n:i32){self.n=self.n+n;}} fn main(){let mut s=Cell{n:1};s.bump(s.read());}",
            "fn read(p:&i32)->i32{*p} fn set(p:&mut i32,n:i32){*p=n;} fn main(){let mut n=1;let r=&mut n;set(r,read(r));*r=3;}",
            "struct Cell{n:i32} impl Cell{fn read(&self)->i32{self.n} fn bump(&mut self,n:i32){self.n=self.n+n;}} fn main(){let mut s=Cell{n:1};let mut i=0;while i<2{s.bump(s.read());i=i+1;}}",
            "fn read(p:&i32)->i32{*p} fn take(p:&mut i32,n:i32)->&mut i32{*p=n;p} fn main(){let mut n=1;let r=&mut n;let q=take(r,read(r));let v=*q;*r=v;}",
        ] {
            let template = reference_template(source, "main");
            assert!(
                template
                    .function()
                    .loan_sites
                    .iter()
                    .any(|s| matches!(s.timing, cfg::LoanTiming::TwoPhase { .. }))
            );
            let proof = verify_template(template)
                .unwrap_or_else(|failure| panic!("{source}: {:?}", failure.diagnostics));
            assert!(proof.analysis().passed());
        }
        for source in [
            "struct Cell{n:i32} impl Cell{fn read(&self)->i32{self.n} fn bump(&mut self,n:i32){self.n=n;}} fn main(){let mut s=Cell{n:1};Cell::bump(&mut s,s.read());}",
            "fn read(p:&i32)->i32{*p} fn set(p:&mut i32,n:i32){*p=n;} fn main(){let mut n=1;let r=&mut n;set(&mut *r,read(r));}",
            "fn set(p:&mut i32,n:i32){*p=n;} fn main(){let mut n=1;let r=&mut n;set(r,{*r=2;3});}",
            "fn two(a:&mut i32,b:&mut i32){} fn main(){let mut n=1;let r=&mut n;two(r,r);}",
            "fn both(a:&mut i32,b:&i32){} fn main(){let mut n=1;let r=&mut n;both(r,r);}",
            "fn read(p:&i32)->i32{*p} fn take(p:&mut i32,n:i32)->&mut i32{*p=n;p} fn main(){let mut n=1;let r=&mut n;let q=take(r,read(r));*r=3;let v=*q;}",
        ] {
            let failure = verify_template(reference_template(source, "main")).unwrap_err();
            assert!(failure.complete, "{source}: {:?}", failure.diagnostics);
            assert!(
                failure
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
                "{source}: {:?}",
                failure.diagnostics
            );
        }
    }

    #[test]
    fn generic_references_subtree_views_preserve_input_anchors_and_exact_authority() {
        use super::super::ir::{InputEnvelope, ReturnContract};
        let source = "struct Inner{a:i32,b:i32} struct Outer{pad:bool,inner:Inner} trait View{fn view(&self)->&Inner;} impl View for Outer{fn view(&self)->&Inner{&self.inner}} fn forward<T:View>(x:&T)->&i32{let v=x.view();&v.b} fn main(){}";
        let template = reference_template(source, "forward");
        assert!(
            matches!(&template.function().ref_contract.results[0].returns, ReturnContract::SharedInputEnvelopes(inputs) if inputs.as_ref() == [InputEnvelope { input: input_reference(0) }])
        );
        let proof = verify_template(template).unwrap();
        let f = proof.function();
        let mut budget = Budget::for_plan(Limits::default().work, Some(proof.budget()));
        let plan = borrow::Plan::build(f, &mut budget, Limits::default().state_bytes).unwrap();
        let last = cfg::LoanId(f.loan_sites.len() - 1);
        let targets = plan.issued(last, &mut budget).unwrap();
        assert!(!targets.is_empty());
        assert!(
            targets
                .iter()
                .all(|t| t.precision == TargetPrecision::Subtree
                    && t.place.root
                        == CanonicalRoot::ParameterReferent(
                            f.params[0],
                            borrow::fixed_location(ReferencePosition { depth: 0 })
                        )
                    && t.place.projection.is_empty())
        );
        // `Inner.b` is not `Outer` field1, nor a field of opaque T. The
        // conservative input anchor must not acquire the returned view suffix.
        let mut forged = f.as_ref().clone();
        first_return(&mut forged.ref_contract).returns = ReturnContract::SharedInputEnvelopes(
            vec![InputEnvelope {
                input: input_reference(1),
            }]
            .into(),
        );
        let failure = analyze_with_facts(
            &forged,
            Limits::default(),
            FactsCtx::Validated(proof.facts()),
            Some(proof.budget()),
        );
        assert!(failure.complete);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_)))
        );
        for source in [
            "fn identity<T>(x:&T)->&T{x} fn main(){}",
            "fn identity<T>(x:&mut T)->&mut T{x} fn main(){}",
            "fn identity<T>(x:&(T,[T;0]))->&(T,[T;0]){x} fn main(){}",
        ] {
            assert!(
                verify_template(reference_template(source, "identity"))
                    .unwrap()
                    .analysis()
                    .passed()
            );
        }
        for scalar in ["i32", "bool"] {
            let source = format!(
                "fn id<T>(x:&T)->&T{{x}} fn wrap(x:&{scalar})->&{scalar}{{id(x)}} fn main(){{}}"
            );
            let proof = verify_template(reference_template(&source, "wrap")).unwrap();
            assert!(proof.analysis().passed());
            let mut mismatched = proof.function().as_ref().clone();
            mismatched.result = Ty::Ref {
                kind: super::super::ir::BorrowKind::Shared,
                referent: Arc::new(if scalar == "i32" {
                    Ty::Bool
                } else {
                    Ty::Int(IntTy::I32)
                }),
            };
            assert!(
                !analyze_with_facts(
                    &mismatched,
                    Limits::default(),
                    FactsCtx::Validated(proof.facts()),
                    Some(proof.budget())
                )
                .passed()
            );
        }
        // General aggregate footprint refinement is not granted by the leaf
        // proof, and remains a legal unsupported boundary, not invalid Rust.
        let aggregate =
            "fn id<T>(x:&T)->&T{x} fn wrap(x:&(i32,bool))->&(i32,bool){id(x)} fn main(){}";
        let failure = verify_template(reference_template(aggregate, "wrap")).unwrap_err();
        assert!(failure.complete);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::UnsupportedBorrowing(_)))
        );
    }

    #[test]
    fn generic_references_dead_activation_metadata_and_dense_resource_caps_fail_closed() {
        let template = reference_template(
            "fn set(p:&mut i32,n:i32){*p=n;} fn main(){let mut n=1;let r=&mut n;set(r,{return;3});}",
            "main",
        );
        let proof = verify_template(template).unwrap();
        let f = proof.function();
        assert!(proof.analysis().stats.blocks_reachable < f.blocks.len());
        let (loan, activation, argument) = f
            .loan_sites
            .iter()
            .enumerate()
            .find_map(|(loan, site)| match site.timing {
                cfg::LoanTiming::TwoPhase {
                    activation,
                    argument,
                } => Some((loan, activation, argument)),
                cfg::LoanTiming::Immediate => None,
            })
            .unwrap();
        assert!(matches!(
            f.blocks[activation.0].terminator.value,
            Terminator::Call { .. }
        ));
        for bad in [
            cfg::LoanTiming::TwoPhase {
                activation: BlockId(f.blocks.len()),
                argument,
            },
            cfg::LoanTiming::TwoPhase {
                activation,
                argument: usize::MAX,
            },
        ] {
            let mut forged = f.as_ref().clone();
            forged.loan_sites[loan].timing = bad;
            let failure = analyze_with_facts(
                &forged,
                Limits::default(),
                FactsCtx::Validated(proof.facts()),
                Some(proof.budget()),
            );
            assert!(failure.complete);
            assert!(
                failure
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_)))
            );
        }
        let mut forged = f.as_ref().clone();
        let Terminator::Call { args, .. } = &forged.blocks[activation.0].terminator.value else {
            unreachable!()
        };
        let Operand::Move(carrier) = &args[argument] else {
            unreachable!()
        };
        let carrier = carrier.clone();
        forged.blocks[activation.0]
            .statements
            .push(located(Statement::Inspect(carrier)));
        assert!(
            !analyze_with_facts(
                &forged,
                Limits::default(),
                FactsCtx::Validated(proof.facts()),
                Some(proof.budget())
            )
            .passed()
        );
        let mut tiny = Budget::for_plan(Limits::default().work, Some(proof.budget()));
        assert!(borrow::Plan::build(f, &mut tiny, 0).is_err());
        let foreign = super::super::ir::PlanBudget::default();
        assert!(
            !analyze_with_facts(
                f,
                Limits::default(),
                FactsCtx::Validated(proof.facts()),
                Some(&foreign)
            )
            .passed()
        );
    }

    #[test]
    fn trait_facts_bind_exact_template_and_reject_schema_and_foreign_ledger() {
        use super::super::ir::PlanBudget;
        let budget = PlanBudget::default();
        let (source, facts) = trait_callable_fixture(
            "trait Take { fn take(self) -> Self; } fn forward<T: Take>(value:T)->T { value.take() } fn main() {}",
            "forward",
            &budget,
        );
        let parameter = &source.locals[source.params[0].0].ty;
        assert!(!facts.must_copy(parameter).unwrap());
        let template = cfg::lower_template(source.clone(), facts.clone(), &budget).unwrap();
        let function = template.function().clone();
        let schema = facts.world().schema_facts(required_item(&source)).unwrap();
        assert!(schema.must_copy(&Ty::Unit).is_err());
        assert!(cfg::lower_template(source.clone(), schema.clone(), &budget).is_err());
        let invalid_schema = analyze_with_facts(
            &function,
            Limits::default(),
            FactsCtx::Validated(&schema),
            Some(&budget),
        );
        assert!(!invalid_schema.passed());
        let mut path_budget = Budget::for_plan(Limits::default().work, Some(&budget));
        assert!(
            Paths::with_facts(
                &function,
                Limits::default().paths,
                FactsCtx::Validated(&schema),
                &mut path_budget,
            )
            .is_err()
        );
        let foreign_budget = PlanBudget::default();
        assert!(
            !analyze_with_facts(
                &function,
                Limits::default(),
                FactsCtx::Validated(&facts),
                Some(&foreign_budget),
            )
            .passed()
        );
        let proof = verify_template(template).unwrap();
        assert!(Arc::ptr_eq(proof.facts(), &facts));
        assert!(Arc::ptr_eq(proof.env(), facts.env()));
        assert!(Arc::ptr_eq(proof.source(), &source));
        assert!(Arc::ptr_eq(proof.function(), &function));
        assert!(proof.analysis().passed());
        assert!(proof.analysis().stats.moves > 0);
        assert!(proof.analysis().stats.operands_read > proof.analysis().stats.moves);
    }

    #[test]
    fn trait_named_copy_cannot_grant_permission_and_explicit_moves_still_consume() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::PlanBudget;
        let budget = PlanBudget::default();
        let source = "trait Copy {} fn once<T: Copy>(value:T)->T { value } fn twice<T: Copy>(value:T)->(T,T) { (value,value) } fn main() {}";
        let (once, facts) = trait_callable_fixture(source, "once", &budget);
        assert!(!facts.predicates().predicates().is_empty());
        assert!(!facts.must_copy(&once.locals[once.params[0].0].ty).unwrap());
        let proof = verify_template(cfg::lower_template(once, facts, &budget).unwrap()).unwrap();
        assert!(proof.analysis().passed());
        let (twice, facts) = trait_callable_fixture(source, "twice", &budget);
        let failure =
            verify_template(cfg::lower_template(twice, facts, &budget).unwrap()).unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
        let mut explicit = fixture();
        explicit.locals.extend([
            local(Ty::Int(IntTy::I32)),
            local(Ty::Int(IntTy::I32)),
            local(Ty::Int(IntTy::I32)),
        ]);
        explicit.params.push(LocalId(1));
        explicit.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let failure = analyze_closed(close_fixture(&modules, Arc::new(explicit), &budget).unwrap())
            .unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
    }

    #[test]
    fn trait_dead_foreign_item_metadata_and_unproved_closed_instances_fail_sealed() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::{CallTarget, Callee, PlanBudget};
        let budget = PlanBudget::default();
        let source = "trait Take { fn take(self)->Self; } fn forward<T: Take>(value:T)->T { value.take() } fn main() {}";
        let (checked, facts) = trait_callable_fixture(source, "forward", &budget);
        let (foreign, foreign_facts) = trait_callable_fixture(source, "forward", &budget);
        assert!(!Arc::ptr_eq(facts.world(), foreign_facts.world()));
        let template = cfg::lower_template(checked, facts.clone(), &budget).unwrap();
        assert!(
            !analyze_with_facts(
                template.function(),
                Limits::default(),
                FactsCtx::Validated(&foreign_facts),
                Some(&budget),
            )
            .passed()
        );
        let mut forged = template.function().as_ref().clone();
        let mut dead = forged
            .blocks
            .iter()
            .find(|block| {
                matches!(
                    &block.terminator.value,
                    Terminator::Call {
                        target: Callee::TraitItem(_),
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        let Terminator::Call {
            target: Callee::TraitItem(call),
            ..
        } = &mut dead.terminator.value
        else {
            unreachable!()
        };
        call.item = required_item(&foreign);
        forged.blocks.push(dead);
        assert!(
            !analyze_with_facts(
                &forged,
                Limits::default(),
                FactsCtx::Validated(&facts),
                Some(&budget),
            )
            .passed()
        );
        let target = CallTarget {
            declaration: facts.declaration(),
            type_args: Arc::from([Ty::Int(IntTy::I32)]),
        };
        let failure = facts.world().instance_facts(&target).unwrap_err();
        assert_eq!(
            failure.kind(),
            super::super::traits::FactsErrorKind::Unproved
        );
        let closed = close_fixture(&modules, Arc::new(fixture()), &budget).unwrap();
        let exact_cfg = closed.function().clone();
        let closed_facts = closed.facts().clone();
        assert!(cfg::close(template.function().clone(), closed_facts.clone(), &budget).is_err());
        let proof = analyze_closed(closed).unwrap();
        assert!(Arc::ptr_eq(proof.function(), &exact_cfg));
        assert!(Arc::ptr_eq(proof.facts(), &closed_facts));
        assert!(proof.analysis().passed());
    }

    fn symbolic_parameter() -> (Arc<GenericEnv>, Ty) {
        use super::super::ir::{BinderId, ParamEnv, ParamId, PlanBudget};
        let budget = PlanBudget::default();
        let binder = BinderId {
            declaration: source_decl(SnapshotId::allocate().unwrap(), 0, &budget).unwrap(),
        };
        let scope = Arc::new(ParamEnv::new(binder, 1).unwrap());
        let env = Arc::new(GenericEnv::new(&[scope], &budget).unwrap());
        (
            env,
            Ty::Param(ParamId {
                owner: binder,
                index: 0,
            }),
        )
    }

    fn symbolic_pair() -> (Arc<GenericEnv>, [Ty; 2]) {
        use super::super::ir::{BinderId, ParamEnv, ParamId, PlanBudget};
        let budget = PlanBudget::default();
        let snapshot = SnapshotId::allocate().unwrap();
        let binders = [0, 1].map(|index| BinderId {
            declaration: source_decl(snapshot, index, &budget).unwrap(),
        });
        let scopes = binders.map(|binder| Arc::new(ParamEnv::new(binder, 1).unwrap()));
        let env = GenericEnv::new(&scopes, &budget).unwrap();
        let parameters = binders.map(|owner| Ty::Param(ParamId { owner, index: 0 }));
        (Arc::new(env), parameters)
    }

    #[test]
    fn multi_binder_membership_validates_both_scopes_and_dead_metadata() {
        let (env, parameters) = symbolic_pair();
        let facts = FactsCtx::Universal(&env);
        let mut f = fixture();
        for parameter in &parameters {
            let Ty::Param(id) = parameter else {
                panic!("parameter fixture")
            };
            assert!(env.contains(*id));
            f.locals.push(local(array(parameter.clone(), 0)));
        }
        assert!(analyze_with_facts(&f, Limits::default(), facts, None).passed());
        for index in 0..2 {
            let other = GenericEnv::new(
                &[env.scopes()[1 - index].clone()],
                &super::super::ir::PlanBudget::default(),
            )
            .unwrap();
            assert!(
                !analyze_with_facts(&f, Limits::default(), FactsCtx::Universal(&other), None)
                    .passed()
            );
        }
        let (_, foreign) = symbolic_parameter();
        f.locals.push(local(array(foreign, 0)));
        assert!(!analyze_with_facts(&f, Limits::default(), facts, None).passed());
    }

    #[test]
    fn multi_binder_universal_parameters_remain_noncopy_in_the_same_solver() {
        let (env, parameters) = symbolic_pair();
        for parameter in parameters {
            let facts = FactsCtx::Universal(&env);
            let mut f = fixture();
            f.locals.extend([
                local(parameter.clone()),
                local(parameter.clone()),
                local(parameter),
            ]);
            f.params.push(LocalId(1));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::StorageLive(LocalId(3))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Use(Operand::Move(place(1))),
                }),
            ]);
            assert!(analyze_with_facts(&f, Limits::default(), facts, None).passed());
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }));
            assert!(
                analyze_with_facts(&f, Limits::default(), facts, None)
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
            );
            f.blocks[0].statements.pop();
            f.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Copy(place(1))),
            };
            assert!(
                analyze_with_facts(&f, Limits::default(), facts, None)
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_)))
            );
        }
    }

    #[test]
    fn multi_binder_template_proof_binds_full_environment_source_cfg_and_analysis() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::{Block, Completion, ExprKind, PlanBudget, TypedExpr};
        let (env, parameters) = symbolic_pair();
        let ty = Ty::Tuple(Vec::from(parameters).into());
        let source = Arc::new(CheckedFunction {
            call_evidence: Arc::from([]),
            body_substitution: None,
            ref_contract: Default::default(),
            name: "both_binders".into(),
            params: vec![LocalId(0)],
            result: ty.clone(),
            locals: vec![local(ty.clone())],
            exprs: vec![TypedExpr {
                ty,
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            }],
            body: Block {
                statements: vec![],
                tail: Some(ExprId(0)),
            },
        });
        let budget = PlanBudget::default();
        let template =
            lower_fixture_template(&modules, source.clone(), env.clone(), &budget).unwrap();
        let exact_cfg = template.function().clone();
        let proof = verify_template(template).unwrap();
        assert!(Arc::ptr_eq(proof.env(), &env));
        assert!(Arc::ptr_eq(proof.source(), &source));
        assert!(Arc::ptr_eq(proof.function(), &exact_cfg));
        assert!(proof.analysis().passed());
        assert!(close_fixture(&modules, exact_cfg, &budget).is_err());
        for scope in env.scopes() {
            let partial = Arc::new(GenericEnv::new(std::slice::from_ref(scope), &budget).unwrap());
            assert!(lower_fixture_template(&modules, source.clone(), partial, &budget).is_err());
        }
        let program = super::super::frontend::parse("fn main() {}").unwrap();
        let mut cx = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let actuals = [Ty::Int(IntTy::I32), Ty::Bool];
        let target = super::super::ir::CallTarget {
            declaration: proof.facts().declaration(),
            type_args: Arc::from(actuals.clone()),
        };
        let closed_facts = proof.facts().world().instance_facts(&target).unwrap();
        let instance = super::super::ir::instantiate_checked(
            &proof,
            &actuals,
            &closed_facts,
            &mut cx,
            &budget,
        )
        .unwrap();
        assert_eq!(instance.result, Ty::Tuple(actuals.to_vec().into()));
        let lowered = cfg::lower_with_facts(&instance, &closed_facts, &budget).unwrap();
        let concrete =
            analyze_closed(cfg::close(Arc::new(lowered), closed_facts.clone(), &budget).unwrap())
                .unwrap();
        assert!(concrete.analysis().passed());
        assert!(
            super::super::ir::instantiate_checked(
                &proof,
                &actuals,
                &closed_facts,
                &mut cx,
                &PlanBudget::default()
            )
            .is_err()
        );
        // Swapping caller parameters in the same environment must insert each
        // actual once, not recursively capture it through the other scope.
        let Ty::Tuple(parameters) = &source.result else {
            panic!("tuple fixture")
        };
        let swapped = [parameters[1].clone(), parameters[0].clone()];
        let symbolic = super::super::ir::substitute_ty(
            &source.result,
            &env,
            &swapped,
            FactsCtx::Universal(&env),
            &mut cx,
            &budget,
        )
        .unwrap();
        assert_eq!(symbolic, Ty::Tuple(swapped.to_vec().into()));
        let (_, foreign) = symbolic_parameter();
        let mut forged = source.as_ref().clone();
        forged.locals.push(local(array(foreign, 0)));
        assert!(lower_fixture_template(&modules, Arc::new(forged), env, &budget).is_err());
    }

    #[test]
    fn universal_parameter_moves_partial_paths_and_joins_use_the_same_solver() {
        let (env, parameter) = symbolic_parameter();
        let facts = FactsCtx::Universal(&env);
        let mut f = fixture();
        f.locals.extend([
            local(parameter.clone()),
            local(parameter.clone()),
            local(parameter.clone()),
        ]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let result = analyze_with_facts(&f, Limits::default(), facts, None);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.move_paths, 4);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(place(1))),
        }));
        assert!(!analyze_with_facts(&f, Limits::default(), facts, None).passed());
        f.blocks[0].statements.pop();
        f.locals[1].ty = Ty::Tuple(vec![parameter.clone(), parameter.clone()].into());
        let field = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        f.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Move(field(0))),
        };
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(field(1))),
        }));
        assert!(analyze_with_facts(&f, Limits::default(), facts, None).passed());
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(1))));
        assert!(!analyze_with_facts(&f, Limits::default(), facts, None).passed());
        f.locals[1].ty = parameter;
        f.blocks[0].statements.truncate(4);
        f.blocks[0].terminator = located(Terminator::Switch {
            condition: Operand::Constant(Constant::Bool(true)),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Use(Operand::Move(place(1))),
                })],
                terminator: located(Terminator::Goto(BlockId(3))),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Goto(BlockId(3))),
            },
            BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Use(Operand::Move(place(1))),
                })],
                terminator: located(Terminator::Return),
            },
        ]);
        assert!(!analyze_with_facts(&f, Limits::default(), facts, None).passed());
        f.blocks[3].statements[0].value = Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Copy(place(1))),
        };
        assert!(
            analyze_with_facts(&f, Limits::default(), facts, None)
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_)))
        );
    }

    #[test]
    fn generic_capabilities_bind_exact_source_and_cfg_and_refuse_failed_or_open_native_input() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::{Block, Completion, ExprKind, PlanBudget, TypedExpr};
        let (env, parameter) = symbolic_parameter();
        let source = Arc::new(CheckedFunction {
            call_evidence: Arc::from([]),
            body_substitution: None,
            ref_contract: Default::default(),
            name: "identity".into(),
            params: vec![LocalId(0)],
            result: parameter.clone(),
            locals: vec![local(parameter.clone())],
            exprs: vec![TypedExpr {
                ty: parameter.clone(),
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            }],
            body: Block {
                statements: vec![],
                tail: Some(ExprId(0)),
            },
        });
        let budget = PlanBudget::default();
        let structural =
            lower_fixture_template(&modules, source.clone(), env.clone(), &budget).unwrap();
        let exact_cfg = structural.function().clone();
        let verified = verify_template(structural).unwrap();
        assert!(Arc::ptr_eq(verified.source(), &source));
        assert!(Arc::ptr_eq(verified.function(), &exact_cfg));
        assert!(verified.analysis().passed());
        assert!(close_fixture(&modules, exact_cfg, &budget).is_err());
        let mut duplicated = source.as_ref().clone();
        duplicated.result = Ty::Tuple(vec![parameter.clone(), parameter.clone()].into());
        duplicated.exprs.push(TypedExpr {
            ty: duplicated.result.clone(),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![ExprId(0), ExprId(0)]),
        });
        duplicated.body.tail = Some(ExprId(1));
        let structural =
            lower_fixture_template(&modules, Arc::new(duplicated), env.clone(), &budget).unwrap();
        assert!(verify_template(structural).is_err());
        let exact_closed = Arc::new(fixture());
        let concrete =
            analyze_closed(close_fixture(&modules, exact_closed.clone(), &budget).unwrap())
                .unwrap();
        assert!(Arc::ptr_eq(concrete.function(), &exact_closed));
        assert!(concrete.analysis().passed());
        let mut invalid = fixture();
        invalid.locals.push(local(Ty::Int(IntTy::I32)));
        invalid.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Inspect(place(1))),
        ]);
        assert!(
            analyze_closed(close_fixture(&modules, Arc::new(invalid), &budget).unwrap()).is_err()
        );
        let (foreign_env, _) = symbolic_parameter();
        assert!(lower_fixture_template(&modules, source, foreign_env, &budget).is_err());
        let mut hidden = fixture();
        hidden.locals.push(local(array(
            Ty::Ref {
                kind: super::super::ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            },
            0,
        )));
        assert!(
            !analyze_with_facts(&hidden, Limits::default(), FactsCtx::Universal(&env), None)
                .passed()
        );
        // Application identity alone cannot hide an open or foreign-binder
        // type argument inside a constant in a disconnected aggregate.
        let closed_enum = unit_enum(&modules);
        let (_, foreign_parameter) = symbolic_parameter();
        let mut forged_definition = closed_enum.def().clone();
        forged_definition.type_args = Arc::from([foreign_parameter]);
        let forged_enum = EnumTy::new(forged_definition);
        let aggregate_ty = Ty::Tuple(vec![Ty::Enum(closed_enum)].into());
        let mut forged = fixture();
        forged.locals.push(local(aggregate_ty.clone()));
        forged.blocks.push(BasicBlock {
            statements: vec![located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Aggregate {
                    ty: aggregate_ty,
                    fields: vec![Operand::Constant(Constant::Enum {
                        ty: forged_enum,
                        variant: 0,
                    })],
                },
            })],
            terminator: located(Terminator::Unreachable),
        });
        assert!(close_fixture(&modules, Arc::new(forged.clone()), &budget).is_err());
        assert!(
            !analyze_with_facts(&forged, Limits::default(), FactsCtx::Universal(&env), None)
                .passed()
        );
    }

    #[test]
    fn generic_shared_work_memory_and_expanded_shape_caps_fail_incomplete() {
        use super::super::ir::{PlanBudget, PlanLimits};
        let f = fixture();
        for limits in [
            PlanLimits {
                work: 1,
                ..PlanLimits::default()
            },
            PlanLimits {
                bytes: 1,
                ..PlanLimits::default()
            },
            PlanLimits {
                nodes: 0,
                ..PlanLimits::default()
            },
        ] {
            let budget = PlanBudget::new(limits);
            let result =
                analyze_with_facts(&f, Limits::default(), FactsCtx::Concrete, Some(&budget));
            assert!(!result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ResourceLimit)
            );
        }
        let budget = PlanBudget::new(PlanLimits {
            work: 100000,
            ..PlanLimits::default()
        });
        budget.clone().charge_work(99999).unwrap();
        assert!(
            !analyze_with_facts(&f, Limits::default(), FactsCtx::Concrete, Some(&budget)).complete
        );
        let (env, parameter) = symbolic_parameter();
        let mut expanded = fixture();
        expanded.locals.push(local(array(parameter.clone(), 4)));
        let result = analyze_with_facts(
            &expanded,
            Limits {
                paths: 3,
                ..Limits::default()
            },
            FactsCtx::Universal(&env),
            None,
        );
        assert!(!result.complete);
        expanded.locals[1].ty = array(parameter, 0);
        let result = analyze_with_facts(
            &expanded,
            Limits::default(),
            FactsCtx::Universal(&env),
            None,
        );
        assert!(result.passed());
        assert_eq!(result.stats.move_paths, 2);
    }

    #[test]
    fn heterogeneous_shifts_read_both_operands_and_retain_operand_loans() {
        use super::super::frontend::BinaryOp;
        for op in [BinaryOp::Shl, BinaryOp::Shr] {
            for (lhs, rhs) in [(IntTy::I8, IntTy::U64), (IntTy::U64, IntTy::I8)] {
                let mut f = fixture();
                f.locals.extend([
                    local(Ty::Int(lhs)),
                    local(Ty::Int(rhs)),
                    local(Ty::Int(lhs)),
                ]);
                f.params.push(LocalId(1));
                f.blocks[0].statements.extend([
                    located(Statement::StorageLive(LocalId(2))),
                    located(Statement::StorageLive(LocalId(3))),
                    located(Statement::Assign {
                        destination: place(3),
                        value: Rvalue::Binary {
                            op,
                            left: Operand::Copy(place(1)),
                            right: Operand::Copy(place(2)),
                        },
                    }),
                ]);
                let result = analyze(&f);
                assert!(result.complete);
                assert_eq!(result.stats.operands_read, 4);
                assert!(
                    result
                        .diagnostics
                        .iter()
                        .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
                );
                f.params.push(LocalId(2));
                f.blocks[0]
                    .statements
                    .retain(|s| !matches!(s.value, Statement::StorageLive(LocalId(2))));
                assert!(analyze(&f).passed());
            }
        }
        for operator in ["<<", ">>"] {
            for (expression, owner) in [
                (format!("*r {operator} 1u64"), "x"),
                (format!("1u8 {operator} *r"), "x"),
            ] {
                let valid = borrowed_source(&format!(
                    "fn f(input:u8)->u8{{let mut x=input;let r=&x;let y={expression};{owner}=2;y}}"
                ));
                assert!(analyze(&valid).passed());
                let invalid = borrowed_source(&format!(
                    "fn f(input:u8)->u8{{let mut x=input;let r=&x;{owner}=2;{expression}}}"
                ));
                assert!(
                    analyze(&invalid)
                        .diagnostics
                        .iter()
                        .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
                );
            }
        }
    }

    #[test]
    fn shift_bad_constant_counts_in_dead_cfg_fail_closed() {
        use super::super::frontend::BinaryOp;
        for op in [BinaryOp::Shl, BinaryOp::Shr] {
            for count in [
                IntValue {
                    ty: IntTy::I8,
                    bits: 255,
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: u128::from(u64::MAX),
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: 8,
                },
            ] {
                let mut f = fixture();
                f.locals.push(local(Ty::Int(IntTy::U8)));
                f.blocks.push(BasicBlock {
                    statements: vec![located(Statement::Assign {
                        destination: place(1),
                        value: Rvalue::Binary {
                            op,
                            left: Operand::Constant(Constant::Int(IntValue {
                                ty: IntTy::U8,
                                bits: 1,
                            })),
                            right: Operand::Constant(Constant::Int(count)),
                        },
                    })],
                    terminator: located(Terminator::Return),
                });
                let result = analyze(&f);
                assert!(!result.passed());
                assert!(
                    result
                        .diagnostics
                        .iter()
                        .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_))),
                    "{:?}",
                    result.diagnostics
                );
            }
        }
    }

    #[test]
    fn eager_bitwise_reads_both_operands_once_in_left_to_right_order() {
        use super::super::frontend::BinaryOp;
        for op in [BinaryOp::BitAnd, BinaryOp::BitOr, BinaryOp::BitXor] {
            for ty in [Ty::Bool, Ty::Int(IntTy::U8), Ty::Int(IntTy::I64)] {
                let constant = if ty == Ty::Bool {
                    Constant::Bool(matches!(op, BinaryOp::BitOr))
                } else {
                    let Ty::Int(kind) = ty else { unreachable!() };
                    Constant::Int(IntValue {
                        ty: kind,
                        bits: if matches!(op, BinaryOp::BitOr) {
                            kind.mask()
                        } else {
                            0
                        },
                    })
                };
                let mut f = fixture();
                f.locals.extend([local(ty.clone()), local(ty)]);
                f.blocks[0].statements.extend([
                    located(Statement::StorageLive(LocalId(1))),
                    located(Statement::StorageLive(LocalId(2))),
                    located(Statement::Assign {
                        destination: place(2),
                        value: Rvalue::Binary {
                            op,
                            left: Operand::Constant(constant),
                            right: Operand::Copy(place(1)),
                        },
                    }),
                ]);
                let result = analyze(&f);
                assert!(result.complete);
                assert_eq!(result.stats.operands_read, 4);
                assert_eq!(
                    result.diagnostics.len(),
                    1,
                    "{op:?}: {:?}",
                    result.diagnostics
                );
                assert_eq!(
                    result.diagnostics[0].kind,
                    ErrorKind::ReadNotDefinitelyInitialized
                );
                f.params.push(LocalId(1));
                f.blocks[0]
                    .statements
                    .retain(|item| !matches!(item.value, Statement::StorageLive(LocalId(1))));
                for moved_first in [false, true] {
                    let mut ordered = f.clone();
                    ordered.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
                        destination: place(2),
                        value: Rvalue::Binary {
                            op,
                            left: if moved_first {
                                Operand::Move(place(1))
                            } else {
                                Operand::Copy(place(1))
                            },
                            right: if moved_first {
                                Operand::Copy(place(1))
                            } else {
                                Operand::Move(place(1))
                            },
                        },
                    };
                    let result = analyze(&ordered);
                    assert_eq!(
                        result.passed(),
                        !moved_first,
                        "{op:?}: {:?}",
                        result.diagnostics
                    );
                    assert_eq!(result.stats.operands_read, 4);
                    assert_eq!(result.stats.moves, 1);
                }
            }
        }
    }

    #[test]
    fn eager_boolean_bitwise_deref_retains_each_operand_loan_until_last_read() {
        use super::super::{frontend::BinaryOp, ir::BorrowKind};
        for op in [BinaryOp::BitAnd, BinaryOp::BitOr, BinaryOp::BitXor] {
            for deref_left in [false, true] {
                let mut f = fixture();
                f.locals.extend([
                    local(Ty::Bool),
                    local(Ty::Ref {
                        kind: BorrowKind::Shared,
                        referent: std::sync::Arc::new(Ty::Bool),
                    }),
                    local(Ty::Bool),
                ]);
                f.params.push(LocalId(1));
                f.loan_sites.push(cfg::LoanSite {
                    timing: cfg::LoanTiming::Immediate,
                    kind: BorrowKind::Shared,
                    place: place(1),
                    origin: None,
                });
                let source = Operand::Copy(Place {
                    local: LocalId(2),
                    projection: vec![Projection::Deref],
                });
                let constant = Operand::Constant(Constant::Bool(matches!(op, BinaryOp::BitOr)));
                let value = Rvalue::Binary {
                    op,
                    left: if deref_left {
                        source.clone()
                    } else {
                        constant.clone()
                    },
                    right: if deref_left { constant } else { source },
                };
                let write = located(Statement::Assign {
                    destination: place(1),
                    value: Rvalue::Use(Operand::Constant(Constant::Bool(true))),
                });
                f.blocks[0].statements.extend([
                    located(Statement::StorageLive(LocalId(2))),
                    located(Statement::StorageLive(LocalId(3))),
                    located(Statement::Assign {
                        destination: place(2),
                        value: Rvalue::Borrow(cfg::LoanId(0)),
                    }),
                    write.clone(),
                    located(Statement::Assign {
                        destination: place(3),
                        value: value.clone(),
                    }),
                ]);
                let result = analyze(&f);
                assert!(
                    result
                        .diagnostics
                        .iter()
                        .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
                    "{op:?}/{deref_left}: {:?}",
                    result.diagnostics
                );
                let length = f.blocks[0].statements.len();
                f.blocks[0].statements.remove(length - 2);
                f.blocks[0].statements.push(write);
                assert!(analyze(&f).passed());
                f.blocks[0].statements.pop();
                f.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
                    destination: place(1),
                    value,
                };
                let result = analyze(&f);
                assert!(
                    result.passed(),
                    "{op:?}/{deref_left}: {:?}",
                    result.diagnostics
                );
            }
        }
    }

    #[test]
    fn integer_cast_reads_one_operand_and_preserves_private_move_semantics() {
        for moved in [false, true] {
            let mut f = fixture();
            f.locals
                .extend([local(Ty::Int(IntTy::I64)), local(Ty::Int(IntTy::U8))]);
            f.params.push(LocalId(1));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::IntCast {
                        value: if moved {
                            Operand::Move(place(1))
                        } else {
                            Operand::Copy(place(1))
                        },
                        to: IntTy::U8,
                    },
                }),
                located(Statement::Inspect(place(2))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "{:?}", result.diagnostics);
            assert_eq!(result.stats.operands_read, 3);
            assert_eq!(result.stats.moves, usize::from(moved));
            f.blocks[0]
                .statements
                .push(located(Statement::Inspect(place(1))));
            assert_eq!(analyze(&f).passed(), !moved);
            f.params.clear();
            f.blocks[0]
                .statements
                .insert(0, located(Statement::StorageLive(LocalId(1))));
            let result = analyze(&f);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
            );
        }
        let mut cast = fixture();
        cast.locals
            .extend([local(Ty::Int(IntTy::I64)), local(Ty::Int(IntTy::I64))]);
        cast.params.push(LocalId(1));
        cast.blocks[0].statements.push(located(Statement::Assign {
            destination: place(2),
            value: Rvalue::IntCast {
                value: Operand::Copy(place(1)),
                to: IntTy::I64,
            },
        }));
        let mut ordinary = cast.clone();
        ordinary.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Copy(place(1))),
        };
        let cost = |f: &cfg::Function| {
            (0..1024)
                .find(|operations| {
                    preflight(
                        f,
                        Limits {
                            operations: *operations,
                            ..Limits::default()
                        },
                    )
                    .is_ok()
                })
                .unwrap()
        };
        assert_eq!(cost(&cast), cost(&ordinary) + 1);
    }

    #[test]
    fn integer_cast_deref_keeps_carrier_live_and_last_read_ends_before_write() {
        use super::super::ir::BorrowKind;
        let mut f = fixture();
        f.locals.extend([
            local(Ty::Int(IntTy::I32)),
            local(Ty::Ref {
                kind: BorrowKind::Shared,
                referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            }),
            local(Ty::Int(IntTy::I32)),
        ]);
        f.params.push(LocalId(1));
        f.loan_sites.push(cfg::LoanSite {
            timing: cfg::LoanTiming::Immediate,
            kind: BorrowKind::Shared,
            place: place(1),
            origin: None,
        });
        let cast = Rvalue::IntCast {
            value: Operand::Copy(Place {
                local: LocalId(2),
                projection: vec![Projection::Deref],
            }),
            to: IntTy::I32,
        };
        let write = located(Statement::Assign {
            destination: place(1),
            value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue::i32(2)))),
        });
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Borrow(cfg::LoanId(0)),
            }),
            write.clone(),
            located(Statement::Assign {
                destination: place(3),
                value: cast.clone(),
            }),
        ]);
        let result = analyze(&f);
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
            "{:?}",
            result.diagnostics
        );
        let length = f.blocks[0].statements.len();
        f.blocks[0].statements.remove(length - 2);
        f.blocks[0].statements.push(write);
        assert!(analyze(&f).passed());
        f.blocks[0].statements.pop();
        f.blocks[0].statements.last_mut().unwrap().value = Statement::Assign {
            destination: place(1),
            value: cast,
        };
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn integer_cast_nonnumeric_and_noncanonical_dead_operands_cannot_create_origins() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::BorrowKind;
        for source in [
            Ty::Bool,
            Ty::Unit,
            nominal(&modules, vec![Ty::Int(IntTy::I32)]),
            Ty::Ref {
                kind: BorrowKind::Mut,
                referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            },
        ] {
            let mut f = fixture();
            f.locals.extend([local(source), local(Ty::Int(IntTy::U8))]);
            f.blocks.push(BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::IntCast {
                        value: Operand::Move(place(1)),
                        to: IntTy::U8,
                    },
                })],
                terminator: located(Terminator::Unreachable),
            });
            let result = analyze(&f);
            assert!(result.complete);
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
        }
        let mut f = fixture();
        f.locals.push(local(Ty::Int(IntTy::U64)));
        f.blocks.push(BasicBlock {
            statements: vec![located(Statement::Assign {
                destination: place(1),
                value: Rvalue::IntCast {
                    value: Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::U8,
                        bits: 256,
                    })),
                    to: IntTy::U64,
                },
            })],
            terminator: located(Terminator::Unreachable),
        });
        let result = analyze(&f);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn integer_primitives_remain_copy_scalar_paths_and_reference_reads() {
        for kind in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
            IntTy::I128,
            IntTy::U128,
        ] {
            let ty = Ty::Int(kind);
            assert!(ty.is_copy());
            assert!(preflight_ty(&ty, 0, &mut Budget::new(1)).is_ok());
            let mut f = dynamic_fixture(ty);
            f.blocks[0].statements.extend([
                located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                        ty: kind,
                        bits: kind.mask(),
                    }))),
                }),
                dynamic_read(),
                dynamic_inspect(),
                located(Statement::Inspect(place(1))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "{kind:?}: {:?}", result.diagnostics);
            assert_eq!(result.stats.move_paths, 6);
            assert_eq!(result.stats.moves, 0);
            let source = format!("fn f(p:&{})->{}{{!*p}}", kind.name(), kind.name());
            let result = analyze(&borrowed_source(&source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn integer_noncanonical_dead_constants_and_u64_runtime_indices_fail_closed() {
        for kind in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::I64,
            IntTy::U64,
            IntTy::Isize,
            IntTy::Usize,
        ] {
            let mut f = fixture();
            f.locals.push(local(Ty::Int(kind)));
            f.blocks.push(BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(1),
                    value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                        ty: kind,
                        bits: 1u128 << kind.bits(),
                    }))),
                })],
                terminator: located(Terminator::Unreachable),
            });
            let result = analyze(&f);
            assert!(result.complete);
            assert!(
                matches!(result.diagnostics[0].kind, ErrorKind::InvalidCfg(_)),
                "{kind:?}: {:?}",
                result.diagnostics
            );
        }
        let mut f = dynamic_fixture(Ty::Int(IntTy::U64));
        f.blocks.push(BasicBlock {
            statements: vec![located(Statement::Assign {
                destination: place(3),
                value: Rvalue::IndexCopy {
                    array: place(1),
                    index: Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::U64,
                        bits: 0,
                    })),
                },
            })],
            terminator: located(Terminator::Unreachable),
        });
        let result = analyze(&f);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn mixed_result_candidates_preserve_capabilities_and_reject_shared_to_mut() {
        for source in [
            "fn f<'a>(p:&'a mut i32)->&'a i32{p}",
            "fn f<'a>(p:&'a mut (i32,bool),q:&'a i32,b:bool)->&'a i32{if b{&p.0}else{q}}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=view(r);let n=*q;*r=2;n+x} fn view(p:&mut i32)->&i32{p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let source = "fn f()->i32{let mut x=1;let r=&mut x;let q=view(r);let n=*r;n+*q} fn view(p:&mut i32)->&i32{p}";
        assert!(!analyze(&borrowed_source(source)).passed());
        let mut forged = borrowed_source("fn f<'a>(p:&'a i32,q:&'a mut i32)->&'a mut i32{q}");
        first_return(&mut forged.ref_contract).returns =
            super::super::ir::ReturnContract::MutInputPaths(
                vec![
                    super::super::ir::InputPath {
                        input: input_reference(0),
                        projection: vec![].into(),
                    },
                    super::super::ir::InputPath {
                        input: input_reference(1),
                        projection: vec![].into(),
                    },
                ]
                .into(),
            );
        assert!(forged.validate().is_err());
        let result = analyze(&forged);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn mixed_noncandidate_dependencies_retain_only_declared_lifetimes() {
        let same = "fn f()->i32{let x=7;let r:&i32;{let mut s=true;r=pick(&mut s,&x);}*r} fn pick<'a>(s:&'a mut bool,x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(same));
        assert!(result.complete);
        assert!(!result.passed());
        let independent = "fn f()->i32{let x=7;let r:&i32;{let mut s=true;r=pick(&mut s,&x);}*r} fn pick<'a,'b>(s:&'a mut bool,x:&'b i32)->&'b i32{x}";
        let result = analyze(&borrowed_source(independent));
        assert!(result.passed(), "{:?}", result.diagnostics);
        let universal = "fn f<'a,'b>(s:&'a mut bool,x:&'b i32)->&'b i32{pick(s,x)} fn pick<'a>(s:&'a mut bool,x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(universal));
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::InvalidReference(_)))
        );
    }

    #[test]
    fn mixed_mutable_noncandidate_call_kills_proofs_without_initializing() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::{
            BorrowKind, InputPath, RefContract, RegionParamId, ReturnContract, Signature,
        };
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: std::sync::Arc::new(Ty::Enum(ty.clone())),
        };
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
        };
        f.locals.extend([
            local(Ty::Int(IntTy::I32)),
            local(mutable.clone()),
            local(shared.clone()),
            local(shared.clone()),
        ]);
        f.params.push(LocalId(3));
        f.loan_sites.extend([
            cfg::LoanSite {
                timing: cfg::LoanTiming::Immediate,
                kind: BorrowKind::Mut,
                place: place(1),
                origin: None,
            },
            cfg::LoanSite {
                timing: cfg::LoanTiming::Immediate,
                kind: BorrowKind::Shared,
                place: place(3),
                origin: None,
            },
        ]);
        split_payload(&mut f, &ty);
        for index in 4..=6 {
            f.blocks[1]
                .statements
                .push(located(Statement::StorageLive(LocalId(index))));
        }
        for (destination, loan) in [(4, 0), (5, 1)] {
            f.blocks[1].statements.push(located(Statement::Assign {
                destination: place(destination),
                value: Rvalue::Borrow(cfg::LoanId(loan)),
            }));
        }
        f.blocks[1].terminator = located(Terminator::Call {
            target: synthetic_call_target(),
            name: "mixed".into(),
            signature: Signature {
                ref_contract: std::sync::Arc::new(RefContract {
                    region_count: 1,
                    input_regions: (0..2)
                        .map(|param| super::super::ir::InputRegion {
                            input: input_reference(param),
                            region: RegionParamId(0),
                        })
                        .collect::<Vec<_>>()
                        .into(),
                    outlives: vec![].into(),
                    results: vec![super::super::ir::ResultReference {
                        position: ReferencePosition { depth: 0 },
                        region: RegionParamId(0),
                        returns: ReturnContract::SharedInputPaths(
                            vec![InputPath {
                                input: input_reference(1),
                                projection: vec![].into(),
                            }]
                            .into(),
                        ),
                    }]
                    .into(),
                }),
                params: vec![mutable, shared.clone()],
                result: shared,
            },
            args: vec![Operand::Move(place(4)), Operand::Copy(place(5))],
            destination: place(6),
            next: BlockId(3),
        });
        f.blocks.push(BasicBlock {
            statements: vec![
                read_payload(payload_field(&ty, 0, 0)),
                located(Statement::Inspect(place(1))),
            ],
            terminator: located(Terminator::Return),
        });
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        assert_eq!(result.stats.variant_proofs_killed, 1);
    }

    #[test]
    fn projected_returns_preserve_nested_places_and_unique_parent_freeze() {
        for source in [
            "fn f<'a>(p:&'a (bool,[i32;2]))->&'a i32{&(*p).1[1]}",
            "struct S{a:bool,b:(i32,[i32;2])} fn f<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).b.1[1]}",
            "fn f()->i32{let mut p=(1,2);let r=&mut p;let q=get(r);*q=3;(*r).1} fn get<'a>(p:&'a mut (i32,i32))->&'a mut i32{&mut (*p).0}",
            "fn f<'a>(p:&'a (i32,bool))->&'a i32{id(get(p))} fn get<'a>(p:&'a (i32,bool))->&'a i32{&(*p).0} fn id<'a>(p:&'a i32)->&'a i32{p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let source = "fn f()->i32{let mut p=(1,2);let r=&mut p;let q=get(r);let n=(*r).1;*q+n} fn get<'a>(p:&'a mut (i32,i32))->&'a mut i32{&mut (*p).0}";
        assert!(!analyze(&borrowed_source(source)).passed());
    }

    #[test]
    fn projected_call_candidates_remain_correlated_with_original_arguments() {
        let f = borrowed_source(
            "fn f(p:&(i32,bool),q:&(bool,i32),b:bool)->i32{*pick(b,p,q)} fn pick<'a>(b:bool,p:&'a (i32,bool),q:&'a (bool,i32))->&'a i32{if b{&(*p).0}else{&(*q).1}}",
        );
        assert!(analyze(&f).passed());
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &Place {
                    local: destination.local,
                    projection: vec![Projection::Deref],
                },
                block,
                None,
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 2);
        for (param, field) in [(f.params[0], 0), (f.params[1], 1)] {
            assert!(targets.iter().any(|target| target.place.root
                == CanonicalRoot::ParameterReferent(
                    param,
                    borrow::fixed_location(ReferencePosition { depth: 0 })
                )
                && target.place.projection == [Projection::Field(field)]));
        }
        assert!(targets[0].authority != targets[1].authority);
        let f = borrowed_source(
            "fn f()->i32{let x=(1,2);*pick(&x,&x)} fn pick<'a>(p:&'a (i32,i32),q:&'a (i32,i32))->&'a i32{&(*p).0}",
        );
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &Place {
                    local: destination.local,
                    projection: vec![Projection::Deref],
                },
                block,
                None,
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 4);
        assert!(targets.iter().any(|a| {
            targets
                .iter()
                .any(|b| a.place == b.place && a.authority != b.authority)
        }));
        assert!(analyze(&f).passed());
    }

    #[test]
    fn projected_metadata_limits_precede_structural_helper_allocation() {
        use super::super::ir::{InputPath, ReturnContract};
        for (count, depth) in [(4097, 0), (1025, 128), (1, 129)] {
            let mut f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
            let path = InputPath {
                input: input_reference(0),
                projection: vec![Projection::Field(0); depth].into(),
            };
            first_return(&mut f.ref_contract).returns =
                ReturnContract::SharedInputPaths(vec![path; count].into());
            let result = analyze(&f);
            assert!(
                !result.complete,
                "{count}/{depth}: {:?}",
                result.diagnostics
            );
            assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        }
        let mut f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
        first_return(&mut f.ref_contract).returns = ReturnContract::SharedInputPaths(
            vec![InputPath {
                input: input_reference(0),
                projection: vec![Projection::Deref].into(),
            }]
            .into(),
        );
        let result = analyze(&f);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn projected_candidate_products_and_generated_scratch_fail_closed() {
        let f = borrowed_source(
            "fn f(p:&[i32;32])->i32{*get(p)} fn get<'a>(p:&'a [i32;32])->&'a i32{&(*p)[0]}",
        );
        assert!(analyze(&f).passed());
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let mut budget = Budget::new(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
    }

    #[test]
    fn mutable_returns_transfer_unique_authority_and_resume_after_last_use() {
        for source in [
            "fn f<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*q=2;*r=3;x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(id(&mut x));let moved=q;*moved=3;x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(&mut x);let s=&*q;let n=*s;*q=3;n+x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;*id(&mut x)} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn mutable_return_freezes_parent_and_preserves_lifetime_dependencies() {
        for source in [
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*r;*q+n} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*r=2;*q} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(&mut x);let moved=q;let n=*q;*moved+n} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f<'a,'b>(s:&'a mut bool,x:&'b mut i32)->&'b mut i32{pick(s,x)} fn pick<'a>(s:&'a mut bool,x:&'a mut i32)->&'a mut i32{x}",
            "fn f()->i32{let mut x=7;let r:&mut i32;{let mut s=true;r=pick(&mut s,&mut x);}*r} fn pick<'a>(s:&'a mut bool,x:&'a mut i32)->&'a mut i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete, "{source}: {:?}", result.diagnostics);
            assert!(!result.passed(), "accepted {source}");
        }
    }

    #[test]
    fn mutable_return_origins_keep_existing_memory_and_work_caps() {
        let f = borrowed_source(
            "fn f(p:&mut i32)->i32{*id(p)} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
        );
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let mut budget = Budget::new(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        for limits in [
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
            Limits {
                work: 1,
                ..Limits::default()
            },
        ] {
            let result = analyze_with_limits(&f, limits);
            assert!(!result.complete);
            assert!(!result.passed());
        }
        assert!(analyze(&f).passed());
    }

    #[test]
    fn mutable_normal_return_moves_handle_and_rejects_forged_copy() {
        let f = borrowed_source("fn f<'a>(p:&'a mut i32)->&'a mut i32{p}");
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        let explicit_moves = f
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|item| {
                matches!(
                    item.value,
                    Statement::Assign {
                        value: Rvalue::Use(Operand::Move(_)),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(result.stats.moves, explicit_moves + 1);
        let mut forged = f.clone();
        let value = forged
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|item| match &mut item.value {
                Statement::Assign {
                    value: Rvalue::Use(op @ Operand::Move(_)),
                    ..
                } => Some(op),
                _ => None,
            })
            .unwrap();
        let Operand::Move(source) = value else {
            unreachable!()
        };
        *value = Operand::Copy(source.clone());
        assert!(forged.validate().is_err());
        assert!(!analyze(&forged).passed());
    }

    #[test]
    fn mutable_result_union_write_kills_all_proofs_without_restoring_init() {
        let f = borrowed_source(
            "enum E{A(i32),B(i32)} fn f(b:bool){let mut x=E::A(1);let mut y=E::A(2);let q=pick(b,&mut x,&mut y);*q=E::A(3);} fn pick<'a>(b:bool,x:&'a mut E,y:&'a mut E)->&'a mut E{if b{x}else{y}}",
        );
        assert!(analyze(&f).passed());
        let (block, statement, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(block, bb)| {
                bb.statements
                    .iter()
                    .enumerate()
                    .find_map(|(statement, item)| match &item.value {
                        Statement::Assign { destination, .. }
                            if destination.projection.first() == Some(&Projection::Deref) =>
                        {
                            Some((BlockId(block), statement, destination.clone()))
                        }
                        _ => None,
                    })
            })
            .unwrap();
        let paths = Paths::new(&f, 131072).unwrap();
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &destination,
                block,
                Some(statement),
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 2);
        let nodes: Vec<_> = targets
            .iter()
            .map(|target| paths.resolve_canonical(&target.place))
            .collect();
        for initialized in [true, false] {
            let mut state = State::new(f.locals.len(), paths.nodes.len());
            for local in 0..f.locals.len() {
                state.must_live.set(local, true);
                state.may_live.set(local, true);
            }
            state
                .must_init
                .range(0, paths.nodes.len(), true, &mut budget)
                .unwrap();
            state
                .may_init
                .range(0, paths.nodes.len(), true, &mut budget)
                .unwrap();
            for node in &nodes {
                state.selected[*node] = 0;
            }
            if !initialized {
                state.must_init.set(nodes[0] + 1, false);
            }
            let must = state.must_init.clone();
            let may = state.may_init.clone();
            Transfer {
                facts: FactsCtx::Concrete,
                f: &f,
                paths: &paths,
                budget: &mut budget,
                report: None,
                block,
                statement: Some(statement),
                origin: None,
                diagnostic_limit: 10,
                loans: Some(&plan),
                phase: Phase::Write,
            }
            .write(&mut state, &destination)
            .unwrap();
            assert_eq!(state.must_init, must);
            assert_eq!(state.may_init, may);
            for node in &nodes {
                assert_eq!(state.selected[*node], usize::MAX);
            }
        }
    }

    #[test]
    fn shared_returns_preserve_transitive_origins_and_parent_authority() {
        for source in [
            "fn f<'a>(p:&'a i32)->&'a i32{p}",
            "fn f<'a:'b,'b>(s:&'a bool,x:&'b i32)->&'b i32{pick(s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let x=7;*id(id(&x))} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*r;n+*q} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*q;*r=2;n+*r} fn id<'a>(x:&'a i32)->&'a i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn shared_return_lifetime_dependencies_do_not_launder_universal_bounds() {
        for source in [
            "fn f<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{pick(s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
            "fn f<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{id(pick(s,x))} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f<'a>(x:&'a i32)->&'a i32{let s=true;pick(&s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(!result.passed(), "accepted {source}");
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::InvalidReference(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn shared_returns_pin_non_target_inputs_but_not_independent_regions() {
        let same = "fn f()->i32{let x=7;let r:&i32;{let s=true;r=pick(&s,&x);}*r} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(same));
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
        let independent = "fn f()->i32{let x=7;let r:&i32;{let s=true;r=pick(&s,&x);}*r} fn pick<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{x}";
        let result = analyze(&borrowed_source(independent));
        assert!(result.passed(), "{:?}", result.diagnostics);
        let mutation = "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*r=2;*q} fn id<'a>(x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(mutation));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
    }

    #[test]
    fn shared_return_call_defines_real_outgoing_origin_without_sites() {
        let f = borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
        assert!(f.loan_sites.is_empty());
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let deref = Place {
            local: destination.local,
            projection: vec![Projection::Deref],
        };
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        assert!(
            plan.resolve(&deref, block, None, Phase::Read, &mut budget)
                .unwrap()
                .is_err()
        );
        let after = plan
            .resolve(&deref, block, None, Phase::Write, &mut budget)
            .unwrap()
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].place.root,
            CanonicalRoot::ParameterReferent(
                f.params[0],
                borrow::fixed_location(ReferencePosition { depth: 0 })
            )
        );
        let mut budget = Budget::new(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        assert!(analyze(&f).passed());
    }

    #[test]
    fn shared_return_origin_contract_and_scratch_caps_fail_incomplete() {
        let mut f = borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
        let call = f
            .blocks
            .iter()
            .find(|block| matches!(block.terminator.value, Terminator::Call { .. }))
            .unwrap()
            .clone();
        f.blocks
            .extend(std::iter::repeat_n(call, super::super::ir::MAX_LOAN_SITES));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        for count in [64, 65] {
            let mut f =
                borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
            for block in &mut f.blocks {
                if let Terminator::Call { signature, .. } = &mut block.terminator.value {
                    std::sync::Arc::make_mut(&mut signature.ref_contract).region_count = count;
                }
            }
            let result = analyze_with_limits(
                &f,
                Limits {
                    operations: 512,
                    ..Limits::default()
                },
            );
            assert!(!result.complete, "count={count}: {:?}", result.diagnostics);
            assert!(!result.passed());
        }
        let f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let mut malformed = f;
        first_return(&mut malformed.ref_contract).region = super::super::ir::RegionParamId(4);
        let result = analyze(&malformed);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn shared_return_cycles_fail_closed_and_empty_diverging_contract_can_pass() {
        let f = borrowed_source(
            "fn f(p:&i32)->i32{let mut r=p;loop{r=id(r);if true{break;}}*r} fn id<'a>(x:&'a i32)->&'a i32{x}",
        );
        let result = analyze(&f);
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::UnsupportedBorrowing(_))),
            "{:?}",
            result.diagnostics
        );
        // Loop-result contextual typing is a separate frontend boundary. This
        // valid private CFG has no normal return and therefore no obligation
        // to fabricate an eligible returned pointer.
        let mut f = borrowed_source("fn f<'a,'b>(p:&'a i32)->&'a i32{p}");
        let contract = std::sync::Arc::make_mut(&mut f.ref_contract);
        // Canonical source normalization removes unused 'b. This private
        // diverging fixture deliberately introduces a distinct output region,
        // so declare that region before assigning its canonical ID.
        contract.region_count = 2;
        let result = &mut std::sync::Arc::make_mut(&mut contract.results)[0];
        result.region = super::super::ir::RegionParamId(1);
        result.returns = super::super::ir::ReturnContract::SharedInputPaths(vec![].into());
        f.blocks = vec![BasicBlock {
            statements: vec![],
            terminator: located(Terminator::Goto(BlockId(0))),
        }];
        f.entry = BlockId(0);
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn reference_parameters_without_sites_have_independent_external_paths() {
        for source in [
            "fn f(p:&mut i32)->i32{*p=3;*p}",
            "fn f(p:&i32,q:&i32)->i32{*p+*q}",
        ] {
            let f = borrowed_source(source);
            assert!(f.loan_sites.is_empty());
            let result = analyze(&f);
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
            let paths = Paths::new(&f, 131072).unwrap();
            assert!(paths.nodes.len() > f.locals.len());
            let mut budget = Budget::new(32 * 1024 * 1024);
            let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
            assert!(plan.bytes > 0);
            let mut budget = Budget::new(32 * 1024 * 1024);
            assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        }
    }

    #[test]
    fn reference_parameter_referent_outlives_moved_carrier_storage() {
        let f =
            borrowed_source("fn f(p:&mut i32)->i32{let c:&mut i32;{let q=p;c=&mut *q;}*c=3;*c}");
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn reference_parameter_bounds_count_formals_and_external_expansion() {
        let mut f = borrowed_source("fn f(p:&i32,q:&i32)->i32{*p+*q}");
        let result = analyze_with_limits(
            &f,
            Limits {
                paths: f.locals.len(),
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let p = f.params[0];
        f.loan_sites = vec![
            cfg::LoanSite {
                timing: cfg::LoanTiming::Immediate,
                kind: super::super::ir::BorrowKind::Shared,
                place: Place {
                    local: p,
                    projection: vec![Projection::Deref]
                },
                origin: None
            };
            super::super::ir::MAX_LOAN_SITES - 1
        ];
        let result = analyze(&f);
        assert!(!result.complete);
        assert!(!result.passed());
    }

    #[test]
    fn borrowing_last_shared_read_ends_before_same_assignment_write() {
        let mut f = borrowed_source("fn f()->i32{let mut x=1;let r=&x;x=*r;x}");
        let x = LocalId(f.locals.iter().position(|local| local.name == "x").unwrap());
        let r = LocalId(f.locals.iter().position(|local| local.name == "r").unwrap());
        // Source lowering may snapshot the RHS. Force the private CFG's last
        // reference use into the owner assignment itself to test both phases.
        let value = f
            .blocks
            .iter_mut()
            .rev()
            .flat_map(|block| block.statements.iter_mut().rev())
            .find_map(|statement| match &mut statement.value {
                Statement::Assign { destination, value } if destination.local == x => Some(value),
                _ => None,
            })
            .unwrap();
        *value = Rvalue::Use(Operand::Copy(Place {
            local: r,
            projection: vec![Projection::Deref],
        }));
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn borrowing_unused_issuance_checks_initialization_and_mutability() {
        for source in [
            "fn f(){let x:i32;let _=&x;}",
            "fn f(){let x=1;let _=&mut x;}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(!result.passed(), "accepted {source}");
        }
    }

    #[test]
    fn borrowing_parent_retarget_and_storage_end_preserve_child_target() {
        for source in [
            "fn f()->i32{let mut x=1;let mut y=2;let mut r=&mut x;let c=&mut *r;r=&mut y;*c=3;*r=4;x+y}",
            "fn f()->i32{let mut x=1;let c:&mut i32;{let r=&mut x;c=&mut *r;}*c=2;x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn borrowing_live_child_blocks_parent_move_and_owner_end() {
        for source in [
            "fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;let moved=r;*c=2;*moved}",
            "fn f()->i32{let c:&mut i32;{let mut x=1;let r=&mut x;c=&mut *r;}*c=2;0}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn borrowing_child_authority_is_candidate_specific() {
        let shared =
            borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&*r;let n=*r;n+*c}");
        let result = analyze(&shared);
        assert!(result.passed(), "{:?}", result.diagnostics);
        let unique =
            borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;let n=*r;n+*c}");
        let result = analyze(&unique);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
    }

    #[test]
    fn borrowing_preflight_and_scratch_caps_fail_closed() {
        let f = borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;*c=2;x}");
        let mut budget = Budget::new(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        assert!(plan.bytes > 4096);
        let mut budget = Budget::new(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let result = analyze_with_limits(
            &f,
            Limits {
                work: 1,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
    }

    fn located<T>(value: T) -> Located<T> {
        Located {
            value,
            origin: None,
        }
    }
    fn place(local: usize) -> Place {
        Place {
            local: LocalId(local),
            projection: vec![],
        }
    }
    fn local(ty: Ty) -> LocalDecl {
        LocalDecl {
            name: String::new(),
            ty,
            mutable: true,
        }
    }
    fn synthetic_call_target() -> super::super::ir::Callee {
        super::super::ir::Callee::Direct(super::super::ir::CallTarget {
            declaration: source_decl(
                SnapshotId::allocate().unwrap(),
                0,
                &super::super::ir::PlanBudget::default(),
            )
            .unwrap(),
            type_args: Arc::from([]),
        })
    }
    fn nominal(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        fields: Vec<Ty>,
    ) -> Ty {
        let snapshot = SnapshotId::allocate().unwrap();
        Ty::Struct(StructTy::new(StructDef {
            id: NominalAppId { snapshot, index: 0 },
            source_decl: source_decl(snapshot, 0, &super::super::ir::PlanBudget::default())
                .unwrap(),
            type_args: Arc::from([]),
            name: "Paths".into(),
            module: modules.source_root().unwrap(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.source_root().unwrap()),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    visibility: crate::compiler::ir::Visibility::Restricted(
                        modules.source_root().unwrap(),
                    ),
                })
                .collect::<Vec<_>>()
                .into(),
        }))
    }
    fn fixture() -> cfg::Function {
        cfg::Function {
            body_substitution: None,
            call_evidence: Arc::from([]),
            lowering_proof: None,
            ref_contract: Default::default(),
            name: "caps".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![local(Ty::Unit)],
            entry: BlockId(0),
            return_local: LocalId(0),
            loan_sites: vec![],
            blocks: vec![BasicBlock {
                statements: vec![
                    located(Statement::StorageLive(LocalId(0))),
                    located(Statement::Assign {
                        destination: place(0),
                        value: Rvalue::Use(Operand::Constant(Constant::Unit)),
                    }),
                ],
                terminator: located(Terminator::Return),
            }],
        }
    }

    fn unit_enum(modules: &std::sync::Arc<super::super::module_identity::ModuleArena>) -> EnumTy {
        let snapshot = SnapshotId::allocate().unwrap();
        EnumTy::new(EnumDef {
            id: NominalAppId { snapshot, index: 0 },
            source_decl: source_decl(snapshot, 0, &super::super::ir::PlanBudget::default())
                .unwrap(),
            type_args: Arc::from([]),
            name: "Tag".into(),
            module: modules.source_root().unwrap(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.source_root().unwrap()),
            variants: ["A", "B"]
                .into_iter()
                .map(|name| VariantDef {
                    name: name.into(),
                    shape: VariantShape::Unit,
                    fields: vec![].into(),
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn enum_fixture(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
    ) -> cfg::Function {
        let mut f = fixture();
        let ty = Ty::Enum(unit_enum(modules));
        f.locals
            .extend([local(ty.clone()), local(Ty::Int(IntTy::I32)), local(ty)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
        ]);
        f
    }

    fn discriminant() -> Located<Statement> {
        located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Discriminant(place(1)),
        })
    }

    fn payload_enum(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        variants: Vec<Vec<Ty>>,
    ) -> EnumTy {
        let snapshot = SnapshotId::allocate().unwrap();
        EnumTy::new(EnumDef {
            id: NominalAppId { snapshot, index: 0 },
            source_decl: source_decl(snapshot, 0, &super::super::ir::PlanBudget::default())
                .unwrap(),
            type_args: Arc::from([]),
            name: "Payload".into(),
            module: modules.source_root().unwrap(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.source_root().unwrap()),
            variants: variants
                .into_iter()
                .enumerate()
                .map(|(variant, fields)| VariantDef {
                    name: format!("V{variant}"),
                    shape: VariantShape::Tuple,
                    fields: fields
                        .into_iter()
                        .enumerate()
                        .map(|(index, ty)| FieldDef {
                            name: index.to_string(),
                            ty,
                            visibility: crate::compiler::ir::Visibility::Restricted(
                                modules.source_root().unwrap(),
                            ),
                        })
                        .collect::<Vec<_>>()
                        .into(),
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn variant(ty: &EnumTy, index: usize) -> VariantId {
        VariantId {
            owner: ty.def().id,
            index,
        }
    }

    fn payload_field(ty: &EnumTy, selected: usize, field: usize) -> Place {
        Place {
            local: LocalId(1),
            projection: vec![
                Projection::Downcast(variant(ty, selected)),
                Projection::Field(field),
            ],
        }
    }

    fn payload_fixture(ty: &EnumTy) -> cfg::Function {
        let mut f = fixture();
        f.locals
            .extend([local(Ty::Enum(ty.clone())), local(Ty::Int(IntTy::I32))]);
        f.params.push(LocalId(1));
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(2))));
        f
    }

    fn read_payload(source: Place) -> Located<Statement> {
        located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Copy(source)),
        })
    }

    fn split_payload(f: &mut cfg::Function, ty: &EnumTy) {
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: place(1),
            variant: variant(ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
        ]);
    }

    #[test]
    fn reference_calls_capture_effects_before_move_and_preserve_initialization() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::BorrowKind;
        for kind in [BorrowKind::Shared, BorrowKind::Mut] {
            let ty = payload_enum(
                &modules,
                vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
            );
            let mut f = payload_fixture(&ty);
            let reference = Ty::Ref {
                kind,
                referent: std::sync::Arc::new(Ty::Enum(ty.clone())),
            };
            f.locals.push(local(reference.clone()));
            f.loan_sites.push(cfg::LoanSite {
                timing: cfg::LoanTiming::Immediate,
                kind,
                place: place(1),
                origin: None,
            });
            split_payload(&mut f, &ty);
            f.blocks[1].statements.extend([
                located(Statement::StorageLive(LocalId(3))),
                located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Borrow(cfg::LoanId(0)),
                }),
            ]);
            f.blocks[1].terminator = located(Terminator::Call {
                target: synthetic_call_target(),
                name: "callee".into(),
                signature: super::super::ir::Signature {
                    ref_contract: Default::default(),
                    params: vec![reference],
                    result: Ty::Unit,
                },
                args: vec![Operand::Move(place(3))],
                destination: place(0),
                next: BlockId(3),
            });
            f.blocks.push(BasicBlock {
                statements: vec![
                    read_payload(payload_field(&ty, 0, 0)),
                    located(Statement::Inspect(place(1))),
                ],
                terminator: located(Terminator::Return),
            });
            f.validate().unwrap();
            let result = analyze(&f);
            assert!(result.complete);
            if kind == BorrowKind::Shared {
                assert!(result.passed(), "{:?}", result.diagnostics);
                assert_eq!(result.stats.variant_proofs_killed, 0);
            } else {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(
                    result.diagnostics[0].kind,
                    ErrorKind::VariantNotDefinitelySelected
                );
                assert_eq!(result.stats.variant_proofs_killed, 1);
            }
        }
    }

    #[test]
    fn reference_call_union_mutation_kills_every_candidate_proof() {
        let (_module_names, modules) = fixture_modules();
        use super::super::ir::BorrowKind;
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let pair = Ty::Tuple(vec![Ty::Enum(ty.clone()), Ty::Enum(ty.clone())].into());
        let reference = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: std::sync::Arc::new(Ty::Enum(ty.clone())),
        };
        let mut f = fixture();
        f.locals.extend([
            local(pair),
            local(Ty::Int(IntTy::I32)),
            local(reference.clone()),
            local(Ty::Bool),
        ]);
        f.params.extend([LocalId(1), LocalId(4)]);
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
        ]);
        let member = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        for index in 0..2 {
            f.loan_sites.push(cfg::LoanSite {
                timing: cfg::LoanTiming::Immediate,
                kind: BorrowKind::Mut,
                place: member(index),
                origin: None,
            });
        }
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: member(0),
            variant: variant(&ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::SwitchVariant {
                    place: member(1),
                    variant: variant(&ty, 0),
                    then_block: BlockId(3),
                    else_block: BlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Switch {
                    condition: Operand::Copy(place(4)),
                    then_block: BlockId(4),
                    else_block: BlockId(5),
                }),
            },
        ]);
        for loan in 0..2 {
            f.blocks.push(BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Borrow(cfg::LoanId(loan)),
                })],
                terminator: located(Terminator::Goto(BlockId(6))),
            });
        }
        f.blocks.push(BasicBlock {
            statements: vec![],
            terminator: located(Terminator::Call {
                target: synthetic_call_target(),
                name: "callee".into(),
                signature: super::super::ir::Signature {
                    ref_contract: Default::default(),
                    params: vec![reference],
                    result: Ty::Unit,
                },
                args: vec![Operand::Move(place(3))],
                destination: place(0),
                next: BlockId(7),
            }),
        });
        let reads = (0..2)
            .map(|index| {
                let mut field = member(index);
                field
                    .projection
                    .extend([Projection::Downcast(variant(&ty, 0)), Projection::Field(0)]);
                read_payload(field)
            })
            .chain([located(Statement::Inspect(place(1)))])
            .collect();
        f.blocks.push(BasicBlock {
            statements: reads,
            terminator: located(Terminator::Return),
        });
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::VariantNotDefinitelySelected)
        );
        assert_eq!(result.stats.variant_proofs_killed, 2);
    }

    fn array(element: Ty, len: usize) -> Ty {
        Ty::Array {
            element: std::sync::Arc::new(element),
            len,
        }
    }

    fn indexed(local: usize, index: usize) -> Place {
        Place {
            local: LocalId(local),
            projection: vec![Projection::ConstantIndex(index)],
        }
    }

    fn dynamic_fixture(element: Ty) -> cfg::Function {
        let mut f = fixture();
        f.locals.extend([
            local(array(element.clone(), 2)),
            local(Ty::Int(IntTy::Usize)),
            local(element),
        ]);
        f.params.extend([LocalId(1), LocalId(2)]);
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(3))));
        f
    }

    fn dynamic_read() -> Located<Statement> {
        located(Statement::Assign {
            destination: place(3),
            value: Rvalue::IndexCopy {
                array: place(1),
                index: Operand::Copy(place(2)),
            },
        })
    }

    fn dynamic_inspect() -> Located<Statement> {
        located(Statement::IndexInspect {
            array: place(1),
            index: Operand::Copy(place(2)),
        })
    }

    #[test]
    fn compound_index_reads_snapshots_before_initialized_array_and_never_initializes_it() {
        use super::super::frontend::BinaryOp;
        for op in [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
            BinaryOp::Shl,
            BinaryOp::Shr,
        ] {
            let mut f = dynamic_fixture(Ty::Int(IntTy::Usize));
            let update = Statement::IndexAssignOp {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Copy(place(2)),
                op,
            };
            f.blocks[0].statements.push(located(update.clone()));
            assert!(analyze(&f).passed());
            let mut uninitialized = f.clone();
            uninitialized.params.retain(|id| *id != LocalId(1));
            uninitialized.blocks[0]
                .statements
                .insert(0, located(Statement::StorageLive(LocalId(1))));
            let result = analyze(&uninitialized);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
            );
            f.locals[1].mutable = false;
            let immutable = analyze(&f);
            assert!(
                immutable.complete && !immutable.passed(),
                "{op:?}: {:?}",
                immutable.diagnostics
            );
            assert!(
                immutable.diagnostics.iter().any(|d| matches!(
                    &d.kind,
                    ErrorKind::InvalidCfg(message)
                        if message == "compound indexed mutation requires mutable binding"
                )),
                "{op:?}: {:?}",
                immutable.diagnostics
            );
            f.locals[1].mutable = true;
            f.blocks[0].statements.last_mut().unwrap().value = Statement::IndexAssignOp {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Move(place(2)),
                op,
            };
            let result = analyze(&f);
            assert!(
                result.diagnostics.iter().any(|d| d.place == Some(place(2))
                    && d.kind == ErrorKind::ReadNotDefinitelyInitialized)
            );
            f.blocks[0].statements = vec![located(update)];
            let paths = Paths::new(&f, 100).unwrap();
            let mut state = State::new(f.locals.len(), paths.nodes.len());
            state.must_live.set(1, true);
            state.must_live.set(2, true);
            state.must_init.set(paths.resolve(&place(2)), true);
            state.may_init.set(paths.resolve(&place(2)), true);
            state.must_init.set(paths.resolve(&indexed(1, 0)), true);
            state.may_init.set(paths.resolve(&indexed(1, 1)), true);
            let must = state.must_init.clone();
            let may = state.may_init.clone();
            let mut budget = Budget::new(10000);
            Transfer {
                facts: FactsCtx::Concrete,
                f: &f,
                paths: &paths,
                budget: &mut budget,
                report: None,
                block: BlockId(0),
                statement: None,
                origin: None,
                diagnostic_limit: 10,
                loans: None,
                phase: Phase::Read,
            }
            .block(&mut state)
            .unwrap();
            assert_eq!(state.must_init, must);
            assert_eq!(state.may_init, may);
        }
    }

    #[test]
    fn compound_source_uses_rhs_initialized_current_carrier_and_active_loan_permissions() {
        for source in [
            "fn f()->i32{let mut x:i32;x += {x=10;1};x}",
            "fn f()->i32{let mut x=1;let mut y=2;let mut r=&mut x;*r += {r=&mut y;3};x+y}",
            "fn f(i:usize)->i32{let mut a=[1,2];let r=&a[0];let n=*r;a[i]+=1;n}",
            "fn f(i:usize)->i32{let mut a:[i32;2];a[{a=[7,8];i}]+=1;a[0]}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        for source in [
            "fn f()->i32{let mut x:i32;x+=1;x}",
            "fn f()->i32{let x=1;let r=&x;*r+=1;x}",
            "fn f(i:usize)->i32{let mut a=[1,2];let r=&a[0];a[i]+=1;*r}",
        ] {
            assert!(!analyze(&borrowed_source(source)).passed(), "{source}");
        }
    }

    #[test]
    fn wide_compound_index_reference_paths_preserve_exclusive_scalar_access() {
        for source in [
            "fn f(a:&mut [u128;2],i:usize,n:u128){a[i]<<=n;}",
            "fn f(a:&mut [u128;2],i:usize,n:u128){(*a)[i]>>=n;}",
            "fn f(a:&mut ([u128;2],u8),i:usize,n:u128){a.0[i]<<=n;}",
            "fn f(i:usize,n:u128){let mut a=[1u128,2u128];let t=(&mut a,0u8);t.0[i]<<=n;}",
            "fn f(i:usize,n:u128){let mut a=[1i128,2i128];let t=(&mut a,0u8);t.0[i]>>=n;}",
            "fn f(i:usize)->u128{let mut x=[1u128,2u128];let mut y=[3u128,4u128];let mut a=&mut x;a[i]<<={a=&mut y;1u128};x[0]+y[0]}",
            "fn f()->u128{let mut x=[1u128,2u128];let mut y=[3u128,4u128];let mut a=&mut x;a[{a=&mut y;0usize}]<<=1u128;x[0]+y[0]}",
        ] {
            let proof = verify_template(carrier_reference_template(source))
                .unwrap_or_else(|analysis| panic!("{source}: {:?}", analysis.diagnostics));
            assert!(proof.analysis().passed());
        }
        for source in [
            "fn f(a:&mut [u128;2],i:usize,n:u128)->u128{let r=&(*a)[0];a[i]<<=n;*r}",
            "fn f(i:usize,n:u128)->u128{let mut a=[1u128,2u128];let t=(&mut a,0u8);let r=&(*t.0)[0];t.0[i]>>=n;*r}",
        ] {
            let result = verify_template(carrier_reference_template(source)).unwrap_err();
            assert!(!result.passed(), "{source}");
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| matches!(diagnostic.kind, ErrorKind::BorrowConflict(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn compound_index_dead_validation_and_operator_preflight_fail_closed() {
        use super::super::frontend::BinaryOp;
        let base = dynamic_fixture(Ty::Int(IntTy::Usize));
        let ordinary = Statement::IndexAssign {
            array: place(1),
            index: Operand::Copy(place(2)),
            value: Operand::Copy(place(2)),
        };
        let compound = Statement::IndexAssignOp {
            array: place(1),
            index: Operand::Copy(place(2)),
            value: Operand::Copy(place(2)),
            op: BinaryOp::Add,
        };
        let mut costs = Vec::new();
        for statement in [ordinary, compound] {
            let mut f = base.clone();
            f.blocks[0].statements.push(located(statement));
            let minimum = (1..1000)
                .find(|operations| {
                    preflight(
                        &f,
                        Limits {
                            operations: *operations,
                            ..Limits::default()
                        },
                    )
                    .is_ok()
                })
                .expect("small fixture has bounded preflight");
            costs.push(minimum);
        }
        assert_eq!(costs[1] - costs[0], 1);
        for (op, index, value) in [
            (
                BinaryOp::And,
                Operand::Copy(place(2)),
                Operand::Copy(place(2)),
            ),
            (
                BinaryOp::Eq,
                Operand::Copy(place(2)),
                Operand::Copy(place(2)),
            ),
            (
                BinaryOp::Add,
                Operand::Constant(Constant::Bool(true)),
                Operand::Copy(place(2)),
            ),
            (
                BinaryOp::Add,
                Operand::Copy(place(2)),
                Operand::Constant(Constant::Bool(true)),
            ),
            (
                BinaryOp::Shl,
                Operand::Copy(place(2)),
                Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::I8,
                    bits: 255,
                })),
            ),
        ] {
            let mut f = base.clone();
            f.blocks.push(BasicBlock {
                statements: vec![located(Statement::IndexAssignOp {
                    array: place(1),
                    index,
                    value,
                    op,
                })],
                terminator: located(Terminator::Return),
            });
            let result = analyze(&f);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_))),
                "{:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn dynamic_copy_reads_and_ignored_noncopy_values_do_not_consume_array() {
        let (_module_names, modules) = fixture_modules();
        for element in [
            Ty::Int(IntTy::I32),
            Ty::Unit,
            Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Int(IntTy::Usize)].into()),
        ] {
            let mut f = dynamic_fixture(element);
            f.blocks[0].statements.extend([
                dynamic_read(),
                dynamic_read(),
                dynamic_inspect(),
                located(Statement::Inspect(place(1))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "{:?}", result.diagnostics);
            assert_eq!(result.stats.moves, 0);
        }
        let mut f = dynamic_fixture(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
        f.blocks[0]
            .statements
            .extend([dynamic_inspect(), located(Statement::Inspect(place(1)))]);
        assert!(analyze(&f).passed());
        f.blocks[0].statements.push(dynamic_read());
        assert!(matches!(
            analyze(&f).diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn dynamic_index_readiness_follows_ordered_initialization_but_not_earlier_static_checks() {
        let mut f = dynamic_fixture(Ty::Int(IntTy::I32));
        f.params.retain(|local| *local != LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Aggregate {
                    ty: array(Ty::Int(IntTy::I32), 2),
                    fields: vec![
                        Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::I32,
                            bits: 7,
                        })),
                        Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::I32,
                            bits: 8,
                        })),
                    ],
                },
            }),
            dynamic_read(),
        ]);
        assert!(analyze(&f).passed());
        // Represents an earlier static indexed-base readiness check. Its
        // diagnostic cannot be erased by initialization in the later index.
        f.blocks[0]
            .statements
            .insert(4, located(Statement::Inspect(place(1))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_inspect_rejects_unready_array_or_index_without_consuming_either() {
        for missing in [1, 2] {
            let mut f = dynamic_fixture(Ty::Int(IntTy::I32));
            f.params.retain(|local| *local != LocalId(missing));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(missing))),
                dynamic_inspect(),
            ]);
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].place, Some(place(missing)));
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
            assert_eq!(result.stats.moves, 0);
        }
    }

    #[test]
    fn dynamic_operations_consume_index_before_array_read_and_rhs_before_index() {
        let mut f = dynamic_fixture(Ty::Int(IntTy::Usize));
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::IndexCopy {
                array: place(1),
                index: Operand::Move(indexed(1, 0)),
            },
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(1)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );

        let mut f = dynamic_fixture(Ty::Int(IntTy::Usize));
        f.blocks[0].statements.push(located(Statement::IndexAssign {
            array: place(1),
            index: Operand::Copy(place(2)),
            value: Operand::Move(place(2)),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(2)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_noncopy_replacement_consumes_snapshot_but_preserves_array_initialization() {
        let (_module_names, modules) = fixture_modules();
        let element = nominal(&modules, vec![Ty::Int(IntTy::I32)]);
        let mut f = dynamic_fixture(element.clone());
        f.locals.push(local(element));
        f.params.push(LocalId(4));
        f.blocks[0].statements.extend([
            located(Statement::IndexAssign {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Move(place(4)),
            }),
            dynamic_inspect(),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(4))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].place, Some(place(4)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_replacement_requires_mutable_initialized_array() {
        for (mutable, initialized, expected) in [
            (true, true, None),
            (false, true, Some(ErrorKind::ImmutableReassignment)),
            (true, false, Some(ErrorKind::ReadNotDefinitelyInitialized)),
        ] {
            let mut f = dynamic_fixture(Ty::Int(IntTy::I32));
            f.locals[1].mutable = mutable;
            if !initialized {
                f.params.retain(|local| *local != LocalId(1));
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0].statements.push(located(Statement::IndexAssign {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::I32,
                    bits: 9,
                })),
            }));
            let result = analyze(&f);
            if let Some(expected) = expected {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(result.diagnostics[0].kind, expected);
            } else {
                assert!(result.passed(), "{:?}", result.diagnostics);
            }
        }
    }

    #[test]
    fn dynamic_replacement_never_changes_must_or_may_initialization_bits() {
        let f = dynamic_fixture(Ty::Int(IntTy::I32));
        let paths = Paths::new(&f, 100).unwrap();
        let mut state = State::new(f.locals.len(), paths.nodes.len());
        let node = paths.resolve(&place(1));
        state.must_live.set(1, true);
        state.must_init.set(node, true);
        state.must_init.set(paths.resolve(&indexed(1, 1)), true);
        state.may_init.set(node, true);
        state.may_init.set(paths.resolve(&indexed(1, 0)), true);
        let must = state.must_init.clone();
        let may = state.may_init.clone();
        let mut budget = Budget::new(1000);
        Transfer {
            facts: FactsCtx::Concrete,
            f: &f,
            paths: &paths,
            budget: &mut budget,
            report: None,
            block: BlockId(0),
            statement: None,
            origin: None,
            diagnostic_limit: 10,
            loans: None,
            phase: Phase::Read,
        }
        .index_write(&mut state, &place(1))
        .unwrap();
        assert_eq!(state.must_init, must);
        assert_eq!(state.may_init, may);
        assert!(state.ever_written.get(node));
    }

    #[test]
    fn dynamic_replacement_kills_all_contained_proofs_but_preserves_outer_enum() {
        let (_module_names, modules) = fixture_modules();
        let element = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let outer = payload_enum(
            &modules,
            vec![
                vec![array(Ty::Enum(element.clone()), 2), Ty::Int(IntTy::I32)],
                vec![Ty::Int(IntTy::I32)],
            ],
        );
        let mut f = fixture();
        f.locals.extend([
            local(Ty::Enum(outer.clone())),
            local(Ty::Int(IntTy::Usize)),
            local(Ty::Enum(element.clone())),
            local(Ty::Int(IntTy::I32)),
        ]);
        f.params.extend([LocalId(1), LocalId(2), LocalId(3)]);
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(4))));
        split_payload(&mut f, &outer);
        let array = payload_field(&outer, 0, 0);
        let item = |index| {
            let mut place = array.clone();
            place.projection.push(Projection::ConstantIndex(index));
            place
        };
        f.blocks[1].terminator = located(Terminator::SwitchVariant {
            place: item(0),
            variant: variant(&element, 0),
            then_block: BlockId(3),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::SwitchVariant {
                    place: item(1),
                    variant: variant(&element, 0),
                    then_block: BlockId(4),
                    else_block: BlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![
                    located(Statement::IndexAssign {
                        array: array.clone(),
                        index: Operand::Copy(place(2)),
                        value: Operand::Move(place(3)),
                    }),
                    located(Statement::Inspect(payload_field(&outer, 0, 1))),
                    located(Statement::Inspect(array.clone())),
                ],
                terminator: located(Terminator::Return),
            },
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.variant_proofs_killed, 2);
        for index in [0, 1] {
            let mut field = item(index);
            field.projection.extend([
                Projection::Downcast(variant(&element, 0)),
                Projection::Field(0),
            ]);
            f.blocks[4].statements.push(located(Statement::Assign {
                destination: place(4),
                value: Rvalue::Use(Operand::Copy(field)),
            }));
        }
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::VariantNotDefinitelySelected)
        );
    }

    #[test]
    fn usize_constants_are_scalar_leaves_without_index_narrowing() {
        assert!(Ty::Int(IntTy::Usize).is_copy());
        let mut f = dynamic_fixture(Ty::Int(IntTy::Usize));
        f.blocks[0].statements.extend([
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::Usize,
                    bits: u128::from(u64::MAX),
                }))),
            }),
            dynamic_read(),
            dynamic_inspect(),
        ]);
        let result = analyze(&f);
        // Static ownership does not narrow or guess a runtime index. Bounds
        // failure remains the runtime operation's responsibility, even at MAX.
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.move_paths, 6);
        assert!(preflight_ty(&Ty::Int(IntTy::Usize), 0, &mut Budget::new(1)).is_ok());
        f.locals[1].ty = array(Ty::Int(IntTy::Usize), 0);
        assert!(analyze(&f).passed());
    }

    #[test]
    fn malformed_runtime_indices_are_rejected_in_dead_blocks() {
        for operation in 0..3 {
            let mut f = dynamic_fixture(Ty::Int(IntTy::I32));
            let index = Operand::Constant(Constant::Int(IntValue {
                ty: IntTy::I32,
                bits: 0,
            }));
            let value = match operation {
                0 => Statement::Assign {
                    destination: place(3),
                    value: Rvalue::IndexCopy {
                        array: place(1),
                        index,
                    },
                },
                1 => Statement::IndexInspect {
                    array: place(1),
                    index,
                },
                _ => Statement::IndexAssign {
                    array: place(1),
                    index,
                    value: Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 7,
                    })),
                },
            };
            f.blocks.push(BasicBlock {
                statements: vec![located(value)],
                terminator: located(Terminator::Unreachable),
            });
            let result = analyze(&f);
            assert!(!result.passed());
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
            assert_eq!(result.stats.worklist_pops, 0);
        }
    }

    #[test]
    fn runtime_index_operations_preflight_array_subtrees_before_validation() {
        let mut base = dynamic_fixture(Ty::Int(IntTy::I32));
        base.locals[1].ty = array(Ty::Int(IntTy::I32), 64);
        let base_cost = (0..=1024)
            .find(|operations| {
                preflight(
                    &base,
                    Limits {
                        operations: *operations,
                        ..Limits::default()
                    },
                )
                .is_ok()
            })
            .expect("small fixture fits bounded preflight");
        for operation in 0..3 {
            let mut f = dynamic_fixture(Ty::Int(IntTy::I32));
            f.locals[1].ty = array(Ty::Int(IntTy::I32), 64);
            let index = Operand::Copy(place(2));
            let value = match operation {
                0 => Statement::Assign {
                    destination: place(3),
                    value: Rvalue::IndexCopy {
                        array: place(1),
                        index,
                    },
                },
                1 => Statement::IndexInspect {
                    array: place(1),
                    index,
                },
                _ => Statement::IndexAssign {
                    array: place(1),
                    index,
                    value: Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 7,
                    })),
                },
            };
            f.blocks[0].statements.push(located(value));
            let result = analyze_with_limits(
                &f,
                Limits {
                    operations: base_cost + 128,
                    ..Limits::default()
                },
            );
            assert!(!result.complete, "operation={operation}");
            assert_eq!(result.stats.move_paths, 0);
            assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        }
    }

    #[test]
    fn array_paths_are_distinct_and_zero_arrays_have_only_a_root() {
        let mut f = fixture();
        f.locals.extend([
            local(array(array(Ty::Int(IntTy::I32), 2), 2)),
            local(array(Ty::Int(IntTy::I32), 0)),
        ]);
        let paths = Paths::new(&f, 9).unwrap();
        assert_eq!(paths.nodes.len(), 9);
        assert!(Paths::new(&f, 8).is_err());
        assert_ne!(paths.resolve(&indexed(1, 0)), paths.resolve(&indexed(1, 1)));
        let nested = |index| Place {
            local: LocalId(1),
            projection: vec![
                Projection::ConstantIndex(index),
                Projection::ConstantIndex(1),
            ],
        };
        assert_ne!(paths.resolve(&nested(0)), paths.resolve(&nested(1)));
        assert!(paths.nodes[paths.roots[2]].children.is_empty());
    }

    #[test]
    fn repeat_zero_and_one_consume_noncopy_operand_once() {
        let (_module_names, modules) = fixture_modules();
        for len in [0, 1] {
            let element = nominal(&modules, vec![Ty::Int(IntTy::I32)]);
            let ty = array(element.clone(), len);
            let mut f = fixture();
            f.locals.extend([local(element), local(ty.clone())]);
            f.params.push(LocalId(1));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Repeat {
                        ty,
                        value: Operand::Move(place(1)),
                    },
                }),
                located(Statement::Inspect(place(2))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "len={len}: {:?}", result.diagnostics);
            assert_eq!(result.stats.moves, 1);
            assert_eq!(result.stats.operands_read, 3);
            f.blocks[0]
                .statements
                .push(located(Statement::Inspect(place(1))));
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
    }

    #[test]
    fn repeat_copy_operand_is_evaluated_once_and_noncopy_many_is_rejected() {
        let (_module_names, modules) = fixture_modules();
        let mut f = fixture();
        let ty = array(Ty::Int(IntTy::I32), 4);
        f.locals
            .extend([local(Ty::Int(IntTy::I32)), local(ty.clone())]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Repeat {
                    ty,
                    value: Operand::Copy(place(1)),
                },
            }),
            located(Statement::Inspect(place(2))),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.operands_read, 3);
        assert_eq!(result.stats.moves, 0);

        let element = nominal(&modules, vec![Ty::Int(IntTy::I32)]);
        f.locals[1].ty = element.clone();
        f.locals[2].ty = array(element, 2);
        let Statement::Assign { value, .. } = &mut f.blocks[0].statements[3].value else {
            unreachable!()
        };
        *value = Rvalue::Repeat {
            ty: f.locals[2].ty.clone(),
            value: Operand::Move(place(1)),
        };
        let result = analyze(&f);
        assert!(matches!(
            &result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn while_let_moves_require_reinitialization_only_on_actual_backedges() {
        for source in [
            "struct Cell{n:i32} enum E{A(Cell),B} fn f(p:E)->E{let mut state=p;while let E::A(cell)=state{let n=cell.n;state=E::B;continue;}state}",
            "struct Cell{n:i32} enum E{A(Cell),B} fn f(p:E)->E{while let E::A(cell)=p{return E::B;}p}",
            "struct Cell{n:i32} enum E{A(Cell),B} fn f(p:E)->E{while let E::A(..)=p{break;}p}",
            "enum E{A(i32),B} fn f(p:E)->E{while let E::A(n)=p{if n>0{continue;}break;}p}",
            "fn f(p:[i32;3])->[i32;3]{while let [first,..,last]=p{let n=first+last;break;}p}",
            "struct Cell{n:i32} fn f(p:[Cell;3])->[Cell;3]{while let [..]=p{break;}p}",
            "fn f(flag:bool)->i32{let mut state=flag;let mut n=0;'again:while let true=state{state=false;n=n+1;continue 'again;}n}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn while_let_noncopy_payload_cannot_be_reused_on_uninitialized_backedge() {
        for source in [
            "struct Cell{n:i32} enum E{A(Cell),B} fn f(p:E){while let E::A(cell)=p{let n=cell.n;}}",
            "struct Cell{n:i32} enum E{A(Cell),B} fn f(p:E){let mut state=p;while let E::A(cell)=state{if cell.n>0{continue;}state=E::B;}}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn if_let_selected_moves_preserve_failed_edges_omitted_fields_and_divergent_joins() {
        for source in [
            "struct Cell{n:i32} enum E{A(Cell,i32),B} fn f(p:E)->i32{if let E::A(cell,..)=p{return cell.n;}else{match p{E::A(..)=>0,E::B=>1}}}",
            "struct Cell{n:i32} enum E{A(Cell,i32),B} fn f(p:E)->E{if let E::A(..)=p{}p}",
            "struct Cell{n:i32} enum E{A(Cell,i32),B} fn f(p:E)->E{if let E::A(cell,..)=p{return E::B;}p}",
            "struct Cell{n:i32} fn f(p:(Cell,i32))->i32{if let (cell,_)=p{cell.n+p.1}else{0}}",
            "fn f(p:[i32;3])->[i32;3]{let n=if let [first,..,last]=p{first+last}else{0};p}",
            "struct Cell{n:i32} fn f(p:[Cell;3])->[Cell;3]{if let [..]=p{}p}",
            "fn f(n:i32)->i32{let a=if let n=n{n}else{n};a+n}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn if_let_conditional_noncopy_move_prevents_whole_parent_reuse_after_join() {
        for source in [
            "struct Cell{n:i32} enum E{A(Cell,i32),B} fn f(p:E)->E{if let E::A(cell,..)=p{let n=cell.n;}p}",
            "struct Cell{n:i32} fn f(p:(Cell,i32))->(Cell,i32){if let (cell,_)=p{let n=cell.n;}p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn array_patterns_copy_selected_leaves_and_bindingless_noncopy_patterns_do_not_move() {
        for source in [
            "fn f(p:[i32;3])->[i32;3]{let [first,middle,last]=p;let n=first+middle+last;p}",
            "fn f(p:[i32;3])->[i32;3]{let n=match p{[first,..,last]=>first+last};p}",
            "struct Cell{n:i32} fn f(p:[Cell;3])->[Cell;3]{let n=match p{[Cell{n:first},..,Cell{n:last}]=>first+last};p}",
            "struct Cell{n:i32} fn f(p:[Cell;3])->[Cell;3]{let [..]=p;p}",
            "struct Cell{n:i32} fn f(p:[Cell;3])->[Cell;3]{match p{[_,_,_]=>()};p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn forged_noncopy_array_index_move_is_invalid_even_in_disconnected_block() {
        let (_module_names, modules) = fixture_modules();
        let element = nominal(&modules, vec![]);
        let mut f = fixture();
        f.locals
            .extend([local(array(element.clone(), 3)), local(element)]);
        f.params.push(LocalId(1));
        f.blocks.push(BasicBlock {
            statements: vec![located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(indexed(1, 2))),
            })],
            terminator: located(Terminator::Unreachable),
        });
        assert!(
            f.validate()
                .unwrap_err()
                .contains("cannot move non-Copy value out of array index")
        );
        let result = analyze(&f);
        assert!(result.complete);
        assert!(
            matches!(&result.diagnostics[0].kind, ErrorKind::InvalidCfg(message) if message.contains("cannot move non-Copy value out of array index"))
        );
    }

    #[test]
    fn enum_tuple_rest_selected_moves_preserve_omitted_payload_under_refinement() {
        let (_module_names, modules) = fixture_modules();
        let child = nominal(&modules, vec![]);
        let ty = payload_enum(
            &modules,
            vec![vec![child.clone(), child.clone(), child.clone()], vec![]],
        );
        let mut f = payload_fixture(&ty);
        f.locals[2].ty = child.clone();
        f.locals.push(local(child));
        split_payload(&mut f, &ty);
        f.blocks[1].statements.extend([
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 0))),
            }),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 2))),
            }),
            located(Statement::Inspect(payload_field(&ty, 0, 1))),
            located(Statement::Inspect(place(2))),
            located(Statement::Inspect(place(3))),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        for invalid in [payload_field(&ty, 0, 0), payload_field(&ty, 0, 2), place(1)] {
            let mut malformed = f.clone();
            malformed.blocks[1]
                .statements
                .push(located(Statement::Inspect(invalid.clone())));
            let result = analyze(&malformed);
            assert!(result.complete);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].place, Some(invalid));
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
        let mut unrefined = f.clone();
        unrefined.blocks[2]
            .statements
            .push(located(Statement::Inspect(payload_field(&ty, 0, 1))));
        let result = analyze(&unrefined);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ErrorKind::VariantNotDefinitelySelected)
        );
    }

    #[test]
    fn enum_tuple_rest_source_single_variant_bindings_and_rest_only_keep_move_rules() {
        for source in [
            "struct Cell{n:i32} enum E{A(Cell,Cell,Cell)} fn f(p:E)->i32{let E::A(first,..,last)=p;first.n+last.n}",
            "struct Cell{n:i32} enum E{A(Cell,Cell,Cell)} fn f(p:E)->E{let E::A(..)=p;p}",
            "struct Cell{n:i32} enum E{A(Cell,Cell,Cell),B} fn f(p:E)->E{match p{E::A(..)=>(),E::B=>()};p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let result = analyze(&borrowed_source(
            "struct Cell{n:i32} enum E{A(Cell,Cell,Cell)} fn f(p:E)->E{let E::A(first,..,last)=p;let n=first.n+last.n;p}",
        ));
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
    }

    #[test]
    fn positional_rest_preserves_middle_and_rejects_selected_or_parent_reuse() {
        for source in [
            "struct Cell{n:i32} struct Trio(Cell,Cell,Cell); fn f(p:Trio)->Cell{let n=match p{Trio(first,..,last)=>first.n+last.n};p.1}",
            "struct Cell{n:i32} struct Trio(Cell,Cell,Cell); fn f(p:Trio)->Cell{let Trio(first,..,last)=p;let n=first.n+last.n;p.1}",
            "struct Cell{n:i32} fn f(p:(Cell,Cell,Cell))->Cell{let n=match p{(first,..,last)=>first.n+last.n};p.1}",
            "struct Cell{n:i32} struct Trio(i32,Cell,i32); fn f(p:Trio)->Trio{let n=match p{Trio(first,..,last)=>first+last};p}",
            "struct Cell{n:i32} struct Trio(Cell,Cell,Cell); fn f(p:Trio)->Trio{match p{Trio(..)=>()};p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        for source in [
            "struct Cell{n:i32} struct Trio(Cell,Cell,Cell); fn f(p:Trio)->Trio{match p{Trio(first,..,last)=>first.n+last.n};p}",
            "struct Cell{n:i32} struct Trio(Cell,Cell,Cell); fn f(p:Trio)->Cell{match p{Trio(first,..,last)=>first.n+last.n};p.2}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn record_rest_match_omitted_noncopy_fields_remain_owned() {
        for source in [
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Cell{let n=match p{Record{selected,..}=>selected.n};p.omitted}",
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Record{match p{Record{..}=>()};p}",
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Record{match p{Record{selected:_,..}=>()};p}",
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Record{let n=match p{Record{selected:Cell{n},..}=>n};p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        for source in [
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Record{match p{Record{selected,..}=>selected.n};p}",
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Cell{match p{Record{selected,..}=>selected.n};p.selected}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn aggregate_match_sources_preserve_ignored_fields_and_reject_parent_reuse_after_move() {
        for source in [
            "struct Cell{i:i32} struct Pair(Cell,i32); fn f(p:Pair)->i32{let n=match p{Pair(cell,_)=>cell.i};n+p.1}",
            "struct Cell{i:i32} struct Record{cell:Cell,n:i32} fn f(p:Record)->i32{let n=match p{Record{cell,n:_}=>cell.i};n+p.n}",
            "struct Cell{i:i32} fn f(p:(Cell,i32))->i32{let n=match p{(cell,_)=>cell.i};n+p.1}",
            "struct Pair(i32,i32); fn f(p:Pair)->Pair{match p{Pair(_,_)=>()};p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let source = "struct Cell{i:i32} struct Pair(Cell,i32); fn f(p:Pair)->Pair{match p{Pair(cell,_)=>cell.i};p}";
        let result = analyze(&borrowed_source(source));
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
    }

    #[test]
    fn tuple_struct_carrier_partial_move_preserves_only_unmoved_sibling() {
        let (_module_names, modules) = fixture_modules();
        let child = nominal(&modules, vec![]);
        let ty = nominal(&modules, vec![child.clone(), Ty::Int(IntTy::I32)]);
        let mut f = fixture();
        f.locals.extend([local(ty), local(child)]);
        f.params.push(LocalId(1));
        let selected = Place {
            local: LocalId(1),
            projection: vec![Projection::Field(0)],
        };
        let sibling = Place {
            local: LocalId(1),
            projection: vec![Projection::Field(1)],
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(selected.clone())),
            }),
            located(Statement::Inspect(sibling)),
            located(Statement::Inspect(place(2))),
        ]);
        assert!(analyze(&f).passed());
        for invalid in [selected, place(1)] {
            let mut invalid_read = f.clone();
            invalid_read.blocks[0]
                .statements
                .push(located(Statement::Inspect(invalid.clone())));
            let result = analyze(&invalid_read);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].place, Some(invalid));
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
    }

    #[test]
    fn zero_field_nominal_has_one_owned_path_and_whole_move_clears_it() {
        let (_module_names, modules) = fixture_modules();
        let ty = nominal(&modules, vec![]);
        assert!(!ty.is_copy());
        let mut f = fixture();
        f.locals.extend([local(ty.clone()), local(ty)]);
        f.params.push(LocalId(1));
        let paths = Paths::new(&f, 3).unwrap();
        assert_eq!(paths.nodes.len(), 3);
        assert!(Paths::new(&f, 2).is_err());
        let root = paths.roots[1];
        assert!(paths.nodes[root].children.is_empty());
        assert_eq!(paths.nodes[root].end, root + 1);
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Inspect(place(1))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
            located(Statement::Inspect(place(2))),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(1))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(1)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn zero_field_aggregate_initializes_real_nominal_storage() {
        let (_module_names, modules) = fixture_modules();
        let ty = nominal(&modules, vec![]);
        let mut f = fixture();
        f.locals.push(local(ty.clone()));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Aggregate { ty, fields: vec![] },
            }),
            located(Statement::Inspect(place(1))),
        ]);
        assert!(analyze(&f).passed());
    }

    #[test]
    fn zero_length_noncopy_array_whole_move_clears_root_marker() {
        let (_module_names, modules) = fixture_modules();
        let ty = array(nominal(&modules, vec![Ty::Int(IntTy::I32)]), 0);
        assert!(!ty.is_copy());
        let mut f = fixture();
        f.locals.extend([local(ty.clone()), local(ty)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Inspect(place(1))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
            located(Statement::Inspect(place(2))),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(1)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn indexed_copy_field_reads_preserve_noncopy_array_for_whole_move() {
        let (_module_names, modules) = fixture_modules();
        let element = nominal(
            &modules,
            vec![
                nominal(&modules, vec![Ty::Int(IntTy::I32)]),
                Ty::Int(IntTy::I32),
            ],
        );
        let ty = array(element, 2);
        let mut f = fixture();
        f.locals
            .extend([local(ty.clone()), local(Ty::Int(IntTy::I32)), local(ty)]);
        f.params.push(LocalId(1));
        let mut scalar = indexed(1, 0);
        scalar.projection.push(Projection::Field(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Inspect(place(1))),
            read_payload(scalar.clone()),
            read_payload(scalar),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
    }

    #[test]
    fn indexed_noncopy_moves_are_rejected_even_through_projected_subfields() {
        let (_module_names, modules) = fixture_modules();
        let inner = nominal(&modules, vec![Ty::Int(IntTy::I32)]);
        let element = nominal(&modules, vec![inner.clone(), Ty::Int(IntTy::I32)]);
        for nested in [false, true] {
            let mut f = fixture();
            f.locals.extend([
                local(array(element.clone(), 2)),
                local(if nested {
                    inner.clone()
                } else {
                    element.clone()
                }),
            ]);
            f.params.push(LocalId(1));
            let mut source = indexed(1, 0);
            if nested {
                source.projection.push(Projection::Field(0));
            }
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Use(Operand::Move(source)),
                }),
            ]);
            let result = analyze(&f);
            assert!(!result.passed());
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
        }
    }

    #[test]
    fn indexed_private_copy_move_remains_a_consuming_transfer() {
        let mut f = fixture();
        f.locals.extend([
            local(array(Ty::Int(IntTy::I32), 2)),
            local(Ty::Int(IntTy::I32)),
        ]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(indexed(1, 0))),
            }),
            located(Statement::Inspect(indexed(1, 1))),
            located(Statement::Inspect(indexed(1, 0))),
        ]);
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.stats.moves, 1);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
        assert_eq!(result.diagnostics[0].place, Some(indexed(1, 0)));
    }

    #[test]
    fn array_base_inspection_rejects_uninitialized_or_moved_wildcard_bases() {
        let (_module_names, modules) = fixture_modules();
        let ty = array(nominal(&modules, vec![Ty::Int(IntTy::I32)]), 2);
        for moved in [false, true] {
            let mut f = fixture();
            f.locals.extend([local(ty.clone()), local(ty.clone())]);
            if moved {
                f.params.push(LocalId(1));
                f.blocks[0].statements.extend([
                    located(Statement::StorageLive(LocalId(2))),
                    located(Statement::Inspect(place(1))),
                    located(Statement::Assign {
                        destination: place(2),
                        value: Rvalue::Use(Operand::Move(place(1))),
                    }),
                ]);
            } else {
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0]
                .statements
                .push(located(Statement::Inspect(place(1))));
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
    }

    #[test]
    fn projected_array_inspection_does_not_read_unrelated_aggregate_siblings() {
        let mut f = fixture();
        f.locals.extend([
            local(Ty::Tuple(
                vec![array(Ty::Int(IntTy::I32), 2), Ty::Int(IntTy::I32)].into(),
            )),
            local(Ty::Int(IntTy::I32)),
        ]);
        f.params.push(LocalId(1));
        let field = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(field(1))),
            }),
            located(Statement::Inspect(field(0))),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(1))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn indexed_assignment_requires_mutable_initialized_ancestors() {
        for (mutable, initialized, expected) in [
            (true, true, None),
            (false, true, Some(ErrorKind::ImmutableReassignment)),
            (
                true,
                false,
                Some(ErrorKind::WriteIntoUninitializedAggregate),
            ),
        ] {
            let mut f = fixture();
            f.locals.push(local(array(Ty::Int(IntTy::I32), 2)));
            f.locals[1].mutable = mutable;
            if initialized {
                f.params.push(LocalId(1));
            } else {
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: indexed(1, 0),
                value: Rvalue::Use(if initialized {
                    Operand::Copy(indexed(1, 1))
                } else {
                    Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 4,
                    }))
                }),
            }));
            let result = analyze(&f);
            if let Some(expected) = expected {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(result.diagnostics[0].kind, expected);
            } else {
                assert!(result.passed(), "{:?}", result.diagnostics);
            }
        }
    }

    #[test]
    fn array_element_variant_proofs_are_independent_and_inspection_preserves_them() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = fixture();
        f.locals.extend([
            local(array(Ty::Enum(ty.clone()), 2)),
            local(Ty::Int(IntTy::I32)),
        ]);
        f.params.push(LocalId(1));
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(2))));
        split_payload(&mut f, &ty);
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: indexed(1, 0),
            variant: variant(&ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        let field = |index| {
            let mut place = indexed(1, index);
            place
                .projection
                .extend([Projection::Downcast(variant(&ty, 0)), Projection::Field(0)]);
            place
        };
        f.blocks[1].statements.extend([
            located(Statement::Inspect(place(1))),
            read_payload(field(0)),
            located(Statement::Assign {
                destination: indexed(1, 1),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 9,
                    }))],
                },
            }),
            read_payload(field(0)),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.variant_proofs_killed, 0);
        f.blocks[1].statements.push(read_payload(field(1)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        f.blocks[1].statements.pop();
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: indexed(1, 0),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 8,
                    }))],
                },
            }),
            read_payload(field(0)),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        assert_eq!(result.stats.variant_proofs_killed, 1);
    }

    #[test]
    fn array_preflight_bounds_expansion_and_checks_zero_element_types() {
        let ty = array(Ty::Int(IntTy::I32), super::super::ir::MAX_ARRAY_LEN);
        let exact = 2 * super::super::ir::MAX_ARRAY_LEN + 1;
        assert!(preflight_ty(&ty, 0, &mut Budget::new(exact - 1)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget::new(exact)).is_ok());
        assert!(
            preflight_ty(
                &array(Ty::Int(IntTy::I32), super::super::ir::MAX_ARRAY_LEN + 1),
                0,
                &mut Budget::new(1_000_000)
            )
            .is_err()
        );
        let mut huge = Ty::Int(IntTy::I32);
        for _ in 0..8 {
            huge = array(huge, 16);
        }
        let mut f = fixture();
        f.locals.push(local(huge));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        let mut deep = Ty::Int(IntTy::I32);
        for _ in 0..130 {
            deep = array(deep, 0);
        }
        assert!(preflight_ty(&deep, 0, &mut Budget::new(1_000_000)).is_err());
    }

    #[test]
    fn payload_downcast_requires_successful_edge_proof() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        f.blocks[0]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );

        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        assert!(analyze(&f).passed());
        f.blocks[2]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].block, BlockId(2));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
    }

    #[test]
    fn payload_proof_intersects_for_shared_targets_and_joins() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        for shared_target in [false, true] {
            let mut f = payload_fixture(&ty);
            split_payload(&mut f, &ty);
            f.blocks.push(BasicBlock {
                statements: vec![read_payload(payload_field(&ty, 0, 0))],
                terminator: located(Terminator::Return),
            });
            if shared_target {
                f.blocks[0].terminator = located(Terminator::SwitchVariant {
                    place: place(1),
                    variant: variant(&ty, 0),
                    then_block: BlockId(3),
                    else_block: BlockId(3),
                });
            } else {
                f.blocks[1].terminator = located(Terminator::Goto(BlockId(3)));
                f.blocks[2].terminator = located(Terminator::Goto(BlockId(3)));
            }
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::VariantNotDefinitelySelected
            );
        }
    }

    #[test]
    fn whole_enum_assignment_kills_stale_variant_proof() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::I32,
                        bits: 9,
                    }))],
                },
            }),
            read_payload(payload_field(&ty, 0, 0)),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        assert_eq!(result.stats.variant_proofs_killed, 1);
        assert_eq!(result.stats.variant_proof_checks, 1);
        assert_eq!(result.stats.variant_refinements, 1);
    }

    #[test]
    fn payload_destination_requires_proof_even_when_inactive_bits_are_true() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: payload_field(&ty, 0, 0),
            value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                ty: IntTy::I32,
                bits: 3,
            }))),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
    }

    #[test]
    fn singleton_switch_does_not_read_partially_moved_or_uninitialized_root() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 0))),
        }));
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 1)));
        assert!(analyze(&f).passed(), "{:?}", analyze(&f).diagnostics);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );

        let mut f = payload_fixture(&ty);
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        split_payload(&mut f, &ty);
        assert!(analyze(&f).passed());
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn selected_variant_refinement_never_restores_initialization() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let mut f = payload_fixture(&ty);
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
    }

    #[test]
    fn payload_move_retains_outer_proof_but_full_enum_read_fails() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![
                vec![Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)],
                vec![Ty::Int(IntTy::I32)],
            ],
        );
        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 0))),
            }),
            read_payload(payload_field(&ty, 0, 1)),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[1].statements.push(discriminant());
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn nested_proof_invalidation_is_subtree_scoped() {
        let (_module_names, modules) = fixture_modules();
        let inner = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32)], vec![Ty::Int(IntTy::I32)]],
        );
        let outer = payload_enum(
            &modules,
            vec![
                vec![Ty::Enum(inner), Ty::Int(IntTy::I32)],
                vec![Ty::Int(IntTy::I32)],
            ],
        );
        let f = payload_fixture(&outer);
        let paths = Paths::new(&f, 100).unwrap();
        let outer_node = paths.resolve(&place(1));
        let inner_node = paths.resolve(&payload_field(&outer, 0, 0));
        let sibling = paths.resolve(&payload_field(&outer, 0, 1));
        let mut state = State::new(f.locals.len(), paths.nodes.len());
        state.selected[outer_node] = 0;
        state.selected[inner_node] = 0;
        let mut budget = Budget::new(1000);
        let mut transfer = Transfer {
            facts: FactsCtx::Concrete,
            f: &f,
            paths: &paths,
            budget: &mut budget,
            report: None,
            block: BlockId(0),
            statement: None,
            origin: None,
            diagnostic_limit: 10,
            loans: None,
            phase: Phase::Read,
        };
        transfer.initialize(&mut state, sibling, true).unwrap();
        assert_eq!(state.selected[outer_node], 0);
        assert_eq!(state.selected[inner_node], 0);
        transfer.initialize(&mut state, inner_node, false).unwrap();
        assert_eq!(state.selected[outer_node], 0);
        assert_eq!(state.selected[inner_node], usize::MAX);
        state.selected[inner_node] = 0;
        transfer.initialize(&mut state, outer_node, true).unwrap();
        assert_eq!(state.selected[outer_node], usize::MAX);
        assert_eq!(state.selected[inner_node], usize::MAX);
    }

    #[test]
    fn payload_metadata_and_proof_storage_are_resource_bounded() {
        let (_module_names, modules) = fixture_modules();
        let ty = payload_enum(
            &modules,
            vec![vec![Ty::Int(IntTy::I32); 10], vec![Ty::Int(IntTy::I32); 10]],
        );
        assert!(preflight_ty(&Ty::Enum(ty.clone()), 0, &mut Budget::new(40)).is_err());
        let f = payload_fixture(&ty);
        let paths = Paths::new(&f, 23).unwrap();
        assert_eq!(paths.nodes.len(), 23);
        assert!(Paths::new(&f, 22).is_err());
        let old_words =
            Bits::new(f.locals.len()).0.len() * 2 + Bits::new(paths.nodes.len()).0.len() * 3;
        let old_state_bytes = old_words * 8 + std::mem::size_of::<State>();
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: old_state_bytes * (f.blocks.len() + 3),
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert_eq!(result.stats.worklist_pops, 0);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
    }

    #[test]
    fn enum_discriminant_reads_do_not_consume_noncopy_root() {
        let (_module_names, modules) = fixture_modules();
        let mut f = enum_fixture(&modules);
        f.blocks[0].statements.extend([
            discriminant(),
            discriminant(),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
        assert_eq!(result.stats.move_paths, 4);
    }

    #[test]
    fn enum_discriminant_requires_live_initialized_root() {
        let (_module_names, modules) = fixture_modules();
        for (setup, expected) in [
            (
                Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Use(Operand::Move(place(1))),
                },
                ErrorKind::ReadNotDefinitelyInitialized,
            ),
            (
                Statement::StorageDead(LocalId(1)),
                ErrorKind::StorageNotDefinitelyLive,
            ),
        ] {
            let mut f = enum_fixture(&modules);
            f.blocks[0].statements.push(located(setup));
            f.blocks[0].statements.push(discriminant());
            let result = analyze(&f);
            assert!(result.complete);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].kind, expected);
            assert_eq!(result.diagnostics[0].place, Some(place(1)));
        }

        let mut f = enum_fixture(&modules);
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        f.blocks[0].statements.push(discriminant());
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn unreachable_discriminant_does_not_read_moved_enum() {
        let (_module_names, modules) = fixture_modules();
        let mut f = enum_fixture(&modules);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(place(1))),
        }));
        f.blocks.push(BasicBlock {
            statements: vec![discriminant()],
            terminator: located(Terminator::Unreachable),
        });
        assert!(analyze(&f).passed());
    }

    #[test]
    fn enum_metadata_is_budgeted_without_adding_child_paths() {
        let (_module_names, modules) = fixture_modules();
        let ty = Ty::Enum(unit_enum(&modules));
        assert!(preflight_ty(&ty, 0, &mut Budget::new(2)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget::new(3)).is_ok());
        let f = enum_fixture(&modules);
        let paths = Paths::new(&f, 4).unwrap();
        assert!(paths.nodes[paths.roots[1]].children.is_empty());
    }

    #[test]
    fn constant_switch_false_edge_still_checks_moved_enum_reads() {
        let (_module_names, modules) = fixture_modules();
        let mut f = enum_fixture(&modules);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(place(1))),
        }));
        f.blocks[0].terminator = located(Terminator::Switch {
            condition: Operand::Constant(Constant::Bool(true)),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![discriminant()],
                terminator: located(Terminator::Return),
            },
        ]);
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.stats.blocks_reachable, 3);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].block, BlockId(2));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn enum_constant_initializes_root_for_discriminant() {
        let (_module_names, modules) = fixture_modules();
        let mut f = enum_fixture(&modules);
        f.params.clear();
        let Ty::Enum(ty) = &f.locals[1].ty else {
            unreachable!()
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Use(Operand::Constant(Constant::Enum {
                    ty: ty.clone(),
                    variant: 0,
                })),
            }),
            discriminant(),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn projected_enum_discriminant_checks_its_own_initialization() {
        let (_module_names, modules) = fixture_modules();
        let mut f = fixture();
        let ty = Ty::Enum(unit_enum(&modules));
        f.locals.extend([
            local(Ty::Tuple(vec![ty.clone(), ty.clone()].into())),
            local(Ty::Int(IntTy::I32)),
            local(ty),
        ]);
        f.params.push(LocalId(1));
        let field = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(field(0))),
            }),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Discriminant(field(1)),
            }),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Discriminant(field(0)),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
        assert_eq!(result.diagnostics[0].place, Some(field(0)));
    }

    #[test]
    fn bit_ranges_cover_empty_intervals_and_word_boundaries() {
        for start in 0..=131 {
            for end in start..=131 {
                let mut bits = Bits::new(131);
                let mut budget = Budget::new(1000);
                bits.range(start, end, true, &mut budget).unwrap();
                for index in 0..131 {
                    assert_eq!(bits.get(index), (start..end).contains(&index));
                }
                assert!(bits.all(start, end, &mut budget).unwrap());
                if start < end {
                    bits.set(end - 1, false);
                    assert!(!bits.all(start, end, &mut budget).unwrap());
                }
                bits.range(start, end, false, &mut budget).unwrap();
                assert!((0..131).all(|index| !bits.get(index)));
            }
        }
    }

    #[test]
    fn every_private_resource_cap_fails_closed() {
        let f = fixture();
        assert!(analyze(&f).passed());
        for limits in [
            Limits {
                blocks: 0,
                ..Limits::default()
            },
            Limits {
                locals: 0,
                ..Limits::default()
            },
            Limits {
                paths: 0,
                ..Limits::default()
            },
            Limits {
                operations: 0,
                ..Limits::default()
            },
            Limits {
                state_bytes: 0,
                ..Limits::default()
            },
            Limits {
                work: 0,
                ..Limits::default()
            },
        ] {
            let result = analyze_with_limits(&f, limits);
            assert!(!result.passed());
            assert!(!result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ResourceLimit)
            );
        }
        assert!(
            analyze_with_limits(
                &f,
                Limits {
                    diagnostics: 0,
                    ..Limits::default()
                }
            )
            .passed()
        );
    }

    #[test]
    fn diagnostic_truncation_is_explicit_and_never_success() {
        let mut f = fixture();
        f.locals.push(local(Ty::Unit));
        f.blocks[0].statements = vec![
            located(Statement::StorageLive(LocalId(0))),
            located(Statement::StorageLive(LocalId(1))),
        ];
        for _ in 0..5 {
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: place(0),
                value: Rvalue::Use(Operand::Copy(place(1))),
            }));
        }
        let result = analyze_with_limits(
            &f,
            Limits {
                diagnostics: 2,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        assert_eq!(result.diagnostics.len(), 2);
        assert_eq!(result.diagnostics[0].statement, Some(2));
        assert_eq!(result.diagnostics[1].kind, ErrorKind::ResourceLimit);
        assert!(result.stats.diagnostics_suppressed >= 3);
        let zero = analyze_with_limits(
            &f,
            Limits {
                diagnostics: 0,
                ..Limits::default()
            },
        );
        assert!(!zero.complete);
        assert_eq!(zero.diagnostics.len(), 1);
        assert_eq!(zero.diagnostics[0].kind, ErrorKind::ResourceLimit);
    }

    #[test]
    fn diagnostic_replay_exhaustion_cannot_report_success() {
        // Search a small fixed bound rather than pinning an internal operation
        // count. Extra proof accounting must not turn this into a solver-only
        // exhaustion test or let replay exhaustion masquerade as success.
        let result = (0..=256)
            .map(|work| {
                analyze_with_limits(
                    &fixture(),
                    Limits {
                        work,
                        ..Limits::default()
                    },
                )
            })
            .find(|result| {
                !result.complete && result.stats.worklist_pops == 1 && result.stats.writes == 1
            })
            .expect("bounded work budget must exercise mid-replay exhaustion");
        assert!(!result.complete);
        assert!(!result.passed());
        assert_eq!(result.stats.worklist_pops, 1);
        assert_eq!(
            result.stats.writes, 1,
            "final diagnostic replay must have begun"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.kind == ErrorKind::ResourceLimit)
        );
    }

    #[test]
    fn repeated_wide_type_comparisons_are_bounded_before_validation() {
        let mut f = fixture();
        // Distinct allocations defeat pointer-equality shortcuts in Eq.
        let first = Ty::Tuple(vec![Ty::Int(IntTy::I32); 20_000].into());
        let second = Ty::Tuple(vec![Ty::Int(IntTy::I32); 20_000].into());
        f.locals.extend([local(first), local(second)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements = vec![located(Statement::StorageLive(LocalId(2)))];
        for _ in 0..64 {
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Copy(place(1))),
            }));
        }
        f.blocks[0].terminator = located(Terminator::Unreachable);
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.stats.worklist_pops, 0);
    }

    #[test]
    fn nested_coercion_preflight_charges_revisited_subtrees() {
        let mut ty = Ty::Int(IntTy::I32);
        for _ in 0..64 {
            ty = Ty::Tuple(vec![ty].into());
        }
        assert!(preflight_ty(&ty, 0, &mut Budget::new(1000)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget::new(3000)).is_ok());
    }

    #[test]
    fn nominal_fields_get_distinct_nested_move_paths() {
        let (_module_names, modules) = fixture_modules();
        let inner = nominal(&modules, vec![Ty::Unit, Ty::Bool]);
        let outer = nominal(
            &modules,
            vec![
                Ty::Int(IntTy::I32),
                Ty::Tuple(vec![inner.clone(), inner].into()),
            ],
        );
        let mut f = fixture();
        f.locals.push(local(outer.clone()));
        let paths = Paths::new(&f, 10).unwrap();
        assert_eq!(paths.nodes.len(), 10);
        assert!(Paths::new(&f, 9).is_err());
        let projected = |fields: Vec<usize>| Place {
            local: LocalId(1),
            projection: fields.into_iter().map(Projection::Field).collect(),
        };
        assert_eq!(paths.resolve(&projected(vec![1, 0, 0])), 5);
        assert_eq!(paths.resolve(&projected(vec![1, 1, 0])), 8);
        assert_eq!(paths.nodes[8].parent, Some(7));
        assert_eq!(paths.nodes[7].end, 10);
        assert!(preflight_ty(&outer, 0, &mut Budget::new(15)).is_err());
        assert!(preflight_ty(&outer, 0, &mut Budget::new(30)).is_ok());
    }

    #[test]
    fn shared_nominal_dag_expansion_is_bounded_before_paths() {
        let (_module_names, modules) = fixture_modules();
        let mut ty = Ty::Int(IntTy::I32);
        for _ in 0..20 {
            ty = nominal(&modules, vec![ty.clone(), ty]);
        }
        let mut f = fixture();
        f.locals.push(local(ty));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.stats.worklist_pops, 0);
    }
}
