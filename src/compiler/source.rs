//! Bounded configured source preparation and original-file byte provenance.
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[path = "source_attrs.rs"]
pub mod source_attrs;
#[path = "source_cfg.rs"]
pub mod source_cfg;
#[path = "source_derive.rs"]
mod source_derive;
#[path = "source_lex.rs"]
pub mod source_lex;
#[path = "source_map.rs"]
pub mod source_map;
#[path = "source_probe.rs"]
pub mod source_probe;
pub use super::token::Edition;
pub use source_cfg::CfgSet;
pub use source_lex::{PreparedLexInput, PreparedMode, PreparedViewId, ProjectedSpan};
pub use source_map::{FileId, OriginSegment, SourceMap, SourceSpan};
pub use source_probe::{SourceProbeRecord, SourceProbeRequest, attach_probe, prepare_probe_file};
#[path = "source_embedded.rs"]
mod source_embedded;
pub(super) use source_embedded::EmbeddedOriginalFile;
#[path = "source_prelude.rs"]
pub mod source_prelude;
const MAX_FILE_DEPTH: usize = 64;
const MAX_MODULE_DEPTH: usize = 128;
pub(super) const MAX_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_FILES: usize = 1024;
const MAX_ORIGINS: usize = 1024 * 1024;

#[derive(Clone, Debug)]
/// Native x86_64 Linux configuration plus bounded user predicates.
/// The default pins abort and enables debug assertions, not complete Rust panic semantics.
pub struct SourceOptions {
    /// Explicit language input. Unsupported editions reject before source IO.
    pub edition: Edition,
    /// Native ELF object emission/link inputs, frozen before graph execution.
    pub native_link: super::c_abi::elf::LinkOptions,
    pub cfg: CfgSet,
    /// Attach the embedded native prelude library (`libraries/prelude`) after
    /// the user crate in graph compilation. See PRELUDE.md.
    pub prelude: bool,
    /// Optimizer mode (OPTIMIZATION.md section 6). An explicit compile input:
    /// the compiler never reads `JR_OPT` itself.
    pub opt: super::ssa::OptMode,
    /// Runtime integer overflow checks, independent of debug assertions and optimization.
    /// False wraps arithmetic and masks shifts; division and remainder still trap.
    pub overflow_checks: bool,
    /// Dependency crates compiled with this crate, each mounted as a hidden
    /// root module `__justrust_extern_<name>` and nameable as `<name>::`.
    /// Order is attachment order; the graph keys them as SOURCE `[2 + i]`.
    pub externs: Vec<ExternCrate>,
    /// Library target: no `main` is required. A check/build of the library
    /// gets an inert entry; under `cfg(test)` the harness is the entry.
    pub library: bool,
    /// Libtest selection applied when synthesizing the `cfg(test)` harness.
    pub test_filter: TestFilter,
}
/// Libtest command-line selection: name filters (substring, or exact with
/// `--exact`), `--skip`, `--ignored` and `--include-ignored`.
#[derive(Clone, Debug, Default)]
pub struct TestFilter {
    pub filters: Vec<String>,
    pub skip: Vec<String>,
    pub exact: bool,
    pub ignored: bool,
    pub include_ignored: bool,
}
impl TestFilter {
    fn selects(&self, name: &str) -> bool {
        let matches = |pattern: &String| {
            if self.exact {
                name == pattern
            } else {
                name.contains(pattern.as_str())
            }
        };
        (self.filters.is_empty() || self.filters.iter().any(matches))
            && !self.skip.iter().any(matches)
    }
}
impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            edition: Edition::E2024,
            native_link: super::c_abi::elf::LinkOptions::default(),
            cfg: CfgSet::native_x86_64_linux(),
            prelude: true,
            opt: super::ssa::OptMode::default(),
            overflow_checks: true,
            externs: Vec::new(),
            library: false,
            test_filter: TestFilter::default(),
        }
    }
}
/// One dependency crate: its extern name, root source file, and own cfg.
#[derive(Clone, Debug)]
pub struct ExternCrate {
    pub name: String,
    pub root: PathBuf,
    pub cfg: CfgSet,
}
/// Hidden root module prefix of a mounted dependency crate.
pub const EXTERN_PREFIX: &str = "__justrust_extern_";
impl SourceOptions {
    /// Options for the source file at graph ordinal `ordinal`: dependency
    /// crates (`[2 + i, ...]`) use their own cfg and never `cfg(test)`.
    pub fn for_ordinal(&self, ordinal: &[u64]) -> std::borrow::Cow<'_, SourceOptions> {
        match ordinal.first() {
            Some(&first) if first >= 2 => match self.externs.get(first as usize - 2) {
                Some(krate) => {
                    let mut options = self.clone();
                    options.cfg = krate.cfg.clone();
                    options
                        .cfg
                        .set_debug_assertions(self.cfg.debug_assertions());
                    options.cfg.names.remove("test");
                    options.library = true;
                    std::borrow::Cow::Owned(options)
                }
                None => std::borrow::Cow::Borrowed(self),
            },
            _ => std::borrow::Cow::Borrowed(self),
        }
    }
}
/// Root unit of dependency crate `index` (graph ordinal `[2 + index]`).
pub fn prepare_extern_unit(
    options: &SourceOptions,
    index: usize,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    let Some(krate) = options.externs.get(index) else {
        return Err(UnitFailure {
            error: Loader::error(SourceErrorKind::Contract, None, "unknown dependency crate"),
            anchored: false,
        });
    };
    let options = options.for_ordinal(&[2 + index as u64]);
    prepare_root_unit(&krate.root, &options, budget)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceErrorKind {
    Io,
    Syntax,
    Unsupported,
    Limit,
    Cycle,
    Module,
    /// Invalid provenance or foreign checked utility handles, not user syntax.
    Contract,
    /// Allocation or issuing-authority failure, not unsupported Rust syntax.
    Infrastructure,
}
#[derive(Clone, Debug)]
pub struct SourceLocation {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}
#[derive(Debug)]
pub struct SourceError {
    pub kind: SourceErrorKind,
    pub primary: Option<SourceSpan>,
    pub message: String,
    // Variable-size expanded diagnostics keep their original storage lease.
    // This field follows the String so payload destruction precedes release.
    pub(crate) _diagnostic_lease: Option<super::expansion_budget::ByteLease>,
    pub location: Option<SourceLocation>,
}
impl SourceError {
    pub fn from_token(error: super::token_session::TokenError) -> Self {
        use super::expansion_budget::BudgetFailureKind;
        use super::token_session::TokenError;
        let kind = match &error {
            TokenError::Budget(failure) => match failure.kind {
                BudgetFailureKind::Limit | BudgetFailureKind::Overflow => SourceErrorKind::Limit,
                BudgetFailureKind::Allocation | BudgetFailureKind::Infrastructure => {
                    SourceErrorKind::Infrastructure
                }
            },
            TokenError::SourceLimit { .. } | TokenError::TraceTruncated => SourceErrorKind::Limit,
            TokenError::IdExhausted | TokenError::AtomCollision => SourceErrorKind::Infrastructure,
            TokenError::UnbalancedDelimiter { .. } => SourceErrorKind::Syntax,
            TokenError::ForeignHandle
            | TokenError::InvalidHandle
            | TokenError::InvalidRange
            | TokenError::InvalidUtf8Boundary
            | TokenError::InvalidInput(_) => SourceErrorKind::Contract,
        };
        Self {
            kind,
            primary: None,
            message: error.to_string(),
            _diagnostic_lease: None,
            location: None,
        }
    }
    pub fn syntax(primary: Option<SourceSpan>, message: impl Into<String>) -> Self {
        Self {
            kind: SourceErrorKind::Syntax,
            primary,
            message: message.into(),
            _diagnostic_lease: None,
            location: None,
        }
    }
    pub fn with_sources(mut self, sources: &SourceMap) -> Self {
        if let Some((path, line, column)) = self.primary.and_then(|s| sources.location(s)) {
            self.location = Some(SourceLocation {
                path: path.to_owned(),
                line,
                column,
            });
        }
        self
    }
}
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(l) = &self.location {
            write!(f, "{}:{}:{}: ", l.path.display(), l.line, l.column)?;
        }
        match self.kind {
            SourceErrorKind::Unsupported => f.write_str("unsupported: ")?,
            SourceErrorKind::Contract => f.write_str("contract: ")?,
            SourceErrorKind::Infrastructure => f.write_str("infrastructure: ")?,
            _ => {}
        }
        f.write_str(&self.message)
    }
}
impl std::error::Error for SourceError {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedAttribute {
    pub span: SourceSpan,
    pub name: String,
    pub effective: bool,
}
#[derive(Clone, Debug)]
pub(super) struct CratePolicy {
    root: FileId,
    original: Arc<str>,
    no_std: bool,
    forbid_unsafe_code: bool,
}
impl CratePolicy {
    pub(super) fn forbid_unsafe_root(
        &self,
        source: &PreparedSource,
    ) -> Result<Option<usize>, SourceError> {
        // Reuse the exact original-source authentication, even when no_std is false.
        self.no_std_root(source)?;
        Ok(self.forbid_unsafe_code.then_some(self.root.0 as usize))
    }
    pub(super) fn no_std_root(
        &self,
        source: &PreparedSource,
    ) -> Result<Option<usize>, SourceError> {
        let index = self.root.0 as usize;
        if source.crate_roots.get(index) != Some(&self.root)
            || !source
                .sources
                .files
                .get(index)
                .is_some_and(|f| f.contents == self.original)
        {
            return Err(Loader::error(
                SourceErrorKind::Contract,
                None,
                "crate policy source identity mismatch",
            ));
        }
        Ok(self.no_std.then_some(index))
    }
}
#[derive(Clone, Debug)]
pub struct PreparedSource {
    edition: Edition,
    embedded_files: Vec<source_embedded::EmbeddedFile>,
    embedded_transport: Option<source_embedded::AssemblyTransport>,
    debug_assertions: bool,
    overflow_checks: bool,
    /// Assembly-owned crate root for each original file. Empty only for raw
    /// caller-built maps, whose files all belong to the single input crate.
    pub(super) crate_roots: Vec<FileId>,
    pub(super) crate_policies: Vec<CratePolicy>,
    pub text: String,
    pub sources: SourceMap,
    pub origins: Vec<OriginSegment>,
    pub attributes: Vec<PreparedAttribute>,
    /// Reviewed source pin audit records, never semantic or publication authority.
    pub probes: Vec<SourceProbeRecord>,
    pub source_bytes: usize,
    pub files: usize,
}
impl Default for PreparedSource {
    fn default() -> Self {
        Self {
            edition: Edition::E2024,
            embedded_files: Vec::new(),
            embedded_transport: None,
            debug_assertions: CfgSet::DEFAULT_DEBUG_ASSERTIONS,
            overflow_checks: true,
            crate_roots: Vec::new(),
            crate_policies: Vec::new(),
            text: String::new(),
            sources: SourceMap::default(),
            origins: Vec::new(),
            attributes: Vec::new(),
            probes: Vec::new(),
            source_bytes: 0,
            files: 0,
        }
    }
}
/// One file's own prepared text for per-file parsing (PARSE-SPLIT.md step
/// A). Each `mod child;` stays an empty `{}` group; `child_sites[i]` is the
/// byte offset inside that group where child `i`'s items belong. Spans
/// render at `base`, the file's depth-first assembly FileId, so diagnostics
/// keep assembled order and paths.
#[derive(Debug)]
pub struct FileView {
    pub prepared: PreparedSource,
    pub base: u32,
    pub child_sites: Vec<usize>,
}
impl FileView {
    /// The shallow view of `unit`, mounted at assembly FileId `base`.
    pub fn of(unit: &FileUnit, base: u32, options: &SourceOptions) -> Result<Self, SourceError> {
        let mut prepared = PreparedSource {
            edition: unit.edition,
            debug_assertions: options.cfg.debug_assertions(),
            overflow_checks: options.overflow_checks,
            ..PreparedSource::default()
        };
        let id = prepared
            .sources
            .add(unit.path.clone(), unit.contents.clone());
        prepared.crate_roots.push(id);
        if unit.forbid_unsafe_code {
            prepared.crate_policies.push(CratePolicy {
                root: id,
                original: unit.contents.clone(),
                no_std: unit.no_std,
                forbid_unsafe_code: true,
            });
        }
        prepared.source_bytes = unit.contents.len();
        prepared.files = 1;
        for segment in &unit.origins {
            append_segment(
                &mut prepared,
                &unit.text[segment.prepared.clone()],
                segment.origin,
                segment.synthetic,
            )?;
        }
        for attribute in &unit.attributes {
            prepared.attributes.push(attribute.clone());
        }
        if let Some(error) = &unit.terminal {
            return Err(copy_error(error, error.primary).with_sources(&prepared.sources));
        }
        Ok(Self {
            prepared,
            base,
            child_sites: unit.children.iter().map(|child| child.at).collect(),
        })
    }
}

impl PreparedSource {
    /// Language edition captured from the source preparation input.
    pub fn edition(&self) -> Edition {
        self.edition
    }
    pub(super) fn embedded_issuers(
        &self,
        snapshot: &super::token_session::SourceSnapshot,
        budget: &super::expansion_budget::ExpansionBudget,
    ) -> Result<Vec<EmbeddedOriginalFile>, SourceError> {
        if !self.embedded_files.is_empty() {
            self.embedded_transport
                .as_ref()
                .ok_or_else(|| {
                    Loader::error(
                        SourceErrorKind::Contract,
                        None,
                        "embedded source assembly transport is missing",
                    )
                })?
                .validate(self)?;
        }
        budget
            .charge(
                super::expansion_budget::Counter::Work,
                self.embedded_files.len() as u64,
            )
            .map_err(|error| SourceError::from_token(error.into()))?;
        self.embedded_files
            .iter()
            .map(|attachment| EmbeddedOriginalFile::import(attachment, self, snapshot))
            .collect()
    }

