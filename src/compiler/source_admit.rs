//! Per-file source admission (graph kind `ADMIT`).
//!
//! Admission replaces the whole-crate assembly pass. One `ADMIT` node per
//! source file applies the cross-file limits (bytes, file count, file depth,
//! expanded size, provenance segments) and renders the file's assembly
//! `FileId` in exactly the depth-first order sequential assembly used, without
//! copying any file text. Output 0 is the state after the file itself is
//! admitted (its entry). Output 1 is the state after its whole subtree and
//! trailing text (its exit). Child `i` starts from its parent's entry
//! (`i == 0`) or its previous sibling's exit, so the first error in source
//! order stops discovery before any later file is requested.
//!
//! Crate roots chain `[0]`, then dependency crates `[2..]`, then the prelude
//! `[1]`. The final root's exit carries the crate-wide accounting.

use std::path::PathBuf;
use std::sync::Arc;

use super::source_attrs::TestItem;
use super::source_map::{FileId, OriginSegment, SourceMap, SourceSpan};
use super::{
    EXTERN_PREFIX, LOCAL_FILE, Loader, MAX_BYTES, MAX_FILE_DEPTH, MAX_FILES, MAX_ORIGINS,
    SourceError, SourceErrorKind, SourceOptions, UnitResult, copy_error, remap_span,
    source_embedded, test_harness,
};

/// Immutable persistent list (newest first). Sharing a tail is O(1).
struct Link<T> {
    head: T,
    tail: List<T>,
}
type List<T> = Option<Arc<Link<T>>>;

fn push<T>(tail: &List<T>, head: T) -> List<T> {
    Some(Arc::new(Link {
        head,
        tail: tail.clone(),
    }))
}

fn in_order<T>(mut list: &List<T>) -> Vec<&T> {
    let mut out = Vec::new();
    while let Some(link) = list {
        out.push(&link.head);
        list = &link.tail;
    }
    out.reverse();
    out
}

/// Admission state threaded through the files in assembly order.
#[derive(Clone, Default)]
pub struct AdmitState {
    /// Rendered assembly FileId of the file whose entry this is.
    id: u32,
    /// User-crate source bytes and files (prelude and dependencies excluded).
    pub source_bytes: usize,
    pub files: usize,
    next_id: u32,
    text_len: usize,
    origins: usize,
    last: Option<OriginSegment>,
    sources: List<(PathBuf, Arc<str>)>,
    admitted: List<Vec<u64>>,
    tests: List<TestItem>,
    /// The first error in assembly order. Nothing after it is admitted.
    error: Option<Arc<SourceError>>,
    /// Synthesized crate entry (test harness or inert library `main`), set
    /// by the `[0]` exit and appended to the root file's syntax view.
    entry: Option<Arc<str>>,
}

impl crate::engine::Fact for AdmitState {}

