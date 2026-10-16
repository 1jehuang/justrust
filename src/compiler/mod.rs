//! Source-to-native compiler built on the fact graph. See COMPILER.md.
pub mod backend;
mod borrow;
pub mod cfg;
pub mod frontend;
pub mod ir;
pub mod ownership;
pub mod source;

use std::path::PathBuf;

use crate::{Context, Engine, Error, Key, Provider, Report, TaskFuture};
use std::sync::{Arc, OnceLock};

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

enum Input {
    Text(String),
    File(PathBuf),
}

struct Snapshot {
    input: Input,
    source: OnceLock<source::LoadedSource>,
    parsed: OnceLock<frontend::ParsedProgram>,
    program: OnceLock<frontend::Program>,
    types: OnceLock<frontend::TypeEnv>,
    checked: OnceLock<Vec<OnceLock<ir::CheckedFunction>>>,
    lowered: OnceLock<Vec<OnceLock<Arc<cfg::Function>>>>,
    analyses: OnceLock<Vec<OnceLock<Arc<ownership::Analysis>>>>,
    objects: OnceLock<Vec<OnceLock<backend::Object>>>,
    executable: OnceLock<Vec<u8>>,
}

#[derive(Clone)]
struct CompilerProvider(Arc<Snapshot>);

impl CompilerProvider {
    fn program(&self) -> Result<&frontend::Program, Error> {
        self.0
            .program
            .get()
            .ok_or_else(|| Error::new("missing parsed program"))
    }
}

impl Provider for CompilerProvider {
    fn outputs(&self, _key: &Key) -> usize {
        1
    }

