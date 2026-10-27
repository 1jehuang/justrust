//! Source-to-native compiler built on the fact graph. See COMPILER.md.
mod associated_constant_graph;
pub mod backend;
mod borrow;
pub mod c_abi;
#[cfg(test)]
mod canonical_instance_tests;
pub mod cfg;
mod const_check;
mod const_diagnostics;
pub mod const_eval;
mod const_graph;
mod const_source;
mod expansion_budget;
mod expansion_span;
pub mod frontend;
mod hygiene;
mod instances;
pub mod ir;
pub(crate) mod keys;
mod lexer;
mod literal_scan;
mod literal_values;
mod macro_cursor;
mod macro_fragments;
mod macro_matcher;
pub mod macro_rules;
mod macro_transcribe;
mod memory_layout;
pub mod module_identity;
mod nominal_realization;
#[cfg(test)]
mod opaque_authority_tests;
mod opaque_dispatch;
#[cfg(test)]
mod opaque_dispatch_tests;
mod opaque_instances;
mod opaque_object;
mod opaque_physical;
#[cfg(test)]
mod opaque_source_tests;
mod opaque_specialization;
pub mod owner_path;
pub mod ownership;
pub mod procfs;
mod resource_accounting;
mod root_cleanup;
pub mod sema;
pub mod semantic_name;
pub mod semantic_path;
pub mod semantic_scope;
pub mod source;
mod source_item_macro;
mod source_macro;
mod source_macro_binding;
mod source_replay;
pub mod ssa;
mod token;
pub mod token_primitives;
mod token_session;
mod token_tree;
pub mod traits;

#[cfg(test)]
#[path = "../../tests/support/stock_rustc.rs"]
mod stock_rustc;

use crate::{Context, Engine, Error, Key, Provider, Report, TaskFuture};
use std::path::PathBuf;
use std::sync::Arc;

/// Stop formatting immediately when the shared diagnostic allowance is spent.
/// The complete structured analysis remains published in its graph slot.
pub(super) fn analysis_diagnostic(
    budget: &ir::PlanBudget,
    subject: &str,
    index: impl std::fmt::Display,
    analysis: &ownership::Analysis,
) -> String {
    struct Writer<'a> {
        budget: &'a ir::PlanBudget,
        text: String,
    }
    impl std::fmt::Write for Writer<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let retained = text.len().checked_mul(2).ok_or(std::fmt::Error)?;
            self.budget
                .charge_work(text.len())
                .and_then(|()| self.budget.charge_diagnostic_bytes(text.len()))
                .and_then(|()| self.budget.retain_bytes(retained))
                .map_err(|_| std::fmt::Error)?;
            if self.text.capacity() - self.text.len() < text.len() {
                self.budget
                    .charge_work(self.text.len())
                    .map_err(|_| std::fmt::Error)?;
                self.text
                    .try_reserve_exact(text.len())
                    .map_err(|_| std::fmt::Error)?;
            }
            self.text.push_str(text);
            Ok(())
        }
    }
    let mut writer = Writer {
        budget,
        text: String::new(),
    };
    if std::fmt::write(
        &mut writer,
        format_args!(
            "{subject} {index}: initialization/move analysis (complete={}): {:?}",
            analysis.complete, analysis.diagnostics
        ),
    )
    .is_err()
    {
        return traits::FactsError::resource(budget, "analysis diagnostic resource limit exceeded")
            .into_legacy_diagnostic(budget);
    }
    writer.text
}

/// Present typed source errors only at the existing bounded graph diagnostic boundary.
/// Work units granted per program item to a task whose input is the whole
/// item list (remaining whole-program debt nodes only).
const WORK_PER_PROGRAM_ITEM: u64 = 64 * 1024;
/// Work units granted per unit of one callable's own source input.
const WORK_PER_CALLABLE_UNIT: u64 = 64 * 1024;
fn scale_task_work_by_callable(types: &frontend::PlanCtx, declaration: ir::DeclId) {
    crate::engine::scale_task_work(
        u64::try_from(types.source_callable_input_size(declaration))
            .unwrap_or(u64::MAX)
            .saturating_mul(WORK_PER_CALLABLE_UNIT),
    );
}
/// Work units granted per lowered CFG element of one instance.
const WORK_PER_CFG_UNIT: u64 = 16 * 1024;
fn scale_task_work_by_cfg(function: &cfg::Function) {
    let size = function
        .blocks
        .iter()
        .map(|block| block.statements.len() + 1)
        .sum::<usize>()
        + function.locals.len();
    crate::engine::scale_task_work(
        u64::try_from(size)
            .unwrap_or(u64::MAX)
            .saturating_mul(WORK_PER_CFG_UNIT),
    );
}
fn scale_task_work_by_items(program: &frontend::Program) {
    let items = program.functions.len()
        + program.constants.len()
        + program.structs.len()
        + program.enums.len()
        + program.impls.len()
        + program.traits.len();
    crate::engine::scale_task_work(
        u64::try_from(items)
            .unwrap_or(u64::MAX)
            .saturating_mul(WORK_PER_PROGRAM_ITEM),
    );
}
fn source_fact_error(error: traits::FactsError, budget: &ir::PlanBudget) -> Error {
    const EXHAUSTED: &str = "source planning ResourceLimit (diagnostic transport exhausted)";
    let category = match error.kind() {
        traits::FactsErrorKind::InvalidMetadata => "InvalidMetadata",
        traits::FactsErrorKind::Unsupported => "Unsupported",
        traits::FactsErrorKind::Unproved => "Unproved",
        traits::FactsErrorKind::Ambiguous => "Ambiguous",
        traits::FactsErrorKind::Cycle => "Cycle",
        traits::FactsErrorKind::Resource => "ResourceLimit",
    };
    let Ok(message) = error.into_diagnostic(budget) else {
        return Error::new(EXHAUSTED);
    };
    let Some(bytes) = category
        .len()
        .checked_add(2)
        .and_then(|n| n.checked_add(message.len()))
    else {
        return Error::new(EXHAUSTED);
    };
    if budget
        .charge_work(bytes)
        .and_then(|()| budget.charge_diagnostic_bytes(bytes))
        .and_then(|()| budget.retain_bytes(bytes))
        .is_err()
    {
        return Error::new(EXHAUSTED);
    }
    let mut rendered = String::new();
    if rendered.try_reserve_exact(bytes).is_err() {
        return Error::new(EXHAUSTED);
    }
    rendered.push_str(category);
    rendered.push_str(": ");
    rendered.push_str(&message);
    Error::new(rendered)
}

const ADMIT: u32 = 150;
const PARSE_FILE: u32 = 121;
const SIGNATURES: u32 = 101;
const BODY: u32 = 102;
const OBJECT: u32 = 103;
/// BUILD discovery rounds whose OBJECTs are requested while discovery
/// continues. Real programs close their instance frontier well within this;
/// deeper rounds defer codegen until the frontier is proven closed.
const OVERLAP_ROUNDS: usize = 16;
const SOURCE_ADMISSION: u32 = 128;
const CHECK: u32 = 104;
const BUILD: u32 = 105;
const RESOLVE: u32 = 106;
/// Ordinary free-function resolution, keyed by namespace module ordinal.
/// Namespace/header preparation and final assembly remain explicit RESOLVE debt.
const RESOLVE_MODULE: u32 = 148;
/// Source callable schema computation, keyed by sealed signature module ordinal.
const SIGNATURE_MODULE: u32 = 149;
const SOURCE: u32 = 107;
const LOWER: u32 = 108;
const ANALYZE: u32 = 109;
const TEMPLATE_BODY: u32 = 110;
const UNIVERSAL_LOWER: u32 = 111;
const UNIVERSAL_ANALYZE: u32 = 112;
/// Per-instance direct demand frontier (`instances::DirectDemands`), keyed
/// like every executable instance node by `instance_params`.
const INSTANCE: u32 = 142;
const UNIVERSAL_EFFECTS_READY: u32 = 115;
const BODY_LEXICAL: u32 = 116;
const CAPTURE_ENV: u32 = 117;
const ASSOCIATED_CONST_HEADER: u32 = 134;
const ASSOCIATED_CONST_SELECT: u32 = 135;
const ASSOCIATED_CONST_VALUE: u32 = 136;
const CONST_DECL: u32 = 118;
const CONST_BODY: u32 = 119;
const CONST_VALUE: u32 = 120;
const ROOT_CLEANUP_DEMAND: u32 = 125;
/// Canonical target-keyed body. BODY(index) is only a legacy slot alias.
const CANONICAL_BODY: u32 = 126;
const OPAQUE_REPRESENTATION: u32 = 127;
const OPAQUE_CALL_DISPATCH: u32 = 140;
/// Optimizer nodes (OPTIMIZATION.md section 8). Per instance index; only BUILD
/// (through OBJECT) demands them, and never when the mode is `Off`.
const SSA: u32 = 130;
const LOCAL_OPT: u32 = 131;
const SUMMARY: u32 = 132;
const IPO_OPT: u32 = 133;
const NOMINAL_DECL_WELL_FORMED: u32 = 138;
/// Per-callable immutable generic signature (keyed by callable index).
const SIGNATURE: u32 = 143;
/// Per-trait implementation completeness (keyed by trait declaration).
const TRAIT_IMPLS: u32 = 144;
/// Per-impl coherence against its trait's impls (keyed by impl declaration).
const IMPL_COHERENCE: u32 = 145;
/// Per-callable inherent overlap and C ABI contract (keyed by callable index).
const ITEM_COHERENCE: u32 = 146;
/// Per-module item inventory (keyed by inventory ordinal): the callables,
/// constants, nominals and associated constants that module owns.
const ITEM_INVENTORY: u32 = 141;
/// Per-module canonical source declarations (keyed by module-path key bits).
const MODULE_DECLS: u32 = 147;

enum Input {
    Text(String),
    File(PathBuf),
    ProbeFile(PathBuf, source::SourceProbeRequest),
    Program(Arc<frontend::Program>),
}
/// Typed graph facts. Each is the immutable output of exactly one key and is
/// read by consumers through `Context::read_fact`, so every dependency is a
/// recorded graph edge. Nothing here is a shared side slot.
///
/// `TEMPLATE_BODY` output: the original checked body and, for Source bodies,
/// its canonical publication.
struct TemplateBodyFact {
    checked: Arc<ir::CheckedFunction>,
    closure: Option<frontend::closures::check::CheckedClosure>,
    closure_mode: Option<ir::ClosureKind>,
    publication: Option<Arc<source_replay::CanonicalSourcePublication>>,
}
/// `ITEM_INVENTORY` output: items owned by one module.
struct ItemInventoryFact {
    callables: Vec<usize>,
    universal: Vec<usize>,
    constants: Vec<ir::DeclId>,
    nominals: Vec<ir::DeclId>,
    associated_constants: Vec<ir::DeclId>,
    impls: Vec<ir::DeclId>,
}
impl crate::engine::Fact for ItemInventoryFact {}
struct ClosureBodiesFact {
    bodies: Arc<[frontend::closures::publication::PublishedClosure]>,
    publication: Option<frontend::closures::publication::ClosurePublicationProof>,
}
/// Final immutable hidden representations owned by this declaration's body.
struct OpaqueHiddenTypesFact {
    representations: Arc<[ir::OpaqueHiddenType]>,
}
/// Exact closed hidden record. Reading this fact grants no source-level trait
/// information and does not itself authorize carrier-erasing physical lowering.
struct OpaqueRepresentationFact {
    record: ir::OpaqueHiddenType,
    definition: ir::OpaqueDefinitionProof,
}
struct BodyLexicalFact {
    lexical: frontend::closures::graph_facts::LexicalBody,
}
struct CaptureEnvironmentFact {
    environment: frontend::closures::graph_facts::CaptureEnvironment,
}
/// `UNIVERSAL_ANALYZE` output: the successful verified template proof.
struct UniversalProofFact {
    verified: ownership::VerifiedTemplateCfg,
}
/// `BODY` output: exact closed facts and the instantiated checked body.
struct InstanceBodyFact {
    facts: Arc<traits::ValidatedFacts>,
    checked: Arc<ir::CheckedFunction>,
}
impl InstanceBodyFact {
    fn validate_current_engine(&self, engine: crate::engine::EngineAuthority) -> Result<(), Error> {
        let target = self
            .facts
            .target()
            .ok_or_else(|| Error::new("BODY requires closed target authority"))?;
        let original = self
            .facts
            .original_callable_schema(traits::OriginalCallableSubject::Direct(target.declaration))
            .map_err(|error| Error::new(error.to_string()))?;
        if original.generic_env().total_arity() != target.type_args.len() {
            return Err(Error::new("BODY target changed original callable arity"));
        }
        let requires_substitution = !target.type_args.is_empty()
            || !self
                .facts
                .is_source_callable(target.declaration)
                .map_err(|error| Error::new(error.to_string()))?;
        if requires_substitution && self.checked.body_substitution.is_none() {
            return Err(Error::new(
                "generic or recipe BODY omitted substitution proof",
            ));
        }
        if let Some(proof) = &self.checked.body_substitution {
            proof.validate_current_engine(engine).map_err(Error::new)?;
        }
        Ok(())
    }
}
/// `ANALYZE` output: the sealed concrete proof and its analysis.
struct ConcreteProofFact {
    concrete: ownership::ConcreteCfg,
}
/// Successful source admission for one exact canonical closed body. Generic
/// instances never use this proof: they require their own substituted analysis.
struct ClosedSourceProof {
    concrete: ownership::ConcreteCfg,
    publication: Arc<source_replay::CanonicalSourcePublication>,
    const_context: bool,
}
impl crate::engine::Fact for ClosedSourceProof {}
impl ClosedSourceProof {
    fn reuse(
        &self,
        publication: &Arc<source_replay::CanonicalSourcePublication>,
        body: &InstanceBodyFact,
        target: &ir::CallTarget,
        lowered: &Arc<cfg::Function>,
        const_context: bool,
        budget: &ir::PlanBudget,
    ) -> Result<ownership::ConcreteCfg, Error> {
        if !Arc::ptr_eq(&self.publication, publication)
            || !Arc::ptr_eq(self.concrete.function(), lowered)
            || self.const_context != const_context
            || self.concrete.facts().target() != Some(target)
            || body.facts.target() != Some(target)
            || !Arc::ptr_eq(self.concrete.facts().world(), body.facts.world())
            || !target.type_args.is_empty()
            || !self.concrete.analysis().passed()
            || !self.concrete.budget().same_authority(budget)
        {
            return Err(Error::new(
                "closed source proof changed exact body authority",
            ));
        }
        publication
            .validate_source(&body.checked, &body.facts, budget)
            .map_err(|error| source_fact_error(error, budget))?;
        lowered
            .validate_source_publication(publication, budget)
            .map_err(|error| source_fact_error(error, budget))?;
        Ok(self.concrete.clone())
    }
}
/// `SSA`, `LOCAL_OPT` and `IPO_OPT` output. `None` means the function is
/// outside the optimizer's current subset and keeps the stack emitter.
///
/// Each published fact carries an immutable receipt for the storage it keeps
/// alive, admitted before publication: the engine envelope (the fact value
/// plus the `Arc` that `Context::publish_fact` allocates for it) for every
/// fact, supported or not, plus the body's reconciled capacity and its own
/// `Arc` when the fact owns a body. A fact that re-publishes a shared body
/// pays only its envelope. No mutable budget is reachable from the fact.
struct SsaFact {
    lowered: Option<Arc<ssa::lower::Lowered>>,
    unsupported: Option<&'static str>,
    inlined: usize,
    _receipt: ssa::meter::Receipt,
}
/// Bytes `Context::publish_fact` allocates for a fact of type `T`: the value
/// inside an `Arc` (strong and weak counts) plus the type-erased handle slot.
const fn envelope<T>() -> usize {
    std::mem::size_of::<T>() + 2 * std::mem::size_of::<usize>() + 2 * std::mem::size_of::<usize>()
}
impl SsaFact {
    const ENVELOPE: usize = envelope::<SsaFact>();
    /// The `Arc<Lowered>` header and value slot.
    const BODY_ARC: usize =
        std::mem::size_of::<ssa::lower::Lowered>() + 2 * std::mem::size_of::<usize>();

    fn publish(
        mut owned: ssa::store::Owned<ssa::lower::Lowered>,
        inlined: usize,
        meter: &ssa::meter::Meter<'_>,
    ) -> Result<Self, Error> {
        owned
            .admit(meter, Self::BODY_ARC + Self::ENVELOPE)
            .map_err(Error::new)?;
        let (lowered, receipt) = owned.seal(meter).map_err(Error::new)?;
        Ok(Self {
            lowered: Some(Arc::new(lowered)),
            unsupported: None,
            inlined,
            _receipt: receipt,
        })
    }

    fn envelope_only(meter: &ssa::meter::Meter<'_>) -> Result<ssa::meter::Receipt, Error> {
        meter
            .lease(Self::ENVELOPE)
            .and_then(|lease| lease.seal(meter))
            .map_err(Error::new)
    }

    fn share(
        lowered: Arc<ssa::lower::Lowered>,
        meter: &ssa::meter::Meter<'_>,
    ) -> Result<Self, Error> {
        Ok(Self {
            lowered: Some(lowered),
            unsupported: None,
            inlined: 0,
            _receipt: Self::envelope_only(meter)?,
        })
    }

    fn unsupported(reason: &'static str, meter: &ssa::meter::Meter<'_>) -> Result<Self, Error> {
        Ok(Self {
            lowered: None,
            unsupported: Some(reason),
            inlined: 0,
            _receipt: Self::envelope_only(meter)?,
        })
    }
}
/// `SUMMARY` output: the Copy summary plus its own immutable envelope receipt.
struct SummaryFact {
    summary: Option<ssa::inline::Summary>,
    _receipt: ssa::meter::Receipt,
}
/// Analysis nodes (`UNIVERSAL_ANALYZE`, `ANALYZE`) have a second output: the
/// complete analysis, published on success and on failure (before the node
/// fails), for diagnostics and reporting. Compiler work reads only output 0.
const ANALYSIS_OUTPUT: usize = 1;
/// `SOURCE_ADMISSION` third output: the CFG it analyzed (generic template CFG
/// for universal callables). CHECK-mode CFG accounting reads this, never LOWER.
const SOURCE_CFG_OUTPUT: usize = 2;
/// Exact successful closed-source ownership proof, or None for universal bodies.
const SOURCE_CLOSED_PROOF_OUTPUT: usize = 3;
/// RESOLVE output 3: crate-wide source accounting from the final admission
/// state (and probe evidence for probe inputs).
#[derive(Default)]
struct SourceAccounting {
    source_bytes: usize,
    files: usize,
    compiler_image: Option<Arc<procfs::CompilerImage>>,
    probes: Vec<source::SourceProbeRecord>,
}
impl crate::engine::Fact for SourceAccounting {}
impl SourceAccounting {
    fn validate_runtime(&self) -> Result<(), Error> {
        if self
            .compiler_image
            .as_ref()
            .is_some_and(|image| image.matches_current())
        {
            Ok(())
        } else {
            Err(Error::new(
                "infrastructure: compiler runtime inputs changed or could not be revalidated",
            ))
        }
    }
}
const SOURCE_ACCOUNTING_OUTPUT: usize = 3;