impl AdmitState {
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
    /// The first admission error, located like sequential assembly reports it.
    pub fn failure(&self) -> Option<&SourceError> {
        self.error.as_deref()
    }
    fn source_map(&self) -> SourceMap {
        let mut map = SourceMap::default();
        for (path, contents) in in_order(&self.sources) {
            map.add(path.clone(), contents.clone());
        }
        map
    }
    /// Admitted file ordinals, in rendered FileId order.
    pub fn admitted(&self) -> Vec<Vec<u64>> {
        in_order(&self.admitted).into_iter().cloned().collect()
    }
    /// The file admitted immediately before this entry's file, in rendered
    /// FileId order. O(1): the admission list is newest first.
    pub fn previous(&self) -> Option<&[u64]> {
        let head = self.admitted.as_ref()?;
        head.tail.as_ref().map(|link| link.head.as_slice())
    }
    pub fn entry_text(&self) -> Option<&str> {
        self.entry.as_deref()
    }
    /// Locate against the files admitted so far, as the assembler's `fail`.
    fn fail(&mut self, error: SourceError) {
        let error = error.with_sources(&self.source_map());
        self.error = Some(Arc::new(error));
    }
    /// Mirrors assembly's `append_to` limit and merge accounting exactly.
    fn append(&mut self, len: usize, origin: SourceSpan, synthetic: bool) -> bool {
        if len > MAX_BYTES - self.text_len {
            self.fail(Loader::error(
                SourceErrorKind::Limit,
                Some(origin),
                format!("expanded source size limit ({MAX_BYTES} bytes) exceeded"),
            ));
            return false;
        }
        let start = self.text_len;
        self.text_len += len;
        if let Some(last) = &mut self.last
            && !synthetic
            && !last.synthetic
            && last.prepared.end == start
            && last.origin.file == origin.file
            && last.origin.hi == origin.lo
        {
            last.prepared.end += len;
            last.origin.hi = origin.hi;
            return true;
        }
        if self.origins >= MAX_ORIGINS {
            self.fail(Loader::error(
                SourceErrorKind::Limit,
                Some(origin),
                "source provenance segment limit exceeded",
            ));
            return false;
        }
        self.origins += 1;
        self.last = Some(OriginSegment {
            prepared: start..start + len,
            origin,
            synthetic,
        });
        true
    }
    /// Copy `unit`'s own segments `from..to` (by count of segments ending at
    /// or before a child site), as assembly does between child modules.
    fn segments(&mut self, unit: &super::FileUnit, id: FileId, from: usize, to: usize) -> bool {
        for segment in &unit.origins[from..to] {
            if !self.append(
                segment.prepared.len(),
                remap_span(segment.origin, id),
                segment.synthetic,
            ) {
                return false;
            }
        }
        true
    }
}

fn harness(options: &SourceOptions) -> bool {
    options.cfg.names.contains("test")
}

/// Segments of `unit` that end at or before byte `at` of its prepared text.
fn segments_before(unit: &super::FileUnit, at: usize) -> usize {
    unit.origins
        .iter()
        .take_while(|segment| segment.prepared.end <= at)
        .count()
}

/// The root that precedes `root` in admission order, if any.
pub(crate) fn previous_root(root: u64, options: &SourceOptions) -> Option<u64> {
    let externs = options.externs.len() as u64;
    match root {
        0 => None,
        1 if externs == 0 => Some(0),
        1 => Some(1 + externs),
        2 => Some(0),
        n => Some(n - 1),
    }
}

/// The root whose exit is the final crate-wide admission state.
pub(crate) fn final_root(options: &SourceOptions) -> u64 {
    if options.prelude {
        1
    } else if options.externs.is_empty() {
        0
    } else {
        1 + options.externs.len() as u64
    }
}

/// Before child `index` of `parent`: copy the parent text up to the child's
/// site, then apply the depth and file-count limits. Returns the anchor.
pub(crate) fn before_child(
    state: &mut AdmitState,
    parent: &super::FileUnit,
    parent_id: u32,
    parent_depth: usize,
    index: usize,
) -> Option<SourceSpan> {
    let id = FileId(parent_id);
    let child = parent.children.get(index)?;
    let from = match index.checked_sub(1) {
        Some(previous) => segments_before(parent, parent.children[previous].at),
        None => 0,
    };
    if !state.segments(parent, id, from, segments_before(parent, child.at)) {
        return None;
    }
    let anchor = remap_span(child.decl, id);
    if parent_depth > MAX_FILE_DEPTH {
        state.fail(Loader::error(
            SourceErrorKind::Limit,
            Some(anchor),
            format!("module file depth limit ({MAX_FILE_DEPTH}) exceeded"),
        ));
        return None;
    }
    if state.files >= MAX_FILES {
        state.fail(Loader::error(
            SourceErrorKind::Limit,
            Some(anchor),
            format!("module file limit ({MAX_FILES}) exceeded"),
        ));
        return None;
    }
    Some(anchor)
}

