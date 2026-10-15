//! Source-to-native compiler built on the fact graph. See COMPILER.md.
pub mod backend;
pub mod frontend;
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

enum Input {
    Text(String),
    File(PathBuf),
}

struct Snapshot {
    input: Input,
    source: OnceLock<source::LoadedSource>,
    parsed: OnceLock<frontend::ParsedProgram>,
    program: OnceLock<frontend::Program>,
    signatures: OnceLock<frontend::Signatures>,
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
                    let signatures =
                        frontend::signature_table(this.program()?).map_err(Error::new)?;
                    this.0
                        .signatures
                        .set(signatures)
                        .map_err(|_| Error::new("duplicate signatures"))?;
                }
                BODY | OBJECT => {
                    let [index] = key.params.as_slice() else {
                        return Err(Error::new("missing function index"));
                    };
                    let index = usize::try_from(*index)
                        .map_err(|_| Error::new("invalid function index"))?;
                    if key.kind == BODY {
                        cx.read(cx.request(root(SIGNATURES)).get(0)).await?;
                        let signatures = this
                            .0
                            .signatures
                            .get()
                            .ok_or_else(|| Error::new("missing signatures"))?;
                        frontend::check_function_with_signatures(
                            this.program()?,
                            index,
                            signatures,
                        )
                        .map_err(Error::new)?;
                    } else {
                        cx.read(cx.request(function(BODY, index)).get(0)).await?;
                        let program = this.program()?;
                        let item = program
                            .functions
                            .get(index)
                            .ok_or_else(|| Error::new("function index out of bounds"))?;
                        let object = backend::emit_function(item).map_err(Error::new)?;
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
                    let kind = if key.kind == BUILD { OBJECT } else { BODY };
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

fn compile_input(input: Input, workers: usize, check_only: bool) -> Result<Compilation, Failure> {
    let snapshot = Arc::new(Snapshot {
        input,
        source: OnceLock::new(),
        parsed: OnceLock::new(),
        program: OnceLock::new(),
        signatures: OnceLock::new(),
        objects: OnceLock::new(),
        executable: OnceLock::new(),
    });
    let engine = Engine::new(CompilerProvider(snapshot.clone()));
    let check = engine.request(Key::new(CHECK, vec![]));
    let build = (!check_only).then(|| engine.request(Key::new(BUILD, vec![])));
    let report = engine.run(workers).map_err(|error| Failure {
        error,
        report: None,
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
            })?;
            (loaded.source_bytes, loaded.files)
        }
    };
    Ok(Compilation {
        executable,
        report,
        functions,
        source_bytes,
        source_files,
    })
}
