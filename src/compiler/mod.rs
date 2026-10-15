//! Source-to-native compiler built on the fact graph. See COMPILER.md.
pub mod backend;
pub mod frontend;

use crate::{Context, Engine, Error, Key, Provider, Report, TaskFuture};
use std::sync::{Arc, OnceLock};

const PARSE: u32 = 100;
const SIGNATURES: u32 = 101;
const BODY: u32 = 102;
const OBJECT: u32 = 103;
const CHECK: u32 = 104;
const BUILD: u32 = 105;

struct Snapshot {
    source: String,
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
                PARSE => {
                    let program = frontend::parse(&this.0.source).map_err(Error::new)?;
                    let objects = (0..program.functions.len())
                        .map(|_| OnceLock::new())
                        .collect();
                    this.0
                        .objects
                        .set(objects)
                        .map_err(|_| Error::new("duplicate object arena"))?;
                    this.0
                        .program
                        .set(program)
                        .map_err(|_| Error::new("duplicate program"))?;
                }
                SIGNATURES => {
                    cx.read(cx.request(root(PARSE)).get(0)).await?;
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
    let snapshot = Arc::new(Snapshot {
        source,
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
    Ok(Compilation {
        executable,
        report,
        functions,
    })
}