    /// Immutable native profile fact captured when source options were validated.
    pub fn debug_assertions(&self) -> bool {
        self.debug_assertions
    }
    pub fn overflow_checks(&self) -> bool {
        self.overflow_checks
    }
    pub fn map_range(&self, r: Range<usize>) -> Option<SourceSpan> {
        if r.end > self.text.len()
            || r.start > r.end
            || !self.text.is_char_boundary(r.start)
            || !self.text.is_char_boundary(r.end)
        {
            return None;
        }
        let span = source_map::map_range(&self.origins, r.clone())?;
        let file = self.sources.files.get(span.file.0 as usize)?;
        let original = file.contents.get(span.lo as usize..span.hi as usize)?;
        let index = self
            .origins
            .partition_point(|s| s.prepared.start <= r.start)
            .checked_sub(1)?;
        let segment = self.origins.get(index)?;
        if !segment.synthetic && original != &self.text[r] {
            return None;
        }
        Some(span)
    }
}
#[derive(Debug)]
pub struct LoadedSource {
    pub text: String,
    pub source_bytes: usize,
    pub files: usize,
}
pub fn load(path: &Path) -> Result<LoadedSource, String> {
    prepare_file(path, &SourceOptions::default())
        .map(|p| LoadedSource {
            text: p.text,
            source_bytes: p.source_bytes,
            files: p.files,
        })
        .map_err(|e| e.to_string())
}
pub fn prepare_file(path: &Path, options: &SourceOptions) -> Result<PreparedSource, SourceError> {
    prepare_with_reader(path, options, &mut FsReader)
}
pub(super) fn prepare_file_with_budget(
    path: &Path,
    options: &SourceOptions,
    budget: &super::expansion_budget::ExpansionBudget,
) -> Result<PreparedSource, SourceError> {
    prepare_with_reader_budget(path, options, &mut FsReader, budget)
}
fn prepare_with_reader(
    path: &Path,
    options: &SourceOptions,
    reader: &mut dyn SourceReader,
) -> Result<PreparedSource, SourceError> {
    prepare_with_reader_budget(
        path,
        options,
        reader,
        &super::expansion_budget::ExpansionBudget::default(),
    )
}
/// Whole-tree convenience driver over the per-file units. The graph runs the
/// same units as separate nodes; this sequential driver shares one reader and
/// one lexical authority so direct callers keep their single-call contract.
fn prepare_with_reader_budget(
    path: &Path,
    options: &SourceOptions,
    reader: &mut dyn SourceReader,
    budget: &super::expansion_budget::ExpansionBudget,
) -> Result<PreparedSource, SourceError> {
    let root = Arc::new(root_unit_with(path, options, Some(&mut *reader), budget));
    let mut assembler = SourceAssembler::user_only(options);
    let mut step = assembler.provide(root)?;
    loop {
        match step {
            AssemblyStep::Done(prepared) => return Ok(*prepared),
            AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None if need.ordinal[0] >= 2 => {
                        prepare_extern_unit(options, need.index, budget)
                    }
                    None => prepare_prelude_unit(options, budget),
                    Some(parent) if parent.path.starts_with(source_prelude::PRELUDE_DIR) => {
                        prepare_child_unit(parent, need.index, options, budget)
                    }
                    Some(parent) => {
                        child_unit_with(parent, need.index, options, &mut *reader, budget)
                    }
                };
                step = assembler.provide(Arc::new(unit))?;
            }
        }
    }
}
/// No filesystem capability exists in text preparation, even for path-like names.
pub fn prepare_text(
    name: impl Into<PathBuf>,
    text: &str,
    options: &SourceOptions,
) -> Result<PreparedSource, SourceError> {
    prepare_text_with(SourceAssembler::user_only(options), name, text, options)
}

/// Like [`prepare_text`], but attaches the embedded prelude when
/// `options.prelude` is set, exactly as graph compilation does.
pub fn prepare_text_with_prelude(
    name: impl Into<PathBuf>,
    text: &str,
    options: &SourceOptions,
) -> Result<PreparedSource, SourceError> {
    prepare_text_with(SourceAssembler::new(options), name, text, options)
}

fn prepare_text_with(
    mut assembler: SourceAssembler,
    name: impl Into<PathBuf>,
    text: &str,
    options: &SourceOptions,
) -> Result<PreparedSource, SourceError> {
    let unit = Arc::new(prepare_text_unit(name, text, options));
    let budget = super::expansion_budget::ExpansionBudget::default();
    let mut step = assembler.provide(unit)?;
    loop {
        match step {
            AssemblyStep::Done(prepared) => return Ok(*prepared),
            AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None if need.ordinal[0] >= 2 => {
                        prepare_extern_unit(options, need.index, &budget)
                    }
                    None => prepare_prelude_unit(options, &budget),
                    Some(parent) if parent.path.starts_with(source_prelude::PRELUDE_DIR) => {
                        prepare_child_unit(parent, need.index, options, &budget)
                    }
                    Some(_) => {
                        return Err(Loader::error(
                            SourceErrorKind::Contract,
                            None,
                            "text preparation requested an external module",
                        ));
                    }
                };
                step = assembler.provide(Arc::new(unit))?;
            }
        }
    }
}

/// Placeholder identity of the one file a [`FileUnit`] describes. The
/// assembler maps it to the file's position in the assembled source map.
const LOCAL_FILE: FileId = FileId(0);

/// One `mod name;` declaration found in a file, resolved to the child path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildModule {
    /// Offset in the unit's prepared text where the child's text is spliced.
    pub at: usize,
    /// The declaration in this unit's own file.
    pub decl: SourceSpan,
    pub path: PathBuf,
    pub directory: PathBuf,
    /// Module nesting depth of the child file's top level.
    pub depth: usize,
    /// Module path of the child file below the crate root.
    pub module: Vec<String>,
}

/// Prepared form of exactly one source file: original bytes, configured and
/// module-expanded text with provenance, and the external child modules it
/// declares. Pure in its explicit inputs (file bytes, options, ancestry).
/// Spans use [`LOCAL_FILE`] for this file.
#[derive(Debug)]
pub struct FileUnit {
    edition: Edition,
    embedded: Option<source_embedded::EmbeddedUnit>,
    no_std: bool,
    forbid_unsafe_code: bool,
    pub path: PathBuf,
    pub contents: Arc<str>,
    /// Canonical paths of the open files from the root to this one, for cycle
    /// detection in children. Empty for text input.
    pub chain: Vec<PathBuf>,
    pub text: String,
    pub origins: Vec<OriginSegment>,
    pub attributes: Vec<PreparedAttribute>,
    pub children: Vec<ChildModule>,
    /// Module path of this file below its crate root.
    pub module: Vec<String>,
    /// `#[test]` functions declared in this file (paths relative to `module`).
    pub tests: Vec<source_attrs::TestItem>,
    /// The first error found in this file, reported after every child that
    /// precedes it in source order, exactly where sequential loading stopped.
    pub terminal: Option<SourceError>,
}

/// A file that could not be read or admitted before its bytes were registered.
#[derive(Debug)]
pub struct UnitFailure {
    pub error: SourceError,
    /// The error belongs at the declaring `mod name;` in the parent file.
    pub anchored: bool,
}
pub type UnitResult = Result<FileUnit, UnitFailure>;

/// Read-only handle to an admitted file fact, retaining its original allocation.
#[derive(Clone, Debug)]
pub struct SharedFileUnit(Arc<UnitResult>);

impl std::ops::Deref for SharedFileUnit {
    type Target = FileUnit;

    fn deref(&self) -> &Self::Target {
        match self.0.as_ref() {
            Ok(unit) => unit,
            Err(_) => unreachable!("shared file handles are issued only for admitted units"),
        }
    }
}

/// Root unit of a file input, including option validation.
pub fn prepare_root_unit(
    path: &Path,
    options: &SourceOptions,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    root_unit_with(path, options, Some(&mut FsReader), budget)
}
fn root_unit_with(
    path: &Path,
    options: &SourceOptions,
    reader: Option<&mut dyn SourceReader>,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    let unanchored = |error| UnitFailure {
        error,
        anchored: false,
    };
    budget
        .check()
        .map_err(|error| unanchored(SourceError::from_token(error.into())))?;
    options.edition.validate_native().map_err(|message| {
        unanchored(Loader::error(SourceErrorKind::Unsupported, None, message))
    })?;
    options.cfg.validate_native().map_err(|message| {
        unanchored(Loader::error(SourceErrorKind::Unsupported, None, message))
    })?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    file_unit(Vec::new(), path, directory, 0, &[], options, reader, budget)
}
/// Child `index` of an already prepared parent unit.
pub fn prepare_child_unit(
    parent: &FileUnit,
    index: usize,
    options: &SourceOptions,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    if parent.path.starts_with(source_prelude::PRELUDE_DIR) {
        if let Some(issuer) = &parent.embedded {
            issuer.validate(parent).map_err(|error| UnitFailure {
                error,
                anchored: false,
            })?;
        }
        let mut child = child_unit_with(
            parent,
            index,
            options,
            &mut source_prelude::PreludeReader,
            budget,
        )?;
        // Reading through the embedded table alone is insufficient: a user
        // parent with a spoofed virtual path has no loader-issued capability.
        if parent.embedded.is_some() {
            child.embedded = Some(
                source_embedded::EmbeddedUnit::issue(&child, budget).map_err(|error| {
                    UnitFailure {
                        error,
                        anchored: false,
                    }
                })?,
            );
        }
        return Ok(child);
    }
    child_unit_with(parent, index, options, &mut FsReader, budget)
}
/// Attachment unit of the embedded native prelude library, graph ordinal
/// `[1]`. Its children are the library's own files, read only from the
/// embedded table.
pub fn prepare_prelude_unit(
    options: &SourceOptions,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    let path = Path::new(source_prelude::PRELUDE_ROOT);
    let mut unit = file_unit(
        Vec::new(),
        path,
        Path::new(source_prelude::PRELUDE_DIR),
        0,
        &[],
        options,
        Some(&mut source_prelude::PreludeReader),
        budget,
    )?;
    unit.embedded = Some(
        source_embedded::EmbeddedUnit::issue(&unit, budget).map_err(|error| UnitFailure {
            error,
            anchored: false,
        })?,
    );
    Ok(unit)
}
fn child_unit_with(
    parent: &FileUnit,
    index: usize,
    options: &SourceOptions,
    reader: &mut dyn SourceReader,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    let child = parent.children.get(index).ok_or_else(|| UnitFailure {
        error: Loader::error(
            SourceErrorKind::Contract,
            None,
            "missing child module declaration",
        ),
        anchored: false,
    })?;
    file_unit(
        child.module.clone(),
        &child.path,
        &child.directory,
        child.depth,
        &parent.chain,
        options,
        Some(reader),
        budget,
    )
}
/// Unit of an in-memory text input. It has no filesystem capability.
pub fn prepare_text_unit(
    name: impl Into<PathBuf>,
    text: &str,
    options: &SourceOptions,
) -> UnitResult {
    let unanchored = |error| UnitFailure {
        error,
        anchored: false,
    };
    options.edition.validate_native().map_err(|message| {
        unanchored(Loader::error(SourceErrorKind::Unsupported, None, message))
    })?;
    options.cfg.validate_native().map_err(|message| {
        unanchored(Loader::error(SourceErrorKind::Unsupported, None, message))
    })?;
    snapshot_unit(
        Vec::new(),
        name.into(),
        text.to_owned(),
        Path::new("."),
        0,
        Vec::new(),
        options,
        None,
        &super::expansion_budget::ExpansionBudget::default(),
    )
}
#[allow(clippy::too_many_arguments)]
fn file_unit(
    module: Vec<String>,
    path: &Path,
    directory: &Path,
    depth: usize,
    ancestors: &[PathBuf],
    options: &SourceOptions,
    reader: Option<&mut dyn SourceReader>,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    let anchored = |kind, message: String| UnitFailure {
        error: Loader::error(kind, None, message),
        anchored: true,
    };
    if ancestors.len() > MAX_FILE_DEPTH {
        return Err(anchored(
            SourceErrorKind::Limit,
            format!("module file depth limit ({MAX_FILE_DEPTH}) exceeded"),
        ));
    }
    let io_error =
        |e: std::io::Error| anchored(SourceErrorKind::Io, format!("{}: {e}", path.display()));
    let Some(reader) = reader else {
        return Err(anchored(
            SourceErrorKind::Unsupported,
            "external modules are unsupported in text input".into(),
        ));
    };
    let canonical = reader.canonicalize(path).map_err(io_error)?;
    if ancestors.contains(&canonical) {
        return Err(anchored(
            SourceErrorKind::Cycle,
            format!(
                "module inclusion cycle: {} -> {}",
                ancestors
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> "),
                canonical.display()
            ),
        ));
    }
    if !reader.is_file(path).map_err(io_error)? {
        return Err(anchored(
            SourceErrorKind::Io,
            format!("{}: source must be a regular file", path.display()),
        ));
    }
    let bytes = reader.read(path, MAX_BYTES + 1).map_err(io_error)?;
    let text = String::from_utf8(bytes).map_err(|e| {
        anchored(
            SourceErrorKind::Syntax,
            format!("{}: source is not UTF-8: {e}", path.display()),
        )
    })?;
    let mut chain = Vec::with_capacity(ancestors.len() + 1);
    chain.extend_from_slice(ancestors);
    chain.push(canonical);
    snapshot_unit(
        module,
        path.to_owned(),
        text,
        directory,
        depth,
        chain,
        options,
        Some(reader),
        budget,
    )
}
#[allow(clippy::too_many_arguments)]
fn snapshot_unit<'a>(
    module: Vec<String>,
    path: PathBuf,
    text: String,
    directory: &Path,
    depth: usize,
    chain: Vec<PathBuf>,
    options: &'a SourceOptions,
    reader: Option<&'a mut dyn SourceReader>,
    budget: &super::expansion_budget::ExpansionBudget,
) -> UnitResult {
    if text.len() > MAX_BYTES {
        return Err(UnitFailure {
            error: Loader::error(
                SourceErrorKind::Limit,
                None,
                format!("source size limit ({MAX_BYTES} bytes) exceeded"),
            ),
            anchored: false,
        });
    }
    let contents: Arc<str> = text.into();
    let root = chain.len() <= 1 && depth == 0;
    let mut builder = Loader::new(options, reader, LOCAL_FILE, contents.clone(), root);
    builder.lexical_budget = budget.clone();
    let terminal = builder.snapshot(directory, depth).err();
    Ok(FileUnit {
        edition: options.edition,
        embedded: None,
        no_std: builder.no_std,
        forbid_unsafe_code: builder.forbid_unsafe_code,
        path,
        contents,
        chain,
        text: builder.text,
        origins: builder.origins,
        attributes: builder.attributes,
        children: builder.children,
        module,
        tests: builder.tests,
        terminal,
    })
}