#[derive(Debug)]
struct ParseFileFailure {
    kind: source::SourceErrorKind,
    message: Arc<str>,
    _storage: expansion_budget::RetainedBytes,
}
impl ParseFileFailure {
    fn ordered_retry(kind: source::SourceErrorKind) -> bool {
        matches!(
            kind,
            source::SourceErrorKind::Syntax | source::SourceErrorKind::Unsupported
        )
    }
    fn from_error(
        error: source::SourceError,
        budget: &expansion_budget::ExpansionBudget,
    ) -> Result<Self, Error> {
        use expansion_budget::{Counter, Storage};
        let location_bytes = error.location.as_ref().map_or(0, |location| {
            location
                .path
                .as_os_str()
                .len()
                .saturating_mul(3)
                .saturating_add(128)
        });
        let bytes = error
            .message
            .len()
            .checked_add(location_bytes)
            .and_then(|n| n.checked_add(128 + std::mem::size_of::<Self>()))
            .ok_or_else(|| Error::new("parse failure diagnostic size overflow"))?;
        budget
            .charge(Counter::DiagnosticBytes, bytes as u64)
            .map_err(|error| Error::new(error.to_string()))?;
        let lease = budget
            .reserve(Storage::Retained, bytes as u64)
            .map_err(|error| Error::new(error.to_string()))?;
        let temporary = budget
            .reserve(Storage::Temporary, bytes as u64)
            .map_err(|error| Error::new(error.to_string()))?;
        let kind = error.kind;
        let mut rendered = String::new();
        rendered
            .try_reserve_exact(bytes)
            .map_err(|_| Error::new("parse failure diagnostic allocation failed"))?;
        std::fmt::write(&mut rendered, format_args!("{error}"))
            .map_err(|_| Error::new("parse failure diagnostic formatting failed"))?;
        let message = Arc::from(rendered);
        drop(temporary);
        let storage = lease
            .retain()
            .map_err(|error| Error::new(error.to_string()))?;
        Ok(Self {
            kind,
            message,
            _storage: storage,
        })
    }
    fn inherited(&self, budget: &expansion_budget::ExpansionBudget) -> Result<Self, Error> {
        let storage = budget
            .reserve(
                expansion_budget::Storage::Retained,
                std::mem::size_of::<Self>() as u64,
            )
            .and_then(|lease| lease.retain())
            .map_err(|error| Error::new(error.to_string()))?;
        Ok(Self {
            kind: self.kind,
            message: self.message.clone(),
            _storage: storage,
        })
    }
}
impl crate::engine::Fact for ParseFileFailure {}
impl std::fmt::Display for ParseFileFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

struct Snapshot {
    input: Input,
    source_options: source::SourceOptions,
    budget: ir::PlanBudget,
}
#[derive(Clone)]
struct CompilerProvider(Arc<Snapshot>);

/// Reads of the upstream facts one node needs. Each read is a recorded edge.
struct Facts<'a> {
    cx: &'a Context,
}
impl Facts<'_> {
    async fn get<T: std::any::Any + Send + Sync>(&self, key: Key) -> Result<Arc<T>, Error> {
        self.cx.read_fact::<T>(self.cx.request(key).get(0)).await
    }
    async fn types(&self) -> Result<Arc<frontend::PlanCtx>, Error> {
        self.get(keys::SignaturesDebtRoot.key()).await
    }
    /// Decodes a canonical instance key, replaying closure actuals from their
    /// provider `TEMPLATE_BODY` outputs. Each round requests every missing
    /// provider before awaiting any. Nested closure actuals take more rounds.
    async fn decode_instance(
        &self,
        words: &[u64],
        types: &frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CallTarget, Error> {
        let mut templates = instances::graph_key::ClosureTemplates::new();
        loop {
            let missing = std::cell::RefCell::new(Vec::new());
            let result = instances::graph_key::decode_with_closures(
                words,
                types,
                budget,
                &|provider, args| {
                    instances::graph_key::replay_closure(
                        types, &templates, &missing, budget, provider, args,
                    )
                },
            );
            let mut missing = missing.into_inner();
            if result.is_ok() || missing.is_empty() {
                return result.map_err(Error::new);
            }
            missing.sort();
            missing.dedup();
            let mut pending = Vec::new();
            for provider in missing {
                if templates.contains_key(&provider) {
                    return result.map_err(Error::new);
                }
                types
                    .stored_closure_owner(provider)
                    .map_err(Error::new)?
                    .ok_or_else(|| Error::new("closure graph key provider is not a closure"))?;
                let index = types.callable_index(provider).map_err(Error::new)?;
                let handle = self.cx.request(keys::item(TEMPLATE_BODY, index)).get(0);
                pending.push((provider, handle));
            }
            for (provider, handle) in pending {
                let body: Arc<TemplateBodyFact> = self.cx.read_fact(handle).await?;
                let closure = body
                    .closure
                    .as_ref()
                    .ok_or_else(|| Error::new("closure template omitted canonical metadata"))?;
                templates.insert(provider, closure.ty.clone());
            }
        }
    }
    async fn template(&self, index: usize) -> Result<Arc<TemplateBodyFact>, Error> {
        self.get(keys::Item::<TEMPLATE_BODY>(index).key()).await
    }
    async fn callee_templates(
        &self,
        types: &frontend::PlanCtx,
        targets: Vec<(usize, ir::CallTarget)>,
    ) -> Result<Vec<ownership::VerifiedTemplateCfg>, Error> {
        // All independent canonical parent-body reads are requested first.
        let mut pending = Vec::new();
        for (index, target) in targets {
            let handle = if types
                .stored_closure_owner(target.declaration)
                .map_err(Error::new)?
                .is_some()
            {
                Some(self.cx.request(keys::item(TEMPLATE_BODY, index)).get(0))
            } else {
                None
            };
            pending.push((index, target, handle));
        }
        let mut keys = Vec::new();
        for (index, target, handle) in pending {
            let key = if let Some(handle) = handle {
                let template: Arc<TemplateBodyFact> = self.cx.read_fact(handle).await?;
                let closure = template
                    .closure
                    .as_ref()
                    .ok_or_else(|| Error::new("closure admission omitted canonical metadata"))?;
                types
                    .validate_closure_instance(closure, &target)
                    .map_err(Error::new)?;
                let mode = types.closure_instance_mode(&target).map_err(Error::new)?;
                closure_body_key(UNIVERSAL_ANALYZE, index, closure.ty.kind(), mode)
            } else {
                keys::item(UNIVERSAL_ANALYZE, index)
            };
            keys.push(key);
        }
        keys.sort_by(|left, right| left.params.cmp(&right.params));
        keys.dedup();
        let handles: Vec<_> = keys
            .into_iter()
            .map(|key| self.cx.request(key).get(0))
            .collect();
        let mut templates = Vec::new();
        for handle in handles {
            let proof: Arc<UniversalProofFact> = self.cx.read_fact(handle).await?;
            templates.push(proof.verified.clone());
        }
        Ok(templates)
    }
    /// The target and source declaration of an instance-keyed node. Closed
    /// roots are keyed by their callable index; every other instance by its
    /// canonical graph-key words (`instance_params`). Decoding replays closure
    /// actuals and rejects a words key that names a closed root.
    async fn instance_of(
        &self,
        key: &Key,
        budget: &ir::PlanBudget,
    ) -> Result<(ir::CallTarget, usize), Error> {
        let types = self.types().await?;
        if let [slot] = key.params.as_slice() {
            let slot = usize::try_from(*slot).map_err(|_| Error::new("invalid instance slot"))?;
            return root_instance(&types, slot)?
                .ok_or_else(|| Error::new("instance slot is not a closed root"));
        }
        let target = self.decode_instance(&key.params, &types, budget).await?;
        if root_slot_of(&types, &target)?.is_some() {
            return Err(Error::new(
                "noncanonical instance key: closed roots are keyed by slot",
            ));
        }
        let source = types
            .callable_index(target.declaration)
            .map_err(Error::new)?;
        Ok((target, source))
    }
}
/// The closed-root slot of `target`, if it is a nongeneric Source root.
fn root_slot_of(
    types: &frontend::PlanCtx,
    target: &ir::CallTarget,
) -> Result<Option<usize>, Error> {
    if !target.type_args.is_empty() {
        return Ok(None);
    }
    let callable = types
        .callable_index(target.declaration)
        .map_err(Error::new)?;
    Ok(types
        .is_closed_root(callable)
        .map_err(Error::new)?
        .then_some(callable))
}
/// Canonical key parameters of an executable instance: the closed-root slot
/// (its callable index) or the canonical graph-key words. Exactly
/// one form exists per target, so equivalent instances share one node.
fn instance_params(
    types: &frontend::PlanCtx,
    target: &ir::CallTarget,
    budget: &ir::PlanBudget,
) -> Result<Vec<u64>, Error> {
    if let Some(slot) = root_slot_of(types, target)? {
        return Ok(vec![slot as u64]);
    }
    instances::graph_key::encode(target, budget).map_err(Error::new)
}
/// The closed-root slot named by an instance key, if it is slot-keyed.
/// Canonical, stable identity of an instance node for diagnostics: the root
/// slot for closed roots, otherwise the generic definition's name and source
/// item plus the canonical target words of the instance key.
fn instance_identity(key: &Key, source_index: usize, name: &str) -> String {
    match key_root_slot(key) {
        Some(slot) => slot.to_string(),
        None => format!(
            "{name} (generic item {source_index}, target {:x?})",
            key.params
        ),
    }
}
fn key_root_slot(key: &Key) -> Option<usize> {
    match key.params.as_slice() {
        [slot] => usize::try_from(*slot).ok(),
        _ => None,
    }
}
/// Nongeneric Source callables in callable order are the plan's root slots
/// (instances::build enumerates them first, in this order). Returns the
/// `index`-th one, or `None` if `index` is past the roots.
fn root_instance(
    types: &frontend::PlanCtx,
    index: usize,
) -> Result<Option<(ir::CallTarget, usize)>, Error> {
    if index >= types.callable_count() || !types.is_closed_root(index).map_err(Error::new)? {
        return Ok(None);
    }
    let declaration = types.callable_decl(index).map_err(Error::new)?;
    Ok(Some((
        ir::CallTarget {
            declaration,
            type_args: Arc::from([]),
        },
        index,
    )))
}

