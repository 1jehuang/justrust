//! Source-to-native compiler built on the fact graph. See COMPILER.md.
pub mod backend;
mod borrow;
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
mod lexer;
mod literal_scan;
mod macro_cursor;
mod macro_fragments;
mod macro_matcher;
pub mod macro_rules;
mod macro_transcribe;
pub mod module_identity;
pub mod owner_path;
pub mod ownership;
mod resource_accounting;
mod root_cleanup;
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
const BODY_LEXICAL: u32 = 116;
const CAPTURE_ENV: u32 = 117;
const CONST_DECL: u32 = 118;
const CONST_BODY: u32 = 119;
const CONST_VALUE: u32 = 120;
const ROOT_CLEANUP_DEMAND: u32 = 125;
/// Optimizer nodes (OPTIMIZATION.md section 8). Per instance index; only BUILD
/// (through OBJECT) demands them, and never when the mode is `Off`.
const SSA: u32 = 130;
const LOCAL_OPT: u32 = 131;
const SUMMARY: u32 = 132;
const IPO_OPT: u32 = 133;

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
/// `SSA`, `LOCAL_OPT` and `IPO_OPT` output. `None` means the function is
/// outside the optimizer's current subset and keeps the stack emitter.
///
/// Each published fact carries an immutable receipt for the storage it keeps
/// alive: the body's reconciled capacity plus its `Arc` allocation and the
/// engine's fact envelope. A fact that re-publishes a shared body pays only
/// for its own envelope. No mutable budget is reachable from the fact.
struct SsaFact {
    lowered: Option<Arc<ssa::lower::Lowered>>,
    unsupported: Option<&'static str>,
    inlined: usize,
    _receipt: Option<ssa::meter::Receipt>,
}
impl SsaFact {
    /// Bytes of the fact itself plus the `Arc` holding it in the engine.
    const ENVELOPE: usize = std::mem::size_of::<SsaFact>() + 2 * std::mem::size_of::<usize>();
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
        let (lowered, receipt) = owned.seal().map_err(Error::new)?;
        Ok(Self {
            lowered: Some(Arc::new(lowered)),
            unsupported: None,
            inlined,
            _receipt: Some(receipt),
        })
    }

    fn share(
        lowered: Arc<ssa::lower::Lowered>,
        meter: &ssa::meter::Meter<'_>,
    ) -> Result<Self, Error> {
        let receipt = meter
            .lease(Self::ENVELOPE)
            .and_then(ssa::meter::Lease::seal)
            .map_err(Error::new)?;
        Ok(Self {
            lowered: Some(lowered),
            unsupported: None,
            inlined: 0,
            _receipt: Some(receipt),
        })
    }

    fn unsupported(reason: &'static str) -> Self {
        // No owned storage beyond the envelope, which the engine's own
        // publication accounting covers for every fact.
        Self {
            lowered: None,
            unsupported: Some(reason),
            inlined: 0,
            _receipt: None,
        }
    }
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
    /// The target and source declaration of instance `index`. Nongeneric
    /// Source roots occupy the first plan slots in callable order, so they
    /// are derived from SIGNATURES alone; only demanded generic instances
    /// read the whole-program instance plan.
    async fn instance(&self, index: usize) -> Result<(ir::CallTarget, usize), Error> {
        let types = self.types().await?;
        if let Some(root) = root_instance(&types, index)? {
            return Ok(root);
        }
        let plan = self.plan().await?;
        let instance = plan
            .plan
            .entries
            .get(index)
            .ok_or_else(|| Error::new("invalid instance slot"))?;
        Ok((instance.target.clone(), instance.source_index))
    }
}
/// Nongeneric Source callables in callable order are the plan's root slots
/// (instances::build enumerates them first, in this order). Returns the
/// `index`-th one, or `None` if `index` is past the roots.
fn root_instance(
    types: &frontend::PlanCtx,
    index: usize,
) -> Result<Option<(ir::CallTarget, usize)>, Error> {
    let roots = types.closed_roots();
    Ok(roots.get(index).map(|&source_index| {
        (
            ir::CallTarget {
                declaration: types
                    .callable_decl(source_index)
                    .expect("closed root index is a callable"),
                type_args: Arc::from([]),
            },
            source_index,
        )
    }))
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
            UNIVERSAL_ANALYZE | ANALYZE | CONST_BODY => 2,
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
                CONST_DECL | CONST_BODY | CONST_VALUE => this.constant_node(&cx, &key).await,
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
                        // [1] attaches the embedded native prelude library.
                        [1] => source::prepare_prelude_unit(options, &budget),
                        [_] => return Err(Error::new("source root ordinal must be 0 or 1")),
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
                    // Graph route: the session owner is issued from this PARSE
                    // node's own root, never a fresh standalone mint.
                    let parse_budget = expansion_budget::ExpansionBudget::default();
                    let mut issuer = owner_path::OwnerIssuer::new(cx.owner_root()?, &parse_budget)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let owner = issuer
                        .issue(owner_path::OwnerDomain::TokenSession)
                        .map_err(|error| Error::new(format!("owner issuance failed: {error:?}")))?;
                    let parsed = frontend::parse_prepared_owned(&prepared, parse_budget, owner)
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
                    let index = item_index(&key, "missing declaration index")?;
                    let types = facts.types().await?;
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
                                facts.get(Key::new(CAPTURE_ENV, parameters)).await?;
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
                    let (mut values, _values_storage) =
                        this.scratch_vec::<Arc<const_check::ValueFact>>(constant_handles.len())?;
                    for handle in constant_handles {
                        values.push(cx.read_fact(handle).await?);
                    }
                    let constants = types
                        .seal_body_constants(prepared, values, &this.0.budget)
                        .map_err(|error| source_fact_error(error, &this.0.budget))?;
                    let body = types
                        .publish_callable_body_with_constants(
                            declaration,
                            constants,
                            &this.0.budget,
                        )
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
                    let index = item_index(&key, "missing declaration index")?;
                    let types = facts.types().await?;
                    let declaration = types.callable_decl(index).map_err(Error::new)?;
                    if !Self::universal_body(&types, declaration)? {
                        return Err(Error::new("non-universal callable has no admission fact"));
                    }
                    let proof: Arc<UniversalProofFact> =
                        facts.get(function(UNIVERSAL_ANALYZE, index)).await?;
                    let mut callee_indices = Vec::new();
                    for expression in &proof.verified.source().exprs {
                        this.0.budget.charge_work(1).map_err(Error::new)?;
                        let ir::ExprKind::Call { target, .. } = &expression.kind else {
                            continue;
                        };
                        let actuals = match target {
                            ir::Callee::Direct(target) => target.type_args.as_ref(),
                            ir::Callee::TraitItem(call)
                            | ir::Callee::Virtual(ir::VirtualCall { call }) => {
                                call.method_args.as_ref()
                            }
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
                            callee_indices.push(callee);
                        }
                    }
                    callee_indices.sort_unstable();
                    callee_indices.dedup();
                    let handles: Vec<_> = callee_indices
                        .iter()
                        .map(|callee| cx.request(function(UNIVERSAL_ANALYZE, *callee)).get(0))
                        .collect();
                    let mut templates = Vec::with_capacity(handles.len());
                    for handle in handles {
                        let proof: Arc<UniversalProofFact> = cx.read_fact(handle).await?;
                        templates.push(proof.verified.clone());
                    }
                    ownership::CarrierEffectTable::admit_reference_actual_calls(
                        &proof.verified,
                        &templates,
                        &this.0.budget,
                    )
                    .map_err(Error::new)?;
                    cx.publish(0, 1)
                }
                MONO_PLAN => {
                    let types = facts.types().await?;
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
                    let (instance_target, instance_source) = facts.instance(index).await?;
                    let template = facts.template(instance_source).await?;
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
                        let proof: Arc<UniversalProofFact> = facts
                            .get(function(UNIVERSAL_ANALYZE, instance_source))
                            .await?;
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
                            ir::instantiate_checked(
                                verified,
                                &instance_target.type_args,
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
                ROOT_CLEANUP_DEMAND => {
                    let index = item_index(&key, "missing closed root index")?;
                    let types = facts.types().await?;
                    let (target, source) = root_instance(&types, index)?.ok_or_else(|| {
                        Error::new("cleanup demand is restricted to closed roots")
                    })?;
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
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
                    let index = item_index(&key, "missing instance index")?;
                    let types = facts.types().await?;
                    let (instance_target, instance_source) = facts.instance(index).await?;
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
                    let lowered = if !Self::universal_body(&types, instance_target.declaration)? {
                        let template = facts.template(instance_source).await?;
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
                    let (instance_target, instance_source) = facts.instance(index).await?;
                    let lowered: Arc<Arc<cfg::Function>> =
                        facts.get(function(LOWER, index)).await?;
                    let lowered = lowered.as_ref().clone();
                    let body: Arc<InstanceBodyFact> = facts.get(function(BODY, index)).await?;
                    if !Self::universal_body(&types, instance_target.declaration)? {
                        let template = facts.template(instance_source).await?;
                        lowered
                            .validate_source_publication(
                                template.publication.as_ref().ok_or_else(|| {
                                    Error::new("closed Source CFG omitted canonical publication")
                                })?,
                                &this.0.budget,
                            )
                            .map_err(|error| source_fact_error(error, &this.0.budget))?;
                    }
                    if has_drop(&lowered) {
                        if root_instance(&types, index)?.is_some() {
                            let demand: Arc<root_cleanup::RootCleanupDemand> =
                                facts.get(function(ROOT_CLEANUP_DEMAND, index)).await?;
                            demand
                                .validate(&instance_target, &lowered, &this.0.budget)
                                .map_err(Error::new)?;
                        } else {
                            facts
                                .plan()
                                .await?
                                .plan
                                .validate_cleanup_dependencies(&lowered)
                                .map_err(Error::new)?;
                        }
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
                            callee_indices.push(callee);
                        }
                    }
                    callee_indices.sort_unstable();
                    callee_indices.dedup();
                    let handles: Vec<_> = callee_indices
                        .iter()
                        .map(|callee| cx.request(function(UNIVERSAL_ANALYZE, *callee)).get(0))
                        .collect();
                    let mut templates = Vec::with_capacity(handles.len());
                    for handle in handles {
                        let proof: Arc<UniversalProofFact> = cx.read_fact(handle).await?;
                        templates.push(proof.verified.clone());
                    }
                    let effects =
                        ownership::CarrierEffectTable::for_callees(&templates, &this.0.budget)
                            .map_err(Error::new)?;
                    match ownership::analyze_closed_with_effects(closed, &effects) {
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
                    // Only this instance's own proof is required. Executable
                    // publication is gated on every check by BUILD (via CHECK),
                    // so object code for a ready function overlaps unrelated
                    // checking (ARCHITECTURE.md section 17).
                    let proof: Arc<ConcreteProofFact> = facts.get(function(ANALYZE, index)).await?;
                    if has_drop(proof.concrete.function()) {
                        let types = facts.types().await?;
                        if let Some((target, _)) = root_instance(&types, index)? {
                            let demand: Arc<root_cleanup::RootCleanupDemand> =
                                facts.get(function(ROOT_CLEANUP_DEMAND, index)).await?;
                            demand
                                .validate(&target, proof.concrete.function(), &this.0.budget)
                                .map_err(Error::new)?;
                        } else {
                            facts
                                .plan()
                                .await?
                                .plan
                                .validate_cleanup_dependencies(proof.concrete.function())
                                .map_err(Error::new)?;
                        }
                    }
                    // Callee symbols derive from targets alone. BUILD links
                    // against the sealed plan, which rejects any unplanned
                    // symbol, so this needs no whole-program read.
                    let types = facts.types().await?;
                    if this.0.source_options.opt != ssa::OptMode::Off {
                        let optimized: Arc<SsaFact> = facts.get(function(IPO_OPT, index)).await?;
                        if let Some(lowered) = &optimized.lowered {
                            let meter = ssa::meter::Meter::new(&this.0.budget);
                            let types = lowered
                                .verify(&meter)
                                .map_err(|error| Error::new(error.to_string()))?;
                            let budget = &this.0.budget;
                            let (object, _stats) = ssa::machine::emit(
                                lowered,
                                &types,
                                &|text| {
                                    backend::panic_object(&text.message, &text.location, budget)
                                },
                                &meter,
                            )
                            .map_err(Error::new)?;
                            drop(types);
                            // The object's storage is paid by the reconciled
                            // lease; the sealed immutable receipt travels inside
                            // the published object for exactly its lifetime.
                            let (mut object, receipt) = object.seal().map_err(Error::new)?;
                            object.receipt = Some(Arc::new(receipt));
                            return cx.publish_fact(0, object);
                        }
                    }
                    let object = backend::emit_cfg_function(&proof.concrete, &|target| {
                        instances::symbol_of(target, &types, &this.0.budget)
                    })
                    .map_err(Error::new)?;
                    cx.publish_fact(0, object)
                }
                SSA => {
                    let index = item_index(&key, "missing instance index")?;
                    let proof: Arc<ConcreteProofFact> = facts.get(function(ANALYZE, index)).await?;
                    let types = facts.types().await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    let function = proof.concrete.function();
                    let fact = match ssa::lower::lower(
                        function,
                        &|target| instances::symbol_of(target, &types, &this.0.budget),
                        &meter,
                    )
                    .map_err(Error::new)?
                    {
                        ssa::lower::Lowering::Lowered(lowered) => {
                            SsaFact::publish(*lowered, 0, &meter)?
                        }
                        ssa::lower::Lowering::Unsupported(reason) => SsaFact::unsupported(reason),
                    };
                    cx.publish_fact(0, fact)
                }
                LOCAL_OPT => {
                    let index = item_index(&key, "missing instance index")?;
                    let input: Arc<SsaFact> = facts.get(function(SSA, index)).await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    let fact = match &input.lowered {
                        Some(lowered) => SsaFact::publish(local_opt(lowered, &meter)?, 0, &meter)?,
                        None => SsaFact::unsupported(input.unsupported.unwrap_or("unknown")),
                    };
                    cx.publish_fact(0, fact)
                }
                SUMMARY => {
                    let index = item_index(&key, "missing instance index")?;
                    let body: Arc<SsaFact> = facts.get(function(LOCAL_OPT, index)).await?;
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    // Summary is Copy and owns no heap storage; its fact
                    // envelope is admitted once here.
                    let summary = match body.lowered.as_deref() {
                        Some(lowered) => {
                            Some(ssa::inline::Summary::of(lowered, &meter).map_err(Error::new)?)
                        }
                        None => None,
                    };
                    this.retain(std::mem::size_of::<Option<ssa::inline::Summary>>())?;
                    cx.publish_fact(0, summary)
                }
                IPO_OPT => {
                    let index = item_index(&key, "missing instance index")?;
                    let body: Arc<SsaFact> = facts.get(function(LOCAL_OPT, index)).await?;
                    let Some(lowered) = body.lowered.clone() else {
                        return cx.publish_fact(
                            0,
                            SsaFact::unsupported(body.unsupported.unwrap_or("unknown")),
                        );
                    };
                    let meter = ssa::meter::Meter::new(&this.0.budget);
                    // Callers read only their own direct callees' summaries,
                    // requested together, then the bodies they inline. Each
                    // inlined body is a recorded edge (OPTIMIZATION.md 8.3).
                    // Every frontier buffer below is paid scratch, admitted at
                    // its exact capacity and held across the awaits that use it.
                    let types = facts.types().await?;
                    let count = lowered.callees.len();
                    let mut slots = meter.list::<Option<usize>>(count).map_err(Error::new)?;
                    for callee in &lowered.callees {
                        meter.work(1).map_err(Error::new)?;
                        slots
                            .push(instance_slot(&facts, &types, &callee.target).await?)
                            .map_err(Error::new)?;
                    }
                    let mut summary_handles = meter.list(count).map_err(Error::new)?;
                    for slot in slots.iter() {
                        summary_handles
                            .push(
                                slot.filter(|slot| *slot != index)
                                    .map(|slot| cx.request(function(SUMMARY, slot)).get(0)),
                            )
                            .map_err(Error::new)?;
                    }
                    let mut summaries = meter
                        .table::<Option<ssa::inline::Summary>>(count, None)
                        .map_err(Error::new)?;
                    for (slot, handle) in summaries.iter_mut().zip(summary_handles.iter()) {
                        if let Some(handle) = handle {
                            let summary: Arc<Option<ssa::inline::Summary>> =
                                cx.read_fact(handle.clone()).await?;
                            *slot = *summary;
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
                            .ok_or_else(|| Error::new("chosen callee has no slot"))?;
                        body_handles
                            .push(cx.request(function(LOCAL_OPT, slot)).get(0))
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
                CHECK | BUILD => {
                    // Closed source roots have stable slots from SIGNATURES.
                    // Demand them before sealing the additional generic frontier:
                    // otherwise MONO_PLAN becomes a scheduling barrier even
                    // though these pipelines do not depend on its output.
                    let types = facts.types().await?;
                    let root_count = types.closed_roots().len();
                    let (mut constant_handles, _constant_handles_storage) =
                        this.scratch_vec(types.constant_count())?;
                    for index in 0..types.constant_count() {
                        let declaration = types.constant_decl(index).map_err(Error::new)?;
                        constant_handles.push(
                            cx.request(function(CONST_VALUE, declaration.index as usize))
                                .get(0),
                        );
                    }
                    let kind = if key.kind == BUILD { OBJECT } else { ANALYZE };
                    let (mut root_handles, _root_handles_reservation) =
                        this.scratch_vec(root_count)?;
                    for index in 0..root_count {
                        root_handles.push(cx.request(function(kind, index)).get(0));
                    }
                    let plan = facts.plan().await?;
                    for handle in constant_handles {
                        cx.read(handle).await?;
                    }
                    if key.kind == CHECK {
                        // Required universal admission is per-caller, not one
                        // root join. CHECK only waits on the callables that
                        // actually need universal admission.
                        let types = facts.types().await?;
                        let callable_count = types.callable_count();
                        let (mut handles, _handles_reservation) =
                            this.scratch_vec(callable_count.checked_mul(2).ok_or_else(|| {
                                Error::new("universal admission frontier limit exceeded")
                            })?)?;
                        for index in 0..callable_count {
                            handles.push(cx.request(function(TEMPLATE_BODY, index)).get(0));
                            let declaration = types.callable_decl(index).map_err(Error::new)?;
                            if Self::universal_body(&types, declaration)? {
                                handles.push(
                                    cx.request(function(UNIVERSAL_EFFECTS_READY, index)).get(0),
                                );
                            }
                        }
                        for handle in handles {
                            cx.read(handle).await?;
                        }
                    }

                    let additional = plan
                        .plan
                        .entries
                        .len()
                        .checked_sub(root_count)
                        .ok_or_else(|| Error::new("instance plan omitted a closed source root"))?;
                    let (mut handles, _handles_reservation) = this.scratch_vec(additional)?;
                    for index in root_count..plan.plan.entries.len() {
                        handles.push(cx.request(function(kind, index)).get(0));
                    }
                    let handles = root_handles.into_iter().chain(handles);
                    if key.kind == CHECK {
                        for handle in handles {
                            let proof: Arc<ConcreteProofFact> = cx.read_fact(handle).await?;
                            if has_drop(proof.concrete.function()) {
                                // Owner-local admission is not executable inventory authority.
                                plan.plan
                                    .validate_cleanup_dependencies(proof.concrete.function())
                                    .map_err(Error::new)?;
                            }
                        }
                        return cx.publish(0, 1);
                    }
                    let (mut objects, _objects_reservation) =
                        this.scratch_vec(plan.plan.entries.len())?;
                    for (handle, instance) in handles.zip(plan.plan.entries.iter()) {
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

/// The instance slot of a direct call target, if it is a planned instance.
/// Nongeneric roots resolve from SIGNATURES alone; generic targets read the
/// sealed instance plan, which these optimizer nodes already depend on
/// transitively through ANALYZE.
async fn instance_slot(
    facts: &Facts<'_>,
    types: &frontend::PlanCtx,
    target: &ir::CallTarget,
) -> Result<Option<usize>, Error> {
    if target.type_args.is_empty() {
        let callable = types
            .callable_index(target.declaration)
            .map_err(Error::new)?;
        if let Some(slot) = types
            .closed_roots()
            .iter()
            .position(|root| *root == callable)
        {
            return Ok(Some(slot));
        }
    }
    let plan = facts.plan().await?;
    Ok(plan.plan.slot(target))
}
fn has_drop(function: &cfg::Function) -> bool {
    function
        .blocks
        .iter()
        .any(|block| matches!(block.terminator.value, cfg::Terminator::Drop { .. }))
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
        error: const_diagnostics::qualification_cycle_error(
            &engine,
            report,
            &error,
            &snapshot.budget,
        )
        .unwrap_or(error),
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
    let mut optimizer = OptimizerReport {
        mode: snapshot.source_options.opt,
        ..OptimizerReport::default()
    };
    for index in 0..instances {
        if let Some(fact) = engine.fact_of::<SsaFact>(&Key::new(IPO_OPT, vec![index as u64]), 0) {
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
fn metered_usage(budget: &ir::PlanBudget, report: &Report) -> ir::PlanUsageSnapshot {
    let mut usage = budget.usage_snapshot();
    usage.work = usage
        .work
        .saturating_add(usize::try_from(report.metered_work).unwrap_or(usize::MAX));
    usage.nodes = usage
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
mod owner_route_tests {
    use super::*;

    /// The parsed program of one graph compile and its engine authority.
    fn graph_parsed(source: &str) -> (Arc<frontend::ParsedProgram>, u64) {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(source.into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let authority = engine.authority().get();
        let parse = engine.request(Key::new(PARSE, vec![]));
        engine.run(2).unwrap();
        parse.completion().unwrap();
        (parse.fact::<frontend::ParsedProgram>(0).unwrap(), authority)
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
        let direct = frontend::parse_unresolved(SOURCE).unwrap();
        // The graph session owner was minted under its producing engine's
        // authority: the PARSE node's own root, not a standalone mint.
        assert_eq!(minted_by(owner(&a)), Some(a_engine));
        assert_eq!(minted_by(owner(&b)), Some(b_engine));
        // A direct parse mints under a fresh standalone authority.
        let direct_root = minted_by(owner(&direct)).expect("direct parse tag recorded");
        assert_ne!(direct_root, a_engine);
        assert_ne!(direct_root, b_engine);
        assert_ne!(owner(&a), owner(&b), "two graph compiles share an owner");
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