/// Admit the file `unit` at `ordinal` (the assembler's `provide`).
pub(crate) fn admit(
    state: &mut AdmitState,
    ordinal: &[u64],
    unit: &UnitResult,
    anchor: Option<SourceSpan>,
    options: &SourceOptions,
) {
    let user = ordinal.first() == Some(&0);
    let unit = match unit {
        Err(failure) => {
            let primary = if failure.anchored {
                anchor
            } else {
                failure.error.primary
            };
            return state.fail(copy_error(&failure.error, primary));
        }
        Ok(unit) => unit,
    };
    if unit.edition != options.edition {
        return state.fail(Loader::error(
            SourceErrorKind::Unsupported,
            None,
            "mixed source editions require per-crate lexical transport",
        ));
    }
    let library = options.library && !harness(options);
    if ordinal == [0] && unit.no_std && !library && !harness(options) {
        return state.fail(Loader::error(
            SourceErrorKind::Unsupported,
            None,
            "no_std executable startup and panic-handler semantics are unsupported; only library targets and dependencies are admitted",
        ));
    }
    if unit.contents.len() > MAX_BYTES - state.source_bytes {
        return state.fail(Loader::error(
            SourceErrorKind::Limit,
            None,
            format!("source size limit ({MAX_BYTES} bytes) exceeded"),
        ));
    }
    if user {
        state.source_bytes += unit.contents.len();
        state.files += 1;
        if harness(options) {
            for test in &unit.tests {
                let mut test = test.clone();
                let mut path = unit.module.clone();
                path.append(&mut test.path);
                test.path = path;
                state.tests = push(&state.tests, test);
            }
        }
    }
    let id = FileId(state.next_id);
    state.id = state.next_id;
    state.next_id += 1;
    state.sources = push(&state.sources, (unit.path.clone(), unit.contents.clone()));
    state.admitted = push(&state.admitted, ordinal.to_vec());
    // Assembly propagates attachment errors unlocated.
    if let Err(error) = source_embedded::EmbeddedFile::attach(id, unit) {
        state.error = Some(Arc::new(error));
        return;
    }
    if let [root] = ordinal
        && *root >= 2
        && let Some(krate) = options.externs.get(*root as usize - 2)
    {
        let open = format!("\npub mod {EXTERN_PREFIX}{} {{\n", krate.name);
        let anchor = SourceSpan {
            file: id,
            lo: 0,
            hi: 0,
        };
        state.append(open.len(), anchor, true);
    }
}

/// After the file's last child: its trailing text, its terminal error, the
/// dependency-crate close, and for `[0]` the synthesized crate entry.
pub(crate) fn finish(
    state: &mut AdmitState,
    ordinal: &[u64],
    unit: &super::FileUnit,
    id: u32,
    options: &SourceOptions,
) {
    if state.failed() {
        return;
    }
    let id = FileId(id);
    let from = unit
        .children
        .last()
        .map_or(0, |child| segments_before(unit, child.at));
    if !state.segments(unit, id, from, unit.origins.len()) {
        return;
    }
    if let Some(error) = &unit.terminal {
        let primary = error.primary.map(|span| remap_span(span, id));
        return state.fail(copy_error(error, primary));
    }
    if let [root] = ordinal
        && *root >= 2
    {
        let anchor = SourceSpan {
            file: id,
            lo: 0,
            hi: 0,
        };
        if !state.append(3, anchor, true) {
            return;
        }
    }
    if ordinal == [0] {
        let entry = if harness(options) {
            let tests = in_order(&state.tests).into_iter().cloned().collect();
            Some(test_harness(tests, &options.test_filter))
        } else if options.library {
            Some("\nfn main() {}\n".to_string())
        } else {
            None
        };
        if let Some(entry) = entry {
            let anchor = SourceSpan {
                file: LOCAL_FILE,
                lo: 0,
                hi: 0,
            };
            if state.append(entry.len(), anchor, true) {
                state.entry = Some(Arc::from(entry));
            }
        }
    }
}