impl CompilerProvider {
    /// The original checked body of callable `index`, revalidated against its
    /// canonical publication exactly as before.
    fn validated_body(
        &self,
        types: &frontend::PlanCtx,
        index: usize,
        template: &TemplateBodyFact,
    ) -> Result<Arc<ir::CheckedFunction>, Error> {
        let source = &template.checked;
        let declaration = types.callable_decl(index).map_err(Error::new)?;
        let facts = if let Some(closure) = &template.closure {
            types
                .trait_world()
                .map_err(Error::new)?
                .closure_callable_facts_for(
                    closure.ty.clone(),
                    template
                        .closure_mode
                        .ok_or_else(|| Error::new("closure template has no capability"))?,
                )
                .map_err(|e| source_fact_error(e, &self.0.budget))?
        } else if let Some(publication) = &template.publication {
            publication.facts().clone()
        } else {
            types.callable_facts(declaration).map_err(Error::new)?
        };
        if facts.declaration() != declaration {
            return Err(Error::new(
                "source publication changed its caller declaration",
            ));
        }
        if facts
            .is_source_callable(declaration)
            .map_err(|error| source_fact_error(error, &self.0.budget))?
        {
            let publication = template
                .publication
                .as_ref()
                .ok_or_else(|| Error::new("native Source body has no canonical publication"))?;
            publication
                .validate_source(source, &facts, &self.0.budget)
                .map_err(|error| source_fact_error(error, &self.0.budget))?;
        } else if template.publication.is_some() {
            return Err(Error::new(
                "Recipe body cannot substitute a Source publication",
            ));
        }
        Ok(source.clone())
    }
    fn scratch_vec<T>(&self, count: usize) -> Result<(Vec<T>, ir::PlanReservation), Error> {
        self.0.budget.charge_work(count).map_err(Error::new)?;
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| Error::new("generic frontier byte limit exceeded"))?;
        let reservation = self.0.budget.reserve_bytes(bytes).map_err(Error::new)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| Error::new("generic frontier allocation limit exceeded"))?;
        Ok((values, reservation))
    }
    fn universal_body(types: &frontend::PlanCtx, declaration: ir::DeclId) -> Result<bool, Error> {
        match types.callable_provider(declaration).map_err(Error::new)? {
            frontend::CallableProvider::Source { .. } => Ok(types
                .callable_env(declaration)
                .map_err(Error::new)?
                .is_generic()),
            frontend::CallableProvider::Recipe { .. } => Ok(true),
            frontend::CallableProvider::UnavailableProvided { .. } => Err(Error::new(
                "unsupported: unavailable provided body in executable declaration inventory",
            )),
        }
    }
    /// The crate's syntax, from per-file admission and per-file parses.
    /// Probe inputs and splice-ordered syntax diagnostics still replay the
    /// assembled parser inside RESOLVE (whole-program debt).
    async fn crate_syntax(
        &self,
        cx: &Context,
    ) -> Result<(frontend::ParsedProgram, SourceAccounting), Error> {
        let options = &self.0.source_options;
        let root = source::source_admit::final_root(options);
        let admitted: Arc<source::source_admit::AdmitState> = cx
            .read_item(&keys::Admit(keys::SourceFile::from_words(&[root])?), 1)
            .await?;
        if let Some(error) = admitted.failure() {
            return Err(Error::new(error.to_string()));
        }
        let mut accounting = SourceAccounting {
            source_bytes: admitted.source_bytes,
            files: admitted.files,
            compiler_image: options.compiler_image.clone(),
            probes: Vec::new(),
        };
        let budget = expansion_budget::ExpansionBudget::default();
        let mut issuer = owner_path::OwnerIssuer::new(cx.owner_root()?, &budget)
            .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
        let snapshot = issuer
            .issue(owner_path::OwnerDomain::SourceSnapshot)
            .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
        let owner = issuer
            .issue(owner_path::OwnerDomain::TokenSession)
            .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
        if let Input::ProbeFile(_, request) = &self.0.input {
            let prepared = self.assemble(cx).await?;
            let prepared = source::attach_probe(prepared, options, request)
                .map_err(|error| Error::new(error.to_string()))?;
            accounting.source_bytes = prepared.source_bytes;
            accounting.files = prepared.files;
            accounting.probes = prepared.probes.clone();
            let parsed = frontend::parse_prepared_owned(&prepared, budget, snapshot, owner)
                .map_err(|error| Error::new(error.to_string()))?;
            return Ok((parsed, accounting));
        }
        let file_keys = admitted
            .admitted()
            .iter()
            .map(|ordinal| keys::SourceFile::from_words(ordinal).map(keys::ParseFile))
            .collect::<Result<Vec<_>, _>>()?;
        let handles = cx.request_all(file_keys.iter(), 0);
        let mut parsed_files = Vec::with_capacity(handles.len());
        let mut retry = false;
        for handle in handles {
            let parsed: Arc<Result<frontend::ParsedProgram, ParseFileFailure>> =
                cx.read_fact(handle).await?;
            match parsed.as_ref() {
                Ok(_) => parsed_files.push(parsed),
                Err(error) if ParseFileFailure::ordered_retry(error.kind) => retry = true,
                Err(error) => return Err(Error::new(error.to_string())),
            }
        }
        // Only syntax/unsupported errors replay the assembled parser for
        // splice-order diagnostics. Authority, resource, source and
        // infrastructure failures always fail closed.
        let parsed = if retry {
            let prepared = self.assemble(cx).await?;
            frontend::parse_prepared_owned(&prepared, budget, snapshot, owner)
                .map_err(|error| Error::new(error.to_string()))?
        } else {
            let files = parsed_files
                .iter()
                .filter_map(|file| file.as_ref().as_ref().ok())
                .collect::<Vec<_>>();
            frontend::merge_crate_file_refs(&files, budget, owner).map_err(Error::new)?
        };
        Ok((parsed, accounting))
    }
    /// Sequential assembly of every admitted file (diagnostic replay only).
    async fn assemble(&self, cx: &Context) -> Result<source::PreparedSource, Error> {
        let mut assembler = source::SourceAssembler::new(&self.0.source_options);
        let root: Arc<source::UnitResult> = cx.read_item(&keys::SourceFile::input(), 0).await?;
        let mut step = assembler.provide(root);
        loop {
            match step.map_err(|error| Error::new(error.to_string()))? {
                source::AssemblyStep::Done(prepared) => return Ok(*prepared),
                source::AssemblyStep::Need(need) => {
                    let unit: Arc<source::UnitResult> = cx
                        .read_item(&keys::SourceFile::from_words(&need.ordinal)?, 0)
                        .await?;
                    step = assembler.provide(unit);
                }
            }
        }
    }
    /// Account retained fact storage in the shared planning ledger.
    fn retain(&self, bytes: usize) -> Result<(), Error> {
        self.0.budget.retain_bytes(bytes).map_err(Error::new)
    }
}
impl Provider for CompilerProvider {
    fn outputs(&self, key: &Key) -> usize {
        match key.kind {
            UNIVERSAL_ANALYZE
            | ANALYZE
            | CONST_BODY
            | ASSOCIATED_CONST_HEADER
            | ASSOCIATED_CONST_SELECT
            | OPAQUE_CALL_DISPATCH => 2,
            TEMPLATE_BODY => 3,
            SOURCE_ADMISSION => 4,
            ADMIT => 2,
            RESOLVE => 4,
            SIGNATURES => 2,
            _ => 1,
        }
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        let this = self.clone();
        if key.kind == nominal_realization::KIND {
            return Box::pin(async move { nominal_realization::run(&cx, key).await });
        }
        // Keep declaration-local publication out of the legacy giant polling
        // frame used by unrelated source-checking producers.
        if key.kind == NOMINAL_DECL_WELL_FORMED {
            return Box::pin(async move {
                if key.params.len() != 2 {
                    return Err(Error::new(
                        "nominal declaration key requires exact snapshot and declaration",
                    ));
                }
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(key.params[0]),
                    index: key.params[1],
                };
                let facts = Facts { cx: &cx };
                let types = facts.types().await?;
                let proof = types
                    .publish_nominal_declaration(declaration)
                    .map_err(Error::new)?;
                cx.publish_fact(0, proof)
            });
        }
        Box::pin(async move {
            let facts = Facts { cx: &cx };
            let function = keys::item;
            // The same instance under another per-instance node kind.
            let sibling = |kind: u32| keys::words(kind, key.params.clone());
            match key.kind {
                OPAQUE_CALL_DISPATCH => opaque_dispatch::produce(&this, &cx, &key).await,
                CONST_DECL | CONST_BODY | CONST_VALUE => this.constant_node(&cx, &key).await,
                ASSOCIATED_CONST_HEADER => this.associated_constant_header_node(&cx, &key).await,
                ASSOCIATED_CONST_SELECT | ASSOCIATED_CONST_VALUE => {
                    this.associated_constant_use_node(&cx, &key).await
                }
                SOURCE => {
                    // One node per source file, keyed by its child-ordinal path
                    // from the root file: [0] is the root, [0, i] its i-th
                    // external child module, and so on.
                    let options = &this.0.source_options;
                    if options.source_snapshot.is_some() && options.compiler_image.is_none() {
                        return Err(Error::new(
                            "infrastructure: immutable source compilation requires running compiler identity",
                        ));
                    }
                    let budget = expansion_budget::ExpansionBudget::default();
                    let file = keys::SourceFile::from_words(&key.params)?;
                    let unit = match key.params.as_slice() {
                        [] => return Err(Error::new("source file key has no ordinal")),
                        [0] => match &this.0.input {
                            Input::File(path) | Input::ProbeFile(path, _) => {
                                source::prepare_root_unit(path, options, &budget)
                            }
                            Input::Text(text) => source::prepare_text_unit("<text>", text, options),
                            Input::Program(_) => {
                                return Err(Error::new("prepared program has no source input"));
                            }
                        },
                        // [1] attaches the embedded native prelude library.
                        [1] => source::prepare_prelude_unit(options, &budget),
                        // [2 + i] attaches dependency crate i.
                        [n] if (*n as usize).wrapping_sub(2) < options.externs.len() => {
                            source::prepare_extern_unit(options, *n as usize - 2, &budget)
                        }
                        [_] => return Err(Error::new("unknown source root ordinal")),
                        [.., index] => {
                            let parent: Arc<source::UnitResult> = cx
                                .read_item(
                                    &file
                                        .parent()
                                        .ok_or_else(|| Error::new("child source has no parent"))?,
                                    0,
                                )
                                .await?;
                            let parent = parent.as_ref().as_ref().map_err(|_| {
                                Error::new("child module requested from a failed source file")
                            })?;
                            let options = options.for_ordinal(&key.params);
                            source::prepare_child_unit(parent, *index as usize, &options, &budget)
                        }
                    };
                    // Discover a bounded independent frontier before the
                    // assembly consumer asks for its first child. Until source
                    // admission is fully per-file, unbounded recursive prefetch
                    // could expand an enormous inclusion tree before assembly
                    // reports its existing input limit. This is a speculation
                    // policy, not an admission limit: deeper/later files remain
                    // demandable normally. Eager expansion follows only the
                    // first three levels and four children per parent, without
                    // turning a deeply branching invalid input into unbounded
                    // speculative work. There is no shared
                    // budget or scheduler state in compiler facts.
                    // Errors remain values, selected in assembly source order.
                    if let Ok(unit) = &unit
                        && unit.terminal.is_none()
                        && file.depth() < 3
                    {
                        for index in 0..unit.children.len().min(4) {
                            cx.request_item(&file.child(index));
                        }
                    }
                    cx.publish_fact(0, unit)
                }
                PARSE_FILE => {
                    let file = keys::SourceFile::from_words(&key.params)?;
                    let attachment = key.params.len() == 1 && key.params[0] != 0;
                    let admitted = cx.request_item(&keys::Admit(file.clone())).get(0);
                    // The cfg(test) harness entry closes over every admitted
                    // test, so only the harness root waits on the crate exit.
                    let harness = (key.params == [0]
                        && this.0.source_options.cfg.names.contains("test"))
                    .then(|| cx.request_item(&keys::Admit(file.clone())).get(1));
                    let source = cx.request_item(&file).get(0);
                    let parent = file
                        .parent()
                        .or_else(|| attachment.then(keys::SourceFile::input))
                        .map(|parent| cx.request_item(&keys::ParseFile(parent)).get(0));
                    let admitted: Arc<source::source_admit::AdmitState> =
                        cx.read_fact(admitted).await?;
                    if admitted.failed() {
                        return Err(Error::new("parse file was not admitted"));
                    }
                    let base = admitted.id();
                    let entry = match harness {
                        Some(handle) => {
                            let exit: Arc<source::source_admit::AdmitState> =
                                cx.read_fact(handle).await?;
                            if exit.failed() {
                                return Err(Error::new("parse file crate was not admitted"));
                            }
                            exit.entry_text().map(str::to_owned)
                        }
                        None => None,
                    };
                    let source: Arc<source::UnitResult> = cx.read_fact(source).await?;
                    let unit = source
                        .as_ref()
                        .as_ref()
                        .map_err(|_| Error::new("admitted parse file has a failed source"))?;
                    let options = this.0.source_options.for_ordinal(&key.params);
                    let view =
                        source::FileView::for_graph(unit, base, &options, &key.params, entry)
                            .map_err(|error| Error::new(error.to_string()))?;
                    let parent: Option<Arc<Result<frontend::ParsedProgram, ParseFileFailure>>> =
                        match parent {
                            Some(handle) => Some(cx.read_fact(handle).await?),
                            None => None,
                        };
                    let budget = expansion_budget::ExpansionBudget::default();
                    let mut issuer = owner_path::OwnerIssuer::new(cx.owner_root()?, &budget)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let snapshot = issuer
                        .issue(owner_path::OwnerDomain::SourceSnapshot)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let owner = issuer
                        .issue(owner_path::OwnerDomain::TokenSession)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let parsed = match parent.as_deref() {
                        None => {
                            frontend::parse_file_view_owned(&view, budget.clone(), snapshot, owner)
                        }
                        Some(Ok(parent)) if attachment => frontend::parse_attachment_view_owned(
                            &view,
                            parent,
                            budget.clone(),
                            snapshot,
                            owner,
                        ),
                        Some(Ok(parent)) => frontend::parse_child_view_owned(
                            &view,
                            parent,
                            *key.params.last().unwrap() as usize,
                            budget.clone(),
                            snapshot,
                            owner,
                        ),
                        Some(Err(error)) => {
                            return cx.publish_fact(
                                0,
                                Err::<frontend::ParsedProgram, _>(error.inherited(&budget)?),
                            );
                        }
                    };
                    let parsed = match parsed {
                        Ok(parsed) => Ok(parsed),
                        Err(error) => Err(ParseFileFailure::from_error(error, &budget)?),
                    };
                    cx.publish_fact(0, parsed)
                }
                ADMIT => {
                    let options = &this.0.source_options;
                    let file = keys::SourceFile::from_words(&key.params)?;
                    let mut anchor = None;
                    let mut state = match file.parent() {
                        None => match source::source_admit::previous_root(key.params[0], options) {
                            None => source::source_admit::AdmitState::default(),
                            Some(root) => {
                                let previous: Arc<source::source_admit::AdmitState> = cx
                                    .read_item(
                                        &keys::Admit(keys::SourceFile::from_words(&[root])?),
                                        1,
                                    )
                                    .await?;
                                previous.as_ref().clone()
                            }
                        },
                        Some(parent) => {
                            let index = *key.params.last().unwrap() as usize;
                            // Request every predecessor before awaiting any.
                            let entry = cx.request_item(&keys::Admit(parent.clone())).get(0);
                            let previous = index.checked_sub(1).map(|previous| {
                                cx.request_item(&keys::Admit(parent.child(previous))).get(1)
                            });
                            let parent_unit = cx.request_item(&parent).get(0);
                            let entry: Arc<source::source_admit::AdmitState> =
                                cx.read_fact(entry).await?;
                            let mut state = match previous {
                                Some(previous) => {
                                    let previous: Arc<source::source_admit::AdmitState> =
                                        cx.read_fact(previous).await?;
                                    previous.as_ref().clone()
                                }
                                None => entry.as_ref().clone(),
                            };
                            if !state.failed() {
                                let parent_unit: Arc<source::UnitResult> =
                                    cx.read_fact(parent_unit).await?;
                                let parent_unit = parent_unit.as_ref().as_ref().map_err(|_| {
                                    Error::new("admitted child of a failed source file")
                                })?;
                                anchor = source::source_admit::before_child(
                                    &mut state,
                                    parent_unit,
                                    entry.id(),
                                    parent.depth() + 1,
                                    index,
                                );
                            }
                            state
                        }
                    };
                    if state.failed() {
                        cx.publish_fact(0, state.clone())?;
                        return cx.publish_fact(1, state);
                    }
                    // Only an admitted position demands its file, so discovery
                    // stops at the first error in source order.
                    let unit: Arc<source::UnitResult> = cx.read_item(&file, 0).await?;
                    source::source_admit::admit(&mut state, &key.params, &unit, anchor, options);
                    cx.publish_fact(0, state.clone())?;
                    let Ok(unit) = unit.as_ref() else {
                        return cx.publish_fact(1, state);
                    };
                    if state.failed() {
                        return cx.publish_fact(1, state);
                    }
                    let id = state.id();
                    if let Some(last) = unit.children.len().checked_sub(1) {
                        let exit: Arc<source::source_admit::AdmitState> =
                            cx.read_item(&keys::Admit(file.child(last)), 1).await?;
                        state = exit.as_ref().clone();
                    }
                    source::source_admit::finish(&mut state, &key.params, unit, id, options);
                    cx.publish_fact(1, state)
                }
                RESOLVE_MODULE => {
                    let ordinal = item_index(&key, "missing resolution module")?;
                    let namespace_handle = cx.request(keys::ResolveDebtRoot.key()).get(1);
                    let namespace: Arc<Arc<frontend::ResolutionNamespace>> =
                        cx.read_fact(namespace_handle).await?;
                    crate::engine::scale_task_work(
                        u64::try_from(
                            frontend::resolution_module_indices(&namespace, ordinal)
                                .map_err(Error::new)?
                                .len(),
                        )
                        .unwrap_or(u64::MAX)
                        .saturating_mul(WORK_PER_PROGRAM_ITEM),
                    );
                    let items =
                        frontend::resolve_module_items(&namespace, ordinal).map_err(Error::new)?;
                    cx.publish_fact(0, items)
                }
                RESOLVE => {
                    this.retain(envelope::<frontend::ValidatedProgram>())?;
                    let program = match &this.0.input {
                        Input::Program(program) => {
                            cx.publish_fact(3, SourceAccounting::default())?;
                            scale_task_work_by_items(program);
                            let sealed = frontend::ValidatedProgram::new(program.clone())
                                .map_err(Error::new)?;
                            cx.publish_fact(2, sealed)?;
                            cx.publish(1, 1)?;
                            program.clone()
                        }
                        _ => {
                            let (parsed, accounting) = this.crate_syntax(&cx).await?;
                            cx.publish_fact(3, accounting)?;
                            let parsed = Arc::new(parsed);
                            this.retain(
                                std::mem::size_of::<frontend::Program>()
                                    + 2 * std::mem::size_of::<usize>(),
                            )?;
                            let namespace = frontend::prepare_resolution_owned(parsed.clone())
                                .map_err(Error::new)?;
                            let module_keys = (0..frontend::resolution_modules(&namespace).len())
                                .map(keys::Item::<RESOLVE_MODULE>)
                                .collect::<Vec<_>>();
                            let namespace = Arc::new(namespace);
                            cx.publish_fact(1, namespace.clone())?;
                            let handles = cx.request_all(module_keys.iter(), 0);
                            let mut items = Vec::with_capacity(handles.len());
                            for handle in handles {
                                let item: Arc<frontend::ResolvedModuleItems> =
                                    cx.read_fact(handle).await?;
                                items.push(item.as_ref().clone());
                            }
                            let (program, sealed) =
                                frontend::finish_resolution_sealed(&namespace, items)
                                    .map_err(Error::new)?;
                            cx.publish_fact(2, sealed)?;
                            program
                        }
                    };
                    cx.publish_fact(0, program)
                }
                SIGNATURE_MODULE => {
                    let ordinal = item_index(&key, "missing signature module")?;
                    let inputs: Arc<Arc<frontend::signature_graph::CallableSignatureInputs>> = cx
                        .read_fact(cx.request(keys::SignaturesDebtRoot.key()).get(1))
                        .await?;
                    crate::engine::scale_task_work(
                        u64::try_from(inputs.module_input_size(ordinal).map_err(Error::new)?)
                            .unwrap_or(u64::MAX)
                            .saturating_mul(WORK_PER_CALLABLE_UNIT),
                    );
                    let output = frontend::signature_graph::compute_module_signatures(
                        inputs.as_ref().clone(),
                        ordinal,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish_fact(0, output)
                }
                SIGNATURES => {
                    let program: Arc<frontend::ValidatedProgram> = cx
                        .read_fact(cx.request(keys::ResolveDebtRoot.key()).get(2))
                        .await?;
                    // Debt: whole-program node. Its input is the whole program,
                    // so its per-task work limit scales with that input.
                    scale_task_work_by_items(program.program());
                    // The catalog's declaration namespace and the trait world
                    // are owners issued by this node, never a global counter.
                    let owner_budget = expansion_budget::ExpansionBudget::default();
                    let mut issuer = owner_path::OwnerIssuer::new(cx.owner_root()?, &owner_budget)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let catalog = issuer
                        .issue(owner_path::OwnerDomain::DeclarationSnapshot)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let world = issuer
                        .issue(owner_path::OwnerDomain::TraitWorld)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let signature_cx = &cx;
                    let context = frontend::PlanCtx::from_validated_program_with_signatures(
                        program.as_ref(),
                        &this.0.budget,
                        false,
                        catalog,
                        world,
                        |inputs| async move {
                            let module_keys = (0..inputs.module_count())
                                .map(keys::Item::<SIGNATURE_MODULE>)
                                .collect::<Vec<_>>();
                            signature_cx
                                .publish_fact(1, inputs.clone())
                                .map_err(|error| error.to_string())?;
                            let handles = signature_cx.request_all(module_keys.iter(), 0);
                            let mut outputs = Vec::with_capacity(handles.len());
                            for handle in handles {
                                let output: Arc<frontend::signature_graph::ModuleSignatureOutput> =
                                    signature_cx
                                        .read_fact(handle)
                                        .await
                                        .map_err(|error| error.to_string())?;
                                outputs.push(output);
                            }
                            frontend::signature_graph::finish_module_signatures(&inputs, outputs)
                        },
                    )
                    .await
                    .map_err(Error::new)?;
                    cx.publish_fact(0, context)
                }
                SIGNATURE => {
                    let index = item_index(&key, "missing signature callable")?;
                    let types = facts.types().await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let signature = types.signature(declaration).map_err(Error::new)?;
                    cx.publish_fact(
                        0,
                        SignatureFact {
                            declaration,
                            signature,
                        },
                    )
                }
                MODULE_DECLS => {
                    let [module] = key.params.as_slice() else {
                        return Err(Error::new("module declarations key requires a module"));
                    };
                    let program: Arc<Arc<frontend::Program>> =
                        facts.get(keys::ResolveDebtRoot.key()).await?;
                    let declarations = frontend::module_declarations::module_declarations(
                        &program,
                        *module,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish_fact(0, ModuleDeclsFact { declarations })
                }
                TRAIT_IMPLS => {
                    // Explicit per-trait completeness: every module's
                    // impl->trait edges, requested together, then filtered.
                    let declaration = decl_param(&key, "trait impls key")?;
                    let program: Arc<Arc<frontend::Program>> =
                        facts.get(keys::ResolveDebtRoot.key()).await?;
                    let modules =
                        frontend::module_declarations::declaring_modules(&program, &this.0.budget)
                            .map_err(Error::new)?;
                    let module_keys: Vec<keys::Params<MODULE_DECLS>> = modules
                        .into_iter()
                        .map(|module| keys::Params(module, Vec::new()))
                        .collect();
                    let handles = cx.request_all(&module_keys, 0);
                    let mut impls = Vec::new();
                    for handle in handles {
                        let module: Arc<ModuleDeclsFact> = cx.read_fact(handle).await?;
                        for &(implementation, implemented) in &module.declarations.impl_traits {
                            if implemented == declaration.index {
                                impls.push(ir::DeclId {
                                    snapshot: declaration.snapshot,
                                    index: implementation,
                                });
                            }
                        }
                    }
                    cx.publish_fact(0, TraitImplsFact { impls })
                }
                IMPL_COHERENCE => {
                    let declaration = decl_param(&key, "impl coherence key")?;
                    let types = facts.types().await?;
                    let Some(trait_decl) = types.impl_trait(declaration).map_err(Error::new)?
                    else {
                        return cx.publish(0, 1);
                    };
                    let peers: Arc<TraitImplsFact> = facts
                        .get(
                            keys::Params::<TRAIT_IMPLS>(
                                trait_decl.snapshot.graph_word(),
                                vec![trait_decl.index],
                            )
                            .key(),
                        )
                        .await?;
                    types
                        .validate_impl_coherence(declaration, &peers.impls)
                        .map_err(Error::new)?;
                    cx.publish(0, 1)
                }
                ITEM_COHERENCE => {
                    let index = item_index(&key, "missing item coherence callable")?;
                    let types = facts.types().await?;
                    types
                        .validate_item_coherence(index, &this.0.budget)
                        .map_err(Error::new)?;
                    cx.publish(0, 1)
                }
                BODY_LEXICAL => {
                    let index = item_index(&key, "missing lexical owner declaration")?;
                    if key.params.len() != 1 {
                        return Err(Error::new(
                            "lexical body key must contain exactly one owner",
                        ));
                    }
                    let types = facts.types().await?;
                    let lexical = types
                        .publish_closure_lexical(index, &this.0.budget)
                        .map_err(Error::new)?;
                    cx.publish_fact(0, BodyLexicalFact { lexical })
                }
                CAPTURE_ENV => {
                    let index = usize::try_from(
                        *key.params
                            .first()
                            .ok_or_else(|| Error::new("missing capture owner declaration"))?,
                    )
                    .map_err(|_| Error::new("invalid capture owner index"))?;
                    let length = key
                        .params
                        .len()
                        .checked_sub(1)
                        .ok_or_else(|| Error::new("missing capture source path"))?;
                    if length == 0 || length % 2 != 0 {
                        return Err(Error::new(
                            "capture source key must contain structural role/index pairs",
                        ));
                    }
                    this.0.budget.check_depth(length / 2).map_err(Error::new)?;
                    let (mut path, _reservation) = this.scratch_vec::<u32>(length)?;
                    for component in &key.params[1..] {
                        path.push(
                            u32::try_from(*component)
                                .map_err(|_| Error::new("capture source path overflow"))?,
                        );
                    }
                    let lexical: Arc<BodyLexicalFact> =
                        facts.get(function(BODY_LEXICAL, index)).await?;
                    let types = facts.types().await?;
                    let environment = types
                        .publish_capture_environment(&lexical.lexical, &path, &this.0.budget)
                        .map_err(Error::new)?;
                    cx.publish_fact(0, CaptureEnvironmentFact { environment })
                }
                TEMPLATE_BODY => {
                    let (index, requested_mode) = closure_body_index(&key)?;
                    let types = facts.types().await?;
                    if let Some((owner, site)) = types
                        .stored_closure_owner(types.callable_decl(index).map_err(Error::new)?)
                        .map_err(Error::new)?
                    {
                        let declaration = types.callable_decl(index).map_err(Error::new)?;
                        // All nested closures are finalized by the outermost
                        // source callable. Their immediate lexical owner and
                        // site still authenticate selection below.
                        let publication_owner = types
                            .closure_publication_owner(declaration, &this.0.budget)
                            .map_err(Error::new)?;
                        let owner_index = types
                            .callable_index(publication_owner)
                            .map_err(Error::new)?;
                        let parent = cx.request(function(TEMPLATE_BODY, owner_index));
                        let bodies: Arc<ClosureBodiesFact> = cx.read_fact(parent.get(1)).await?;
                        let parent_body: Arc<TemplateBodyFact> =
                            cx.read_fact(parent.get(0)).await?;
                        let closure = bodies
                            .bodies
                            .iter()
                            .find(|body| {
                                body.ty.provider() == declaration
                                    && body.ty.owner() == owner
                                    && body.ty.site() == site
                            })
                            .ok_or_else(|| {
                                Error::new("closure body absent from its canonical parent fact")
                            })?
                            .clone();
                        let publication = parent_body.publication.as_ref().ok_or_else(|| {
                            Error::new("closure parent omitted finalized source publication")
                        })?;
                        let closure = bodies
                            .publication
                            .as_ref()
                            .ok_or_else(|| {
                                Error::new(
                                    "closure parent omitted recipe/publication correspondence",
                                )
                            })?
                            .restore_closure(&types, publication, &closure, &this.0.budget)
                            .map_err(Error::new)?;
                        let mode = requested_mode.unwrap_or(closure.ty.kind());
                        if requested_mode == Some(closure.ty.kind()) {
                            return Err(Error::new("noncanonical primary closure capability key"));
                        }
                        let body = closure.body_for(mode).map_err(Error::new)?;
                        let closure_facts = types
                            .trait_world()
                            .map_err(Error::new)?
                            .closure_callable_facts_for(closure.ty.clone(), mode)
                            .map_err(|e| source_fact_error(e, &this.0.budget))?;
                        ir::validate_call_evidence(&body, &closure_facts, &this.0.budget)
                            .map_err(Error::new)?;
                        cx.publish_fact(
                            1,
                            ClosureBodiesFact {
                                bodies: Arc::from([]),
                                publication: None,
                            },
                        )?;
                        cx.publish_fact(
                            2,
                            OpaqueHiddenTypesFact {
                                representations: Arc::from([]),
                            },
                        )?;
                        return cx.publish_fact(
                            0,
                            TemplateBodyFact {
                                checked: body,
                                publication: None,
                                closure: Some(closure),
                                closure_mode: Some(mode),
                            },
                        );
                    }
                    if requested_mode.is_some() {
                        return Err(Error::new("receiver capability key on non-closure body"));
                    }
                    for owner in types
                        .closure_graph_owners(index, &this.0.budget)
                        .map_err(Error::new)?
                    {
                        let lexical: Arc<BodyLexicalFact> =
                            facts.get(function(BODY_LEXICAL, owner)).await?;
                        for site in lexical.lexical.sites.iter() {
                            this.0.budget.charge_work(1).map_err(Error::new)?;
                            if owner != index && site.declaration != index {
                                continue;
                            }
                            let capacity = site
                                .node
                                .path
                                .len()
                                .checked_add(1)
                                .ok_or_else(|| Error::new("capture key capacity overflow"))?;
                            let (mut parameters, _reservation) =
                                this.scratch_vec::<u64>(capacity)?;
                            parameters.push(
                                u64::try_from(owner)
                                    .map_err(|_| Error::new("capture owner key overflow"))?,
                            );
                            parameters.extend(site.node.path.iter().map(|value| u64::from(*value)));
                            let environment: Arc<CaptureEnvironmentFact> =
                                facts.get(keys::words(CAPTURE_ENV, parameters)?).await?;
                            let environment = &environment.environment;
                            if environment.parent != owner
                                || environment.declaration != site.declaration
                                || environment.site != site.node
                                || environment.fields.len() != site.captures.len()
                            {
                                return Err(Error::new(
                                    "capture environment handle changed original source site",
                                ));
                            }
                            for (field, original) in
                                environment.fields.iter().zip(site.captures.iter())
                            {
                                this.0.budget.charge_work(1).map_err(Error::new)?;
                                if field.parameter != original.parameter
                                    || field.binding != original.binding
                                    || field.scalar != original.scalar
                                    || field.kind != original.kind
                                {
                                    return Err(Error::new(
                                        "capture environment handle changed original binding",
                                    ));
                                }
                            }
                        }
                    }
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    scale_task_work_by_callable(&types, declaration);
                    let prepared = types
                        .prepare_body_constants(declaration, &this.0.budget)
                        .map_err(|error| source_fact_error(error, &this.0.budget))?;
                    let (mut constant_handles, _constant_handles_storage) =
                        this.scratch_vec(prepared.direct().len())?;
                    for declaration in prepared.direct() {
                        constant_handles.push(
                            cx.request(function(CONST_VALUE, declaration.index as usize))
                                .get(0),
                        );
                    }
                    let generic = types
                        .callable_env(declaration)
                        .map_err(Error::new)?
                        .is_generic();
                    let associated_count = if generic {
                        0
                    } else {
                        prepared.associated_paths().count()
                    };
                    let caller = ir::CallTarget {
                        declaration,
                        type_args: Arc::from([]),
                    };
                    let (mut associated_value_handles, _associated_value_handles_storage) =
                        this.scratch_vec(associated_count)?;
                    for path in prepared.associated_paths().take(associated_count) {
                        associated_value_handles.push(
                            cx.request(associated_constant_graph::member_use_key(
                                ASSOCIATED_CONST_VALUE,
                                &caller,
                                path,
                                &this.0.budget,
                            )?)
                            .get(0),
                        );
                    }
                    let (mut values, _values_storage) =
                        this.scratch_vec::<Arc<const_check::ValueFact>>(constant_handles.len())?;
                    for handle in constant_handles {
                        values.push(cx.read_fact(handle).await?);
                    }
                    let (mut associated_values, _associated_values_storage) =
                        this.scratch_vec(associated_count)?;
                    for handle in associated_value_handles {
                        associated_values.push(cx.read_fact(handle).await?);
                    }
                    let constants = types
                        .seal_body_constants_with_associated(
                            prepared,
                            values,
                            associated_values,
                            &this.0.budget,
                        )
                        .map_err(|error| source_fact_error(error, &this.0.budget))?;
                    let closed_source = !generic
                        && matches!(
                            types.callable_provider(declaration).map_err(Error::new)?,
                            frontend::CallableProvider::Source { .. }
                        )
                        && !matches!(
                            types
                                .function_schema(declaration)
                                .map_err(Error::new)?
                                .signature
                                .result,
                            ir::Ty::Opaque(_)
                        )
                        && types
                            .callable_context(declaration)
                            .map_err(Error::new)?
                            .self_ty
                            .is_none();
                    let (body, closures, hidden) = if closed_source {
                        nominal_realization::publish_callable(
                            &cx,
                            &types,
                            declaration,
                            constants,
                            &this.0.budget,
                        )
                        .await?
                    } else {
                        types
                            .publish_callable_body_outputs(declaration, constants, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?
                    };
                    let fact = match body {
                        frontend::PublishedCallableBody::Source(publication) => {
                            if !publication
                                .facts()
                                .is_source_callable(declaration)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?
                            {
                                return Err(Error::new("Source publication has a Recipe provider"));
                            }
                            publication
                                .validate_source(
                                    publication.source(),
                                    publication.facts(),
                                    &this.0.budget,
                                )
                                .map_err(|error| source_fact_error(error, &this.0.budget))?;
                            TemplateBodyFact {
                                checked: publication.source().clone(),
                                closure: None,
                                closure_mode: None,
                                publication: Some(publication),
                            }
                        }
                        frontend::PublishedCallableBody::Recipe(checked) => {
                            let callable = types.callable_facts(declaration).map_err(Error::new)?;
                            if callable
                                .is_source_callable(declaration)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?
                            {
                                return Err(Error::new(
                                    "Source provider omitted canonical publication",
                                ));
                            }
                            TemplateBodyFact {
                                checked,
                                closure: None,
                                closure_mode: None,
                                publication: None,
                            }
                        }
                    };
                    let closure_publication = fact
                        .publication
                        .as_ref()
                        .map(|publication| {
                            frontend::closures::publication::ClosurePublicationProof::capture(
                                &types,
                                publication,
                                &closures,
                                &this.0.budget,
                            )
                        })
                        .transpose()
                        .map_err(Error::new)?;
                    if !closures.is_empty() && closure_publication.is_none() {
                        return Err(Error::new("closures require finalized parent publication"));
                    }
                    this.0
                        .budget
                        .retain_bytes(
                            closures
                                .len()
                                .checked_mul(std::mem::size_of::<
                                    frontend::closures::publication::PublishedClosure,
                                >())
                                .ok_or_else(|| {
                                    Error::new("closure publication capacity overflow")
                                })?,
                        )
                        .map_err(Error::new)?;
                    let mut published_closures = Vec::with_capacity(closures.len());
                    for closure in &closures {
                        published_closures.push(
                            frontend::closures::publication::PublishedClosure::capture(
                                &types, closure,
                            )
                            .map_err(Error::new)?,
                        );
                    }
                    cx.publish_fact(
                        1,
                        ClosureBodiesFact {
                            bodies: published_closures.into(),
                            publication: closure_publication,
                        },
                    )?;
                    cx.publish_fact(
                        2,
                        OpaqueHiddenTypesFact {
                            representations: hidden.into(),
                        },
                    )?;
                    cx.publish_fact(0, fact)
                }
                UNIVERSAL_LOWER => {
                    let (index, _) = closure_body_index(&key)?;
                    let types = facts.types().await?;
                    let template: Arc<TemplateBodyFact> = facts
                        .get(keys::words(TEMPLATE_BODY, key.params.clone())?)
                        .await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let callable = if let Some(closure) = &template.closure {
                        types
                            .trait_world()
                            .map_err(Error::new)?
                            .closure_callable_facts_for(
                                closure.ty.clone(),
                                template.closure_mode.ok_or_else(|| {
                                    Error::new("closure template has no capability")
                                })?,
                            )
                            .map_err(|e| source_fact_error(e, &this.0.budget))?
                    } else {
                        types.callable_facts(declaration).map_err(Error::new)?
                    };
                    if !Self::universal_body(&types, declaration)?
                        && !template.publication.as_ref().is_some_and(|publication| {
                            publication.facts().phase() == traits::FactsPhase::Universal
                        })
                    {
                        return Err(Error::new(
                            "universal lowering requires an abstract source publication or registered recipe",
                        ));
                    }
                    let source = this.validated_body(&types, index, &template)?;
                    let lowered = if callable
                        .is_source_callable(declaration)
                        .map_err(|error| source_fact_error(error, &this.0.budget))?
                    {
                        cfg::lower_source_template(
                            template.publication.as_ref().ok_or_else(|| {
                                Error::new("missing canonical universal Source publication")
                            })?,
                            &this.0.budget,
                        )
                        .map_err(|error| source_fact_error(error, &this.0.budget))?
                    } else if let Some(closure) = &template.closure {
                        cfg::lower_closure_template(
                            source,
                            callable,
                            closure,
                            template.closure_mode.ok_or_else(|| {
                                Error::new("closure lowering lacks canonical mode")
                            })?,
                            &this.0.budget,
                        )
                        .map_err(Error::new)?
                    } else {
                        cfg::lower_template(source, callable, &this.0.budget).map_err(Error::new)?
                    };
                    cx.publish_fact(0, lowered)
                }
                UNIVERSAL_ANALYZE => {
                    let (index, _) = closure_body_index(&key)?;
                    let lower_key = keys::words(UNIVERSAL_LOWER, key.params.clone())?;
                    let lowered_handle = cx.request(lower_key).get(0);
                    let types_handle = cx.request(keys::SignaturesDebtRoot.key()).get(0);
                    let lowered: Arc<cfg::TemplateCfg> = cx.read_fact(lowered_handle).await?;
                    let template_name = lowered.source().name.clone();
                    let types: Arc<frontend::PlanCtx> = cx.read_fact(types_handle).await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let const_context = key.params.len() == 1
                        && types.callable_constness(declaration).map_err(Error::new)?
                            == frontend::Constness::Const;
                    match ownership::verify_template_in_const_context(
                        lowered.as_ref().clone(),
                        const_context,
                    ) {
                        Ok(verified) => {
                            cx.publish_fact(ANALYSIS_OUTPUT, verified.analysis().clone())?;
                            cx.publish_fact(0, UniversalProofFact { verified })
                        }
                        Err(analysis) => {
                            let message = analysis_diagnostic(
                                &this.0.budget,
                                "universal generic declaration",
                                index,
                                &analysis,
                            );
                            let message = format!("{template_name}: {message}");
                            cx.publish_fact(ANALYSIS_OUTPUT, Arc::new(analysis))?;
                            Err(Error::new(message))
                        }
                    }
                }
                UNIVERSAL_EFFECTS_READY => {
                    let (index, _) = closure_body_index(&key)?;
                    let types = facts.types().await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let template = facts.template(index).await?;
                    if !Self::universal_body(&types, declaration)?
                        && !template.publication.as_ref().is_some_and(|publication| {
                            publication.facts().phase() == traits::FactsPhase::Universal
                        })
                    {
                        return Err(Error::new("non-universal callable has no admission fact"));
                    }
                    let proof: Arc<UniversalProofFact> = facts
                        .get(keys::words(UNIVERSAL_ANALYZE, key.params.clone())?)
                        .await?;
                    let mut callee_indices = Vec::new();
                    for expression in &proof.verified.source().exprs {
                        this.0.budget.charge_work(1).map_err(Error::new)?;
                        let ir::ExprKind::Call { target, .. } = &expression.kind else {
                            continue;
                        };
                        let actuals = match target {
                            ir::Callee::Direct(target) => target.type_args.as_ref(),
                            ir::Callee::TraitItem(call)
                            | ir::Callee::Virtual(ir::VirtualCall { call })
                            | ir::Callee::Untupled(call) => call.method_args.as_ref(),
                        };
                        this.0
                            .budget
                            .charge_work(actuals.len())
                            .map_err(Error::new)?;
                        if !actuals.iter().any(|ty| matches!(ty, ir::Ty::Ref { .. })) {
                            continue;
                        }
                        let ir::Callee::Direct(target) = target else {
                            return Err(Error::new(
                                "unsupported abstract method reference-actual body admission",
                            ));
                        };
                        if !Self::universal_body(&types, target.declaration)? {
                            return Err(Error::new(
                                "reference-actual call has no canonical universal body",
                            ));
                        }
                        let callee = types
                            .callable_index(target.declaration)
                            .map_err(Error::new)?;
                        if callee != index {
                            callee_indices.push((callee, target.clone()));
                        }
                    }
                    let templates = facts.callee_templates(&types, callee_indices).await?;
                    ownership::CarrierEffectTable::admit_reference_actual_calls(
                        &proof.verified,
                        &templates,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish(0, 1)
                }
                INSTANCE => {
                    let types = facts.types().await?;
                    let (target, source_index) = facts.instance_of(&key, &this.0.budget).await?;
                    let template = facts.template(source_index).await?;
                    let source = this.validated_body(&types, source_index, &template)?;
                    let mut context = types.as_ref().clone();
                    let mut demands = instances::direct_demands(
                        &target,
                        &source,
                        &mut context,
                        &this.0.budget,
                        Some(cx.authority()),
                    )
                    .map_err(|error| {
                        Error::new(format!("{error}; instance caller={:?}", target.declaration))
                    })?;
                    opaque_dispatch::extend_instance_demands(
                        &this,
                        &cx,
                        &target,
                        &template,
                        &types,
                        &mut demands,
                    )
                    .await?;
                    cx.publish_fact(0, demands)
                }
                OPAQUE_REPRESENTATION => {
                    let types = facts.types().await?;
                    let opaque =
                        opaque_instances::decode(&facts, &key.params, &types, &this.0.budget)
                            .await?;
                    let source = types.callable_index(opaque.owner()).map_err(Error::new)?;
                    let request = cx.request(function(TEMPLATE_BODY, source));
                    let template_handle = request.get(0);
                    let hidden_handle = request.get(2);
                    let template: Arc<TemplateBodyFact> = cx.read_fact(template_handle).await?;
                    let hidden: Arc<OpaqueHiddenTypesFact> = cx.read_fact(hidden_handle).await?;
                    this.validated_body(&types, source, &template)?;
                    let publication = template.publication.as_ref().ok_or_else(|| {
                        Error::new(
                            "opaque representation requires its canonical defining publication",
                        )
                    })?;
                    let mut found = None;
                    for record in hidden.representations.iter() {
                        if record.opaque().owner() == opaque.owner()
                            && record.opaque().site() == opaque.site()
                            && found.replace(record).is_some()
                        {
                            return Err(Error::new(
                                "opaque body published duplicate hidden occurrences",
                            ));
                        }
                    }
                    let record = found.ok_or_else(|| {
                        Error::new("unsupported: opaque return has no final hidden representation")
                    })?;
                    let definition = ir::OpaqueDefinitionProof::seal(
                        publication.clone(),
                        record,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    let mut context = types.as_ref().clone();
                    let record = definition
                        .instantiate(&opaque, publication, &mut context, &this.0.budget)
                        .map_err(Error::new)?;
                    cx.publish_fact(0, OpaqueRepresentationFact { record, definition })
                }
                BODY => {
                    // BODY is the instance-keyed alias of exactly one canonical
                    // target-keyed producer.
                    let (target, _) = facts.instance_of(&key, &this.0.budget).await?;
                    let params = instances::graph_key::encode(&target, &this.0.budget)
                        .map_err(Error::new)?;
                    let canonical: Arc<InstanceBodyFact> =
                        facts.get(keys::words(CANONICAL_BODY, params)?).await?;
                    canonical.validate_current_engine(cx.authority())?;
                    cx.publish_fact(
                        0,
                        InstanceBodyFact {
                            facts: canonical.facts.clone(),
                            checked: canonical.checked.clone(),
                        },
                    )
                }
                CANONICAL_BODY => {
                    let types = facts.types().await?;
                    let instance_target = facts
                        .decode_instance(&key.params, &types, &this.0.budget)
                        .await?;
                    let instance_source = types
                        .callable_index(instance_target.declaration)
                        .map_err(Error::new)?;
                    let mut template = facts.template(instance_source).await?;
                    let mut proof_key = function(UNIVERSAL_ANALYZE, instance_source);
                    if let Some(closure) = &template.closure {
                        types
                            .validate_closure_instance(closure, &instance_target)
                            .map_err(Error::new)?;
                        let mode = types
                            .closure_instance_mode(&instance_target)
                            .map_err(Error::new)?;
                        proof_key = closure_body_key(
                            UNIVERSAL_ANALYZE,
                            instance_source,
                            closure.ty.kind(),
                            mode,
                        );
                        let template_key = closure_body_key(
                            TEMPLATE_BODY,
                            instance_source,
                            closure.ty.kind(),
                            mode,
                        );
                        template = facts.get(template_key).await?;
                    }
                    let source = this.validated_body(&types, instance_source, &template)?;
                    let fact = if !Self::universal_body(&types, instance_target.declaration)? {
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("nongeneric Source has no original publication")
                        })?;
                        let closed = publication.facts();
                        if closed.phase() != traits::FactsPhase::Closed
                            || closed.target() != Some(&instance_target)
                            || !closed
                                .is_source_callable(instance_target.declaration)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?
                        {
                            return Err(Error::new(
                                "nongeneric Source publication changed its closed target",
                            ));
                        }
                        publication
                            .validate_source(&source, closed, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        InstanceBodyFact {
                            facts: closed.clone(),
                            checked: source,
                        }
                    } else {
                        let closed = types.instance_facts(&instance_target).map_err(Error::new)?;
                        let mut context = types.as_ref().clone();
                        let proof_handle = cx.request(proof_key).get(0);
                        let count = source
                            .exprs
                            .iter()
                            .filter(|expression| {
                                matches!(expression.kind, ir::ExprKind::AssociatedConst(_))
                            })
                            .count();
                        let (mut constant_handles, _constant_handles_storage) =
                            this.scratch_vec(count)?;
                        for expression in &source.exprs {
                            this.0.budget.charge_work(1).map_err(Error::new)?;
                            if let ir::ExprKind::AssociatedConst(operand) = &expression.kind {
                                constant_handles.push(
                                    cx.request(associated_constant_graph::member_use_key(
                                        ASSOCIATED_CONST_VALUE,
                                        &instance_target,
                                        operand.path(),
                                        &this.0.budget,
                                    )?)
                                    .get(0),
                                );
                            }
                        }
                        let proof: Arc<UniversalProofFact> = cx.read_fact(proof_handle).await?;
                        let (mut constant_values, _constant_values_storage) =
                            this.scratch_vec(count)?;
                        for handle in constant_handles {
                            let value: Arc<associated_constant_graph::AssociatedConstantValueFact> =
                                cx.read_fact(handle).await?;
                            value.validate(
                                &types,
                                &instance_target,
                                value.authorization.path(),
                                &this.0.budget,
                            )?;
                            constant_values.push(value);
                        }
                        let verified = &proof.verified;
                        if closed
                            .is_source_callable(instance_target.declaration)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?
                        {
                            verified
                                .function()
                                .validate_source_publication(
                                    template.publication.as_ref().ok_or_else(|| {
                                        Error::new(
                                            "generic Source template omitted canonical publication",
                                        )
                                    })?,
                                    &this.0.budget,
                                )
                                .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        }
                        let checked = Arc::new(
                            nominal_realization::substitution::instantiate_checked_with_constants(
                                verified,
                                &instance_target.type_args,
                                &closed,
                                &mut context,
                                &this.0.budget,
                                &constant_values,
                                cx.authority(),
                            )
                            .map_err(Error::new)?,
                        );
                        InstanceBodyFact {
                            facts: closed,
                            checked,
                        }
                    };
                    cx.publish_fact(0, fact)
                }
                ROOT_CLEANUP_DEMAND => {
                    let index = item_index(&key, "missing closed root index")?;
                    let types = facts.types().await?;
                    let (target, source) = root_instance(&types, index)?.ok_or_else(|| {
                        Error::new("cleanup demand is restricted to closed roots")
                    })?;
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
                    body.validate_current_engine(cx.authority())?;
                    let template = facts.template(source).await?;
                    let publication = template
                        .publication
                        .as_ref()
                        .ok_or_else(|| Error::new("cleanup demand omitted Source publication"))?;
                    let demand = root_cleanup::RootCleanupDemand::derive(
                        &target,
                        &body,
                        publication,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish_fact(0, demand)
                }
                LOWER => {
                    let types = facts.types().await?;
                    let (instance_target, instance_source) =
                        facts.instance_of(&key, &this.0.budget).await?;
                    let body: Arc<InstanceBodyFact> = facts.get(sibling(BODY)?).await?;
                    body.validate_current_engine(cx.authority())?;
                    let lowered = if !Self::universal_body(&types, instance_target.declaration)? {
                        let template = facts.template(instance_source).await?;
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("closed Source lowering has no canonical publication")
                        })?;
                        publication
                            .validate_source(&body.checked, &body.facts, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        let lowered: Arc<Arc<cfg::Function>> = cx
                            .read_item(
                                &keys::Item::<SOURCE_ADMISSION>(instance_source),
                                SOURCE_CFG_OUTPUT,
                            )
                            .await?;
                        lowered
                            .validate_source_publication(publication, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        lowered.as_ref().clone()
                    } else {
                        Arc::new(
                            cfg::lower_with_facts(&body.checked, &body.facts, &this.0.budget)
                                .map_err(Error::new)?,
                        )
                    };
                    cx.publish_fact(0, lowered)
                }
                ANALYZE => {
                    let types = facts.types().await?;
                    let (instance_target, instance_source) =
                        facts.instance_of(&key, &this.0.budget).await?;
                    let lowered: Arc<Arc<cfg::Function>> = facts.get(sibling(LOWER)?).await?;
                    let lowered = lowered.as_ref().clone();
                    let body: Arc<InstanceBodyFact> = facts.get(sibling(BODY)?).await?;
                    body.validate_current_engine(cx.authority())?;
                    scale_task_work_by_cfg(&lowered);
                    let identity = instance_identity(&key, instance_source, &lowered.name);
                    if has_drop(&lowered) {
                        if let Some(index) = key_root_slot(&key) {
                            let demand: Arc<root_cleanup::RootCleanupDemand> =
                                facts.get(function(ROOT_CLEANUP_DEMAND, index)).await?;
                            demand
                                .validate(&instance_target, &lowered, &this.0.budget)
                                .map_err(Error::new)?;
                        } else {
                            let demands: Arc<instances::DirectDemands> =
                                facts.get(sibling(INSTANCE)?).await?;
                            demands
                                .validate_cleanup(&lowered, &this.0.budget)
                                .map_err(Error::new)?;
                        }
                    }
                    if !Self::universal_body(&types, instance_target.declaration)? {
                        let template = facts.template(instance_source).await?;
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("closed source proof omitted canonical publication")
                        })?;
                        let proof: Arc<Option<ClosedSourceProof>> = cx
                            .read_item(
                                &keys::Item::<SOURCE_ADMISSION>(instance_source),
                                SOURCE_CLOSED_PROOF_OUTPUT,
                            )
                            .await?;
                        let proof = proof.as_ref().as_ref().ok_or_else(|| {
                            Error::new("closed source admission omitted successful proof")
                        })?;
                        let const_context = types
                            .callable_constness(instance_target.declaration)
                            .map_err(Error::new)?
                            == frontend::Constness::Const;
                        let concrete = proof.reuse(
                            publication,
                            &body,
                            &instance_target,
                            &lowered,
                            const_context,
                            &this.0.budget,
                        )?;
                        cx.publish_fact(ANALYSIS_OUTPUT, concrete.analysis().clone())?;
                        return cx.publish_fact(0, ConcreteProofFact { concrete });
                    }
                    let closed = cfg::close(lowered, body.facts.clone(), &this.0.budget)
                        .map_err(Error::new)?;
                    // Read only the universal proofs of this instance's own
                    // direct callees, instead of a program-wide effects join.
                    // A declaration absent from the table conservatively uses
                    // its envelope, so every universal callee is included.
                    let mut callee_indices = Vec::new();
                    for block in &closed.function().blocks {
                        this.0.budget.charge_work(1).map_err(Error::new)?;
                        if let cfg::Terminator::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } = &block.terminator.value
                            && Self::universal_body(&types, target.declaration)?
                        {
                            let callee = types
                                .callable_index(target.declaration)
                                .map_err(Error::new)?;
                            callee_indices.push((callee, target.clone()));
                        }
                    }
                    let templates = facts.callee_templates(&types, callee_indices).await?;
                    let effects =
                        ownership::CarrierEffectTable::for_callees(&templates, &this.0.budget)
                            .map_err(Error::new)?;
                    let const_context = types
                        .callable_constness(instance_target.declaration)
                        .map_err(Error::new)?
                        == frontend::Constness::Const;
                    match ownership::analyze_closed_with_effects_in_const_context(
                        closed,
                        &effects,
                        const_context,
                    ) {
                        Ok(concrete) => {
                            cx.publish_fact(ANALYSIS_OUTPUT, concrete.analysis().clone())?;
                            cx.publish_fact(0, ConcreteProofFact { concrete })
                        }
                        Err(analysis) => {
                            let message = analysis_diagnostic(
                                &this.0.budget,
                                "instance",
                                &identity,
                                &analysis,
                            );
                            cx.publish_fact(ANALYSIS_OUTPUT, Arc::new(analysis))?;
                            Err(Error::new(message))
                        }
                    }
                }
                OBJECT => {
                    if let Some(object) = opaque_object::produce(&this, &cx, &key).await? {
                        return cx.publish_fact(0, object);
                    }
                    // Only this instance's own proof is required. Executable
                    // publication is gated on every check by BUILD (via CHECK),
                    // so object code for a ready function overlaps unrelated
                    // checking (ARCHITECTURE.md section 17).
                    let proof: Arc<ConcreteProofFact> = facts.get(sibling(ANALYZE)?).await?;
                    let root = key_root_slot(&key);
                    if has_drop(proof.concrete.function()) {
                        let types = facts.types().await?;
                        if let Some((target, _)) = root
                            .map(|index| root_instance(&types, index))
                            .transpose()?
                            .flatten()
                        {
                            let demand: Arc<root_cleanup::RootCleanupDemand> =
                                facts.get(sibling(ROOT_CLEANUP_DEMAND)?).await?;
                            demand
                                .validate(&target, proof.concrete.function(), &this.0.budget)
                                .map_err(Error::new)?;
                        } else {
                            let demands: Arc<instances::DirectDemands> =
                                facts.get(sibling(INSTANCE)?).await?;
                            demands
                                .validate_cleanup(proof.concrete.function(), &this.0.budget)
                                .map_err(Error::new)?;
                        }
                    }
                    // Callee symbols derive from targets alone. BUILD links
                    // against the sealed plan, which rejects any unplanned
                    // symbol, so this needs no whole-program read.
                    let types = facts.types().await?;
                    let c_boundary = if let Some((target, _)) = root
                        .map(|index| root_instance(&types, index))
                        .transpose()?
                        .flatten()
                    {
                        types
                            .c_abi_for(target.declaration)
                            .map_err(Error::new)?
                            .map(|(abi, signature)| (target, abi.imported, abi.exported, signature))
                    } else {
                        None
                    };
                    if let Some((target, true, _, signature)) = &c_boundary {
                        let symbol = types.c_abi_symbol(target.declaration).map_err(Error::new)?;
                        let object = c_abi::import(signature, &symbol, &this.0.budget)
                            .map_err(Error::new)?;
                        return cx.publish_fact(0, object);
                    }
                    if this.0.source_options.opt != ssa::OptMode::Off && c_boundary.is_none() {
                        let optimized: Arc<SsaFact> = facts.get(sibling(IPO_OPT)?).await?;
                        if let Some(lowered) = &optimized.lowered {
                            let meter = ssa::meter::Meter::new(&this.0.budget);
                            let types = lowered
                                .verify(&meter)
                                .map_err(|error| Error::new(error.to_string()))?;
                            let (object, _stats) =
                                ssa::machine::emit(lowered, &types, &meter).map_err(Error::new)?;
                            drop(types);
                            // `object_bytes` already covers the Object value,
                            // its Arc, and the Arc<Receipt>; all of it was
                            // admitted before the receipt Arc is allocated.
                            let (mut object, receipt) = object.seal(&meter).map_err(Error::new)?;
                            object.receipt = Some(Arc::new(receipt));
                            return cx.publish_fact(0, object);
                        }
                    }
                    let mut object = backend::emit_cfg_function_with_overflow(
                        &proof.concrete,
                        &|target| instances::symbol_of(target, &types, &this.0.budget),
                        this.0.source_options.overflow_checks,
                    )
                    .map_err(Error::new)?;
                    if let Some((target, false, true, signature)) = c_boundary {
                        let private = instances::symbol_of(&target, &types, &this.0.budget)
                            .map_err(Error::new)?;
                        let symbol = types.c_abi_symbol(target.declaration).map_err(Error::new)?;
                        let wrapper = c_abi::export(&signature, &private, &this.0.budget)
                            .map_err(Error::new)?;
                        c_abi::attach_export(&mut object, wrapper, symbol).map_err(Error::new)?;
                    }
                    cx.publish_fact(0, object)
                }
                SSA => {
                    let proof: Arc<ConcreteProofFact> = facts.get(sibling(ANALYZE)?).await?;
                    let types = facts.types().await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    if let Some((target, _)) = key_root_slot(&key)
                        .map(|index| root_instance(&types, index))
                        .transpose()?
                        .flatten()
                        && types
                            .c_abi_for(target.declaration)
                            .map_err(Error::new)?
                            .is_some_and(|(abi, _)| abi.imported)
                    {
                        return cx.publish_fact(
                            0,
                            SsaFact::unsupported(
                                "external C declaration has no Rust SSA body",
                                &meter,
                            )?,
                        );
                    }
                    let function = proof.concrete.function();
                    let fact = match ssa::lower::lower_with_overflow(
                        function,
                        &|target| instances::symbol_of(target, &types, &this.0.budget),
                        &meter,
                        this.0.source_options.overflow_checks,
                    )
                    .map_err(Error::new)?
                    {
                        ssa::lower::Lowering::Lowered(lowered) => {
                            SsaFact::publish(*lowered, 0, &meter)?
                        }
                        ssa::lower::Lowering::Unsupported(reason) => {
                            SsaFact::unsupported(reason, &meter)?
                        }
                    };
                    cx.publish_fact(0, fact)
                }
                LOCAL_OPT => {
                    let input: Arc<SsaFact> = facts.get(sibling(SSA)?).await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    let fact = match &input.lowered {
                        Some(lowered) => SsaFact::publish(local_opt(lowered, &meter)?, 0, &meter)?,
                        None => {
                            SsaFact::unsupported(input.unsupported.unwrap_or("unknown"), &meter)?
                        }
                    };
                    cx.publish_fact(0, fact)
                }
                SUMMARY => {
                    let body: Arc<SsaFact> = facts.get(sibling(LOCAL_OPT)?).await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    // Summary is Copy and owns no heap storage; its fact
                    // envelope is admitted before publication and kept as
                    // the fact's own receipt.
                    let summary = match body.lowered.as_deref() {
                        Some(lowered) => {
                            Some(ssa::inline::Summary::of(lowered, &meter).map_err(Error::new)?)
                        }
                        None => None,
                    };
                    let receipt = meter
                        .lease(envelope::<SummaryFact>())
                        .and_then(|lease| lease.seal(&meter))
                        .map_err(Error::new)?;
                    cx.publish_fact(
                        0,
                        SummaryFact {
                            summary,
                            _receipt: receipt,
                        },
                    )
                }
                IPO_OPT => {
                    let body: Arc<SsaFact> = facts.get(sibling(LOCAL_OPT)?).await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    let Some(lowered) = body.lowered.clone() else {
                        return cx.publish_fact(
                            0,
                            SsaFact::unsupported(body.unsupported.unwrap_or("unknown"), &meter)?,
                        );
                    };
                    // Callers read only their own direct callees' summaries,
                    // requested together, then the bodies they inline. Each
                    // inlined body is a recorded edge (OPTIMIZATION.md 8.3).
                    // Every frontier buffer below is paid scratch, admitted at
                    // its exact capacity and held across the awaits that use it.
                    let types = facts.types().await?;
                    let count = lowered.callees.len();
                    let mut slots = meter.list::<Option<Vec<u64>>>(count).map_err(Error::new)?;
                    for callee in &lowered.callees {
                        meter.work(1).map_err(Error::new)?;
                        slots
                            .push(instance_params(&types, &callee.target, &this.0.budget).ok())
                            .map_err(Error::new)?;
                    }
                    let mut summary_handles = meter.list(count).map_err(Error::new)?;
                    for slot in slots.iter() {
                        let handle = match slot.as_ref().filter(|slot| **slot != key.params) {
                            Some(slot) => {
                                Some(cx.request(keys::words(SUMMARY, slot.clone())?).get(0))
                            }
                            None => None,
                        };
                        summary_handles.push(handle).map_err(Error::new)?;
                    }
                    let mut summaries = meter
                        .table::<Option<ssa::inline::Summary>>(count, None)
                        .map_err(Error::new)?;
                    for (slot, handle) in summaries.iter_mut().zip(summary_handles.iter()) {
                        if let Some(handle) = handle {
                            let summary: Arc<SummaryFact> = cx.read_fact(handle.clone()).await?;
                            *slot = summary.summary;
                        }
                    }
                    drop(summary_handles);
                    let chosen =
                        ssa::inline::candidates(&lowered, &|callee| summaries[callee], &meter)
                            .map_err(Error::new)?;
                    drop(summaries);
                    if chosen.is_empty() {
                        // The LOCAL_OPT body is shared, not copied: this fact
                        // re-publishes the same Arc and admits only its own
                        // envelope.
                        return cx.publish_fact(0, SsaFact::share(lowered, &meter)?);
                    }
                    let mut body_handles = meter.list(chosen.len()).map_err(Error::new)?;
                    for callee in chosen.iter() {
                        let slot = slots[*callee]
                            .clone()
                            .ok_or_else(|| Error::new("chosen callee has no slot"))?;
                        body_handles
                            .push(cx.request(keys::words(LOCAL_OPT, slot)?).get(0))
                            .map_err(Error::new)?;
                    }
                    let mut bodies = meter
                        .list::<Arc<SsaFact>>(chosen.len())
                        .map_err(Error::new)?;
                    for handle in body_handles.iter() {
                        bodies
                            .push(cx.read_fact(handle.clone()).await?)
                            .map_err(Error::new)?;
                    }
                    drop(body_handles);
                    let mut pairs = meter
                        .list::<(usize, &ssa::lower::Lowered)>(chosen.len())
                        .map_err(Error::new)?;
                    for (callee, body) in chosen.iter().zip(bodies.iter()) {
                        if let Some(b) = body.lowered.as_deref() {
                            pairs.push((*callee, b)).map_err(Error::new)?;
                        }
                    }
                    let inlined = pairs.len();
                    // Peak: the merged body and the optimized result are both
                    // live and both paid until `merged` drops after local_opt.
                    let merged =
                        ssa::inline::inline(&lowered, &pairs, &meter).map_err(Error::new)?;
                    drop(pairs);
                    let optimized = local_opt(&merged.value, &meter)?;
                    drop(merged);
                    cx.publish_fact(0, SsaFact::publish(optimized, inlined, &meter)?)
                }
                SOURCE_ADMISSION => {
                    let index = item_index(&key, "missing source admission declaration")?;
                    let types = facts.types().await?;
                    let template = facts.template(index).await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let abstract_source =
                        template.publication.as_ref().is_some_and(|publication| {
                            publication.facts().phase() == traits::FactsPhase::Universal
                        });
                    if abstract_source || Self::universal_body(&types, declaration)? {
                        cx.publish_fact(SOURCE_CLOSED_PROOF_OUTPUT, None::<ClosedSourceProof>)?;
                        let effects = cx.request(function(UNIVERSAL_EFFECTS_READY, index)).get(0);
                        let proof = cx
                            .request(function(UNIVERSAL_ANALYZE, index))
                            .get(ANALYSIS_OUTPUT);
                        let lowered = cx.request(function(UNIVERSAL_LOWER, index)).get(0);
                        cx.read(effects).await?;
                        let analysis: Arc<Arc<ownership::Analysis>> = cx.read_fact(proof).await?;
                        let lowered: Arc<cfg::TemplateCfg> = cx.read_fact(lowered).await?;
                        cx.publish_fact(ANALYSIS_OUTPUT, analysis.as_ref().clone())?;
                        cx.publish_fact(SOURCE_CFG_OUTPUT, lowered.function().clone())?;
                    } else {
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("closed source admission omitted its canonical publication")
                        })?;
                        // Declare this node's own input before lowering charges it.
                        scale_task_work_by_callable(&types, declaration);
                        let lowered = cfg::lower_source_closed(publication, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        scale_task_work_by_cfg(&lowered);
                        let lowered = Arc::new(lowered);
                        cx.publish_fact(SOURCE_CFG_OUTPUT, lowered.clone())?;
                        let closed =
                            cfg::close(lowered, publication.facts().clone(), &this.0.budget)
                                .map_err(Error::new)?;
                        let mut callees = Vec::new();
                        for block in &closed.function().blocks {
                            this.0.budget.charge_work(1).map_err(Error::new)?;
                            if let cfg::Terminator::Call {
                                target: ir::Callee::Direct(target),
                                ..
                            } = &block.terminator.value
                                && Self::universal_body(&types, target.declaration)?
                            {
                                callees.push(
                                    types
                                        .callable_index(target.declaration)
                                        .map_err(Error::new)?,
                                );
                            }
                        }
                        callees.sort_unstable();
                        callees.dedup();
                        let handles: Vec<_> = callees
                            .iter()
                            .map(|callee| cx.request(function(UNIVERSAL_ANALYZE, *callee)).get(0))
                            .collect();
                        let mut proofs = Vec::with_capacity(handles.len());
                        for handle in handles {
                            let proof: Arc<UniversalProofFact> = cx.read_fact(handle).await?;
                            proofs.push(proof.verified.clone());
                        }
                        let effects =
                            ownership::CarrierEffectTable::for_callees(&proofs, &this.0.budget)
                                .map_err(Error::new)?;
                        let const_context =
                            types.callable_constness(declaration).map_err(Error::new)?
                                == frontend::Constness::Const;
                        match ownership::analyze_closed_with_effects_in_const_context(
                            closed,
                            &effects,
                            const_context,
                        ) {
                            Ok(concrete) => {
                                cx.publish_fact(ANALYSIS_OUTPUT, concrete.analysis().clone())?;
                                this.retain(envelope::<Option<ClosedSourceProof>>())?;
                                cx.publish_fact(
                                    SOURCE_CLOSED_PROOF_OUTPUT,
                                    Some(ClosedSourceProof {
                                        concrete,
                                        publication: publication.clone(),
                                        const_context,
                                    }),
                                )?;
                            }
                            Err(analysis) => {
                                let message = analysis_diagnostic(
                                    &this.0.budget,
                                    "source declaration",
                                    index,
                                    &analysis,
                                );
                                cx.publish_fact(ANALYSIS_OUTPUT, Arc::new(analysis))?;
                                return Err(Error::new(message));
                            }
                        }
                    }
                    cx.publish(0, 1)
                }
                ITEM_INVENTORY => {
                    let ordinal = item_index(&key, "missing module inventory ordinal")?;
                    let types = facts.types().await?;
                    let items = types.module_inventory(ordinal).map_err(Error::new)?;
                    let mut universal = Vec::new();
                    for &index in &items.callables {
                        let declaration = types.callable_decl(index).map_err(Error::new)?;
                        if Self::universal_body(&types, declaration)? {
                            universal.push(index);
                        }
                    }
                    cx.publish_fact(
                        0,
                        ItemInventoryFact {
                            callables: items.callables.clone(),
                            universal,
                            constants: items.constants.clone(),
                            nominals: items.nominals.clone(),
                            associated_constants: items.associated_constants.clone(),
                            impls: items.impls.clone(),
                        },
                    )
                }
                CHECK | BUILD => {
                    // Demand independently of signatures, then consume only at
                    // the success boundary. The source graph owns this evidence.
                    let runtime = this.0.source_options.source_snapshot.as_ref().map(|_| {
                        cx.request(keys::ResolveDebtRoot.key())
                            .get(SOURCE_ACCOUNTING_OUTPUT)
                    });
                    // Closed source roots are keyed by callable index;
                    // BUILD discovers the generic frontier per instance.
                    let types = facts.types().await?;
                    // Item checks fan out through per-module inventory facts,
                    // never a scan of whole-program item counts.
                    let inventory_keys: Vec<keys::Item<ITEM_INVENTORY>> = (0..types
                        .module_inventory_count())
                        .map(keys::Item)
                        .collect();
                    let inventory_handles = cx.request_all(&inventory_keys, 0);
                    let mut inventories = Vec::with_capacity(inventory_handles.len());
                    for handle in inventory_handles {
                        let inventory: Arc<ItemInventoryFact> = cx.read_fact(handle).await?;
                        inventories.push(inventory);
                    }
                    let (mut constant_keys, mut admission_keys, mut nominal_keys) =
                        (Vec::new(), Vec::new(), Vec::new());
                    let mut associated = Vec::new();
                    for inventory in &inventories {
                        this.0
                            .budget
                            .charge_work(
                                inventory.constants.len()
                                    + inventory.callables.len()
                                    + inventory.nominals.len()
                                    + inventory.associated_constants.len(),
                            )
                            .map_err(Error::new)?;
                        constant_keys.extend(
                            inventory
                                .constants
                                .iter()
                                .map(|d| keys::Item::<CONST_VALUE>(d.index as usize)),
                        );
                        admission_keys.extend(
                            inventory
                                .callables
                                .iter()
                                .map(|&index| keys::Item::<SOURCE_ADMISSION>(index)),
                        );
                        nominal_keys.extend(inventory.nominals.iter().map(|d| {
                            keys::Params::<NOMINAL_DECL_WELL_FORMED>(
                                d.snapshot.graph_word(),
                                vec![u64::from(d.index)],
                            )
                        }));
                        associated.extend(inventory.associated_constants.iter().copied());
                    }
                    let associated_keys: Vec<keys::Params<ASSOCIATED_CONST_HEADER>> = associated
                        .iter()
                        .map(|d| keys::Params(d.snapshot.graph_word(), vec![u64::from(d.index)]))
                        .collect();
                    let constant_handles = cx.request_all(&constant_keys, 0);
                    let associated_handles = cx.request_all(&associated_keys, 1);
                    let nominal_handles = cx.request_all(&nominal_keys, 0);
                    if key.kind == CHECK {
                        // SOURCE_ADMISSION itself demands UNIVERSAL_EFFECTS_READY
                        // for exactly the universal callables.
                        let admission_handles = cx.request_all(&admission_keys, 0);
                        let universal_keys: Vec<keys::Item<UNIVERSAL_EFFECTS_READY>> = inventories
                            .iter()
                            .flat_map(|inventory| inventory.universal.iter().copied())
                            .map(keys::Item)
                            .collect();
                        let universal_handles = cx.request_all(&universal_keys, 0);
                        let item_keys: Vec<keys::Item<ITEM_COHERENCE>> = inventories
                            .iter()
                            .flat_map(|inventory| inventory.callables.iter().copied())
                            .map(keys::Item)
                            .collect();
                        let item_handles = cx.request_all(&item_keys, 0);
                        let impl_keys: Vec<keys::Params<IMPL_COHERENCE>> = inventories
                            .iter()
                            .flat_map(|inventory| inventory.impls.iter())
                            .map(|d| {
                                keys::Params(d.snapshot.graph_word(), vec![u64::from(d.index)])
                            })
                            .collect();
                        let impl_handles = cx.request_all(&impl_keys, 0);
                        for handle in item_handles
                            .into_iter()
                            .chain(impl_handles)
                            .chain(constant_handles)
                            .chain(admission_handles)
                            .chain(universal_handles)
                            .chain(nominal_handles)
                        {
                            cx.read(handle).await?;
                        }
                        for (declaration, handle) in associated.into_iter().zip(associated_handles)
                        {
                            let checked: Arc<
                                associated_constant_graph::AssociatedConstantCheckedFact,
                            > = cx.read_fact(handle).await?;
                            checked.validate(&types, declaration, &this.0.budget)?;
                        }
                        if let Some(runtime) = runtime {
                            let accounting: Arc<SourceAccounting> = cx.read_fact(runtime).await?;
                            accounting.validate_runtime()?;
                        }
                        return cx.publish(0, 1);
                    }
                    // Link inventory is the closed direct-demand frontier from
                    // the closed roots: each round requests every new
                    // instance's INSTANCE (and, in early rounds, OBJECT)
                    // before awaiting any, and awaits INSTANCEs before any
                    // OBJECT so failure surfaces first. Equivalent targets share
                    // one canonical key, and one symbol names one object.
                    let mut seen: std::collections::HashSet<Vec<u64>> =
                        std::collections::HashSet::new();
                    let mut symbols: std::collections::HashMap<String, ir::CallTarget> =
                        std::collections::HashMap::new();
                    let mut frontier = Vec::new();
                    // Closed roots come from the per-module inventories, keyed
                    // by callable index (a per-item identity).
                    for index in inventories
                        .iter()
                        .flat_map(|inventory| inventory.callables.iter().copied())
                    {
                        if let Some((target, _)) = root_instance(&types, index)? {
                            frontier.push((vec![index as u64], target));
                        }
                    }
                    let mut pending = Vec::new();
                    let mut pending_symbols: Vec<String> = Vec::new();
                    let mut depth = 0usize;
                    while !frontier.is_empty() {
                        this.0.budget.check_depth(depth).map_err(Error::new)?;
                        let mut round = Vec::new();
                        for (params, target) in std::mem::take(&mut frontier) {
                            this.0
                                .budget
                                .charge_work(params.len())
                                .map_err(Error::new)?;
                            if !seen.insert(params.clone()) {
                                continue;
                            }
                            let symbol = instances::symbol_of(&target, &types, &this.0.budget)
                                .map_err(Error::new)?;
                            if let Some(previous) = symbols.get(&symbol) {
                                if instances::admit_representation_alias(
                                    previous,
                                    &target,
                                    &types,
                                    &this.0.budget,
                                )
                                .map_err(Error::new)?
                                {
                                    continue;
                                }
                                return Err(Error::new(
                                    "generic canonical instance identity collision",
                                ));
                            }
                            symbols.insert(symbol.clone(), target.clone());
                            let demands = cx.request(keys::words(INSTANCE, params.clone())?).get(0);
                            round.push((demands, target, symbol));
                            // Early rounds overlap codegen with discovery. A
                            // frontier still growing past OVERLAP_ROUNDS is
                            // deferred until it closes, so a non-terminating
                            // chain fails at its INSTANCE limit without
                            // lowering every level (the engine keeps running
                            // orphan work after BUILD fails).
                            if depth < OVERLAP_ROUNDS {
                                pending.push(Err(cx.request(keys::words(OBJECT, params)?).get(0)));
                            } else {
                                pending.push(Ok(params));
                            }
                        }
                        for (handle, target, _) in &round {
                            let demands: Arc<instances::DirectDemands> =
                                cx.read_fact(handle.clone()).await?;
                            for next in demands.targets.iter() {
                                let params = instance_params(&types, next, &this.0.budget)
                                    .map_err(|error| {
                                        Error::new(format!(
                                            "{error}; instance caller={:?}",
                                            target.declaration
                                        ))
                                    })?;
                                frontier.push((params, next.clone()));
                            }
                        }
                        pending_symbols.extend(round.into_iter().map(|(_, _, symbol)| symbol));
                        depth += 1;
                    }
                    for handle in nominal_handles {
                        let _: Arc<traits::NominalDeclarationProof> = cx.read_fact(handle).await?;
                    }
                    for handle in constant_handles {
                        cx.read(handle).await?;
                    }
                    for (declaration, handle) in associated.into_iter().zip(associated_handles) {
                        let checked: Arc<associated_constant_graph::AssociatedConstantCheckedFact> =
                            cx.read_fact(handle).await?;
                        checked.validate(&types, declaration, &this.0.budget)?;
                    }
                    // Deferred objects of deep rounds are requested together
                    // once the frontier is closed and every INSTANCE succeeded.
                    let mut object_handles = Vec::with_capacity(pending.len());
                    for entry in pending {
                        object_handles.push(match entry {
                            Err(handle) => handle,
                            Ok(params) => cx.request(keys::words(OBJECT, params)?).get(0),
                        });
                    }
                    let (mut objects, _objects_reservation) =
                        this.scratch_vec(object_handles.len())?;
                    for (handle, symbol) in object_handles.into_iter().zip(pending_symbols) {
                        let object: Arc<backend::Object> = cx.read_fact(handle).await?;
                        let mut bytes = object.code.len();
                        // Static payloads are copied word-wise by the linker;
                        // their storage is retained in full below.
                        let mut copy_work = object.code.len();
                        for (_, name) in &object.calls {
                            bytes = bytes
                                .checked_add(name.len())
                                .and_then(|n| n.checked_add(std::mem::size_of::<(usize, String)>()))
                                .ok_or_else(|| {
                                    Error::new("generic link input byte limit exceeded")
                                })?;
                            copy_work = copy_work
                                .saturating_add(name.len())
                                .saturating_add(std::mem::size_of::<(usize, String)>());
                        }
                        for (_, value) in &object.strings {
                            bytes = bytes
                                .checked_add(value.len())
                                .and_then(|n| {
                                    n.checked_add(std::mem::size_of::<(usize, Vec<u8>)>())
                                })
                                .ok_or_else(|| {
                                    Error::new("generic link input byte limit exceeded")
                                })?;
                            copy_work = copy_work
                                .saturating_add(value.len().div_ceil(8))
                                .saturating_add(std::mem::size_of::<(usize, Vec<u8>)>());
                        }
                        this.0.budget.retain_bytes(bytes).map_err(Error::new)?;
                        this.0.budget.charge_work(copy_work).map_err(Error::new)?;
                        objects.push((symbol, object.as_ref().clone()));
                    }
                    // The linked artifact is released only after the explicit
                    // "every required check passed" fact.
                    cx.read(cx.request(keys::CheckRoot.key()).get(0)).await?;
                    let native = &this.0.source_options.native_link;
                    let bytes = if types.has_c_abi()
                        || native.emit_object
                        || !native.objects.is_empty()
                        || !native.libraries.is_empty()
                    {
                        if this.0.source_options.source_snapshot.is_some() && !native.emit_object {
                            return Err(Error::new("unsupported: immutable source compilation cannot invoke the uncontrolled native linker"));
                        }
                        c_abi::elf::link(&objects, native, &this.0.budget)
                    } else {
                        backend::link(&objects)
                    }
                    .map_err(Error::new)?;
                    if let Some(runtime) = runtime {
                        let accounting: Arc<SourceAccounting> = cx.read_fact(runtime).await?;
                        accounting.validate_runtime()?;
                    }
                    cx.publish_fact(0, bytes)
                }
                _ => Err(Error::new("unknown compiler operation")),
            }
        })
    }
}
/// Run the metered local pipeline. The result's callee table is a paid
/// copy; the returned value owns a lease covering both.
fn local_opt(
    lowered: &ssa::lower::Lowered,
    meter: &ssa::meter::Meter<'_>,
) -> Result<ssa::store::Owned<ssa::lower::Lowered>, Error> {
    let (function, _stats) = ssa::opt::local_metered(
        &lowered.function,
        &|callee| lowered.signature(callee),
        meter,
    )
    .map_err(|error| Error::new(error.to_string()))?;
    let callees = ssa::store::copy_callees(meter, &lowered.callees).map_err(Error::new)?;
    ssa::store::Owned::combine(function, callees, meter, |function, callees| {
        ssa::lower::Lowered { function, callees }
    })
    .map_err(Error::new)
}