    fn start(&self, cx: Context, key: Key) -> TaskFuture {
        let this = self.clone();
        Box::pin(async move {
            let root = |kind| Key::new(kind, vec![]);
            let function = |kind, index| Key::new(kind, vec![index as u64]);
            match key.kind {
                SOURCE => {
                    let Input::File(path) = &this.0.input else {
                        return Err(Error::new("source loading requires file input"));
                    };
                    let loaded = source::load(path).map_err(Error::new)?;
                    this.0
                        .source
                        .set(loaded)
                        .map_err(|_| Error::new("duplicate source snapshot"))?;
                }
                PARSE => {
                    let text = match &this.0.input {
                        Input::Text(text) => text,
                        Input::File(_) => {
                            cx.read(cx.request(root(SOURCE)).get(0)).await?;
                            &this
                                .0
                                .source
                                .get()
                                .ok_or_else(|| Error::new("missing source snapshot"))?
                                .text
                        }
                    };
                    let parsed = frontend::parse_unresolved(text).map_err(Error::new)?;
                    let objects = (0..parsed.program.functions.len())
                        .map(|_| OnceLock::new())
                        .collect();
                    let checked = (0..parsed.program.functions.len())
                        .map(|_| OnceLock::new())
                        .collect();
                    let lowered = (0..parsed.program.functions.len())
                        .map(|_| OnceLock::new())
                        .collect();
                    this.0
                        .analyses
                        .set(
                            (0..parsed.program.functions.len())
                                .map(|_| OnceLock::new())
                                .collect(),
                        )
                        .map_err(|_| Error::new("duplicate analysis arena"))?;
                    this.0
                        .lowered
                        .set(lowered)
                        .map_err(|_| Error::new("duplicate CFG arena"))?;
                    this.0
                        .checked
                        .set(checked)
                        .map_err(|_| Error::new("duplicate checked function arena"))?;
                    this.0
                        .objects
                        .set(objects)
                        .map_err(|_| Error::new("duplicate object arena"))?;
                    this.0
                        .parsed
                        .set(parsed)
                        .map_err(|_| Error::new("duplicate parsed snapshot"))?;
                }
                RESOLVE => {
                    cx.read(cx.request(root(PARSE)).get(0)).await?;
                    let parsed = this
                        .0
                        .parsed
                        .get()
                        .ok_or_else(|| Error::new("missing parsed snapshot"))?;
                    let program = frontend::resolve(parsed).map_err(Error::new)?;
                    this.0
                        .program
                        .set(program)
                        .map_err(|_| Error::new("duplicate resolved program"))?;
                }
                SIGNATURES => {
                    cx.read(cx.request(root(RESOLVE)).get(0)).await?;
                    let types = frontend::type_env(this.program()?).map_err(Error::new)?;
                    this.0
                        .types
                        .set(types)
                        .map_err(|_| Error::new("duplicate type environment"))?;
                }
                BODY | LOWER | ANALYZE | OBJECT => {
                    let [index] = key.params.as_slice() else {
                        return Err(Error::new("missing function index"));
                    };
                    let index = usize::try_from(*index)
                        .map_err(|_| Error::new("invalid function index"))?;
                    if key.kind == BODY {
                        cx.read(cx.request(root(SIGNATURES)).get(0)).await?;
                        let types = this
                            .0
                            .types
                            .get()
                            .ok_or_else(|| Error::new("missing type environment"))?;
                        let checked =
                            frontend::type_function_with_env(this.program()?, index, types)
                                .map_err(Error::new)?;
                        this.0
                            .checked
                            .get()
                            .and_then(|v| v.get(index))
                            .ok_or_else(|| Error::new("missing checked function slot"))?
                            .set(checked)
                            .map_err(|_| Error::new("duplicate checked function"))?;
                    } else if key.kind == LOWER {
                        cx.read(cx.request(function(BODY, index)).get(0)).await?;
                        let item = this
                            .0
                            .checked
                            .get()
                            .and_then(|v| v.get(index))
                            .and_then(OnceLock::get)
                            .ok_or_else(|| Error::new("missing checked function"))?;
                        let lowered = cfg::lower(item).map_err(Error::new)?;

                        this.0
                            .lowered
                            .get()
                            .and_then(|v| v.get(index))
                            .ok_or_else(|| Error::new("missing CFG slot"))?
                            .set(Arc::new(lowered))
                            .map_err(|_| Error::new("duplicate CFG fact"))?;
                    } else if key.kind == ANALYZE {
                        cx.read(cx.request(function(LOWER, index)).get(0)).await?;
                        let item = this
                            .0
                            .lowered
                            .get()
                            .and_then(|v| v.get(index))
                            .and_then(OnceLock::get)
                            .ok_or_else(|| Error::new("missing CFG fact"))?;
                        let analysis = Arc::new(ownership::analyze(item));
                        let passed = analysis.passed();
                        let diagnostics = (!passed).then(|| {
                            format!(
                                "{}: initialization/move analysis (complete={}): {:?}",
                                item.name, analysis.complete, analysis.diagnostics
                            )
                        });
                        this.0
                            .analyses
                            .get()
                            .and_then(|v| v.get(index))
                            .ok_or_else(|| Error::new("missing analysis slot"))?
                            .set(analysis)
                            .map_err(|_| Error::new("duplicate analysis fact"))?;
                        if let Some(diagnostics) = diagnostics {
                            return Err(Error::new(diagnostics));
                        }
                    } else {
                        cx.read(cx.request(function(ANALYZE, index)).get(0)).await?;
                        let item = this
                            .0
                            .lowered
                            .get()
                            .and_then(|v| v.get(index))
                            .and_then(OnceLock::get)
                            .ok_or_else(|| Error::new("missing CFG fact"))?;
                        let object = backend::emit_cfg_function(item).map_err(Error::new)?;
                        this.0
                            .objects
                            .get()
                            .and_then(|v| v.get(index))
                            .ok_or_else(|| Error::new("missing object slot"))?
                            .set(object)
                            .map_err(|_| Error::new("duplicate object"))?;
                    }
                }
                CHECK | BUILD => {
                    cx.read(cx.request(root(SIGNATURES)).get(0)).await?;
                    let program = this.program()?;
                    // Expose the entire independent function frontier before awaiting.
                    let kind = if key.kind == BUILD { OBJECT } else { ANALYZE };
                    let inputs: Vec<_> = (0..program.functions.len())
                        .map(|i| cx.request(function(kind, i)).get(0))
                        .collect();
                    for handle in inputs {
                        cx.read(handle).await?;
                    }
                    if key.kind == BUILD {
                        let objects = this
                            .0
                            .objects
                            .get()
                            .ok_or_else(|| Error::new("missing objects"))?;
                        let objects: Vec<_> = program
                            .functions
                            .iter()
                            .zip(objects)
                            .map(|(f, o)| {
                                o.get()
                                    .map(|o| (f.name.clone(), o.clone()))
                                    .ok_or_else(|| Error::new("unpublished function object"))
                            })
                            .collect::<Result<_, _>>()?;
                        let bytes = backend::link(&objects).map_err(Error::new)?;
                        this.0
                            .executable
                            .set(bytes)
                            .map_err(|_| Error::new("duplicate executable"))?;
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

/// Load a source tree through an observable graph operation before parsing.
/// File loading currently uses one coarse snapshot node, not per-file nodes.
pub fn compile_file_observed(
    path: PathBuf,
    workers: usize,
    check_only: bool,
) -> Result<Compilation, Failure> {
    compile_input(Input::File(path), workers, check_only)
}

fn retained_analyses(snapshot: &Snapshot) -> Vec<(usize, Arc<ownership::Analysis>)> {
    snapshot
        .analyses
        .get()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, slot)| slot.get().map(|analysis| (index, analysis.clone())))
        .collect()
}

fn compile_input(input: Input, workers: usize, check_only: bool) -> Result<Compilation, Failure> {
    let snapshot = Arc::new(Snapshot {
        input,
        source: OnceLock::new(),
        parsed: OnceLock::new(),
        program: OnceLock::new(),
        types: OnceLock::new(),
        checked: OnceLock::new(),
        lowered: OnceLock::new(),
        analyses: OnceLock::new(),
        objects: OnceLock::new(),
        executable: OnceLock::new(),
    });
    let engine = Engine::new(CompilerProvider(snapshot.clone()));
    let check = engine.request(Key::new(CHECK, vec![]));
    let build = (!check_only).then(|| engine.request(Key::new(BUILD, vec![])));
    let report = engine.run(workers).map_err(|error| Failure {
        error,
        report: None,
        analyses: Vec::new(),
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
        });
    }
    let functions = snapshot.program.get().map_or(0, |p| p.functions.len());
    let executable = snapshot.executable.get().cloned();
    let (source_bytes, source_files) = match &snapshot.input {
        Input::Text(text) => (text.len(), 1),
        Input::File(_) => {
            let loaded = snapshot.source.get().ok_or_else(|| Failure {
                error: Error::new("missing loaded source accounting"),
                report: Some(Box::new(report.clone())),
                analyses: retained_analyses(&snapshot),
            })?;
            (loaded.source_bytes, loaded.files)
        }
    };
    let control_flow: Vec<_> = snapshot
        .lowered
        .get()
        .into_iter()
        .flatten()
        .filter_map(OnceLock::get)
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
            .analyses
            .get()
            .into_iter()
            .flatten()
            .filter_map(OnceLock::get)
            .cloned()
            .collect(),
        control_flow,
        source_bytes,
        source_files,
    })
}