/// A child unit the assembler needs next, in source order.
pub struct NeedUnit {
    /// Graph identity of the child file: `[0, i, j, ...]` child ordinals.
    pub ordinal: Vec<u64>,
    /// `None` for crate attachment units: the prelude (ordinal `[1]`) and
    /// dependency crates (`[2 + index]`).
    pub parent: Option<SharedFileUnit>,
    pub index: usize,
}
pub enum AssemblyStep {
    Need(NeedUnit),
    Done(Box<PreparedSource>),
}
/// Splices per-file units into the one prepared source in depth-first source
/// order. Applies the cross-file limits (total bytes, file count, file depth,
/// expanded size) and reports the first error in the order sequential loading
/// would. It never reads files itself.
pub struct SourceAssembler {
    prepared: PreparedSource,
    frames: Vec<Frame>,
    awaiting: Option<(Vec<u64>, SourceSpan)>,
    started: bool,
    /// Attach the prelude library after the user crate (pending until asked).
    prelude: bool,
    /// Prelude units are library inputs, not user source telemetry.
    in_prelude: bool,
    /// `cfg(test)`: synthesize the native test harness entry after the crate.
    harness: Option<Vec<source_attrs::TestItem>>,
    test_filter: TestFilter,
    /// Library target without a harness: append an inert entry.
    library: bool,
    externs: Vec<String>,
    next_extern: usize,
    in_extern: bool,
}
struct Frame {
    unit: SharedFileUnit,
    id: FileId,
    ordinal: Vec<u64>,
    next_child: usize,
    segment: usize,
}
fn remap_span(span: SourceSpan, id: FileId) -> SourceSpan {
    SourceSpan {
        file: if span.file == LOCAL_FILE {
            id
        } else {
            span.file
        },
        ..span
    }
}
fn copy_error(error: &SourceError, primary: Option<SourceSpan>) -> SourceError {
    SourceError {
        kind: error.kind,
        primary,
        message: error.message.clone(),
        _diagnostic_lease: None,
        location: None,
    }
}
impl SourceAssembler {
    pub fn new(options: &SourceOptions) -> Self {
        Self {
            prepared: PreparedSource {
                edition: options.edition,
                debug_assertions: options.cfg.debug_assertions(),
                overflow_checks: options.overflow_checks,
                ..PreparedSource::default()
            },
            frames: Vec::new(),
            awaiting: None,
            started: false,
            prelude: options.prelude,
            in_prelude: false,
            harness: options.cfg.names.contains("test").then(Vec::new),
            library: options.library && !options.cfg.names.contains("test"),
            test_filter: options.test_filter.clone(),
            externs: options.externs.iter().map(|e| e.name.clone()).collect(),
            next_extern: 0,
            in_extern: false,
        }
    }
    /// Assemble only the user crate, without the prelude library.
    pub fn user_only(options: &SourceOptions) -> Self {
        Self {
            prelude: false,
            externs: Vec::new(),
            ..Self::new(options)
        }
    }
    fn fail(&self, error: SourceError) -> SourceError {
        error.with_sources(&self.prepared.sources)
    }
    /// Supply the unit for the root (first call) or the last requested child.
    pub fn provide(&mut self, unit: Arc<UnitResult>) -> Result<AssemblyStep, SourceError> {
        let (ordinal, anchor) = match self.awaiting.take() {
            Some((ordinal, _)) if ordinal.len() == 1 && ordinal[0] >= 1 => (ordinal, None),
            Some((ordinal, anchor)) => (ordinal, Some(anchor)),
            None if !self.started => (vec![0], None),
            None => {
                return Err(Loader::error(
                    SourceErrorKind::Contract,
                    None,
                    "source assembler received an unrequested unit",
                ));
            }
        };
        self.started = true;
        let unit = match unit.as_ref() {
            Err(failure) => {
                let primary = if failure.anchored {
                    anchor
                } else {
                    failure.error.primary
                };
                return Err(self.fail(copy_error(&failure.error, primary)));
            }
            Ok(_) => SharedFileUnit(unit),
        };
        if unit.edition != self.prepared.edition {
            return Err(self.fail(Loader::error(
                SourceErrorKind::Unsupported,
                None,
                "mixed source editions require per-crate lexical transport",
            )));
        }
        if ordinal == [0] && unit.no_std && (!self.library || self.harness.is_some()) {
            return Err(self.fail(Loader::error(
                SourceErrorKind::Unsupported,
                None,
                "no_std executable startup and panic-handler semantics are unsupported; only library targets and dependencies are admitted",
            )));
        }
        if unit.contents.len() > MAX_BYTES - self.prepared.source_bytes {
            return Err(self.fail(Loader::error(
                SourceErrorKind::Limit,
                None,
                format!("source size limit ({MAX_BYTES} bytes) exceeded"),
            )));
        }
        if !self.in_prelude && !self.in_extern {
            self.prepared.source_bytes += unit.contents.len();
            self.prepared.files += 1;
            if let Some(tests) = &mut self.harness {
                for test in &unit.tests {
                    let mut test = test.clone();
                    let mut path = unit.module.clone();
                    path.append(&mut test.path);
                    test.path = path;
                    tests.push(test);
                }
            }
        }
        let id = self
            .prepared
            .sources
            .add(unit.path.clone(), unit.contents.clone());
        if let Some(attachment) = source_embedded::EmbeddedFile::attach(id, &unit)? {
            self.prepared.embedded_files.push(attachment);
        }
        // Each attachment starts a separate crate, regardless of its mounted
        // module spelling or filesystem path. Descendant files inherit it.
        let crate_root = self.frames.first().map_or(id, |frame| frame.id);
        self.prepared.crate_roots.push(crate_root);
        if crate_root == id {
            self.prepared.crate_policies.push(CratePolicy {
                root: id,
                original: unit.contents.clone(),
                no_std: unit.no_std,
                forbid_unsafe_code: unit.forbid_unsafe_code,
            });
        }
        if ordinal.len() == 1
            && ordinal[0] >= 2
            && let Some(name) = self.externs.get(ordinal[0] as usize - 2)
        {
            let anchor = SourceSpan {
                file: id,
                lo: 0,
                hi: 0,
            };
            let open = format!("\npub mod {EXTERN_PREFIX}{name} {{\n");
            if let Err(error) = append_segment(&mut self.prepared, &open, anchor, true) {
                return Err(self.fail(error));
            }
        }
        for attribute in &unit.attributes {
            self.prepared.attributes.push(PreparedAttribute {
                span: remap_span(attribute.span, id),
                name: attribute.name.clone(),
                effective: attribute.effective,
            });
        }
        self.frames.push(Frame {
            unit,
            id,
            ordinal,
            next_child: 0,
            segment: 0,
        });
        self.run()
    }
    /// Copy a unit only for tests that deliberately forge source facts.
    #[cfg(test)]
    fn take(unit: Arc<UnitResult>) -> FileUnit {
        match Arc::try_unwrap(unit) {
            Ok(Ok(unit)) => unit,
            Ok(Err(_)) => unreachable!("failures are handled before take"),
            Err(shared) => {
                let Ok(unit) = shared.as_ref() else {
                    unreachable!("failures are handled before take")
                };
                FileUnit {
                    edition: unit.edition,
                    embedded: unit.embedded.clone(),
                    no_std: unit.no_std,
                    forbid_unsafe_code: unit.forbid_unsafe_code,
                    path: unit.path.clone(),
                    contents: unit.contents.clone(),
                    chain: unit.chain.clone(),
                    text: unit.text.clone(),
                    origins: unit.origins.clone(),
                    attributes: unit.attributes.clone(),
                    children: unit.children.clone(),
                    module: unit.module.clone(),
                    tests: unit.tests.clone(),
                    terminal: unit.terminal.as_ref().map(|e| copy_error(e, e.primary)),
                }
            }
        }
    }
    fn run(&mut self) -> Result<AssemblyStep, SourceError> {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                let anchor = SourceSpan {
                    file: FileId(0),
                    lo: 0,
                    hi: 0,
                };
                if !self.in_prelude && !self.in_extern {
                    let entry = match self.harness.take() {
                        Some(tests) => Some(test_harness(tests, &self.test_filter)),
                        None if std::mem::take(&mut self.library) => {
                            Some("\nfn main() {}\n".to_string())
                        }
                        None => None,
                    };
                    if let Some(entry) = entry
                        && let Err(error) = append_segment(&mut self.prepared, &entry, anchor, true)
                    {
                        return Err(self.fail(error));
                    }
                }
                if self.next_extern < self.externs.len() {
                    let index = self.next_extern;
                    self.next_extern += 1;
                    self.in_extern = true;
                    let ordinal = vec![2 + index as u64];
                    self.awaiting = Some((ordinal.clone(), Loader::span(LOCAL_FILE, 0..0)));
                    return Ok(AssemblyStep::Need(NeedUnit {
                        ordinal,
                        parent: None,
                        index,
                    }));
                }
                if self.prelude {
                    self.prelude = false;
                    self.in_prelude = true;
                    let ordinal = vec![1];
                    self.awaiting = Some((ordinal.clone(), Loader::span(LOCAL_FILE, 0..0)));
                    return Ok(AssemblyStep::Need(NeedUnit {
                        ordinal,
                        parent: None,
                        index: 0,
                    }));
                }
                if !self.prepared.embedded_files.is_empty() {
                    self.prepared.embedded_transport =
                        Some(source_embedded::AssemblyTransport::issue(&self.prepared)?);
                }
                return Ok(AssemblyStep::Done(Box::new(std::mem::take(
                    &mut self.prepared,
                ))));
            };
            let unit = frame.unit.clone();
            let id = frame.id;
            let end = unit
                .children
                .get(frame.next_child)
                .map_or(unit.text.len(), |child| child.at);
            while let Some(segment) = unit.origins.get(frame.segment)
                && segment.prepared.end <= end
            {
                frame.segment += 1;
                if let Err(error) = append_segment(
                    &mut self.prepared,
                    &unit.text[segment.prepared.clone()],
                    remap_span(segment.origin, id),
                    segment.synthetic,
                ) {
                    return Err(self.fail(error));
                }
            }
            let frame = self.frames.last_mut().expect("frame checked above");
            if let Some(child) = unit.children.get(frame.next_child) {
                let index = frame.next_child;
                frame.next_child += 1;
                let mut ordinal = frame.ordinal.clone();
                ordinal.push(index as u64);
                let anchor = remap_span(child.decl, id);
                if self.frames.len() > MAX_FILE_DEPTH {
                    return Err(self.fail(Loader::error(
                        SourceErrorKind::Limit,
                        Some(anchor),
                        format!("module file depth limit ({MAX_FILE_DEPTH}) exceeded"),
                    )));
                }
                if self.prepared.files >= MAX_FILES {
                    return Err(self.fail(Loader::error(
                        SourceErrorKind::Limit,
                        Some(anchor),
                        format!("module file limit ({MAX_FILES}) exceeded"),
                    )));
                }
                self.awaiting = Some((ordinal.clone(), anchor));
                return Ok(AssemblyStep::Need(NeedUnit {
                    ordinal,
                    parent: Some(unit),
                    index,
                }));
            }
            if let Some(error) = &unit.terminal {
                let primary = error.primary.map(|span| remap_span(span, id));
                return Err(self.fail(copy_error(error, primary)));
            }
            if let Some(frame) = self.frames.pop()
                && frame.ordinal.len() == 1
                && frame.ordinal[0] >= 2
            {
                let anchor = SourceSpan {
                    file: frame.id,
                    lo: 0,
                    hi: 0,
                };
                if let Err(error) = append_segment(&mut self.prepared, "\n}\n", anchor, true) {
                    return Err(self.fail(error));
                }
            }
        }
    }
}
/// The `cfg(test)` entry point: run every `#[test]` in name order through
/// the native harness (`libraries/prelude/test.rs`), like libtest.
fn test_harness(mut tests: Vec<source_attrs::TestItem>, filter: &TestFilter) -> String {
    tests.sort_by(|a, b| a.path.cmp(&b.path));
    let total = tests.len();
    tests.retain(|test| filter.selects(&test.path.join("::")) && (!filter.ignored || test.ignore));
    let mut out = format!(
        "\nfn main() {{\n    let mut __h = std::test::Harness::start({}usize);\n    __h.filtered_out({}usize);\n",
        tests.len(),
        total - tests.len()
    );
    for test in &mut tests {
        if filter.ignored || filter.include_ignored {
            test.ignore = false;
        }
    }
    for test in &tests {
        let name = test.path.join("::");
        let (mode, expected) = match &test.should_panic {
            None => (0, String::new()),
            Some(None) => (1, String::new()),
            Some(Some(expected)) => (2, expected.clone()),
        };
        let quote = |s: &str| format!("{s:?}");
        out.push_str(&format!(
            "    if __h.begin({name:?}, {}) {{ crate::{name}(); std::process::exit(0); }}\n",
            test.ignore
        ));
        if !test.ignore {
            out.push_str(&format!(
                "    __h.end({name:?}, {mode}usize, {});\n",
                quote(&expected)
            ));
        }
    }
    out.push_str("    __h.finish();\n}\n");
    out
}
/// Append prepared text with provenance, merging contiguous original bytes.
fn append_segment(
    prepared: &mut PreparedSource,
    text: &str,
    origin: SourceSpan,
    synthetic: bool,
) -> Result<(), SourceError> {
    append_to(
        &mut prepared.text,
        &mut prepared.origins,
        text,
        origin,
        synthetic,
    )
}
fn append_to(
    out: &mut String,
    origins: &mut Vec<OriginSegment>,
    text: &str,
    origin: SourceSpan,
    synthetic: bool,
) -> Result<(), SourceError> {
    if text.len() > MAX_BYTES - out.len() {
        return Err(Loader::error(
            SourceErrorKind::Limit,
            Some(origin),
            format!("expanded source size limit ({MAX_BYTES} bytes) exceeded"),
        ));
    }
    let start = out.len();
    out.push_str(text);
    if let Some(last) = origins.last_mut()
        && !synthetic
        && !last.synthetic
        && last.prepared.end == start
        && last.origin.file == origin.file
        && last.origin.hi == origin.lo
    {
        last.prepared.end += text.len();
        last.origin.hi = origin.hi;
        return Ok(());
    }
    if origins.len() >= MAX_ORIGINS {
        return Err(Loader::error(
            SourceErrorKind::Limit,
            Some(origin),
            "source provenance segment limit exceeded",
        ));
    }
    origins.push(OriginSegment {
        prepared: start..start + text.len(),
        origin,
        synthetic,
    });
    Ok(())
}
trait SourceReader {
    fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf>;
    fn is_file(&mut self, p: &Path) -> std::io::Result<bool>;
    fn exists(&mut self, p: &Path) -> std::io::Result<bool>;
    fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>>;
}
struct FsReader;
impl SourceReader for FsReader {
    fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf> {
        p.canonicalize()
    }
    fn is_file(&mut self, p: &Path) -> std::io::Result<bool> {
        Ok(std::fs::metadata(p)?.is_file())
    }
    fn exists(&mut self, p: &Path) -> std::io::Result<bool> {
        p.try_exists()
    }
    fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
        let file = File::open(p)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("source must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
/// Builds one file's unit: validation, configuration, inline-module expansion
/// and discovery of external child modules. Child files are never read here.
struct Loader<'a> {
    options: &'a SourceOptions,
    reader: Option<&'a mut dyn SourceReader>,
    lexical_budget: super::expansion_budget::ExpansionBudget,
    id: FileId,
    contents: Arc<str>,
    /// First contributor to the assembled text (empty-file provenance anchor).
    root: bool,
    text: String,
    origins: Vec<OriginSegment>,
    attributes: Vec<PreparedAttribute>,
    children: Vec<ChildModule>,
    derives: Vec<(usize, String)>,
    /// Inline module names enclosing the text being expanded.
    module_stack: Vec<String>,
    no_std: bool,
    forbid_unsafe_code: bool,
    tests: Vec<source_attrs::TestItem>,
}
impl<'a> Loader<'a> {
    fn new(
        options: &'a SourceOptions,
        reader: Option<&'a mut dyn SourceReader>,
        id: FileId,
        contents: Arc<str>,
        root: bool,
    ) -> Self {
        Self {
            options,
            reader,
            lexical_budget: super::expansion_budget::ExpansionBudget::default(),
            id,
            contents,
            root,
            text: String::new(),
            origins: Vec::new(),
            attributes: Vec::new(),
            children: Vec::new(),
            derives: Vec::new(),
            module_stack: Vec::new(),
            no_std: false,
            forbid_unsafe_code: false,
            tests: Vec::new(),
        }
    }
    fn error(
        kind: SourceErrorKind,
        primary: Option<SourceSpan>,
        message: impl Into<String>,
    ) -> SourceError {
        SourceError {
            kind,
            primary,
            message: message.into(),
            _diagnostic_lease: None,
            location: None,
        }
    }
    fn span(file: FileId, r: Range<usize>) -> SourceSpan {
        SourceSpan {
            file,
            lo: r.start as u32,
            hi: r.end as u32,
        }
    }
    fn snapshot(&mut self, directory: &Path, depth: usize) -> Result<(), SourceError> {
        let id = self.id;
        let original = self.contents.clone();
        self.validate_original(&original, 0)?;
        let filtered = source_attrs::prepare(&original, &self.options.cfg).map_err(|e| {
            Self::error(
                match e.kind {
                    source_attrs::AttributeErrorKind::Syntax => SourceErrorKind::Syntax,
                    source_attrs::AttributeErrorKind::Unsupported => SourceErrorKind::Unsupported,
                    source_attrs::AttributeErrorKind::Limit => SourceErrorKind::Limit,
                },
                Some(Self::span(id, e.range)),
                e.message,
            )
        })?;
        // Disabled syntax is checked without name resolution, typing or child IO.
        // Legal Rust outside this parser's bounded grammar remains Unsupported,
        // not a claim that the disabled program is invalid or full cfg is supported.
        let no_std: Vec<_> = filtered
            .attributes
            .iter()
            .filter(|a| a.name == "no_std" && a.effective)
            .collect();
        if no_std.len() > 1 || (!self.root && !no_std.is_empty()) {
            return Err(Self::error(
                SourceErrorKind::Unsupported,
                no_std.first().map(|a| Self::span(id, a.range.clone())),
                "no_std must occur once at a crate root",
            ));
        }
        self.no_std = !no_std.is_empty();
        if filtered.forbid_unsafe_code && !self.root {
            return Err(Self::error(
                SourceErrorKind::Unsupported,
                None,
                "forbid(unsafe_code) is currently supported only at a crate root",
            ));
        }
        self.forbid_unsafe_code = filtered.forbid_unsafe_code;
        for inactive in filtered.inactive_items {
            let syntax = inactive_syntax(&inactive.text).map_err(|message| {
                Self::error(
                    SourceErrorKind::Syntax,
                    Some(Self::span(id, inactive.range.clone())),
                    message,
                )
            })?;
            super::frontend::validate_inactive_syntax(&syntax).map_err(|e| {
                Self::error(
                    e.kind,
                    e.has_location.then(|| {
                        Self::span(
                            id,
                            inactive.range.start + e.bytes.start
                                ..inactive.range.start + e.bytes.end,
                        )
                    }),
                    format!("inactive item: {}", e.message),
                )
            })?;
        }
        for a in filtered.attributes {
            self.attributes.push(PreparedAttribute {
                span: Self::span(id, a.range),
                name: a.name,
                effective: a.effective,
            });
        }
        self.derives = filtered.derives;
        self.tests = filtered.tests;
        let mut text = filtered.text;
        if self.root
            && let Some(name) = filtered.test_main
        {
            // Under cfg(test) the harness is the entry; the crate's own main
            // stays checked under a private name, as libtest leaves it unused.
            text.replace_range(name.clone(), &" ".repeat(name.len()));
            self.derives
                .push((name.start + 1, "__justrust_user_main".into()));
            self.derives.sort_by_key(|(at, _)| *at);
        }
        self.expand(&text, 0, directory, depth)
    }
    /// Lexical validation only, before configuration processing or module IO.
    /// No token input counts are charged until the configured lexer actually
    /// issues token events.
    fn validate_original(&self, text: &str, base: usize) -> Result<(), SourceError> {
        use super::lexer::{LexError, validate_source};
        let id = self.id;
        validate_source(text, self.options.edition, &self.lexical_budget).map_err(|error| {
            match error {
                LexError::Source {
                    kind,
                    prepared,
                    message,
                    ..
                } => {
                    let valid = text.get(prepared.clone());
                    let start = base.checked_add(prepared.start);
                    let end = base.checked_add(prepared.end);
                    let raw = start
                        .zip(end)
                        .filter(|(lo, hi)| valid.is_some() && self.contents.get(*lo..*hi) == valid);
                    match raw {
                        Some((lo, hi)) => Self::error(kind, Some(Self::span(id, lo..hi)), message),
                        None => Self::error(
                            SourceErrorKind::Contract,
                            None,
                            "raw lexer returned an invalid original-source range",
                        ),
                    }
                }
                LexError::Transport(error) => SourceError::from_token(error),
                LexError::Projection(error) => error,
            }
        })
    }
    fn append(
        &mut self,
        text: &str,
        origin: SourceSpan,
        synthetic: bool,
    ) -> Result<(), SourceError> {
        append_to(&mut self.text, &mut self.origins, text, origin, synthetic)
    }
    fn copy(&mut self, text: &str, base: usize, r: Range<usize>) -> Result<(), SourceError> {
        // Built-in derive output is synthetic text inserted right after its item.
        if let Some(index) = self
            .derives
            .iter()
            .position(|(at, _)| *at > base + r.start && *at <= base + r.end)
        {
            let (at, generated) = self.derives.remove(index);
            let split = at - base;
            self.copy(text, base, r.start..split)?;
            let anchor = Self::span(self.id, at.saturating_sub(1)..at);
            self.append(&generated, anchor, true)?;
            return self.copy(text, base, split..r.end);
        }
        let id = self.id;
        let original = self.contents.clone();
        let mut start = r.start;
        while start < r.end {
            let synthetic = text.as_bytes()[start] != original.as_bytes()[base + start];
            let mut end = start + 1;
            while end < r.end
                && (text.as_bytes()[end] != original.as_bytes()[base + end]) == synthetic
            {
                end += 1;
            }
            self.append(
                &text[start..end],
                Self::span(id, base + start..base + end),
                synthetic,
            )?;
            start = end;
        }
        if r.is_empty() && self.root && self.origins.is_empty() {
            self.append("", Self::span(id, base + r.start..base + r.end), false)?;
        }
        Ok(())
    }
    fn expand(
        &mut self,
        text: &str,
        base: usize,
        directory: &Path,
        depth: usize,
    ) -> Result<(), SourceError> {
        let id = self.id;
        let anchor = Some(Self::span(id, base..base + text.len()));
        if depth > MAX_MODULE_DEPTH {
            return Err(Self::error(
                SourceErrorKind::Limit,
                anchor,
                format!("module nesting exceeds limit {MAX_MODULE_DEPTH}"),
            ));
        }
        let tokens = tokens(text).map_err(|e| Self::error(SourceErrorKind::Syntax, anchor, e))?;
        let mut closing = Vec::new();
        for t in &tokens {
            match t.text(text) {
                "{" => closing.push("}"),
                "[" => closing.push("]"),
                "(" => closing.push(")"),
                c @ ("}" | "]" | ")") => {
                    if closing.pop() != Some(c) {
                        return Err(Self::error(
                            SourceErrorKind::Syntax,
                            anchor,
                            "unbalanced delimiter in source file",
                        ));
                    }
                }
                _ => {}
            }
        }
        if !closing.is_empty() {
            return Err(Self::error(
                SourceErrorKind::Syntax,
                anchor,
                "unclosed delimiter in source file",
            ));
        }
        let mut copied = 0;
        let mut i = 0;
        while i < tokens.len() {
            let mut head = i;
            if tokens[head].text(text) == "pub" {
                head += 1;
                if tokens.get(head).is_some_and(|t| t.text(text) == "(") {
                    head = close_group(&tokens, text, head) + 1;
                }
            }
            if tokens.get(head).is_some_and(|t| t.text(text) == "mod")
                && head + 2 < tokens.len()
                && syntactic_identifier(tokens[head + 1].text(text))
            {
                let name = tokens[head + 1].text(text);
                let delimiter = tokens[head + 2].text(text);
                if delimiter == ";" {
                    let decl =
                        Self::span(id, base + tokens[head].start..base + tokens[head + 2].end);
                    let reader = self.reader.as_deref_mut().ok_or_else(|| {
                        Self::error(
                            SourceErrorKind::Unsupported,
                            Some(decl),
                            "external modules are unsupported in text input",
                        )
                    })?;
                    let flat = directory.join(format!("{name}.rs"));
                    let nested = directory.join(name).join("mod.rs");
                    let a = reader.exists(&flat).map_err(|e| {
                        Self::error(
                            SourceErrorKind::Io,
                            Some(decl),
                            format!("{}: {e}", flat.display()),
                        )
                    })?;
                    let b = reader.exists(&nested).map_err(|e| {
                        Self::error(
                            SourceErrorKind::Io,
                            Some(decl),
                            format!("{}: {e}", nested.display()),
                        )
                    })?;
                    let path = match (a, b) {
                        (true, false) => flat,
                        (false, true) => nested,
                        (true, true) => {
                            return Err(Self::error(
                                SourceErrorKind::Module,
                                Some(decl),
                                format!(
                                    "ambiguous module {name}: both {} and {} exist",
                                    flat.display(),
                                    nested.display()
                                ),
                            ));
                        }
                        (false, false) => {
                            return Err(Self::error(
                                SourceErrorKind::Module,
                                Some(decl),
                                format!(
                                    "module {name} not found: expected {} or {}",
                                    flat.display(),
                                    nested.display()
                                ),
                            ));
                        }
                    };
                    self.copy(text, base, copied..tokens[head + 2].start)?;
                    self.append("{\n", decl, true)?;
                    self.children.push(ChildModule {
                        at: self.text.len(),
                        decl,
                        path,
                        directory: directory.join(name),
                        depth: depth + 1,
                        module: {
                            let mut module = self.module_stack.clone();
                            module.push(name.trim_start_matches("r#").to_string());
                            module
                        },
                    });
                    self.append("\n}", decl, true)?;
                    copied = tokens[head + 2].end;
                    i = head + 3;
                    continue;
                }
                if delimiter == "{" {
                    let end = close_group(&tokens, text, head + 2);
                    self.copy(text, base, copied..tokens[head + 2].end)?;
                    self.module_stack
                        .push(name.trim_start_matches("r#").to_string());
                    let expanded = self.expand(
                        &text[tokens[head + 2].end..tokens[end].start],
                        base + tokens[head + 2].end,
                        &directory.join(name),
                        depth + 1,
                    );
                    self.module_stack.pop();
                    expanded?;
                    copied = tokens[end].start;
                    i = end + 1;
                    continue;
                }
            }
            // Skip the complete non-module item, including signature and macro token trees.
            while i < tokens.len() {
                match tokens[i].text(text) {
                    ";" => {
                        i += 1;
                        break;
                    }
                    "{" => {
                        i = close_group(&tokens, text, i) + 1;
                        break;
                    }
                    "(" | "[" => i = close_group(&tokens, text, i) + 1,
                    _ => i += 1,
                }
            }
        }
        self.copy(text, base, copied..text.len())
    }
}
fn close_group(tokens: &[Token], text: &str, start: usize) -> usize {
    let mut depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(start) {
        match t.text(text) {
            "{" | "[" | "(" => depth += 1,
            "}" | "]" | ")" => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

/// Strip structurally complete external declarations from inactive syntax only.
/// Their grammar is validated here, but no module resolution or IO is possible.
fn inactive_syntax(text: &str) -> Result<String, String> {
    let tokens = tokens(text)?;
    let mut closing = Vec::new();
    for t in &tokens {
        match t.text(text) {
            "{" => closing.push("}"),
            "[" => closing.push("]"),
            "(" => closing.push(")"),
            c @ ("}" | "]" | ")") => {
                if closing.pop() != Some(c) {
                    return Err("unbalanced delimiter in inactive item".into());
                }
            }
            _ => {}
        }
    }
    if !closing.is_empty() {
        return Err("unclosed delimiter in inactive item".into());
    }
    fn items(
        tokens: &[Token],
        text: &str,
        start: usize,
        end: usize,
        depth: usize,
        out: &mut [u8],
    ) -> Result<(), String> {
        if depth > MAX_MODULE_DEPTH {
            return Err(format!("module nesting exceeds limit {MAX_MODULE_DEPTH}"));
        }
        let mut i = start;
        while i < end {
            let mut head = i;
            if tokens[head].text(text) == "pub" {
                head += 1;
                if tokens.get(head).is_some_and(|t| t.text(text) == "(") {
                    head = close_group(tokens, text, head) + 1;
                }
                inactive_visibility(tokens, text, i, head)?;
                if head > i + 1 {
                    for b in &mut out[tokens[i + 1].start..tokens[head - 1].end] {
                        if *b != b'\n' && *b != b'\r' {
                            *b = b' ';
                        }
                    }
                }
            }
            if head + 2 < end
                && tokens[head].text(text) == "mod"
                && syntactic_identifier(tokens[head + 1].text(text))
            {
                if tokens[head + 2].text(text) == ";" {
                    inactive_visibility(tokens, text, i, head)?;
                    for b in &mut out[tokens[i].start..tokens[head + 2].end] {
                        if *b != b'\n' && *b != b'\r' {
                            *b = b' ';
                        }
                    }
                    i = head + 3;
                    continue;
                }
                if tokens[head + 2].text(text) == "{" {
                    let close = close_group(tokens, text, head + 2);
                    items(tokens, text, head + 3, close, depth + 1, out)?;
                    i = close + 1;
                    continue;
                }
            }
            while i < end {
                match tokens[i].text(text) {
                    ";" => {
                        i += 1;
                        break;
                    }
                    "{" => {
                        i = close_group(tokens, text, i) + 1;
                        break;
                    }
                    "(" | "[" => i = close_group(tokens, text, i) + 1,
                    _ => i += 1,
                }
            }
        }
        Ok(())
    }
    let mut out = text.as_bytes().to_vec();
    // The boundary scanner above already checked raw-string termination. Inactive
    // literals carry no values into typing, but still occupy one expression token.
    // Normalize only their transport spelling for the bounded syntax parser.
    for token in &tokens {
        let spelling = token.text(text);
        if spelling.starts_with("r\"")
            || spelling.starts_with("r#")
            || spelling.starts_with("br\"")
            || spelling.starts_with("br#")
        {
            for b in &mut out[token.start..token.end] {
                if *b != b'\n' && *b != b'\r' {
                    *b = b' ';
                }
            }
            out[token.start] = b'0';
        }
    }
    items(&tokens, text, 0, tokens.len(), 0, &mut out)?;
    Ok(String::from_utf8(out).expect("masking complete token ranges preserves UTF-8"))
}

/// Check only the visibility grammar. Disabled paths must not be resolved or
/// constrained to the active module's ancestors.
fn inactive_visibility(
    tokens: &[Token],
    text: &str,
    start: usize,
    head: usize,
) -> Result<(), String> {
    if head == start || head == start + 1 {
        return Ok(());
    }
    let restriction = &tokens[start + 2..head - 1];
    if restriction.len() == 1 && matches!(restriction[0].text(text), "crate" | "self" | "super") {
        return Ok(());
    }
    if restriction.first().is_none_or(|t| t.text(text) != "in") {
        return Err(
            "incorrect visibility restriction: expected crate, self, super, or in path".into(),
        );
    }
    let mut i = 1;
    if restriction.get(i).is_some_and(|t| t.text(text) == ":")
        && restriction.get(i + 1).is_some_and(|t| t.text(text) == ":")
    {
        i += 2;
    }
    loop {
        if restriction.get(i).is_none_or(|t| {
            !matches!(t.text(text), "crate" | "self" | "super")
                && !syntactic_identifier(t.text(text))
        }) {
            return Err("expected identifier in visibility restriction".into());
        }
        i += 1;
        if i == restriction.len() {
            return Ok(());
        }
        if restriction.get(i).is_none_or(|t| t.text(text) != ":")
            || restriction.get(i + 1).is_none_or(|t| t.text(text) != ":")
        {
            return Err("expected path separator in visibility restriction".into());
        }
        i += 2;
    }
}

// Match the existing bounded frontend's identifier contract before stripping any
// inactive declaration. Lexical ASCII shape alone must not hide reserved keywords.
fn syntactic_identifier(s: &str) -> bool {
    identifier(s)
        && s != "_"
        && !matches!(
            s,
            "fn" | "let"
                | "mut"
                | "if"
                | "else"
                | "while"
                | "return"
                | "true"
                | "false"
                | "pub"
                | "const"
                | "static"
                | "loop"
                | "break"
                | "continue"
                | "for"
                | "in"
                | "match"
                | "struct"
                | "enum"
                | "use"
                | "mod"
                | "impl"
                | "trait"
                | "self"
                | "Self"
                | "as"
                | "async"
                | "await"
                | "move"
                | "unsafe"
                | "where"
                | "type"
                | "ref"
                | "extern"
                | "crate"
                | "super"
                | "dyn"
                | "abstract"
                | "become"
                | "box"
                | "do"
                | "final"
                | "macro"
                | "override"
                | "priv"
                | "typeof"
                | "unsized"
                | "virtual"
                | "yield"
                | "try"
                | "gen"
        )
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
struct Token {
    start: usize,
    end: usize,
}
impl Token {
    fn text<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start..self.end]
    }
}

/// A boundary scanner only. The frontend still owns syntax and semantics.
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1usize;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("unterminated block comment".into());
            }
            continue;
        }
        // Raw strings must hide all their braces and apparent module tokens.
        let raw_start = if b[i] == b'r' {
            Some(i + 1)
        } else if b[i..].starts_with(b"br") {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut quote) = raw_start {
            let hashes_start = quote;
            while quote < b.len() && b[quote] == b'#' {
                quote += 1;
            }
            if quote < b.len() && b[quote] == b'"' {
                let hashes = quote - hashes_start;
                i = quote + 1;
                loop {
                    if i >= b.len() {
                        return Err("unterminated raw string".into());
                    }
                    if b[i] == b'"'
                        && b.get(i + 1..i + 1 + hashes)
                            .is_some_and(|s| s.iter().all(|c| *c == b'#'))
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                out.push(Token { start, end: i });
                continue;
            }
        }
        // A named lifetime has no closing quote. Keep the apostrophe in the
        // token so names such as 'mod cannot masquerade as module keywords.
        // A closing quote immediately after the identifier instead denotes a
        // character literal, whose delimiters must remain hidden below.
        if b[i] == b'\''
            && b.get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            let mut end = i + 2;
            while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                end += 1;
            }
            if b.get(end) != Some(&b'\'') {
                i = end;
                out.push(Token { start, end: i });
                continue;
            }
        }
        if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                } else if b[i] == quote {
                    i += 1;
                    closed = true;
                    break;
                } else {
                    i += 1;
                }
            }
            if !closed {
                return Err("unterminated string or character literal".into());
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
        }
        out.push(Token { start, end: i });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_code_policy_is_source_authenticated_and_retained_in_file_views() {
        let options = SourceOptions::default();
        let source = "#![forbid(unsafe_code)] unsafe fn unused(){} fn main(){}";
        let unit = prepare_text_unit("main.rs", source, &options).unwrap();
        let view = FileView::of(&unit, 0, &options).unwrap();
        assert!(super::super::frontend::parse_prepared(&view.prepared).is_err());
        let mut prepared = prepare_text("main.rs", source, &options).unwrap();
        assert!(
            prepared.crate_policies[0]
                .forbid_unsafe_root(&prepared)
                .unwrap()
                .is_some()
        );
        prepared.sources.files[0].contents = Arc::from("fn main(){}");
        assert!(
            prepared.crate_policies[0]
                .forbid_unsafe_root(&prepared)
                .is_err()
        );
    }

    #[test]
    fn changed_overflow_policy_cannot_reuse_assembly_authority() {
        let options = SourceOptions::default();
        let mut prepared =
            prepare_text_with_prelude("profile.rs", "fn main(){}", &options).unwrap();
        prepared.overflow_checks = false;
        assert!(super::super::frontend::parse_prepared(&prepared).is_err());
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Default)]
    struct RecordingReader {
        calls: Vec<(&'static str, PathBuf)>,
        root: String,
        child: Option<String>,
    }
    impl SourceReader for RecordingReader {
        fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf> {
            self.calls.push(("canonicalize", p.to_owned()));
            Ok(p.to_owned())
        }
        fn is_file(&mut self, p: &Path) -> std::io::Result<bool> {
            self.calls.push(("stat", p.to_owned()));
            Ok(true)
        }
        fn exists(&mut self, p: &Path) -> std::io::Result<bool> {
            self.calls.push(("exists", p.to_owned()));
            Ok(self.child.is_some() && p == Path::new("child.rs"))
        }
        fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
            self.calls.push(("open", p.to_owned()));
            self.calls.push(("stat-opened", p.to_owned()));
            self.calls.push(("read", p.to_owned()));
            let text = if p == Path::new("main.rs") {
                &self.root
            } else {
                self.child.as_ref().unwrap()
            };
            Ok(text.as_bytes()[..text.len().min(limit)].to_vec())
        }
    }

    #[test]
    fn prepared_debug_assertions_capture_validated_options() {
        assert!(PreparedSource::default().debug_assertions());
        for enabled in [false, true] {
            let mut options = SourceOptions::default();
            options.cfg.set_debug_assertions(enabled);
            let text = "#[cfg(debug_assertions)] fn checked(){} #[cfg(not(debug_assertions))] fn unchecked(){} fn main(){}";
            let memory = prepare_text("main.rs", text, &options).unwrap();
            let mut reader = RecordingReader {
                root: text.into(),
                ..Default::default()
            };
            let file = prepare_with_reader(Path::new("main.rs"), &options, &mut reader).unwrap();
            options.cfg.set_debug_assertions(!enabled);
            for prepared in [memory, file] {
                assert_eq!(prepared.debug_assertions(), enabled);
                assert_eq!(prepared.clone().debug_assertions(), enabled);
                assert_eq!(prepared.text.contains("fn checked"), enabled);
                assert_eq!(prepared.text.contains("fn unchecked"), !enabled);
            }
        }
        let mut options = SourceOptions::default();
        options.cfg.names.remove("debug_assertions");
        assert!(prepare_text("main.rs", "fn main(){}", &options).is_err());
        let mut reader = RecordingReader::default();
        assert!(prepare_with_reader(Path::new("main.rs"), &options, &mut reader).is_err());
        assert!(reader.calls.is_empty());
    }

    #[test]
    fn source_edition_survives_file_views_and_mismatched_units_reject() {
        let options = SourceOptions::default();
        let mut unit = prepare_text_unit("edition.rs", "fn main(){}", &options).unwrap();
        assert_eq!(unit.edition, Edition::E2024);
        assert_eq!(
            FileView::of(&unit, 0, &options).unwrap().prepared.edition(),
            Edition::E2024
        );
        assert_eq!(
            prepare_text("edition.rs", "fn main(){}", &options)
                .unwrap()
                .edition(),
            Edition::E2024
        );
        unit.edition = Edition::E2021;
        let failure = SourceAssembler::new(&options)
            .provide(Arc::new(Ok(unit)))
            .err()
            .unwrap();
        assert_eq!(failure.kind, SourceErrorKind::Unsupported);
        assert!(failure.message.contains("mixed source editions"));
    }

    #[test]
    fn unsupported_source_editions_reject_before_reading_or_parsing() {
        assert_eq!(SourceOptions::default().edition, Edition::E2024);
        assert_eq!(
            Edition::parse("2021").unwrap(),
            super::super::token::Edition::E2021
        );
        assert!(Edition::parse("2099").is_err());
        assert!(Edition::parse(" 2024").is_err());
        for name in ["2015", "2018", "2021"] {
            let options = SourceOptions {
                edition: Edition::parse(name).unwrap(),
                ..SourceOptions::default()
            };
            let mut reader = RecordingReader::default();
            let file =
                prepare_with_reader(Path::new("main.rs"), &options, &mut reader).unwrap_err();
            assert_eq!(file.kind, SourceErrorKind::Unsupported);
            assert!(file.message.contains(name));
            assert!(reader.calls.is_empty());
            let memory = prepare_text("main.rs", "not valid Rust", &options).unwrap_err();
            assert_eq!(memory.kind, SourceErrorKind::Unsupported);
            assert!(memory.message.contains(name));
        }
    }

    #[test]
    fn disabled_external_modules_make_zero_child_observations() {
        for root in [
            "#[cfg(any())] mod missing; fn main(){}",
            "#[cfg(any())] #[path=\"other.rs\"] mod missing; fn main(){}",
            "mod inline { #![cfg(any())] mod missing; } fn main(){}",
            "#![cfg(any())] mod missing;",
            "#[cfg(any())] mod inline { mod missing; } fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let prepared =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap();
            assert_eq!(prepared.files, 1);
            assert!(
                reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")),
                "{root}: {:?}",
                reader.calls
            );
            assert_eq!(
                reader
                    .calls
                    .iter()
                    .map(|(kind, _)| *kind)
                    .collect::<Vec<_>>(),
                ["canonicalize", "stat", "open", "stat-opened", "read"]
            );
        }
    }
    #[test]
    fn disabled_item_macro_calls_validate_syntax_without_child_observations() {
        for root in [
            "#[cfg(any())] undefined!(); fn main(){}",
            "macro_rules! make{()=>{mod missing;};} #[cfg(any())] make!(); fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let prepared =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap();
            assert_eq!(prepared.files, 1);
            super::super::frontend::parse_prepared(&prepared).unwrap();
            assert_eq!(
                reader
                    .calls
                    .iter()
                    .map(|(kind, path)| {
                        assert_eq!(path, Path::new("main.rs"), "{root}: {:?}", reader.calls);
                        *kind
                    })
                    .collect::<Vec<_>>(),
                ["canonicalize", "stat", "open", "stat-opened", "read"]
            );
        }
    }

    #[test]
    fn unadmitted_generated_modules_and_attributes_make_zero_child_observations() {
        for root in [
            "macro_rules! make{()=>{mod missing;};} make!(); fn main(){}",
            "macro_rules! make{()=>{#[cfg(any())] mod missing;};} make!(); fn main(){}",
            "macro_rules! make{()=>{#[path=\"missing.rs\"] mod missing;};} make!(); fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let prepared =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap();
            assert_eq!(prepared.files, 1);
            let error = super::super::frontend::parse_prepared(&prepared).unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Unsupported, "{root}: {error}");
            assert!(error.primary.is_some(), "{root}: {error}");
            assert_eq!(
                reader
                    .calls
                    .iter()
                    .map(|(kind, path)| {
                        assert_eq!(path, Path::new("main.rs"), "{root}: {:?}", reader.calls);
                        *kind
                    })
                    .collect::<Vec<_>>(),
                ["canonicalize", "stat", "open", "stat-opened", "read"]
            );
        }
    }

    #[test]
    fn associated_doc_policy_precedes_child_io_and_keeps_memory_file_parity() {
        for (root, expected) in [
            (
                "trait T{#[doc=]fn f();}mod child;fn main(){}",
                SourceErrorKind::Syntax,
            ),
            (
                "trait T{#[doc=42]fn f();}mod child;fn main(){}",
                SourceErrorKind::Unsupported,
            ),
            (
                "#[cfg(any())]trait T{#[doc=]fn f();}mod child;fn main(){}",
                SourceErrorKind::Syntax,
            ),
            (
                "#[cfg(any())]trait T{#[doc=\"\\q\"]fn f();}mod child;fn main(){}",
                SourceErrorKind::Syntax,
            ),
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                child: Some("pub fn child(){}".into()),
                ..Default::default()
            };
            let file =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            let memory = prepare_text("main.rs", root, &SourceOptions::default()).unwrap_err();
            assert_eq!(file.kind, expected, "{root}: {file}");
            assert_eq!(memory.kind, expected, "{root}: {memory}");
            assert_eq!(file.primary, memory.primary);
            assert!(
                reader
                    .calls
                    .iter()
                    .all(|(_, path)| path == Path::new("main.rs")),
                "{:?}",
                reader.calls
            );
        }
        let root = "trait T{/// required\nfn f();}#[cfg(any())]mod missing;fn main(){}";
        let mut reader = RecordingReader {
            root: root.into(),
            ..Default::default()
        };
        let file =
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
        let memory = prepare_text("main.rs", root, &SourceOptions::default()).unwrap();
        assert_eq!(file.text, memory.text);
        assert_eq!(file.files, 1);
        assert!(
            reader
                .calls
                .iter()
                .all(|(_, path)| path == Path::new("main.rs"))
        );
        assert_eq!(file.attributes.len(), memory.attributes.len());
    }
    #[test]
    fn full_original_lexing_precedes_configuration_and_all_child_observations() {
        for root in [
            "#[cfg(any())] fn hidden(){let text=\"\\q\";} mod child; fn main(){}",
            "#[cfg(any())] mod child; fn main(){let text=\"\\u{D800}\";}",
            "#[unknown_attribute] fn hidden(){} mod child; fn main(){`}",
            "mod child; fn main(){let text=r###\"unterminated;}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                child: Some("pub fn child(){}".into()),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{root}: {error}");
            assert!(error.primary.is_some(), "{root}: {error}");
            assert_eq!(error.location.as_ref().unwrap().path, Path::new("main.rs"));
            assert!(
                reader
                    .calls
                    .iter()
                    .all(|(_, path)| path == Path::new("main.rs")),
                "{root}: {:?}",
                reader.calls
            );
            assert_eq!(
                reader
                    .calls
                    .iter()
                    .map(|(kind, _)| *kind)
                    .collect::<Vec<_>>(),
                ["canonicalize", "stat", "open", "stat-opened", "read"]
            );
            let error = prepare_text("text.rs", root, &SourceOptions::default()).unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{root}: {error}");
            assert_eq!(error.location.as_ref().unwrap().path, Path::new("text.rs"));
        }
    }
    #[test]
    fn source_validation_uses_one_shared_work_authority_across_files() {
        use super::super::expansion_budget::{Counter, ExpansionBudget, ExpansionLimits};
        use super::super::lexer::validate_source;
        use super::super::token::Edition;
        let root = "mod child; fn main(){}";
        let child = "pub fn child(){}";
        let root_budget = ExpansionBudget::default();
        validate_source(root, Edition::E2024, &root_budget).unwrap();
        let child_budget = ExpansionBudget::default();
        validate_source(child, Edition::E2024, &child_budget).unwrap();
        let total = root_budget.snapshot().counters[Counter::Work as usize]
            + child_budget.snapshot().counters[Counter::Work as usize];
        let mut limits = ExpansionLimits::default();
        limits.counters[Counter::Work as usize] = total - 1;
        let budget = ExpansionBudget::new(limits);
        let mut reader = RecordingReader {
            root: root.into(),
            child: Some(child.into()),
            ..Default::default()
        };
        let error = prepare_with_reader_budget(
            Path::new("main.rs"),
            &SourceOptions::default(),
            &mut reader,
            &budget,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Limit, "{error}");
        assert!(
            reader
                .calls
                .iter()
                .any(|(kind, path)| *kind == "read" && path == Path::new("child.rs"))
        );
        assert_eq!(budget.snapshot().counters[Counter::InputTokens as usize], 0);
    }

    #[test]
    fn inactive_external_file_inner_cfg_does_not_observe_descendants() {
        let mut reader = RecordingReader {
            root: "mod child; fn main(){}".into(),
            child: Some("#![cfg(any())] mod missing;".into()),
            ..Default::default()
        };
        let prepared =
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
        assert_eq!(prepared.files, 2);
        assert!(
            !reader
                .calls
                .iter()
                .any(|(_, p)| p.to_string_lossy().contains("missing"))
        );
        assert_eq!(
            reader
                .calls
                .iter()
                .filter(|(kind, _)| *kind == "exists")
                .count(),
            2
        );
    }

    #[test]
    fn invalid_options_observe_no_files() {
        let mut options = SourceOptions::default();
        options.cfg.names.insert("windows".into());
        let mut reader = RecordingReader::default();
        assert_eq!(
            prepare_with_reader(Path::new("main.rs"), &options, &mut reader)
                .unwrap_err()
                .kind,
            SourceErrorKind::Unsupported
        );
        assert!(reader.calls.is_empty());
    }

    #[test]
    fn disabled_external_visibility_checks_syntax_but_not_path_resolution() {
        for visibility in [
            "pub(crate)",
            "pub(in ::foo)",
            "pub(in foo)",
            "pub(in crate::missing)",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {visibility} mod child; fn main(){{}}"),
                ..Default::default()
            };
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
        for visibility in ["pub(bogus)", "pub(in)"] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {visibility} mod child; fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{error}");
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn disabled_reserved_module_and_visibility_names_are_syntax_errors_without_child_io() {
        for declaration in ["mod fn;", "pub(in fn) mod child;"] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(
                error.kind,
                SourceErrorKind::Syntax,
                "{declaration}: {error}"
            );
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn disabled_leading_visibility_on_all_items_is_syntax_only() {
        for declaration in [
            "pub(in foo) fn unused(){}",
            "pub(in crate::missing) mod inline{mod child;}",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
        for declaration in [
            "pub(bogus) fn unused(){}",
            "pub(in fn) fn unused(){}",
            "pub(bogus) mod inline{}",
            "pub(in fn) mod inline{}",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(
                error.kind,
                SourceErrorKind::Syntax,
                "{declaration}: {error}"
            );
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn malformed_unicode_cfg_preserves_original_span_and_location_in_text_and_child() {
        let text = "// α🙂\r\n#[cfg(a β)] pub fn item(){}";
        let options = SourceOptions::default();
        let tree = Tree::new();
        tree.put("main.rs", "mod child; fn main(){}");
        tree.put("child.rs", text);
        for (error, path) in [
            (
                prepare_text("unicode.rs", text, &options).unwrap_err(),
                PathBuf::from("unicode.rs"),
            ),
            (
                prepare_file(&tree.0.join("main.rs"), &options).unwrap_err(),
                tree.0.join("child.rs"),
            ),
        ] {
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{error}");
            let span = error.primary.unwrap();
            assert!(text.is_char_boundary(span.lo as usize));
            assert!(text.is_char_boundary(span.hi as usize));
            assert_eq!(&text[span.lo as usize..span.hi as usize], "β");
            let location = error
                .location
                .as_ref()
                .expect("original Unicode error location");
            assert_eq!(location.path, path);
            assert_eq!((location.line, location.column), (2, 9));
            assert!(error.to_string().contains(":2:9:"));
        }
    }

    #[test]
    fn inactive_invalid_grammar_rejects_but_unused_semantics_do_not_run() {
        let options = SourceOptions::default();
        assert!(
            prepare_text(
                "test.rs",
                "#[cfg(any())] fn dead(){let = 1;} fn main(){}",
                &options
            )
            .is_err()
        );
        assert!(
            prepare_text(
                "test.rs",
                "#[cfg(any())] fn dead(){unknown();} fn main(){}",
                &options
            )
            .is_ok()
        );
        let raw = "#[cfg(any())] fn dead(){let text=r###\"mod missing; } #[unknown_attr]\"###;} fn main(){}";
        assert!(prepare_text("test.rs", raw, &options).is_ok());
    }

    #[test]
    fn original_bytes_and_synthetic_module_braces_are_distinct() {
        let tree = Tree::new();
        tree.put("main.rs", "// α🙂\r\nmod child; fn main(){}\r\n");
        let child = "// α🙂\r\npub fn child(){}\r\n";
        tree.put("child.rs", child);
        let prepared = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap();
        let start = prepared.text.find("pub fn child").unwrap();
        let span = prepared.map_range(start..start + 3).unwrap();
        assert_eq!(span.file, FileId(1));
        assert_eq!(
            &prepared.sources.files[1].contents[span.lo as usize..span.hi as usize],
            "pub"
        );
        assert_eq!(prepared.sources.location(span).unwrap().1, 2);
        let brace = prepared.text.find("{\n").unwrap();
        let anchor = prepared.map_range(brace..brace + 1).unwrap();
        assert_eq!(anchor.file, FileId(0));
        assert_eq!(
            &prepared.sources.files[0].contents[anchor.lo as usize..anchor.hi as usize],
            "mod child;"
        );
        assert!(prepared.map_range(brace..start + 1).is_none());
        assert!(
            prepared
                .map_range(prepared.text.len()..prepared.text.len())
                .is_some()
        );
        let empty = prepare_text("empty.rs", "", &SourceOptions::default()).unwrap();
        assert_eq!(
            empty.map_range(0..0),
            Some(SourceSpan {
                file: FileId(0),
                lo: 0,
                hi: 0
            })
        );
    }

    #[test]
    fn macro_token_trees_do_not_discover_phantom_modules() {
        for root in [
            "m!(mod missing;); fn main(){}",
            "m![mod missing;]; fn main(){}",
            "m!{mod missing;} fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let _ =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader);
            assert!(
                reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")),
                "{root}"
            );
        }
    }

    #[test]
    fn text_and_file_preparation_apply_identical_options_and_attribute_policy() {
        let tree = Tree::new();
        let text = "#[cfg(feature=\"chosen\")] #[inline(always)] fn selected(){} #[cfg(not(feature=\"chosen\"))] fn absent(){}";
        tree.put("main.rs", text);
        let mut options = SourceOptions::default();
        options.cfg.insert_user("feature=\"chosen\"").unwrap();
        let file = prepare_file(&tree.0.join("main.rs"), &options).unwrap();
        let memory = prepare_text("test.rs", text, &options).unwrap();
        assert_eq!(file.text, memory.text);
        assert_eq!(file.attributes.len(), memory.attributes.len());
        assert!(file.text.contains("fn selected"));
        assert!(!file.text.contains("fn absent"));
        assert_eq!(file.sources.files[0].contents.as_ref(), text);
        assert_eq!(memory.sources.files[0].contents.as_ref(), text);
    }

    #[test]
    fn inline_module_128_boundary_is_uniform_for_files_and_text() {
        let tree = Tree::new();
        for depth in [MAX_MODULE_DEPTH, MAX_MODULE_DEPTH + 1] {
            let text = format!("{}fn f(){{}}{}", "mod a{".repeat(depth), "}".repeat(depth));
            tree.put("main.rs", &text);
            let file = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default());
            let memory = prepare_text("test.rs", &text, &SourceOptions::default());
            if depth == MAX_MODULE_DEPTH {
                assert!(file.is_ok(), "{file:?}");
                assert!(memory.is_ok(), "{memory:?}");
            } else {
                assert_eq!(file.unwrap_err().kind, SourceErrorKind::Limit);
                assert_eq!(memory.unwrap_err().kind, SourceErrorKind::Limit);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn repeated_canonical_file_has_distinct_lexical_occurrence_provenance() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a; mod b;");
        tree.put("shared.rs", "pub fn value(){}");
        std::os::unix::fs::symlink(tree.0.join("shared.rs"), tree.0.join("a.rs")).unwrap();
        std::os::unix::fs::symlink(tree.0.join("shared.rs"), tree.0.join("b.rs")).unwrap();
        let source = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap();
        let occurrences: Vec<_> = source
            .text
            .match_indices("pub fn value")
            .map(|(i, _)| source.map_range(i..i + 3).unwrap())
            .collect();
        assert_eq!(occurrences.len(), 2);
        assert_ne!(occurrences[0].file, occurrences[1].file);
        assert!(
            source.sources.files[occurrences[0].file.0 as usize]
                .lexical_path
                .ends_with("a.rs")
        );
        assert!(
            source.sources.files[occurrences[1].file.0 as usize]
                .lexical_path
                .ends_with("b.rs")
        );
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "jr-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn put(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        fn load(&self) -> Result<LoadedSource, String> {
            load(&self.0.join("main.rs"))
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Graph path: every file unit built independently (no shared reader or
    /// loader state), units prebuilt before assembly and shared through `Arc`,
    /// equals the sequential whole-tree preparation exactly.
    fn assemble_independent(path: &Path) -> Result<PreparedSource, SourceError> {
        use std::collections::HashMap;
        let options = SourceOptions::default();
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        // Prebuild the whole unit tree first, in breadth-first order, as
        // independent graph nodes would, then assemble from the cache.
        let mut units: HashMap<Vec<u64>, Arc<UnitResult>> = HashMap::new();
        let mut queue = vec![vec![0u64]];
        units.insert(
            vec![0],
            Arc::new(prepare_root_unit(path, &options, &budget)),
        );
        while let Some(ordinal) = queue.pop() {
            let unit = units[&ordinal].clone();
            if let Ok(parent) = unit.as_ref() {
                for index in 0..parent.children.len() {
                    let mut child = ordinal.clone();
                    child.push(index as u64);
                    units.insert(
                        child.clone(),
                        Arc::new(prepare_child_unit(parent, index, &options, &budget)),
                    );
                    queue.push(child);
                }
            }
        }
        let mut assembler = SourceAssembler::user_only(&options);
        let mut step = assembler.provide(units[&vec![0]].clone())?;
        loop {
            match step {
                AssemblyStep::Done(prepared) => return Ok(*prepared),
                AssemblyStep::Need(need) => {
                    let mut parent_ordinal = need.ordinal.clone();
                    parent_ordinal.pop();
                    if let Some(parent) = need.parent.as_deref() {
                        let original = units[&parent_ordinal].as_ref().as_ref().unwrap();
                        assert!(
                            std::ptr::eq(parent, original),
                            "assembly must borrow the original immutable file fact, not clone it"
                        );
                    }
                    step = assembler.provide(units[&need.ordinal].clone())?;
                }
            }
        }
    }
    fn same_prepared(a: &PreparedSource, b: &PreparedSource) {
        assert_eq!(a.text, b.text);
        assert_eq!(a.origins, b.origins);
        assert_eq!(a.files, b.files);
        assert_eq!(a.source_bytes, b.source_bytes);
        assert_eq!(a.debug_assertions(), b.debug_assertions());
        assert_eq!(a.sources.files.len(), b.sources.files.len());
        for (x, y) in a.sources.files.iter().zip(&b.sources.files) {
            assert_eq!(x.lexical_path, y.lexical_path);
            assert_eq!(x.contents, y.contents);
            assert_eq!(x.line_starts, y.line_starts);
        }
        let attrs = |p: &PreparedSource| {
            p.attributes
                .iter()
                .map(|a| (a.span, a.name.clone(), a.effective))
                .collect::<Vec<_>>()
        };
        assert_eq!(attrs(a), attrs(b));
    }

    #[test]
    fn independent_file_units_assemble_to_the_sequential_preparation() {
        let tree = Tree::new();
        tree.put(
            "main.rs",
            "#[inline] fn top(){}\nmod a; pub mod b;\nmod inline { mod c; }\nfn main(){}\n",
        );
        tree.put(
            "a.rs",
            "// α\nmod d;\npub fn a(){}\n#[cfg(any())] mod gone;\n",
        );
        tree.put("a/d.rs", "pub fn d(){}");
        tree.put("b/mod.rs", "");
        tree.put("inline/c.rs", "#[inline(always)] pub fn c(){} mod e;");
        tree.put("inline/c/e.rs", "pub fn e(){}");
        let path = tree.0.join("main.rs");
        let sequential = prepare_file(&path, &SourceOptions::default()).unwrap();
        let graph = assemble_independent(&path).unwrap();
        same_prepared(&sequential, &graph);
        assert_eq!(graph.files, 6);
        let start = graph.text.find("pub fn e").unwrap();
        let span = graph.map_range(start..start + 3).unwrap();
        assert!(
            graph.sources.files[span.file.0 as usize]
                .lexical_path
                .ends_with("inline/c/e.rs")
        );
    }

    /// Step B: a child file parsed alone in its own session, mounted at its
    /// depth-first assembly FileId, reports the same error, path, line and
    /// rendered FileId as the whole-crate parse.
    #[test]
    fn file_view_parse_renders_child_errors_at_assembly_base() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;\nmod b;\nfn main(){}\n");
        tree.put("a.rs", "pub fn a(){}\n");
        tree.put("b.rs", "pub fn b(){}\nfn broken() -> {}\n");
        let path = tree.0.join("main.rs");
        let options = SourceOptions::default();
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        let whole = prepare_file(&path, &options).unwrap();
        let whole = match super::super::frontend::parse_prepared(&whole) {
            Err(error) => error,
            Ok(_) => panic!("whole-crate parse must reject b.rs"),
        };
        let root = prepare_root_unit(&path, &options, &budget).unwrap();
        let b = prepare_child_unit(&root, 1, &options, &budget).unwrap();
        let view = FileView::of(&b, 2, &options).unwrap();
        let issue = |domain| super::super::owner_path::standalone_issued(&budget, domain).unwrap();
        let alone = match super::super::frontend::parse_file_view_owned(
            &view,
            budget.clone(),
            issue(super::super::owner_path::OwnerDomain::SourceSnapshot),
            issue(super::super::owner_path::OwnerDomain::TokenSession),
        ) {
            Err(error) => error,
            Ok(_) => panic!("file view parse must reject b.rs"),
        };
        assert_eq!(alone.message, whole.message);
        assert_eq!(alone.kind, whole.kind);
        assert_eq!(alone.primary.map(|s| s.file.0), Some(2));
        assert_eq!(alone.primary.map(|s| s.file), whole.primary.map(|s| s.file));
        let (a, w) = (alone.location.unwrap(), whole.location.unwrap());
        assert!(a.path.ends_with("b.rs"), "{}", a.path.display());
        assert_eq!((a.path, a.line, a.column), (w.path, w.line, w.column));
        // Valid sibling parses alone without any parent session.
        let a_unit = prepare_child_unit(&root, 0, &options, &budget).unwrap();
        let a_view = FileView::of(&a_unit, 1, &options).unwrap();
        assert!(
            super::super::frontend::parse_file_view_owned(
                &a_view,
                budget.clone(),
                issue(super::super::owner_path::OwnerDomain::SourceSnapshot),
                issue(super::super::owner_path::OwnerDomain::TokenSession),
            )
            .is_ok()
        );
    }

    /// Step C1: a parent file view records one entry per external child,
    /// mounted at that child's module, carrying the textual macro scope in
    /// effect at its declaration (later definitions stay invisible).
    #[test]
    fn file_view_parse_records_child_entries_with_macro_scope() {
        let tree = Tree::new();
        tree.put(
            "main.rs",
            "mod a;\nmacro_rules! m { () => {} }\nmod b;\nmacro_rules! n { () => {} }\nfn main(){}\n",
        );
        tree.put("a.rs", "pub fn a(){}\n");
        tree.put("b.rs", "pub fn b(){}\n");
        let path = tree.0.join("main.rs");
        let options = SourceOptions::default();
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        let root = prepare_root_unit(&path, &options, &budget).unwrap();
        let view = FileView::of(&root, 0, &options).unwrap();
        assert_eq!(view.child_sites.len(), 2);
        let issue = |domain| super::super::owner_path::standalone_issued(&budget, domain).unwrap();
        let parsed = super::super::frontend::parse_file_view_owned(
            &view,
            budget.clone(),
            issue(super::super::owner_path::OwnerDomain::SourceSnapshot),
            issue(super::super::owner_path::OwnerDomain::TokenSession),
        )
        .unwrap();
        let entries = parsed.child_entries();
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].module, entries[1].module);
        let names = &parsed.program.names;
        let modules = &parsed.program.modules;
        let spell = |module| {
            let ident = modules.name(module).unwrap().unwrap();
            names.spelling(ident).unwrap().to_string()
        };
        assert_eq!(spell(entries[0].module), "a");
        assert_eq!(spell(entries[1].module), "b");
        let visible = |i: usize| entries[i].macros.as_ref().map_or(0, |c| c.definitions());
        assert_eq!(visible(0), 0, "m is defined after mod a;");
        assert_eq!(visible(1), 1, "only m precedes mod b;");
        // Whole-crate parses splice children and record no entries.
        let whole = prepare_file(&path, &options).unwrap();
        let whole = super::super::frontend::parse_prepared(&whole).unwrap();
        assert!(whole.child_entries().is_empty());
    }

    /// Step C2: a child file parsed alone, mounted at its parent's entry,
    /// expands exactly the parent macros defined before `mod child;`, places
    /// its items under the parent's module, and rejects later macros the
    /// same way the whole-crate parse does.
    #[test]
    fn child_view_parse_mounts_at_parent_entry_with_textual_macros() {
        use super::super::owner_path::{OwnerDomain, standalone_issued};
        let tree = Tree::new();
        tree.put(
            "main.rs",
            "macro_rules! make { ($n:ident) => { pub fn $n() {} } }\nmod b;\nmacro_rules! late { () => {} }\nfn main(){}\n",
        );
        tree.put("b.rs", "make!(made);\npub fn plain(){}\n");
        let path = tree.0.join("main.rs");
        let options = SourceOptions::default();
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        let issue = |domain| standalone_issued(&budget, domain).unwrap();
        let root = prepare_root_unit(&path, &options, &budget).unwrap();
        let parent = super::super::frontend::parse_file_view_owned(
            &FileView::of(&root, 0, &options).unwrap(),
            budget.clone(),
            issue(OwnerDomain::SourceSnapshot),
            issue(OwnerDomain::TokenSession),
        )
        .unwrap();
        let b = prepare_child_unit(&root, 0, &options, &budget).unwrap();
        let child = super::super::frontend::parse_child_view_owned(
            &FileView::of(&b, 1, &options).unwrap(),
            &parent,
            0,
            budget.clone(),
            issue(OwnerDomain::SourceSnapshot),
            issue(OwnerDomain::TokenSession),
        )
        .unwrap();
        let names = &*child.program.names;
        let mut found: Vec<String> = child
            .program
            .functions
            .iter()
            .map(|f| f.name.display(names).unwrap().to_string())
            .collect();
        found.sort();
        assert_eq!(found, ["crate::b::made", "crate::b::plain"]);
        let mount = parent.child_entries()[0].module;
        assert!(child.program.functions.iter().all(|f| f.module == mount));
        // A macro defined after `mod b;` is invisible, as in the whole parse.
        tree.put("b.rs", "late!();\n");
        let whole = prepare_file(&path, &options).unwrap();
        let whole = match super::super::frontend::parse_prepared(&whole) {
            Err(error) => error,
            Ok(_) => panic!("late macro must be invisible to b.rs"),
        };
        let b = prepare_child_unit(&root, 0, &options, &budget).unwrap();
        let alone = match super::super::frontend::parse_child_view_owned(
            &FileView::of(&b, 1, &options).unwrap(),
            &parent,
            0,
            budget.clone(),
            issue(OwnerDomain::SourceSnapshot),
            issue(OwnerDomain::TokenSession),
        ) {
            Err(error) => error,
            Ok(_) => panic!("late macro must be invisible to b.rs alone"),
        };
        assert_eq!(alone.message, whole.message);
        assert_eq!(alone.primary, whole.primary);
    }

    #[test]
    fn independent_units_report_the_sequential_first_error() {
        let tree = Tree::new();
        // The child error precedes the parent's later syntax error in source
        // order, so it must win even though both units are built eagerly.
        tree.put("main.rs", "mod a; fn main(){ let = ; } fn bad( {}");
        tree.put("a.rs", "mod missing;");
        let path = tree.0.join("main.rs");
        let sequential = prepare_file(&path, &SourceOptions::default()).unwrap_err();
        let graph = assemble_independent(&path).unwrap_err();
        assert_eq!(sequential.to_string(), graph.to_string());
        assert_eq!(sequential.kind, graph.kind);
        assert_eq!(sequential.primary, graph.primary);
        for (main, child) in [
            ("mod a; fn main(){}", "pub fn a( {}"),
            ("mod a; mod b; fn main(){}", "/*"),
            ("fn main(){} mod a;", "\u{feff}fn\u{feff} x(){}"),
        ] {
            tree.put("main.rs", main);
            tree.put("a.rs", child);
            tree.put("b.rs", "");
            let sequential = prepare_file(&path, &SourceOptions::default()).unwrap_err();
            let graph = assemble_independent(&path).unwrap_err();
            assert_eq!(
                sequential.to_string(),
                graph.to_string(),
                "{main} / {child}"
            );
            assert_eq!(sequential.primary, graph.primary);
        }
        // Missing child files anchor at the declaring `mod name;`.
        tree.put("main.rs", "fn main(){}\nmod zz;");
        let graph = assemble_independent(&path).unwrap_err();
        assert_eq!(graph.kind, SourceErrorKind::Module);
        assert_eq!(graph.location.unwrap().line, 2);
    }

    #[test]
    fn mixed_layout_preserves_visibility_and_counts() {
        let tree = Tree::new();
        let root = "pub mod a; mod b; fn main(){}";
        let a = "pub mod inline { mod child; }";
        let child = "fn private(){}";
        let b = "pub fn public(){}";
        tree.put("main.rs", root);
        tree.put("a.rs", a);
        tree.put("a/inline/child/mod.rs", child);
        tree.put("b/mod.rs", b);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 4);
        assert_eq!(
            loaded.source_bytes,
            root.len() + a.len() + child.len() + b.len()
        );
        assert!(loaded.text.contains("pub mod a{"));
        assert!(loaded.text.contains("mod child{"));
        assert!(loaded.text.contains("fn private(){}"));
        assert!(loaded.text.contains("pub fn public(){}"));
    }

    #[test]
    fn comments_and_literals_do_not_load_phantom_files() {
        let tree = Tree::new();
        let root = r###"/* mod phantom; /* } */ */ // mod ghost;
fn main(){println!("mod absent; }"); let x = r##"mod raw; {"##; let c = '}';}
"###;
        tree.put("main.rs", root);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.text, root);
        assert_eq!(loaded.files, 1);
    }

    #[test]
    fn named_lifetimes_survive_external_module_expansion() {
        let tree = Tree::new();
        let root = "mod child; fn id<'a>(x:&'a i32)->&'a i32{x} fn main(){}";
        let child = "pub fn shorten<'long:'short,'short>(x:&'long i32)->&'short i32{x}";
        tree.put("main.rs", root);
        tree.put("child.rs", child);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 2);
        assert_eq!(loaded.source_bytes, root.len() + child.len());
        assert!(loaded.text.contains("fn id<'a>(x:&'a i32)->&'a i32{x}"));
        assert!(loaded.text.contains(child));
        super::super::frontend::parse(&loaded.text).unwrap();
    }

    #[test]
    fn lifetime_tokens_do_not_hide_delimiters_or_expose_literal_contents() {
        let text = "fn f<'mod>(x:&'mod i32){let a='}';let b='a';let c='\\'';}";
        let actual: Vec<_> = tokens(text).unwrap().iter().map(|t| t.text(text)).collect();
        assert_eq!(actual.iter().filter(|t| **t == "'mod").count(), 2);
        assert_eq!(actual.iter().filter(|t| **t == "}").count(), 1);
        assert!(actual.contains(&"'}'"));
        assert!(actual.contains(&"'a'"));
        assert!(actual.contains(&"'\\''"));
    }

    #[test]
    fn missing_ambiguous_and_path_attribute_are_explicit() {
        let tree = Tree::new();
        tree.put("main.rs", "mod child;");
        assert!(tree.load().unwrap_err().contains("not found"));
        tree.put("child.rs", "");
        tree.put("child/mod.rs", "");
        assert!(tree.load().unwrap_err().contains("ambiguous"));
        tree.put("main.rs", "#[path = \"other.rs\"] mod child;");
        let error = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{error}");
        assert!(error.message.contains("path"), "{error}");
        tree.put(
            "main.rs",
            "#[cfg_attr(any(), path = \"other.rs\")] mod child;",
        );
        assert!(tree.load().unwrap_err().contains("ambiguous"));
    }

    #[test]
    fn nested_flat_file_uses_stem_directory() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        tree.put("a.rs", "mod b;");
        tree.put("a/b.rs", "mod c;");
        tree.put("a/b/c.rs", "fn leaf(){}");
        assert_eq!(tree.load().unwrap().files, 4);
    }

    #[test]
    fn depth_and_size_are_bounded() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        for depth in 0..=MAX_FILE_DEPTH {
            tree.put(&format!("{}a.rs", "a/".repeat(depth)), "mod a;");
        }
        let error = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Limit, "{error}");
        assert!(error.message.contains("limit"), "{error}");
        tree.put("main.rs", &" ".repeat(MAX_BYTES + 1));
        assert!(tree.load().unwrap_err().contains("size limit"));
        let oversized_text = " ".repeat(MAX_BYTES + 1);
        let error =
            prepare_text("oversized.rs", &oversized_text, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Limit);
        assert!(error.message.contains("size limit"));
        let options = SourceOptions::default();
        tree.put("main.rs", "mod a;");
        let mut assembler = SourceAssembler::user_only(&options);
        assembler.prepared.files = MAX_FILES - 1;
        let root = prepare_root_unit(
            &tree.0.join("main.rs"),
            &options,
            &super::super::expansion_budget::ExpansionBudget::default(),
        );
        assert!(
            assembler
                .provide(Arc::new(root))
                .err()
                .unwrap()
                .to_string()
                .contains("file limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_active_stack_rejects_symlink_cycle() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        std::os::unix::fs::symlink(tree.0.join("main.rs"), tree.0.join("a.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("cycle"));
    }

    #[test]
    fn nonregular_sources_are_rejected() {
        let tree = Tree::new();
        assert!(load(&tree.0).unwrap_err().contains("regular file"));
        tree.put("main.rs", "mod child;");
        std::fs::create_dir(tree.0.join("child.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("regular file"));
    }

    #[test]
    fn malformed_and_non_utf8_sources_fail() {
        let tree = Tree::new();
        for text in [
            "/*",
            "fn main(){",
            "fn main(){\"unterminated}",
            "fn main(){r###\"unfinished}",
        ] {
            tree.put("main.rs", text);
            assert!(tree.load().is_err(), "{text}");
        }
        std::fs::write(tree.0.join("main.rs"), [0xff]).unwrap();
        assert!(tree.load().unwrap_err().contains("UTF-8"));
    }
}

impl crate::engine::Fact for PreparedSource {}

impl crate::engine::Fact for FileUnit {}

impl crate::engine::Fact for UnitFailure {}