fn has_drop(function: &cfg::Function) -> bool {
    function
        .blocks
        .iter()
        .any(|block| matches!(block.terminator.value, cfg::Terminator::Drop { .. }))
}
fn closure_body_index(key: &Key) -> Result<(usize, Option<ir::ClosureKind>), Error> {
    let (index, mode) = match key.params.as_slice() {
        [index] => (*index, None),
        [index, 0] => (*index, Some(ir::ClosureKind::Fn)),
        [index, 1] => (*index, Some(ir::ClosureKind::FnMut)),
        [index, 2] => (*index, Some(ir::ClosureKind::FnOnce)),
        _ => return Err(Error::new("invalid closure body capability key")),
    };
    Ok((
        usize::try_from(index).map_err(|_| Error::new("invalid declaration index"))?,
        mode,
    ))
}
fn closure_body_key(
    kind: u32,
    index: usize,
    primary: ir::ClosureKind,
    mode: ir::ClosureKind,
) -> Key {
    if primary == mode {
        return keys::item(kind, index);
    }
    let mode = match mode {
        ir::ClosureKind::Fn => 0,
        ir::ClosureKind::FnMut => 1,
        ir::ClosureKind::FnOnce => 2,
    };
    keys::params(kind, index as u64, vec![mode])
}
/// One callable's immutable generic signature.
#[derive(Debug)]
#[allow(dead_code)] // consumers move over in step 3
pub(crate) struct SignatureFact {
    pub declaration: ir::DeclId,
    pub signature: ir::GenericSignature,
}
impl crate::engine::Fact for SignatureFact {}
/// Every implementation of one trait: the explicit completeness fact for
/// lookups that genuinely need all impls.
#[derive(Debug)]
#[allow(dead_code)] // consumers move over in step 3
pub(crate) struct TraitImplsFact {
    pub impls: Vec<ir::DeclId>,
}
impl crate::engine::Fact for TraitImplsFact {}
/// `MODULE_DECLS` output: one module's canonical source declarations.
pub(crate) struct ModuleDeclsFact {
    pub declarations: frontend::module_declarations::ModuleDeclarations,
}
impl crate::engine::Fact for ModuleDeclsFact {}
fn decl_param(key: &Key, what: &str) -> Result<ir::DeclId, Error> {
    let [snapshot, index] = key.params.as_slice() else {
        return Err(Error::new(format!(
            "{what} requires snapshot and declaration"
        )));
    };
    Ok(ir::DeclId {
        snapshot: ir::SnapshotId::from_graph_word(*snapshot),
        index: *index,
    })
}
fn item_index(key: &Key, missing: &str) -> Result<usize, Error> {
    let [index] = key.params.as_slice() else {
        return Err(Error::new(missing));
    };
    usize::try_from(*index).map_err(|_| Error::new("invalid item index"))
}

