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
mod source_macro;
mod source_replay;
mod token;
pub mod token_primitives;
mod token_session;
mod token_tree;
pub mod traits;

use crate::{Context, Engine, Error, Key, Provider, Report, TaskFuture};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

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
const CONCRETE_PROOFS: u32 = 114;
const UNIVERSAL_EFFECTS_READY: u32 = 115;

enum Input {
    Text(String),
    File(PathBuf),
    ProbeFile(PathBuf, source::SourceProbeRequest),
    Program(Arc<frontend::Program>),
}
struct DeclarationFacts {
    checked: OnceLock<Arc<ir::CheckedFunction>>,
    publication: OnceLock<Arc<source_replay::CanonicalSourcePublication>>,
    lowered: OnceLock<cfg::TemplateCfg>,
    verified: OnceLock<ownership::VerifiedTemplateCfg>,
    analysis: OnceLock<Arc<ownership::Analysis>>,
}
impl DeclarationFacts {
    fn new() -> Self {
        Self {
            checked: OnceLock::new(),
            publication: OnceLock::new(),
            lowered: OnceLock::new(),
            verified: OnceLock::new(),
            analysis: OnceLock::new(),
        }
    }
}
struct NativeFacts {
    facts: OnceLock<Arc<traits::ValidatedFacts>>,
    checked: OnceLock<Arc<ir::CheckedFunction>>,
    lowered: OnceLock<Arc<cfg::Function>>,
    concrete: OnceLock<ownership::ConcreteCfg>,
    analysis: OnceLock<Arc<ownership::Analysis>>,
    object: OnceLock<backend::Object>,
}
impl NativeFacts {
    fn new() -> Self {
        Self {
            facts: OnceLock::new(),
            checked: OnceLock::new(),
            lowered: OnceLock::new(),
            concrete: OnceLock::new(),
            analysis: OnceLock::new(),
            object: OnceLock::new(),
        }
    }
}
struct Snapshot {
    input: Input,
    source_options: source::SourceOptions,
    budget: ir::PlanBudget,
    reservations: std::sync::Mutex<Vec<ir::PlanReservation>>,
    source: OnceLock<source::PreparedSource>,
    parsed: OnceLock<frontend::ParsedProgram>,
    program: OnceLock<Arc<frontend::Program>>,
    types: OnceLock<frontend::PlanCtx>,
    declarations: OnceLock<Vec<DeclarationFacts>>,
    effects: OnceLock<Arc<ownership::CarrierEffectTable>>,
    plan: OnceLock<instances::Plan>,
    native: OnceLock<Vec<NativeFacts>>,
    executable: OnceLock<Vec<u8>>,
}
#[derive(Clone)]
struct CompilerProvider(Arc<Snapshot>);
impl CompilerProvider {
    fn validated_body(&self, index: usize) -> Result<&Arc<ir::CheckedFunction>, Error> {
        let slot = self.declaration(index)?;
        let source = slot
            .checked
            .get()
            .ok_or_else(|| Error::new("missing original checked body"))?;
        let declaration = self.types()?.callable_decl(index).map_err(Error::new)?;
        let facts = if let Some(publication) = slot.publication.get() {
            publication.facts().clone()
        } else {
            self.types()?
                .callable_facts(declaration)
                .map_err(Error::new)?
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
            let publication = slot
                .publication
                .get()
                .ok_or_else(|| Error::new("native Source body has no canonical publication"))?;
            publication
                .validate_source(source, &facts, &self.0.budget)
                .map_err(|error| source_fact_error(error, &self.0.budget))?;
        } else if slot.publication.get().is_some() {
            return Err(Error::new(
                "Recipe body cannot substitute a Source publication",
            ));
        }
        Ok(source)
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
    fn program(&self) -> Result<&Arc<frontend::Program>, Error> {
        self.0
            .program
            .get()
            .ok_or_else(|| Error::new("missing resolved program"))
    }
    fn universal_body(&self, declaration: ir::DeclId) -> Result<bool, Error> {
        let context = self.types()?;
        match context.callable_provider(declaration).map_err(Error::new)? {
            frontend::CallableProvider::Source { .. } => Ok(context
                .callable_env(declaration)
                .map_err(Error::new)?
                .is_generic()),
            frontend::CallableProvider::Recipe { .. } => Ok(true),
            frontend::CallableProvider::UnavailableProvided { .. } => Err(Error::new(
                "unsupported: unavailable provided body in executable declaration inventory",
            )),
        }
    }
    fn types(&self) -> Result<&frontend::PlanCtx, Error> {
        self.0
            .types
            .get()
            .ok_or_else(|| Error::new("missing declaration registry"))
    }
    fn declaration(&self, index: usize) -> Result<&DeclarationFacts, Error> {
        self.0
            .declarations
            .get()
            .and_then(|v| v.get(index))
            .ok_or_else(|| Error::new("missing declaration slot"))
    }
    fn native(&self, index: usize) -> Result<&NativeFacts, Error> {
        self.0
            .native
            .get()
            .and_then(|v| v.get(index))
            .ok_or_else(|| Error::new("missing native instance slot"))
    }
    fn reserve(&self, bytes: usize) -> Result<(), Error> {
        let bytes = bytes
            .checked_add(4 * std::mem::size_of::<ir::PlanReservation>())
            .ok_or_else(|| Error::new("generic reservation byte limit exceeded"))?;
        let reservation = self.0.budget.reserve_bytes(bytes).map_err(Error::new)?;
        let mut reservations = self
            .0
            .reservations
            .lock()
            .map_err(|_| Error::new("generic reservation lock poisoned"))?;
        reservations
            .try_reserve(1)
            .map_err(|_| Error::new("generic reservation allocation limit exceeded"))?;
        reservations.push(reservation);
        Ok(())
    }
}
fn publish<T>(slot: &OnceLock<T>, value: T) -> Result<(), Error> {
    slot.set(value)
        .map_err(|_| Error::new("duplicate compiler fact"))
}
impl Provider for CompilerProvider {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        let this = self.clone();
        Box::pin(async move {
            let root = |kind| Key::new(kind, vec![]);
            let function = |kind, index: usize| Key::new(kind, vec![index as u64]);
            match key.kind {
                SOURCE => {
                    let prepared = match &this.0.input {
                        Input::File(path) => source::prepare_file(path, &this.0.source_options),
                        Input::ProbeFile(path, request) => {
                            source::prepare_probe_file(path, &this.0.source_options, request)
                        }
                        Input::Text(text) => {
                            source::prepare_text("<text>", text, &this.0.source_options)
                        }
                        Input::Program(_) => {
                            return Err(Error::new("prepared program has no source input"));
                        }
                    };
                    publish(
                        &this.0.source,
                        prepared.map_err(|error| Error::new(error.to_string()))?,
                    )?;
                }
                PARSE => {
                    if !matches!(this.0.input, Input::Program(_)) {
                        cx.read(cx.request(root(SOURCE)).get(0)).await?;
                        let prepared = this
                            .0
                            .source
                            .get()
                            .ok_or_else(|| Error::new("missing source snapshot"))?;
                        publish(
                            &this.0.parsed,
                            frontend::parse_prepared(prepared)
                                .map_err(|error| Error::new(error.to_string()))?,
                        )?;
                    }
                }
                RESOLVE => {
                    cx.read(cx.request(root(PARSE)).get(0)).await?;
                    let program = match &this.0.input {
                        Input::Program(program) => program.clone(),
                        _ => {
                            this.0
                                .budget
                                .retain_bytes(
                                    std::mem::size_of::<frontend::Program>()
                                        + 2 * std::mem::size_of::<usize>(),
                                )
                                .map_err(Error::new)?;
                            Arc::new(
                                frontend::resolve(
                                    this.0
                                        .parsed
                                        .get()
                                        .ok_or_else(|| Error::new("missing parsed snapshot"))?,
                                )
                                .map_err(Error::new)?,
                            )
                        }
                    };
                    publish(&this.0.program, program)?;
                }
                SIGNATURES => {
                    cx.read(cx.request(root(RESOLVE)).get(0)).await?;
                    let context = frontend::PlanCtx::from_shared_program(
                        this.program()?.clone(),
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    let count = context.callable_count();
                    this.reserve(
                        count
                            .checked_mul(std::mem::size_of::<DeclarationFacts>())
                            .ok_or_else(|| Error::new("generic declaration byte limit exceeded"))?,
                    )?;
                    let mut declarations = Vec::new();
                    declarations
                        .try_reserve_exact(count)
                        .map_err(|_| Error::new("generic declaration allocation limit exceeded"))?;
                    declarations.extend((0..count).map(|_| DeclarationFacts::new()));
                    publish(&this.0.declarations, declarations)?;
                    publish(&this.0.types, context)?;
                }
                TEMPLATE_BODY | UNIVERSAL_LOWER | UNIVERSAL_ANALYZE => {
                    let [index] = key.params.as_slice() else {
                        return Err(Error::new("missing declaration index"));
                    };
                    let index = usize::try_from(*index)
                        .map_err(|_| Error::new("invalid declaration index"))?;
                    cx.read(cx.request(root(SIGNATURES)).get(0)).await?;
                    let slot = this.declaration(index)?;
                    if key.kind == TEMPLATE_BODY {
                        let declaration = this.types()?.callable_decl(index).map_err(Error::new)?;
                        let body = this
                            .types()?
                            .publish_callable_body(declaration, &this.0.budget)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        match body {
                            frontend::PublishedCallableBody::Source(publication) => {
                                if !publication
                                    .facts()
                                    .is_source_callable(declaration)
                                    .map_err(|error| source_fact_error(error, &this.0.budget))?
                                {
                                    return Err(Error::new(
                                        "Source publication has a Recipe provider",
                                    ));
                                }
                                publication
                                    .validate_source(
                                        publication.source(),
                                        publication.facts(),
                                        &this.0.budget,
                                    )
                                    .map_err(|error| source_fact_error(error, &this.0.budget))?;
                                publish(&slot.checked, publication.source().clone())?;
                                publish(&slot.publication, publication)?;
                            }
                            frontend::PublishedCallableBody::Recipe(checked) => {
                                let facts = this
                                    .types()?
                                    .callable_facts(declaration)
                                    .map_err(Error::new)?;
                                if facts
                                    .is_source_callable(declaration)
                                    .map_err(|error| source_fact_error(error, &this.0.budget))?
                                {
                                    return Err(Error::new(
                                        "Source provider omitted canonical publication",
                                    ));
                                }
                                publish(&slot.checked, checked)?;
                            }
                        }
                    } else if key.kind == UNIVERSAL_LOWER {
                        cx.read(cx.request(function(TEMPLATE_BODY, index)).get(0))
                            .await?;
                        let declaration = this.types()?.callable_decl(index).map_err(Error::new)?;
                        let facts = this
                            .types()?
                            .callable_facts(declaration)
                            .map_err(Error::new)?;
                        if !this.universal_body(declaration)? {
                            return Err(Error::new(
                                "universal lowering requires a generic source or registered recipe",
                            ));
                        }
                        let source = this.validated_body(index)?.clone();
                        let lowered = if facts
                            .is_source_callable(declaration)
                            .map_err(|error| source_fact_error(error, &this.0.budget))?
                        {
                            cfg::lower_source_template(
                                slot.publication.get().ok_or_else(|| {
                                    Error::new("missing canonical universal Source publication")
                                })?,
                                &this.0.budget,
                            )
                            .map_err(|error| source_fact_error(error, &this.0.budget))?
                        } else {
                            cfg::lower_template(source, facts, &this.0.budget)
                                .map_err(Error::new)?
                        };
                        publish(&slot.lowered, lowered)?;
                    } else {
                        cx.read(cx.request(function(UNIVERSAL_LOWER, index)).get(0))
                            .await?;
                        let lowered = slot
                            .lowered
                            .get()
                            .ok_or_else(|| Error::new("missing universal CFG"))?
                            .clone();
                        match ownership::verify_template(lowered) {
                            Ok(proof) => {
                                publish(&slot.analysis, proof.analysis().clone())?;
                                publish(&slot.verified, proof)?;
                            }
                            Err(analysis) => {
                                let message = analysis_diagnostic(
                                    &this.0.budget,
                                    "universal generic declaration",
                                    index,
                                    &analysis,
                                );
                                publish(&slot.analysis, Arc::new(analysis))?;
                                return Err(Error::new(message));
                            }
                        }
                    }
                }
                UNIVERSAL_EFFECTS_READY => {
                    cx.read(cx.request(root(SIGNATURES)).get(0)).await?;
                    // Join all body typing and every required universal proof before concrete work.
                    let callable_count = this.types()?.callable_count();
                    let count = callable_count
                        .checked_mul(2)
                        .ok_or_else(|| Error::new("generic declaration frontier limit exceeded"))?;
                    let (mut handles, _handles_reservation) = this.scratch_vec(count)?;
                    for index in 0..callable_count {
                        handles.push(cx.request(function(TEMPLATE_BODY, index)).get(0));
                        let declaration = this.types()?.callable_decl(index).map_err(Error::new)?;
                        if this.universal_body(declaration)? {
                            handles.push(cx.request(function(UNIVERSAL_ANALYZE, index)).get(0));
                        }
                    }
                    for handle in handles {
                        cx.read(handle).await?;
                    }
                    let (mut templates, _templates_reservation) =
                        this.scratch_vec(callable_count)?;
                    for index in 0..callable_count {
                        let declaration = this.types()?.callable_decl(index).map_err(Error::new)?;
                        if this.universal_body(declaration)? {
                            templates.push(
                                this.declaration(index)?
                                    .verified
                                    .get()
                                    .cloned()
                                    .ok_or_else(|| {
                                        Error::new("missing universal effect proof source")
                                    })?,
                            );
                        }
                    }
                    publish(
                        &this.0.effects,
                        ownership::CarrierEffectTable::from_verified(&templates, &this.0.budget)
                            .map_err(Error::new)?,
                    )?;
                }
                MONO_PLAN => {
                    cx.read(cx.request(root(UNIVERSAL_EFFECTS_READY)).get(0))
                        .await?;
                    let callable_count = this.types()?.callable_count();
                    let (mut sources, _sources_reservation) = this.scratch_vec(callable_count)?;
                    for (index, _slot) in this
                        .0
                        .declarations
                        .get()
                        .ok_or_else(|| Error::new("missing declaration facts"))?
                        .iter()
                        .enumerate()
                    {
                        sources.push(this.validated_body(index)?.clone());
                    }
                    let mut context = this.types()?.clone();
                    let plan = instances::build(&mut context, &sources, &this.0.budget)
                        .map_err(Error::new)?;
                    this.reserve(
                        plan.entries
                            .len()
                            .checked_mul(std::mem::size_of::<NativeFacts>())
                            .ok_or_else(|| Error::new("generic native fact byte limit exceeded"))?,
                    )?;
                    let mut native = Vec::new();
                    native
                        .try_reserve_exact(plan.entries.len())
                        .map_err(|_| Error::new("generic native fact allocation limit exceeded"))?;
                    native.extend(plan.entries.iter().map(|_| NativeFacts::new()));
                    publish(&this.0.native, native)?;
                    publish(&this.0.plan, plan)?;
                }
                CONCRETE_PROOFS => {
                    cx.read(cx.request(root(MONO_PLAN)).get(0)).await?;
                    let plan = this
                        .0
                        .plan
                        .get()
                        .ok_or_else(|| Error::new("missing instance plan"))?;
                    let (mut handles, _handles_reservation) =
                        this.scratch_vec(plan.entries.len())?;
                    for index in 0..plan.entries.len() {
                        handles.push(cx.request(function(ANALYZE, index)).get(0));
                    }
                    for handle in handles {
                        cx.read(handle).await?;
                    }
                }
                BODY | LOWER | ANALYZE | OBJECT => {
                    let [index] = key.params.as_slice() else {
                        return Err(Error::new("missing instance index"));
                    };
                    let index = usize::try_from(*index)
                        .map_err(|_| Error::new("invalid instance index"))?;
                    cx.read(cx.request(root(MONO_PLAN)).get(0)).await?;
                    let plan = this
                        .0
                        .plan
                        .get()
                        .ok_or_else(|| Error::new("missing sealed instance plan"))?;
                    let instance = plan
                        .entries
                        .get(index)
                        .ok_or_else(|| Error::new("invalid instance slot"))?;
                    let slot = this.native(index)?;
                    if key.kind == BODY {
                        let declaration = this.declaration(instance.source_index)?;
                        let source = this.validated_body(instance.source_index)?;
                        let (facts, checked) = if !this
                            .universal_body(instance.target.declaration)?
                        {
                            let publication = declaration.publication.get().ok_or_else(|| {
                                Error::new("nongeneric Source has no original publication")
                            })?;
                            let facts = publication.facts();
                            if facts.phase() != traits::FactsPhase::Closed
                                || facts.target() != Some(&instance.target)
                                || !facts
                                    .is_source_callable(instance.target.declaration)
                                    .map_err(|error| source_fact_error(error, &this.0.budget))?
                            {
                                return Err(Error::new(
                                    "nongeneric Source publication changed its closed target",
                                ));
                            }
                            publication
                                .validate_source(source, facts, &this.0.budget)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?;
                            (facts.clone(), source.clone())
                        } else {
                            let facts = this
                                .types()?
                                .instance_facts(&instance.target)
                                .map_err(Error::new)?;
                            let mut context = this.types()?.clone();
                            let verified = declaration.verified.get().ok_or_else(|| {
                                Error::new("missing exact universal template proof")
                            })?;
                            if facts
                                .is_source_callable(instance.target.declaration)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?
                            {
                                verified.function().validate_source_publication(
                                    declaration.publication.get().ok_or_else(|| {
                                        Error::new("generic Source template omitted canonical publication")
                                    })?,
                                    &this.0.budget,
                                ).map_err(|error| source_fact_error(error, &this.0.budget))?;
                            }
                            let checked = Arc::new(
                                ir::instantiate_checked(
                                    verified,
                                    &instance.target.type_args,
                                    &facts,
                                    &mut context,
                                    &this.0.budget,
                                )
                                .map_err(Error::new)?,
                            );
                            (facts, checked)
                        };
                        publish(&slot.facts, facts)?;
                        publish(&slot.checked, checked)?;
                    } else if key.kind == LOWER {
                        cx.read(cx.request(function(BODY, index)).get(0)).await?;
                        let checked = slot
                            .checked
                            .get()
                            .ok_or_else(|| Error::new("missing instantiated body"))?;
                        let lowered = if !this.universal_body(instance.target.declaration)? {
                            let declaration = this.declaration(instance.source_index)?;
                            let publication = declaration.publication.get().ok_or_else(|| {
                                Error::new("closed Source lowering has no canonical publication")
                            })?;
                            publication
                                .validate_source(
                                    checked,
                                    slot.facts
                                        .get()
                                        .ok_or_else(|| Error::new("missing closed Source facts"))?,
                                    &this.0.budget,
                                )
                                .map_err(|error| source_fact_error(error, &this.0.budget))?;
                            cfg::lower_source_closed(publication, &this.0.budget)
                                .map_err(|error| source_fact_error(error, &this.0.budget))?
                        } else {
                            cfg::lower_with_facts(
                                checked,
                                slot.facts.get().ok_or_else(|| {
                                    Error::new("missing exact closed instance facts")
                                })?,
                                &this.0.budget,
                            )
                            .map_err(Error::new)?
                        };
                        publish(&slot.lowered, Arc::new(lowered))?;
                    } else if key.kind == ANALYZE {
                        cx.read(cx.request(function(LOWER, index)).get(0)).await?;
                        let lowered = slot
                            .lowered
                            .get()
                            .ok_or_else(|| Error::new("missing concrete CFG"))?
                            .clone();
                        let facts = slot
                            .facts
                            .get()
                            .cloned()
                            .ok_or_else(|| Error::new("missing exact closed instance facts"))?;
                        if !this.universal_body(instance.target.declaration)? {
                            lowered
                                .validate_source_publication(
                                    this.declaration(instance.source_index)?
                                        .publication
                                        .get()
                                        .ok_or_else(|| {
                                            Error::new(
                                                "closed Source CFG omitted canonical publication",
                                            )
                                        })?,
                                    &this.0.budget,
                                )
                                .map_err(|error| source_fact_error(error, &this.0.budget))?;
                        }
                        let closed =
                            cfg::close(lowered, facts, &this.0.budget).map_err(Error::new)?;
                        let effects =
                            this.0.effects.get().ok_or_else(|| {
                                Error::new("missing universal effect publication")
                            })?;
                        match ownership::analyze_closed_with_effects(closed, effects) {
                            Ok(proof) => {
                                publish(&slot.analysis, proof.analysis().clone())?;
                                publish(&slot.concrete, proof)?;
                            }
                            Err(analysis) => {
                                let message = analysis_diagnostic(
                                    &this.0.budget,
                                    "instance",
                                    index,
                                    &analysis,
                                );
                                publish(&slot.analysis, Arc::new(analysis))?;
                                return Err(Error::new(message));
                            }
                        }
                    } else {
                        cx.read(cx.request(root(CONCRETE_PROOFS)).get(0)).await?;
                        cx.read(cx.request(function(ANALYZE, index)).get(0)).await?;
                        let concrete = slot
                            .concrete
                            .get()
                            .ok_or_else(|| Error::new("missing exact concrete analysis seal"))?;
                        let object =
                            backend::emit_cfg_function(concrete, &|target| plan.symbol(target))
                                .map_err(Error::new)?;
                        publish(&slot.object, object)?;
                    }
                }
                CHECK | BUILD => {
                    cx.read(cx.request(root(MONO_PLAN)).get(0)).await?;
                    if key.kind == BUILD {
                        // No object task is requested until every concrete
                        // body has passed the same sealed analysis path.
                        cx.read(cx.request(root(CONCRETE_PROOFS)).get(0)).await?;
                    }
                    let plan = this
                        .0
                        .plan
                        .get()
                        .ok_or_else(|| Error::new("missing instance plan"))?;
                    let kind = if key.kind == BUILD { OBJECT } else { ANALYZE };
                    let (mut handles, _handles_reservation) =
                        this.scratch_vec(plan.entries.len())?;
                    for index in 0..plan.entries.len() {
                        handles.push(cx.request(function(kind, index)).get(0));
                    }
                    for handle in handles {
                        cx.read(handle).await?;
                    }
                    if key.kind == BUILD {
                        let (mut objects, _objects_reservation) =
                            this.scratch_vec(plan.entries.len())?;
                        for (index, instance) in plan.entries.iter().enumerate() {
                            let object =
                                this.native(index)?.object.get().ok_or_else(|| {
                                    Error::new("unpublished native instance object")
                                })?;
                            let mut bytes = object.code.len();
                            for (_, name) in &object.calls {
                                bytes = bytes
                                    .checked_add(name.len())
                                    .and_then(|n| {
                                        n.checked_add(std::mem::size_of::<(usize, String)>())
                                    })
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
                                plan.symbol(&instance.target).map_err(Error::new)?,
                                object.clone(),
                            ));
                        }
                        publish(
                            &this.0.executable,
                            backend::link(&objects).map_err(Error::new)?,
                        )?;
                    }
                }
                _ => return Err(Error::new("unknown compiler operation")),
            }
            cx.publish(0, 1)
        })
    }
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

fn retained_analyses(snapshot: &Snapshot) -> Vec<(usize, Arc<ownership::Analysis>)> {
    let mut analyses = Vec::new();
    if let Some(slots) = snapshot.declarations.get() {
        for (index, slot) in slots.iter().enumerate() {
            if let Some(analysis) = slot.analysis.get() {
                analyses.push((index, analysis.clone()));
            }
        }
    }
    if let Some(slots) = snapshot.native.get() {
        for (index, slot) in slots.iter().enumerate() {
            if let Some(analysis) = slot.analysis.get() {
                analyses.push((index, analysis.clone()));
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
        source: OnceLock::new(),
        parsed: OnceLock::new(),
        program: OnceLock::new(),
        types: OnceLock::new(),
        budget,
        reservations: std::sync::Mutex::new(Vec::new()),
        declarations: OnceLock::new(),
        effects: OnceLock::new(),
        plan: OnceLock::new(),
        native: OnceLock::new(),
        executable: OnceLock::new(),
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
    let outcome = (|| -> Result<(), Error> {
        check.completion()?;
        check.result(0)?;
        if let Some(build) = build {
            build.completion()?;
            build.result(0)?;
        }
        if !report.capture_complete {
            return Err(Error::new("compiler graph capture incomplete"));
        }
        if !check_only && snapshot.executable.get().is_none() {
            return Err(Error::new("missing native executable"));
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        return Err(Failure {
            error,
            report: Some(Box::new(report)),
            analyses: retained_analyses(&snapshot),
            resource_usage: Box::new(snapshot.budget.usage_snapshot()),
        });
    }
    // Public source telemetry remains source-only. Registered recipe bodies are
    // additional universal proof subjects, not newly discovered source functions.
    let functions = (|| -> Result<usize, String> {
        let Some(context) = snapshot.types.get() else {
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
    .map_err(|message| Failure {
        error: Error::new(message),
        report: Some(Box::new(report.clone())),
        analyses: retained_analyses(&snapshot),
        resource_usage: Box::new(snapshot.budget.usage_snapshot()),
    })?;
    let executable = snapshot.executable.get().cloned();
    let (source_bytes, source_files) = match &snapshot.input {
        Input::Program(_) => (0, 0),
        Input::Text(_) | Input::File(_) | Input::ProbeFile(_, _) => {
            let loaded = snapshot.source.get().ok_or_else(|| Failure {
                error: Error::new("missing loaded source accounting"),
                report: Some(Box::new(report.clone())),
                analyses: retained_analyses(&snapshot),
                resource_usage: Box::new(snapshot.budget.usage_snapshot()),
            })?;
            (loaded.source_bytes, loaded.files)
        }
    };
    let control_flow: Vec<_> = snapshot
        .native
        .get()
        .into_iter()
        .flatten()
        .filter_map(|slot| slot.lowered.get())
        .cloned()
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
                cfg::Terminator::Goto(_) | cfg::Terminator::Call { .. } => 1,
                cfg::Terminator::Switch { .. } | cfg::Terminator::SwitchVariant { .. } => 2,
                cfg::Terminator::Return | cfg::Terminator::Unreachable => 0,
            })
            .sum::<usize>();
    }
    Ok(Compilation {
        executable,
        report,
        functions,
        structs: snapshot
            .program
            .get()
            .map_or(0, |program| program.structs.len()),
        enums: snapshot
            .program
            .get()
            .map_or(0, |program| program.enums.len()),
        cfg_blocks,
        cfg_edges,
        cfg_locals,
        analyses: snapshot
            .native
            .get()
            .into_iter()
            .flatten()
            .filter_map(|slot| slot.analysis.get())
            .cloned()
            .collect(),
        control_flow,
        source_bytes,
        source_files,
        resource_usage: snapshot.budget.usage_snapshot(),
    })
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
