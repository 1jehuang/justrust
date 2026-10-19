//! Source-to-native compiler built on the fact graph. See COMPILER.md.
pub mod backend;
mod borrow;
pub mod cfg;
mod expansion_budget;
mod expansion_span;
pub mod frontend;
mod hygiene;
mod instances;
pub mod ir;
mod lexer;
mod literal_scan;
mod macro_cursor;
mod macro_fragments;
mod macro_matcher;
pub mod macro_rules;
mod macro_transcribe;
pub mod module_identity;
pub mod ownership;
mod resource_accounting;
pub mod semantic_name;
pub mod semantic_path;
pub mod semantic_scope;
pub mod source;
mod source_item_macro;
mod source_macro;
mod source_replay;
pub mod ssa;
mod token;
pub mod token_primitives;
mod token_session;
mod token_tree;
pub mod traits;

use crate::{Context, Engine, Error, Key, Provider, Report, TaskFuture};
use std::path::PathBuf;
use std::sync::Arc;

/// Stop formatting immediately when the shared diagnostic allowance is spent.
/// The complete structured analysis remains published in its graph slot.
pub(super) fn analysis_diagnostic(
    budget: &ir::PlanBudget,
    subject: &str,
    index: usize,
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

const PARSE: u32 = 100;
const SIGNATURES: u32 = 101;
const BODY: u32 = 102;
const OBJECT: u32 = 103;
const CHECK: u32 = 104;
const BUILD: u32 = 105;
const RESOLVE: u32 = 106;
const SOURCE: u32 = 107;
const LOWER: u32 = 108;
const ANALYZE: u32 = 109;
const TEMPLATE_BODY: u32 = 110;
const UNIVERSAL_LOWER: u32 = 111;
const UNIVERSAL_ANALYZE: u32 = 112;
const MONO_PLAN: u32 = 113;
const UNIVERSAL_EFFECTS_READY: u32 = 115;

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
    publication: Option<Arc<source_replay::CanonicalSourcePublication>>,
}
/// `UNIVERSAL_ANALYZE` output: the successful verified template proof.
struct UniversalProofFact {
    verified: ownership::VerifiedTemplateCfg,
}
/// `MONO_PLAN` output.
struct PlanFact {
    plan: instances::Plan,
}
/// `BODY` output: exact closed facts and the instantiated checked body.
struct InstanceBodyFact {
    facts: Arc<traits::ValidatedFacts>,
    checked: Arc<ir::CheckedFunction>,
}
/// `ANALYZE` output: the sealed concrete proof and its analysis.
struct ConcreteProofFact {
    concrete: ownership::ConcreteCfg,
}
/// Analysis nodes (`UNIVERSAL_ANALYZE`, `ANALYZE`) have a second output: the
/// complete analysis, published on success and on failure (before the node
/// fails), for diagnostics and reporting. Compiler work reads only output 0.
const ANALYSIS_OUTPUT: usize = 1;
/// `PARSE` outputs: the parsed program, and the assembled prepared source it
/// was parsed from (for source accounting and diagnostics).
const PARSE_OUTPUTS: usize = 2;
const PREPARED_SOURCE_OUTPUT: usize = 1;

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
    fn root(kind: u32) -> Key {
        Key::new(kind, vec![])
    }
    fn item(kind: u32, index: usize) -> Key {
        Key::new(kind, vec![index as u64])
    }
    async fn get<T: std::any::Any + Send + Sync>(&self, key: Key) -> Result<Arc<T>, Error> {
        self.cx.read_fact::<T>(self.cx.request(key).get(0)).await
    }
    async fn types(&self) -> Result<Arc<frontend::PlanCtx>, Error> {
        self.get(Self::root(SIGNATURES)).await
    }
    async fn template(&self, index: usize) -> Result<Arc<TemplateBodyFact>, Error> {
        self.get(Self::item(TEMPLATE_BODY, index)).await
    }
    async fn plan(&self) -> Result<Arc<PlanFact>, Error> {
        self.get(Self::root(MONO_PLAN)).await
    }
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
        let facts = if let Some(publication) = &template.publication {
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
    /// Account retained fact storage in the shared planning ledger.
    fn retain(&self, bytes: usize) -> Result<(), Error> {
        self.0.budget.retain_bytes(bytes).map_err(Error::new)
    }
}
impl Provider for CompilerProvider {
    fn outputs(&self, key: &Key) -> usize {
        match key.kind {
            UNIVERSAL_ANALYZE | ANALYZE => 2,
            PARSE => PARSE_OUTPUTS,
            _ => 1,
        }
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        let this = self.clone();
        Box::pin(async move {
            let facts = Facts { cx: &cx };
            let root = Facts::root;
            let function = Facts::item;
            match key.kind {
                SOURCE => {
                    // One node per source file, keyed by its child-ordinal path
                    // from the root file: [0] is the root, [0, i] its i-th
                    // external child module, and so on.
                    let options = &this.0.source_options;
                    let budget = expansion_budget::ExpansionBudget::default();
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
                        [_] => return Err(Error::new("source root ordinal must be 0")),
                        [parent @ .., index] => {
                            let parent: Arc<source::UnitResult> =
                                facts.get(Key::new(SOURCE, parent.to_vec())).await?;
                            let parent = parent.as_ref().as_ref().map_err(|_| {
                                Error::new("child module requested from a failed source file")
                            })?;
                            source::prepare_child_unit(parent, *index as usize, options, &budget)
                        }
                    };
                    cx.publish_fact(0, unit)
                }
                PARSE => {
                    if matches!(this.0.input, Input::Program(_)) {
                        cx.publish(1, 1)?;
                        return cx.publish(0, 1);
                    }
                    let mut assembler = source::SourceAssembler::new(&this.0.source_options);
                    let root_unit: Arc<source::UnitResult> =
                        facts.get(Key::new(SOURCE, vec![0])).await?;
                    let mut step = assembler.provide(root_unit);
                    let prepared = loop {
                        match step.map_err(|error| Error::new(error.to_string()))? {
                            source::AssemblyStep::Done(prepared) => break *prepared,
                            source::AssemblyStep::Need(need) => {
                                let unit: Arc<source::UnitResult> =
                                    facts.get(Key::new(SOURCE, need.ordinal)).await?;
                                step = assembler.provide(unit);
                            }
                        }
                    };
                    let prepared = match &this.0.input {
                        Input::ProbeFile(_, request) => {
                            source::attach_probe(prepared, &this.0.source_options, request)
                                .map_err(|error| Error::new(error.to_string()))?
                        }
                        _ => prepared,
                    };
                    let parsed = frontend::parse_prepared(&prepared)
                        .map_err(|error| Error::new(error.to_string()));
                    cx.publish_fact(1, prepared)?;
                    cx.publish_fact(0, parsed?)
                }
                RESOLVE => {
                    let program = match &this.0.input {
                        Input::Program(program) => {
                            cx.read(cx.request(root(PARSE)).get(0)).await?;
                            program.clone()
                        }
                        _ => {
                            let parsed: Arc<frontend::ParsedProgram> =
                                facts.get(root(PARSE)).await?;
                            this.retain(
                                std::mem::size_of::<frontend::Program>()
                                    + 2 * std::mem::size_of::<usize>(),
                            )?;
                            Arc::new(frontend::resolve(&parsed).map_err(Error::new)?)
                        }
                    };
                    cx.publish_fact(0, program)
                }
                SIGNATURES => {
                    let program: Arc<Arc<frontend::Program>> = facts.get(root(RESOLVE)).await?;
                    let context = frontend::PlanCtx::from_shared_program(
                        program.as_ref().clone(),
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish_fact(0, context)
                }
                TEMPLATE_BODY => {
                    let index = item_index(&key, "missing declaration index")?;
                    let types = facts.types().await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let body = types
                        .publish_callable_body(declaration, &this.0.budget)
                        .map_err(|error| source_fact_error(error, &this.0.budget))?;
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
                                publication: None,
                            }
                        }
                    };
                    cx.publish_fact(0, fact)
                }
                UNIVERSAL_LOWER => {
                    let index = item_index(&key, "missing declaration index")?;
                    let types = facts.types().await?;
                    let template = facts.template(index).await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    let callable = types.callable_facts(declaration).map_err(Error::new)?;
                    if !Self::universal_body(&types, declaration)? {
                        return Err(Error::new(
                            "universal lowering requires a generic source or registered recipe",
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
                    } else {
                        cfg::lower_template(source, callable, &this.0.budget).map_err(Error::new)?
                    };
                    cx.publish_fact(0, lowered)
                }
                UNIVERSAL_ANALYZE => {
                    let index = item_index(&key, "missing declaration index")?;
                    let lowered: Arc<cfg::TemplateCfg> =
                        facts.get(function(UNIVERSAL_LOWER, index)).await?;
                    match ownership::verify_template(lowered.as_ref().clone()) {
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
                            cx.publish_fact(ANALYSIS_OUTPUT, Arc::new(analysis))?;
                            Err(Error::new(message))
                        }
                    }
                }
                UNIVERSAL_EFFECTS_READY => {
                    let types = facts.types().await?;
                    // Join all body typing and every required universal proof before concrete work.
                    let callable_count = types.callable_count();
                    let count = callable_count
                        .checked_mul(2)
                        .ok_or_else(|| Error::new("generic declaration frontier limit exceeded"))?;
                    let (mut handles, _handles_reservation) = this.scratch_vec(count)?;
                    for index in 0..callable_count {
                        handles.push((
                            index,
                            false,
                            cx.request(function(TEMPLATE_BODY, index)).get(0),
                        ));
                        let declaration = types.callable_decl(index).map_err(Error::new)?;
                        if Self::universal_body(&types, declaration)? {
                            handles.push((
                                index,
                                true,
                                cx.request(function(UNIVERSAL_ANALYZE, index)).get(0),
                            ));
                        }
                    }
                    let (mut templates, _templates_reservation) =
                        this.scratch_vec(callable_count)?;
                    for (_, universal, handle) in handles {
                        if universal {
                            let proof: Arc<UniversalProofFact> = cx.read_fact(handle).await?;
                            templates.push(proof.verified.clone());
                        } else {
                            cx.read(handle).await?;
                        }
                    }
                    let table =
                        ownership::CarrierEffectTable::from_verified(&templates, &this.0.budget)
                            .map_err(Error::new)?;
                    cx.publish_fact(0, table)
                }
                MONO_PLAN => {
                    let types = facts.types().await?;
                    cx.read(cx.request(root(UNIVERSAL_EFFECTS_READY)).get(0))
                        .await?;
                    let callable_count = types.callable_count();
                    let (mut sources, _sources_reservation) = this.scratch_vec(callable_count)?;
                    for index in 0..callable_count {
                        let template = facts.template(index).await?;
                        sources.push(this.validated_body(&types, index, &template)?);
                    }
                    let mut context = types.as_ref().clone();
                    let plan = instances::build(&mut context, &sources, &this.0.budget)
                        .map_err(Error::new)?;
                    cx.publish_fact(0, PlanFact { plan })
                }
                BODY => {
                    let index = item_index(&key, "missing instance index")?;
                    let types = facts.types().await?;
                    let plan = facts.plan().await?;
                    let instance = plan
                        .plan
                        .entries
                        .get(index)
                        .ok_or_else(|| Error::new("invalid instance slot"))?;
                    let template = facts.template(instance.source_index).await?;
                    let source = this.validated_body(&types, instance.source_index, &template)?;
                    let fact = if !Self::universal_body(&types, instance.target.declaration)? {
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("nongeneric Source has no original publication")
                        })?;
                        let closed = publication.facts();
                        if closed.phase() != traits::FactsPhase::Closed
                            || closed.target() != Some(&instance.target)
                            || !closed
                                .is_source_callable(instance.target.declaration)
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
                        let closed = types.instance_facts(&instance.target).map_err(Error::new)?;
                        let mut context = types.as_ref().clone();
                        let proof: Arc<UniversalProofFact> = facts
                            .get(function(UNIVERSAL_ANALYZE, instance.source_index))
                            .await?;
                        let verified = &proof.verified;
                        if closed
                            .is_source_callable(instance.target.declaration)
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
                            ir::instantiate_checked(
                                verified,
                                &instance.target.type_args,
                                &closed,
                                &mut context,
                                &this.0.budget,
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
                LOWER => {
                    let index = item_index(&key, "missing instance index")?;
                    let types = facts.types().await?;
                    let plan = facts.plan().await?;
                    let instance = plan
                        .plan
                        .entries
                        .get(index)
                        .ok_or_else(|| Error::new("invalid instance slot"))?;
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
                    let lowered = if !Self::universal_body(&types, instance.target.declaration)? {
                        let template = facts.template(instance.source_index).await?;
                        let publication = template.publication.as_ref().ok_or_else(|| {
                            Error::new("closed Source lowering has no canonical publication")
                        })?;
                        publication
                            .validate_source(&body.checked, &body.facts, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        cfg::lower_source_closed(publication, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?
                    } else {
                        cfg::lower_with_facts(&body.checked, &body.facts, &this.0.budget)
                            .map_err(Error::new)?
                    };
                    cx.publish_fact(0, Arc::new(lowered))
                }
                ANALYZE => {
                    let index = item_index(&key, "missing instance index")?;
                    let types = facts.types().await?;
                    let plan = facts.plan().await?;
                    let instance = plan
                        .plan
                        .entries
                        .get(index)
                        .ok_or_else(|| Error::new("invalid instance slot"))?;
                    let lowered: Arc<Arc<cfg::Function>> =
                        facts.get(function(LOWER, index)).await?;
                    let lowered = lowered.as_ref().clone();
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
                    if !Self::universal_body(&types, instance.target.declaration)? {
                        let template = facts.template(instance.source_index).await?;
                        lowered
                            .validate_source_publication(
                                template.publication.as_ref().ok_or_else(|| {
                                    Error::new("closed Source CFG omitted canonical publication")
                                })?,
                                &this.0.budget,
                            )
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                    }
                    plan.plan
                        .validate_cleanup_dependencies(&lowered)
                        .map_err(Error::new)?;
                    let closed = cfg::close(lowered, body.facts.clone(), &this.0.budget)
                        .map_err(Error::new)?;
                    let effects: Arc<Arc<ownership::CarrierEffectTable>> =
                        facts.get(root(UNIVERSAL_EFFECTS_READY)).await?;
                    match ownership::analyze_closed_with_effects(closed, effects.as_ref()) {
                        Ok(concrete) => {
                            cx.publish_fact(ANALYSIS_OUTPUT, concrete.analysis().clone())?;
                            cx.publish_fact(0, ConcreteProofFact { concrete })
                        }
                        Err(analysis) => {
                            let message =
                                analysis_diagnostic(&this.0.budget, "instance", index, &analysis);
                            cx.publish_fact(ANALYSIS_OUTPUT, Arc::new(analysis))?;
                            Err(Error::new(message))
                        }
                    }
                }
                OBJECT => {
                    let index = item_index(&key, "missing instance index")?;
                    let plan = facts.plan().await?;
                    // Only this instance's own proof is required. Executable
                    // publication is gated on every check by BUILD (via CHECK),
                    // so object code for a ready function overlaps unrelated
                    // checking (ARCHITECTURE.md section 17).
                    let proof: Arc<ConcreteProofFact> = facts.get(function(ANALYZE, index)).await?;
                    plan.plan
                        .validate_cleanup_dependencies(proof.concrete.function())
                        .map_err(Error::new)?;
                    let object = backend::emit_cfg_function(&proof.concrete, &|target| {
                        plan.plan.symbol(target)
                    })
                    .map_err(Error::new)?;
                    cx.publish_fact(0, object)
                }
                CHECK | BUILD => {
                    let plan = facts.plan().await?;

                    let kind = if key.kind == BUILD { OBJECT } else { ANALYZE };
                    let (mut handles, _handles_reservation) =
                        this.scratch_vec(plan.plan.entries.len())?;
                    for index in 0..plan.plan.entries.len() {
                        handles.push(cx.request(function(kind, index)).get(0));
                    }
                    if key.kind == CHECK {
                        for handle in handles {
                            cx.read(handle).await?;
                        }
                        return cx.publish(0, 1);
                    }
                    let (mut objects, _objects_reservation) =
                        this.scratch_vec(plan.plan.entries.len())?;
                    for (handle, instance) in handles.into_iter().zip(plan.plan.entries.iter()) {
                        let object: Arc<backend::Object> = cx.read_fact(handle).await?;
                        let mut bytes = object.code.len();
                        for (_, name) in &object.calls {
                            bytes = bytes
                                .checked_add(name.len())
                                .and_then(|n| n.checked_add(std::mem::size_of::<(usize, String)>()))
                                .ok_or_else(|| {
                                    Error::new("generic link input byte limit exceeded")
                                })?;
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
                        }
                        this.0.budget.retain_bytes(bytes).map_err(Error::new)?;
                        this.0.budget.charge_work(bytes).map_err(Error::new)?;
                        objects.push((
                            plan.plan.symbol(&instance.target).map_err(Error::new)?,
                            object.as_ref().clone(),
                        ));
                    }
                    // The linked artifact is released only after the explicit
                    // "every required check passed" fact.
                    cx.read(cx.request(root(CHECK)).get(0)).await?;
                    cx.publish_fact(0, backend::link(&objects).map_err(Error::new)?)
                }
                _ => Err(Error::new("unknown compiler operation")),
            }
        })
    }
}
fn item_index(key: &Key, missing: &str) -> Result<usize, Error> {
    let [index] = key.params.as_slice() else {
        return Err(Error::new(missing));
    };
    usize::try_from(*index).map_err(|_| Error::new("invalid item index"))
}

/// One snapshot may serve check and build roots together without redoing checks.
pub struct Compilation {
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
fn retained_analyses(engine: &Engine) -> Vec<(usize, Arc<ownership::Analysis>)> {
    let mut analyses = Vec::new();
    let callables = engine
        .fact_of::<frontend::PlanCtx>(&Key::new(SIGNATURES, vec![]), 0)
        .map_or(0, |types| types.callable_count());
    let instances = engine
        .fact_of::<PlanFact>(&Key::new(MONO_PLAN, vec![]), 0)
        .map_or(0, |plan| plan.plan.entries.len());
    for (kind, count) in [(UNIVERSAL_ANALYZE, callables), (ANALYZE, instances)] {
        for index in 0..count {
            let key = Key::new(kind, vec![index as u64]);
            if let Some(analysis) =
                engine.fact_of::<Arc<ownership::Analysis>>(&key, ANALYSIS_OUTPUT)
            {
                analyses.push((index, analysis.as_ref().clone()));
            }
        }
    }
    analyses
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
    source_options: source::SourceOptions,
) -> Result<Compilation, Failure> {
    let snapshot = Arc::new(Snapshot {
        input,
        source_options,
        budget,
    });
    let engine = Engine::new(CompilerProvider(snapshot.clone()));
    let check = engine.request(Key::new(CHECK, vec![]));
    let build = (!check_only).then(|| engine.request(Key::new(BUILD, vec![])));
    let report = engine.run(workers).map_err(|error| Failure {
        error,
        report: None,
        analyses: Vec::new(),
        resource_usage: Box::new(snapshot.budget.usage_snapshot()),
    })?;
    let failure = |error: Error, report: &Report| Failure {
        error,
        report: Some(Box::new(report.clone())),
        analyses: retained_analyses(&engine),
        resource_usage: Box::new(metered_usage(&snapshot.budget, report)),
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
    let types = engine.fact_of::<frontend::PlanCtx>(&Key::new(SIGNATURES, vec![]), 0);
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
    let (source_bytes, source_files) = match &snapshot.input {
        Input::Program(_) => (0, 0),
        Input::Text(_) | Input::File(_) | Input::ProbeFile(_, _) => {
            let loaded = engine
                .fact_of::<source::PreparedSource>(&Key::new(PARSE, vec![]), PREPARED_SOURCE_OUTPUT)
                .ok_or_else(|| failure(Error::new("missing loaded source accounting"), &report))?;
            (loaded.source_bytes, loaded.files)
        }
    };
    let instances = engine
        .fact_of::<PlanFact>(&Key::new(MONO_PLAN, vec![]), 0)
        .map_or(0, |plan| plan.plan.entries.len());
    let control_flow: Vec<Arc<cfg::Function>> = (0..instances)
        .filter_map(|index| {
            engine.fact_of::<Arc<cfg::Function>>(&Key::new(LOWER, vec![index as u64]), 0)
        })
        .map(|lowered| lowered.as_ref().clone())
        .collect();
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
    let program = engine.fact_of::<Arc<frontend::Program>>(&Key::new(RESOLVE, vec![]), 0);
    let analyses = (0..instances)
        .filter_map(|index| {
            engine.fact_of::<Arc<ownership::Analysis>>(
                &Key::new(ANALYZE, vec![index as u64]),
                ANALYSIS_OUTPUT,
            )
        })
        .map(|analysis| analysis.as_ref().clone())
        .collect();
    let resource_usage = metered_usage(&snapshot.budget, &report);
    Ok(Compilation {
        executable: executable.map(|bytes| bytes.as_ref().clone()),
        report,
        functions,
        structs: program.as_ref().map_or(0, |program| program.structs.len()),
        enums: program.as_ref().map_or(0, |program| program.enums.len()),
        cfg_blocks,
        cfg_edges,
        cfg_locals,
        analyses,
        control_flow,
        source_bytes,
        source_files,
        resource_usage,
    })
}

/// The shared ledger snapshot plus all per-task metered proof work.
fn metered_usage(budget: &ir::PlanBudget, report: &Report) -> ir::PlanUsageSnapshot {
    let mut usage = budget.usage_snapshot();
    usage.work = usage
        .work
        .saturating_add(usize::try_from(report.metered_work).unwrap_or(usize::MAX));
    usage.nodes = usage
        .nodes
        .saturating_add(usize::try_from(report.metered_nodes).unwrap_or(usize::MAX));
    usage.obligations = usage
        .obligations
        .saturating_add(usize::try_from(report.metered_obligations).unwrap_or(usize::MAX));
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
mod diagnostic_tests {
    use super::*;

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