/// One snapshot may serve check and build roots together without redoing checks.
pub struct Compilation {
    /// Running-image evidence read back from the prepared-source graph fact.
    /// Includes executable mapped files and bounded process configuration, not
    /// arbitrary in-memory state. Missing evidence never authorizes reuse.
    pub compiler_image: Option<Arc<procfs::CompilerImage>>,
    pub executable: Option<Vec<u8>>,
    pub report: Report,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    /// Explicit control-flow graph counts, distinct from the work dependency graph.
    pub cfg_blocks: usize,
    pub cfg_edges: usize,
    pub cfg_locals: usize,
    /// Shared CFG facts retained for post-build graph/provenance inspection.
    pub control_flow: Vec<Arc<cfg::Function>>,
    /// Per-function immutable initialization/move analysis facts.
    pub analyses: Vec<Arc<ownership::Analysis>>,
    /// Bytes read from all source files, before external-module expansion.
    pub source_bytes: usize,
    pub source_files: usize,
    /// Shared planning-budget accounting, distinct from allocator/RSS measurements.
    pub resource_usage: ir::PlanUsageSnapshot,
    /// Optimizer outcome per instance, read from retained IPO_OPT facts.
    pub optimizer: OptimizerReport,
}

/// What the optimizer did in one build (OPTIMIZATION.md section 8.9).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptimizerReport {
    pub mode: ssa::OptMode,
    /// Instances compiled by the SSA machine backend.
    pub optimized: usize,
    /// Instances kept on the stack emitter, with the first unsupported reason.
    pub fallback: Vec<&'static str>,
    /// Callee bodies inlined across all instances.
    pub inlined: usize,
}

/// Compile an immutable source snapshot. Executable publication is gated on all
/// required semantic checks, including unused function bodies.
/// A failed compilation retains its graph accounting when execution completed.
#[derive(Debug)]
pub struct Failure {
    pub error: Error,
    pub report: Option<Box<Report>>,
    /// Completed analysis facts paired with their original function indices.
    pub analyses: Vec<(usize, Arc<ownership::Analysis>)>,
    /// Captured after failure rendering; the first failure remains immutable.
    pub resource_usage: Box<ir::PlanUsageSnapshot>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for Failure {}

pub fn compile(source: String, workers: usize, check_only: bool) -> Result<Compilation, Error> {
    compile_observed(source, workers, check_only).map_err(|failure| failure.error)
}

/// Like compile, but preserves diagnostics and graph events on root failure.
pub fn compile_observed(
    source: String,
    workers: usize,
    check_only: bool,
) -> Result<Compilation, Failure> {
    compile_input(Input::Text(source), workers, check_only)
}

/// Compile text with an explicit immutable native source configuration.
pub fn compile_observed_with_options(
    source: String,
    workers: usize,
    check_only: bool,
    options: source::SourceOptions,
) -> Result<Compilation, Failure> {
    compile_input_with_options(Input::Text(source), workers, check_only, options)
}

/// Load a source tree through an observable graph operation before parsing.
/// File loading currently uses one coarse snapshot node, not per-file nodes.
pub fn compile_file_observed(
    path: PathBuf,
    workers: usize,
    check_only: bool,
) -> Result<Compilation, Failure> {
    compile_input(Input::File(path), workers, check_only)
}

/// Prepare a configured file tree before parsing, without host-derived cfg.
pub fn compile_file_observed_with_options(
    path: PathBuf,
    workers: usize,
    check_only: bool,
    options: source::SourceOptions,
) -> Result<Compilation, Failure> {
    compile_input_with_options(Input::File(path), workers, check_only, options)
}

/// Compile a client with an explicitly source-locked provider declaration.
/// The provider still passes ordinary parsing, universal checks and native lowering.
/// This does not compile a library crate or authorize intrinsic implementations.
pub fn compile_probe_file_observed(
    path: PathBuf,
    workers: usize,
    check_only: bool,
    request: source::SourceProbeRequest,
) -> Result<Compilation, Failure> {
    compile_probe_file_observed_with_options(
        path,
        workers,
        check_only,
        source::SourceOptions::default(),
        request,
    )
}

/// Like the probe entry point, with an immutable explicit native cfg set.
pub fn compile_probe_file_observed_with_options(
    path: PathBuf,
    workers: usize,
    check_only: bool,
    options: source::SourceOptions,
    request: source::SourceProbeRequest,
) -> Result<Compilation, Failure> {
    compile_input_with_options(
        Input::ProbeFile(path, request),
        workers,
        check_only,
        options,
    )
}

/// Every analysis the graph published, from the analysis nodes' second output,
/// paired with its declaration or instance index. Reads facts only; demands
/// no new work.
fn retained_analyses(engine: &Engine, report: &Report) -> Vec<(usize, Arc<ownership::Analysis>)> {
    let mut analyses = Vec::new();
    let callables = engine
        .fact_of::<frontend::PlanCtx>(&keys::SignaturesDebtRoot.key(), 0)
        .map_or(0, |types| types.callable_count());
    // CHECK proves source callables through SOURCE_ADMISSION, which publishes
    // its analysis (success or failure) at ANALYSIS_OUTPUT before failing.
    for (kind, count) in [
        (SOURCE_ADMISSION, callables),
        (UNIVERSAL_ANALYZE, callables),
    ] {
        for index in 0..count {
            let key = keys::params(kind, index as u64, vec![]);
            if let Some(analysis) =
                engine.fact_of::<Arc<ownership::Analysis>>(&key, ANALYSIS_OUTPUT)
            {
                analyses.push((index, analysis.as_ref().clone()));
            }
        }
    }
    for (position, key) in discovered_instances(report, ANALYZE)
        .into_iter()
        .enumerate()
    {
        if let Some(analysis) = engine.fact_of::<Arc<ownership::Analysis>>(&key, ANALYSIS_OUTPUT) {
            analyses.push((
                key_root_slot(&key).unwrap_or(position),
                analysis.as_ref().clone(),
            ));
        }
    }
    analyses
}
/// Post-run: every discovered instance node of `kind`, closed roots first in
/// slot order, then the generic frontier in canonical key order. Reporting
/// only; reads the finished report, never the graph.
fn discovered_instances(report: &Report, kind: u32) -> Vec<Key> {
    let mut found: Vec<Key> = report
        .node_evaluations
        .iter()
        .filter(|(key, _)| key.kind == kind)
        .map(|(key, _)| key.clone())
        .collect();
    found.sort_by(|left, right| {
        (left.params.len() > 1, &left.params).cmp(&(right.params.len() > 1, &right.params))
    });
    found.dedup();
    found
}

fn compile_input(input: Input, workers: usize, check_only: bool) -> Result<Compilation, Failure> {
    compile_input_with_budget(input, workers, check_only, ir::PlanBudget::default())
}

fn compile_input_with_options(
    input: Input,
    workers: usize,
    check_only: bool,
    options: source::SourceOptions,
) -> Result<Compilation, Failure> {
    compile_input_configured(
        input,
        workers,
        check_only,
        ir::PlanBudget::default(),
        options,
    )
}

fn compile_input_with_budget(
    input: Input,
    workers: usize,
    check_only: bool,
    budget: ir::PlanBudget,
) -> Result<Compilation, Failure> {
    compile_input_configured(
        input,
        workers,
        check_only,
        budget,
        source::SourceOptions::default(),
    )
}

fn compile_input_configured(
    input: Input,
    workers: usize,
    check_only: bool,
    budget: ir::PlanBudget,
    mut source_options: source::SourceOptions,
) -> Result<Compilation, Failure> {
    // Always capture the actual running image, never trust caller-supplied stamps.
    // Synthetic Program fixtures make no external compiler-input claim.
    source_options.compiler_image = if matches!(&input, Input::Program(_)) {
        None
    } else {
        procfs::CompilerImage::current().ok()
    };
    let snapshot = Arc::new(Snapshot {
        input,
        source_options,
        budget,
    });
    let engine = Engine::new(CompilerProvider(snapshot.clone()));
    let check = engine.request(keys::CheckRoot.key());
    let build = (!check_only).then(|| engine.request(keys::BuildRoot.key()));
    let report = engine.run(workers).map_err(|error| Failure {
        error,
        report: None,
        analyses: Vec::new(),
        resource_usage: Box::new(snapshot.budget.usage_snapshot()),
    })?;
    let failure = |error: Error, report: &Report| Failure {
        error: const_diagnostics::qualification_cycle_error(
            &engine,
            report,
            &error,
            &snapshot.budget,
        )
        .unwrap_or(error),
        report: Some(Box::new(report.clone())),
        analyses: retained_analyses(&engine, report),
        resource_usage: Box::new(metered_usage(&engine, &snapshot.budget, report)),
    };
    let outcome = (|| -> Result<Option<Arc<Vec<u8>>>, Error> {
        check.completion()?;
        check.result(0)?;
        let executable = match &build {
            Some(build) => {
                build.completion()?;
                Some(
                    build
                        .fact::<Vec<u8>>(0)
                        .map_err(|_| Error::new("missing native executable"))?,
                )
            }
            None => None,
        };
        if !report.capture_complete {
            return Err(Error::new("compiler graph capture incomplete"));
        }
        Ok(executable)
    })();
    let executable = match outcome {
        Ok(executable) => executable,
        Err(error) => return Err(failure(error, &report)),
    };
    let types = engine.fact_of::<frontend::PlanCtx>(&keys::SignaturesDebtRoot.key(), 0);
    // Public source telemetry remains source-only. Registered recipe bodies are
    // additional universal proof subjects, not newly discovered source functions.
    let functions = (|| -> Result<usize, String> {
        let Some(context) = &types else {
            return Ok(0);
        };
        snapshot.budget.charge_work(context.callable_count())?;
        let mut count = 0usize;
        for index in 0..context.callable_count() {
            let declaration = context.callable_decl(index)?;
            match context.callable_provider(declaration)? {
                frontend::CallableProvider::Source { .. }
                    if context.is_prelude_decl(declaration)? => {}
                frontend::CallableProvider::Source { .. } => {
                    count = count
                        .checked_add(1)
                        .ok_or("source callable count overflow")?;
                }
                frontend::CallableProvider::Recipe { .. } => {}
                frontend::CallableProvider::UnavailableProvided { .. } => {
                    return Err(
                        "unsupported: unavailable provided body in source accounting".into(),
                    );
                }
            }
        }
        Ok(count)
    })()
    .map_err(|message| failure(Error::new(message), &report))?;
    let (source_bytes, source_files, compiler_image) = match &snapshot.input {
        Input::Program(_) => (0, 0, None),
        Input::Text(_) | Input::File(_) | Input::ProbeFile(_, _) => {
            let loaded = engine
                .fact_of::<SourceAccounting>(&keys::ResolveDebtRoot.key(), SOURCE_ACCOUNTING_OUTPUT)
                .ok_or_else(|| failure(Error::new("missing loaded source accounting"), &report))?;
            (
                loaded.source_bytes,
                loaded.files,
                loaded.compiler_image.clone(),
            )
        }
    };
    let lowered_instances = discovered_instances(&report, LOWER);
    let analyzed_instances = discovered_instances(&report, ANALYZE);
    let optimized_instances = discovered_instances(&report, IPO_OPT);
    // BUILD reports executable instance CFGs (LOWER). CHECK never demands the
    // instance plan, so it reports the per-source-callable CFG that
    // SOURCE_ADMISSION analyzed.
    let source_callables: Vec<usize> = types.as_ref().map_or(Vec::new(), |types| {
        (0..types.callable_count())
            .filter(|&index| {
                types.callable_decl(index).is_ok_and(|declaration| {
                    matches!(
                        types.callable_provider(declaration),
                        Ok(frontend::CallableProvider::Source { .. })
                    ) && types.is_prelude_decl(declaration) == Ok(false)
                })
            })
            .collect()
    });
    let control_flow: Vec<Arc<cfg::Function>> = if check_only {
        source_callables
            .iter()
            .filter_map(|&index| {
                engine.fact_of::<Arc<cfg::Function>>(
                    &keys::Item::<{ SOURCE_ADMISSION }>(index).key(),
                    SOURCE_CFG_OUTPUT,
                )
            })
            .map(|lowered| lowered.as_ref().clone())
            .collect()
    } else {
        lowered_instances
            .iter()
            .filter_map(|key| engine.fact_of::<Arc<cfg::Function>>(key, 0))
            .map(|lowered| lowered.as_ref().clone())
            .collect()
    };
    let mut cfg_blocks = 0;
    let mut cfg_edges = 0;
    let mut cfg_locals = 0;
    for function in &control_flow {
        cfg_blocks += function.blocks.len();
        cfg_locals += function.locals.len();
        cfg_edges += function
            .blocks
            .iter()
            .map(|block| match &block.terminator.value {
                cfg::Terminator::Goto(_)
                | cfg::Terminator::Call { .. }
                | cfg::Terminator::Drop { .. } => 1,
                cfg::Terminator::Switch { .. } | cfg::Terminator::SwitchVariant { .. } => 2,
                cfg::Terminator::Return
                | cfg::Terminator::Panic(_)
                | cfg::Terminator::Unreachable => 0,
            })
            .sum::<usize>();
    }
    let program = engine.fact_of::<Arc<frontend::Program>>(&keys::ResolveDebtRoot.key(), 0);
    // BUILD proves every executable instance (ANALYZE). CHECK never demands
    // the instance plan: it proves each source callable via SOURCE_ADMISSION.
    let analyses = if check_only {
        source_callables
            .iter()
            .filter_map(|&index| {
                engine.fact_of::<Arc<ownership::Analysis>>(
                    &keys::Item::<{ SOURCE_ADMISSION }>(index).key(),
                    ANALYSIS_OUTPUT,
                )
            })
            .map(|analysis| analysis.as_ref().clone())
            .collect()
    } else {
        analyzed_instances
            .iter()
            .filter_map(|key| engine.fact_of::<Arc<ownership::Analysis>>(key, ANALYSIS_OUTPUT))
            .map(|analysis| analysis.as_ref().clone())
            .collect()
    };
    let resource_usage = metered_usage(&engine, &snapshot.budget, &report);
    let mut optimizer = OptimizerReport {
        mode: snapshot.source_options.opt,
        ..OptimizerReport::default()
    };
    for key in &optimized_instances {
        if let Some(fact) = engine.fact_of::<SsaFact>(key, 0) {
            if fact.lowered.is_some() {
                optimizer.optimized += 1;
            } else {
                optimizer
                    .fallback
                    .push(fact.unsupported.unwrap_or("unknown"));
            }
            optimizer.inlined += fact.inlined;
        }
    }
    Ok(Compilation {
        compiler_image,
        executable: executable.map(|bytes| bytes.as_ref().clone()),
        report,
        functions,
        structs: program.as_ref().map_or(0, |program| program.user_structs()),
        enums: program.as_ref().map_or(0, |program| program.user_enums()),
        cfg_blocks,
        cfg_edges,
        cfg_locals,
        analyses,
        control_flow,
        source_bytes,
        source_files,
        resource_usage,
        optimizer,
    })
}

/// The shared ledger snapshot plus all per-task metered work and nodes.
/// Compilation-wide usage report: the direct (non-graph) ledger plus every
/// node's own ledger for this budget, folded after the run. Work and node
/// counts come from the node meters, which also cover expansion work.
/// Reporting only; admission never reads this sum.
fn metered_usage(
    engine: &Engine,
    budget: &ir::PlanBudget,
    report: &Report,
) -> ir::PlanUsageSnapshot {
    let direct = budget.usage_snapshot();
    let mut usage = direct;
    budget.fold_node_ledgers(engine, &mut usage);
    usage.work = direct
        .work
        .saturating_add(usize::try_from(report.metered_work).unwrap_or(usize::MAX));
    usage.nodes = direct
        .nodes
        .saturating_add(usize::try_from(report.metered_nodes).unwrap_or(usize::MAX));
    usage
}

pub(super) fn compile_program_observed(
    program: frontend::Program,
    workers: usize,
    check_only: bool,
) -> Result<Compilation, Failure> {
    let budget = ir::PlanBudget::default();
    budget
        .retain_bytes(std::mem::size_of::<frontend::Program>() + 2 * std::mem::size_of::<usize>())
        .map_err(|message| Failure {
            error: Error::new(message),
            report: None,
            analyses: Vec::new(),
            resource_usage: Box::new(budget.usage_snapshot()),
        })?;
    compile_input_with_budget(
        Input::Program(Arc::new(program)),
        workers,
        check_only,
        budget,
    )
}

#[cfg(test)]
mod closure_mode_tests;

#[cfg(test)]
mod owner_route_tests {
    use super::*;

    #[test]
    fn closed_source_proof_reuse_requires_exact_publication_cfg_and_context() {
        for workers in [1, 8] {
            let budget = ir::PlanBudget::default();
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("fn first() {} fn main() {}".into()),
                source_options: source::SourceOptions::default(),
                budget: budget.clone(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let check = engine.request(keys::CheckRoot.key());
            let report = engine.run(workers).unwrap();
            assert_eq!(report.failures, 0, "{:?}", report.node_errors);
            check.completion().unwrap();
            let load = |index| {
                engine
                    .fact_of::<Option<ClosedSourceProof>>(
                        &keys::Item::<SOURCE_ADMISSION>(index).key(),
                        SOURCE_CLOSED_PROOF_OUTPUT,
                    )
                    .unwrap()
            };
            let first = load(0);
            let second = load(1);
            let proof = first.as_ref().as_ref().unwrap();
            let other = second.as_ref().as_ref().unwrap();
            let body = InstanceBodyFact {
                facts: proof.publication.facts().clone(),
                checked: proof.publication.source().clone(),
            };
            let target = body.facts.target().unwrap();
            let cfg = proof.concrete.function();
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &body,
                        target,
                        cfg,
                        proof.const_context,
                        &budget
                    )
                    .is_ok()
            );
            assert!(
                proof
                    .reuse(
                        &other.publication,
                        &body,
                        target,
                        cfg,
                        proof.const_context,
                        &budget
                    )
                    .is_err()
            );
            let copied_cfg = Arc::new(cfg.as_ref().clone());
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &body,
                        target,
                        &copied_cfg,
                        proof.const_context,
                        &budget
                    )
                    .is_err()
            );
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &body,
                        target,
                        cfg,
                        !proof.const_context,
                        &budget
                    )
                    .is_err()
            );
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &body,
                        other.concrete.facts().target().unwrap(),
                        cfg,
                        proof.const_context,
                        &budget
                    )
                    .is_err()
            );
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &body,
                        target,
                        cfg,
                        proof.const_context,
                        &ir::PlanBudget::default()
                    )
                    .is_err()
            );
            let copied_body = InstanceBodyFact {
                facts: body.facts.clone(),
                checked: Arc::new(body.checked.as_ref().clone()),
            };
            assert!(
                proof
                    .reuse(
                        &proof.publication,
                        &copied_body,
                        target,
                        cfg,
                        proof.const_context,
                        &budget
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn parse_file_failures_retry_only_ordered_user_diagnostics() {
        use source::SourceErrorKind::*;
        for kind in [Syntax, Unsupported] {
            assert!(ParseFileFailure::ordered_retry(kind));
        }
        for kind in [Io, Limit, Cycle, Module, Contract, Infrastructure] {
            assert!(!ParseFileFailure::ordered_retry(kind));
        }
        let budget = expansion_budget::ExpansionBudget::default();
        let failure = ParseFileFailure::from_error(
            source::SourceError::syntax(None, "invalid item"),
            &budget,
        )
        .unwrap();
        assert_eq!(failure.kind, Syntax);
        assert!(failure._storage.bytes() >= failure.message.len() as u64);
        let inherited = failure.inherited(&budget).unwrap();
        assert!(Arc::ptr_eq(&failure.message, &inherited.message));
        assert!(inherited._storage.bytes() >= std::mem::size_of::<ParseFileFailure>() as u64);
        let limits = expansion_budget::ExpansionLimits {
            bytes: 1,
            ..Default::default()
        };
        let exhausted = expansion_budget::ExpansionBudget::new(limits);
        assert!(
            ParseFileFailure::from_error(
                source::SourceError::syntax(None, "cannot bypass limit"),
                &exhausted
            )
            .is_err()
        );
    }

    #[test]
    fn default_prelude_parse_publishes_successful_file_facts() {
        for (library, dependency, text) in [
            (false, false, "fn main() { let x = Some(7i32); }"),
            (true, false, "#![no_std]\npub fn value() -> i32 { 7i32 }"),
            (false, true, "fn main() { let x = geom::double(7i64); }"),
        ] {
            for workers in [1, 8] {
                let snapshot = Arc::new(Snapshot {
                    input: Input::Text(text.into()),
                    source_options: source::SourceOptions {
                        library,
                        externs: if dependency {
                            vec![source::ExternCrate {
                                name: "geom".into(),
                                root: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                                    "tests/fixtures/projects/workspace/crates/geom/src/lib.rs",
                                ),
                                cfg: source::SourceOptions::default().cfg,
                                cap_lints_allow: false,
                            }]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    },
                    budget: ir::PlanBudget::default(),
                });
                let engine = Engine::new(CompilerProvider(snapshot.clone()));
                let parse = engine.request(keys::ResolveDebtRoot.key());
                engine.run(workers).unwrap();
                parse.completion().unwrap();
                let root = source::source_admit::final_root(&snapshot.source_options);
                let admitted = engine
                    .fact_of::<source::source_admit::AdmitState>(
                        &crate::engine::ItemKey::key(&keys::Admit(
                            keys::SourceFile::from_words(&[root]).unwrap(),
                        )),
                        1,
                    )
                    .unwrap();
                let files = admitted
                    .admitted()
                    .iter()
                    .map(|ordinal| keys::SourceFile::from_words(ordinal).unwrap())
                    .collect::<Vec<_>>();
                assert!(files.len() > 20, "prelude files were not split");
                if dependency {
                    assert!(files.contains(&keys::SourceFile::from_words(&[2, 0]).unwrap()));
                }
                for file in &files {
                    let key = crate::engine::ItemKey::key(&keys::ParseFile(file.clone()));
                    let parsed = engine
                        .fact_of::<Result<frontend::ParsedProgram, ParseFileFailure>>(&key, 0)
                        .unwrap();
                    assert!(
                        parsed.is_ok(),
                        "file fell back to assembled parsing: {file:?}: {:?}",
                        parsed.as_ref().as_ref().err().map(|error| &error.message)
                    );
                    parsed
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .program
                        .validate_name_handles()
                        .unwrap_or_else(|error| {
                            panic!("file {file:?} has invalid lexical authority: {error}")
                        });
                }
            }
        }
    }

    #[test]
    fn parse_file_rejects_unadmitted_identity() {
        for workers in [1, 8] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("fn main() {}".into()),
                source_options: source::SourceOptions {
                    prelude: false,
                    ..Default::default()
                },
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let unknown = engine.request(crate::engine::ItemKey::key(&keys::ParseFile(
                keys::SourceFile::input().child(99),
            )));
            engine.run(workers).unwrap();
            assert!(
                unknown
                    .completion()
                    .unwrap_err()
                    .to_string()
                    .contains("not admitted")
            );
            assert!(
                unknown
                    .fact::<Result<frontend::ParsedProgram, ParseFileFailure>>(0)
                    .is_err()
            );
        }
    }

    #[test]
    fn signatures_consume_the_exact_resolve_snapshot_capability() {
        for workers in [1, 8] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("fn unused(){}fn main(){}".into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let signatures = engine.request(keys::SignaturesDebtRoot.key());
            engine.run(workers).unwrap();
            signatures.completion().unwrap();
            let program = engine
                .fact_of::<Arc<frontend::Program>>(&keys::ResolveDebtRoot.key(), 0)
                .unwrap();
            let sealed = engine
                .fact_of::<frontend::ValidatedProgram>(&keys::ResolveDebtRoot.key(), 2)
                .unwrap();
            assert!(std::ptr::eq(program.as_ref().as_ref(), sealed.program()));
        }
    }

    #[test]
    fn graph_program_input_cannot_seal_same_owner_mutated_ast() {
        for workers in [1, 8] {
            let mut program = frontend::parse("fn unused(){}fn main(){}").unwrap();
            program.functions[0].module = program.modules.core_root();
            let snapshot = Arc::new(Snapshot {
                input: Input::Program(Arc::new(program)),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let resolve = engine.request(keys::ResolveDebtRoot.key());
            engine.run(workers).unwrap();
            assert!(resolve.completion().is_err());
            assert!(resolve.fact::<frontend::ValidatedProgram>(2).is_err());
        }
    }

    /// The parsed program of one graph compile and its engine authority.
    fn graph_parsed(source: &str) -> (Arc<Result<frontend::ParsedProgram, ParseFileFailure>>, u64) {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(source.into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let authority = engine.authority().get();
        let key = crate::engine::ItemKey::key(&keys::ParseFile(keys::SourceFile::input()));
        let parse = engine.request(key.clone());
        engine.run(2).unwrap();
        parse.completion().unwrap();
        let parsed = engine
            .fact_of::<Result<frontend::ParsedProgram, ParseFileFailure>>(&key, 0)
            .unwrap();
        (parsed, authority)
    }

    fn minted_by(tag: u64) -> Option<u64> {
        crate::engine::TAG_PROVENANCE
            .lock()
            .unwrap()
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, authority)| *authority)
    }

    fn owner(parsed: &frontend::ParsedProgram) -> u64 {
        parsed.program.names.session().id().bits().get()
    }

    #[test]
    fn graph_parse_owner_is_issued_not_a_standalone_mint() {
        const SOURCE: &str = "fn f() -> i32 { 1 } fn main() { f(); }";
        let (a, a_engine) = graph_parsed(SOURCE);
        let (b, b_engine) = graph_parsed(SOURCE);
        let (a, b) = (a.as_ref().as_ref().unwrap(), b.as_ref().as_ref().unwrap());
        let direct = frontend::parse_unresolved(SOURCE).unwrap();
        // The graph session owner was minted under its producing engine's
        // authority: the PARSE_FILE node's own root, not a standalone mint.
        assert_eq!(minted_by(owner(a)), Some(a_engine));
        assert_eq!(minted_by(owner(b)), Some(b_engine));
        // A direct parse mints under a fresh standalone authority.
        let direct_root = minted_by(owner(&direct)).expect("direct parse tag recorded");
        assert_ne!(direct_root, a_engine);
        assert_ne!(direct_root, b_engine);
        assert_ne!(owner(a), owner(b), "two graph compiles share an owner");
        // Foreign grafts across graph compiles still reject.
        let ident = a.program.functions[0].name.segments()[0];
        assert!(b.program.names.spelling(ident).is_err());
        assert!(a.program.names.spelling(ident).is_ok());
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn generic_instance_identity_is_canonical_not_sentinel() {
        let generic = keys::test_key(ANALYZE, vec![7, 3, 9]);
        let identity = instance_identity(&generic, 4, "crate::twice");
        assert!(identity.contains("crate::twice") && identity.contains("generic item 4"));
        assert!(!identity.contains(&usize::MAX.to_string()));
        assert_eq!(identity, instance_identity(&generic, 4, "crate::twice"));
        assert_ne!(
            identity,
            instance_identity(&keys::test_key(ANALYZE, vec![7, 3, 10]), 4, "crate::twice")
        );
        assert_eq!(
            instance_identity(&keys::test_key(ANALYZE, vec![2]), 4, "f"),
            "2"
        );
    }

    #[test]
    fn analysis_diagnostics_stop_before_rich_payload_limit() {
        let budget = ir::PlanBudget::default();
        let analysis = ownership::Analysis {
            stats: ownership::AnalysisStats::default(),
            diagnostics: vec![ownership::Diagnostic {
                kind: ownership::ErrorKind::InvalidReference("x".repeat(70 * 1024)),
                block: cfg::BlockId(0),
                statement: None,
                origin: None,
                place: None,
            }]
            .into_boxed_slice(),
            complete: false,
        };
        let message = analysis_diagnostic(&budget, "instance", 0, &analysis);
        assert!(message.contains("resource"), "{message}");
        assert!(message.len() <= 80);
        assert_eq!(analysis.diagnostics.len(), 1);
        assert!(!analysis.passed());

        let budget = ir::PlanBudget::default();
        let empty = ownership::Analysis {
            stats: ownership::AnalysisStats::default(),
            diagnostics: Box::new([]),
            complete: true,
        };
        assert_eq!(
            analysis_diagnostic(&budget, "instance", 3, &empty),
            "instance 3: initialization/move analysis (complete=true): []"
        );
        let category_budget = ir::PlanBudget::default();
        let unsupported =
            traits::FactsError::unsupported(&category_budget, "narrow source boundary");
        assert_eq!(
            source_fact_error(unsupported, &category_budget).to_string(),
            "Unsupported: narrow source boundary"
        );
        let exhausted = ir::PlanBudget::default();
        let error =
            traits::FactsError::unsupported(&exhausted, "must not hide exhausted transport");
        for bit in (0..usize::BITS).rev() {
            let _ = exhausted.charge_work(1usize << bit);
        }
        let rendered = source_fact_error(error, &exhausted).to_string();
        assert!(rendered.contains("ResourceLimit"));
        assert!(!rendered.contains("Unsupported"));
        assert!(rendered.len() < 80);
    }
}

/// Per-node optimizer resource tests (OPTIMIZER-ACCOUNTING-AUDIT.md
/// acceptance): each optimizer producer runs inside the real graph with the
/// snapshot budget, so bytes go to the polled node's own meter. These pin
/// that each node publishes a receipt matching its retained storage, that the
/// unsupported path carries no receipt beyond the engine envelope, and that a
/// byte or work limit below a node's need fails the build closed.
#[cfg(test)]
mod optimizer_resource_tests {
    use super::*;

    const SOURCE: &str = "fn leaf(x: i64) -> i64 { x * 3 + 1 }\n\
        fn mid(x: i64) -> i64 { leaf(x) + leaf(x + 1) }\n\
        struct P { a: i32 }\n\
        fn takes(p: P) -> i32 { p.a }\n\
        fn main() { if mid(4) == 29 { println!(\"ok\") } let _ = takes(P { a: 1 }); }";

    fn options(opt: ssa::OptMode) -> source::SourceOptions {
        source::SourceOptions {
            opt,
            ..source::SourceOptions::default()
        }
    }

    fn run(budget: ir::PlanBudget) -> (Engine, Result<Report, Error>) {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(SOURCE.into()),
            source_options: options(ssa::OptMode::Adaptive),
            budget,
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let build = engine.request(keys::BuildRoot.key());
        let report = engine.run(1);
        let outcome = report.and_then(|report| build.completion().map(|()| report));
        (engine, outcome)
    }

    fn instances(report: &Report) -> Vec<Key> {
        discovered_instances(report, OBJECT)
    }

    #[test]
    fn every_optimizer_node_publishes_exact_receipts() {
        let (engine, outcome) = run(ir::PlanBudget::default());
        let report = outcome.expect("optimizer build succeeds at default limits");
        let instance_keys = instances(&report);
        let count = instance_keys.len();
        assert!(count >= 4);
        let mut lowered = 0;
        let mut unsupported = 0;
        let mut objects_with_receipt = 0;
        for (index, instance) in instance_keys.iter().enumerate() {
            let key = |kind| keys::words(kind, instance.params.clone()).unwrap();
            for kind in [SSA, LOCAL_OPT, IPO_OPT] {
                let fact = engine
                    .fact_of::<SsaFact>(&key(kind), 0)
                    .unwrap_or_else(|| panic!("kind {kind} instance {index} published"));
                let receipt = &fact._receipt;
                match &fact.lowered {
                    Some(body) => {
                        lowered += 1;
                        let storage = ssa::store::lowered_capacity_bytes(body).unwrap();
                        // A fact either owns its body (receipt covers body +
                        // Arc + envelope) or shares the LOCAL_OPT body (IPO_OPT
                        // with nothing inlined: envelope only).
                        let owned = storage + SsaFact::BODY_ARC + SsaFact::ENVELOPE;
                        assert!(
                            receipt.bytes() == owned || receipt.bytes() == SsaFact::ENVELOPE,
                            "kind {kind} instance {index}: receipt {} vs storage {owned}",
                            receipt.bytes()
                        );
                    }
                    None => {
                        unsupported += 1;
                        assert!(fact.unsupported.is_some());
                        // Unsupported facts still pay their own envelope.
                        assert_eq!(receipt.bytes(), SsaFact::ENVELOPE);
                    }
                }
            }
            if let Some(summary) = engine.fact_of::<SummaryFact>(&key(SUMMARY), 0) {
                assert_eq!(summary._receipt.bytes(), envelope::<SummaryFact>());
            }
            let object = engine
                .fact_of::<backend::Object>(&key(OBJECT), 0)
                .expect("object published");
            if let Some(receipt) = &object.receipt {
                objects_with_receipt += 1;
                let mut owned = object.code.capacity()
                    + object.calls.capacity() * std::mem::size_of::<(usize, String)>()
                    + object.strings.capacity() * std::mem::size_of::<(usize, Vec<u8>)>();
                for (_, symbol) in &object.calls {
                    owned += symbol.capacity();
                }
                // Moved panic diagnostics are paid exactly once, here.
                for (_, payload) in &object.strings {
                    owned += payload.capacity();
                }
                // Object value + Arc header, and Arc<Receipt> payload + header.
                let arc = 2 * std::mem::size_of::<usize>();
                owned += std::mem::size_of::<backend::Object>() + arc;
                owned += std::mem::size_of::<ssa::meter::Receipt>() + arc;
                assert_eq!(receipt.bytes(), owned, "object {index} receipt");
                assert_eq!(
                    receipt.bytes(),
                    ssa::machine::object_bytes(&object).unwrap()
                );
            }
        }
        assert!(
            lowered >= 6,
            "scalar functions take the SSA path: {lowered}"
        );
        assert!(
            unsupported >= 3,
            "struct/println functions publish unsupported: {unsupported}"
        );
        assert!(objects_with_receipt >= 2);
    }

    #[test]
    fn tiny_byte_and_work_budgets_fail_closed() {
        // Measure the default run, then retry with limits below its need. The
        // build must fail (no executable published) and record a resource
        // failure, never publish a partial or unpaid result.
        let default = ir::PlanBudget::default();
        let (_, outcome) = run(default.clone());
        outcome.unwrap();
        for limits in [
            ir::PlanLimits {
                bytes: 4096,
                ..ir::PlanLimits::default()
            },
            ir::PlanLimits {
                work: 2000,
                ..ir::PlanLimits::default()
            },
        ] {
            let budget = ir::PlanBudget::new(limits);
            let (engine, outcome) = run(budget.clone());
            assert!(outcome.is_err(), "{limits:?} must fail");
            assert!(
                engine
                    .fact_of::<Vec<u8>>(&keys::BuildRoot.key(), 0)
                    .is_none(),
                "no executable after a resource failure"
            );
            let _ = budget.usage_snapshot();
        }
    }
}

impl crate::engine::Fact for BodyLexicalFact {}

impl crate::engine::Fact for CaptureEnvironmentFact {}

impl crate::engine::Fact for ClosureBodiesFact {}

impl crate::engine::Fact for ConcreteProofFact {}

impl crate::engine::Fact for InstanceBodyFact {}

impl crate::engine::Fact for OpaqueHiddenTypesFact {}

impl crate::engine::Fact for OpaqueRepresentationFact {}

impl crate::engine::Fact for instances::DirectDemands {}

impl crate::engine::Fact for SsaFact {}

impl crate::engine::Fact for SummaryFact {}

impl crate::engine::Fact for TemplateBodyFact {}

impl crate::engine::Fact for UniversalProofFact {}

impl crate::engine::Fact for traits::ValidatedTraitWorld {}
