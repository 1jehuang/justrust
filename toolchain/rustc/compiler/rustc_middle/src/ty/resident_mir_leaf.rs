//! Conservative closure evidence for bounded straight-line primitive MIR.
//!
//! This is NOT changed-source authorization, query reuse permission, or evidence
//! about callers/HIR/attributes/ABI. It does not consult or modify reuse gates.
//! Evidence borrows the inspected body and owns exact active-file Arcs. Activation
//! of another revision invalidates its *meaning* as a current-source snapshot;
//! consumers must recheck source revision identity before any future use.

use std::sync::Arc;

use rustc_hir::def::DefKind;
use rustc_hir::def_id::LocalDefId;
use rustc_span::source_map::SourceMap;
use rustc_span::{SourceFile, Span, SyntaxContext};

use crate::mir::interpret::Scalar;
use crate::mir::visit::Visitor;
use crate::ty::{Ty, TyCtxt};
use crate::{mir, ty};

#[derive(Clone, Copy, Debug)]
pub struct LeafLimits {
    pub historical_files: usize,
    pub active_files: usize,
    pub spans: usize,
    pub statements: usize,
    pub locals: usize,
    pub scopes: usize,
    pub debug_entries: usize,
}

impl Default for LeafLimits {
    fn default() -> Self {
        Self {
            historical_files: 4096,
            active_files: 64,
            spans: 512,
            statements: 64,
            locals: 32,
            scopes: 16,
            debug_entries: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafError {
    Limit,
    SourceView,
    Item,
    Body,
    Local,
    Scope,
    Statement,
    Constant,
    Terminator,
    Span,
    InactiveSpan,
}

/// No public constructor and no unbound boolean/hash certificate. The immutable
/// borrow prevents mutation of the actual body while this evidence is retained.
/// Arcs retain exact file allocations, not merely names or content hashes.
pub struct LeafProof<'body, 'tcx> {
    body: &'body mir::Body<'tcx>,
    source_map: &'body SourceMap,
    files: Vec<(usize, Arc<SourceFile>)>,
    spans: Vec<Span>,
    limits: LeafLimits,
}

impl<'body, 'tcx> LeafProof<'body, 'tcx> {
    pub fn body(&self) -> &'body mir::Body<'tcx> {
        self.body
    }
    pub fn source_map(&self) -> &'body SourceMap {
        self.source_map
    }
    pub fn active_files(&self) -> &[(usize, Arc<SourceFile>)] {
        &self.files
    }
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Pointer identity of the actual immutably borrowed MIR, not its hash.
    pub fn is_for(&self, body: &mir::Body<'tcx>) -> bool {
        std::ptr::eq(self.body, body)
    }

    /// Revalidate this point-in-time source closure against the same native
    /// SourceMap's current active snapshot. This is NOT query authorization.
    /// The inspection's source-loading quiescence contract also applies here.
    pub fn is_current_for(&self, source_map: &SourceMap) -> bool {
        if !std::ptr::eq(self.source_map, source_map)
            || source_map.files().len() > self.limits.historical_files
        {
            return false;
        }
        let Ok(Some(current)) = source_map.active_source_files() else {
            return false;
        };
        current.len() <= self.limits.active_files
            && current.len() == self.files.len()
            && current.iter().zip(&self.files).all(|((index, file), (expected_index, expected))| {
                index == expected_index && Arc::ptr_eq(file, expected)
            })
    }
}

/// `tcx` establishes ordinary local item kind and absence of inherited generics,
/// facts not represented by `Body::is_polymorphic`. The active source view is
/// obtained natively from `source_map`, never supplied as an assertion by callers.
/// Traversal and copying are budgeted. The owner must keep source loading and
/// revision activation quiescent during inspection: SourceMap's snapshot API
/// does not provide a bounded atomic snapshot with its historical-file count.
/// No claim of concurrent or subsequent revision freshness is made.
pub fn inspect<'body, 'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'body mir::Body<'tcx>,
    source_map: &'body SourceMap,
    limits: LeafLimits,
) -> Result<LeafProof<'body, 'tcx>, LeafError> {
    let mir::MirSource { instance, promoted } = body.source;
    let ty::InstanceKind::Item(def) = instance else {
        return Err(LeafError::Item);
    };
    if !def.is_local()
        || promoted.is_some()
        || tcx.def_kind(def) != DefKind::Fn
        || tcx.generics_of(def).count() != 0
    {
        return Err(LeafError::Item);
    }
    if body.source_scopes.len() > limits.scopes {
        return Err(LeafError::Limit);
    }
    check_lint_root(body, tcx.local_def_id_to_hir_id(def.expect_local()))?;
    inspect_body_closure(body, source_map, limits)
}

// The MIR visitor deliberately skips scope local_data. Review this retained HIR
// identifier separately rather than silently treating it as transitively closed.
fn check_lint_root(body: &mir::Body<'_>, expected: rustc_hir::HirId) -> Result<(), LeafError> {
    if body.source_scopes.is_empty() {
        return Err(LeafError::Scope);
    }
    for scope in &body.source_scopes {
        match &scope.local_data {
            mir::ClearCrossCrate::Clear => {}
            mir::ClearCrossCrate::Set(mir::SourceScopeLocalData { lint_root })
                if *lint_root == expected => {}
            mir::ClearCrossCrate::Set(_) => return Err(LeafError::Scope),
        }
    }
    Ok(())
}

// Only `inspect` can expose this evidence outside this module. Separating the
// native body/span check permits focused tests without fabricating a TyCtxt.
fn inspect_body_closure<'body, 'tcx>(
    body: &'body mir::Body<'tcx>,
    source_map: &'body SourceMap,
    limits: LeafLimits,
) -> Result<LeafProof<'body, 'tcx>, LeafError> {
    // Bound the snapshot API's allocation as well as our own evidence.
    if source_map.files().len() > limits.historical_files {
        return Err(LeafError::Limit);
    }
    let files = source_map
        .active_source_files()
        .map_err(|_| LeafError::SourceView)?
        .ok_or(LeafError::SourceView)?;
    if files.len() > limits.active_files {
        return Err(LeafError::Limit);
    }
    inspect_shape(body, limits)?;
    let ty::InstanceKind::Item(def) = body.source.instance else {
        return Err(LeafError::Item);
    };
    let Some(owner) = def.as_local() else {
        return Err(LeafError::Item);
    };
    let mut visitor = SpanClosure {
        source_map,
        owner,
        files: &files,
        limit: limits.spans,
        spans: Vec::new(),
        error: None,
    };
    visitor.visit_body(body);
    if let Some(error) = visitor.error {
        return Err(error);
    }
    let spans = visitor.spans;
    Ok(LeafProof { body, source_map, files, spans, limits })
}

fn primitive(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_))
}

fn inspect_shape(body: &mir::Body<'_>, limits: LeafLimits) -> Result<(), LeafError> {
    // Exhaustive struct pattern: newly added body fields require explicit review.
    let mir::Body {
        basic_blocks,
        phase,
        pass_count: _,
        source: _,
        source_scopes,
        coroutine,
        local_decls,
        user_type_annotations,
        arg_count,
        spread_arg,
        var_debug_info,
        span: _,
        required_consts,
        mentioned_items,
        is_polymorphic,
        injection_phase,
        tainted_by_errors,
        coverage_early_info,
        coverage_mir_info,
    } = body;
    if *phase != mir::MirPhase::Runtime(mir::RuntimePhase::Optimized)
        || coroutine.is_some()
        || !user_type_annotations.is_empty()
        || *arg_count >= local_decls.len()
        || spread_arg.is_some()
        || *is_polymorphic
        || injection_phase.is_some()
        || tainted_by_errors.is_some()
        || coverage_early_info.is_some()
        || coverage_mir_info.is_some()
        || !matches!(required_consts, Some(c) if c.is_empty())
        || !matches!(mentioned_items, Some(c) if c.is_empty())
        || basic_blocks.len() != 1
        || local_decls.is_empty()
        || source_scopes.is_empty()
    {
        return Err(LeafError::Body);
    }
    if local_decls.len() > limits.locals
        || source_scopes.len() > limits.scopes
        || var_debug_info.len() > limits.debug_entries
    {
        return Err(LeafError::Limit);
    }
    for local in local_decls {
        let mir::LocalDecl { mutability: _, local_info, ty, user_ty, source_info } = local;
        // run_runtime_cleanup_passes clears borrowck-only local metadata before
        // optimized_mir. Clear retains no hidden types/IDs/spans to close over.
        let boring_or_cleared = match local_info {
            mir::ClearCrossCrate::Clear => true,
            mir::ClearCrossCrate::Set(info) => matches!(**info, mir::LocalInfo::Boring),
        };
        if !primitive(*ty)
            || user_ty.is_some()
            || !boring_or_cleared
            || source_scopes.get(source_info.scope).is_none()
        {
            return Err(LeafError::Local);
        }
    }
    for (index, scope) in source_scopes.iter_enumerated() {
        let mir::SourceScopeData {
            span: _,
            parent_scope,
            inlined,
            inlined_parent_scope,
            local_data: _,
        } = scope;
        // Strictly earlier parents bound and close the lexical scope tree.
        if (index == mir::OUTERMOST_SOURCE_SCOPE && parent_scope.is_some())
            || (index != mir::OUTERMOST_SOURCE_SCOPE
                && !parent_scope.is_some_and(|parent| parent.index() < index.index()))
            || inlined.is_some()
            || inlined_parent_scope.is_some()
        {
            return Err(LeafError::Scope);
        }
    }
    for debug in var_debug_info {
        let mir::VarDebugInfo { name, source_info, composite, value, argument_index } = debug;
        // Symbols contain no definition identity. Bound even their text inspection.
        let name = name.as_str();
        if name.len() > 128
            || name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || composite.is_some()
            || source_scopes.get(source_info.scope).is_none()
            || argument_index.is_some_and(|index| index == 0 || usize::from(index) > *arg_count)
        {
            return Err(LeafError::Local);
        }
        let mir::VarDebugInfoContents::Place(place) = value else {
            return Err(LeafError::Local);
        };
        plain_place_ty(body, *place)?;
    }
    let mir::BasicBlockData { statements, after_last_stmt_debuginfos, terminator, is_cleanup } =
        &basic_blocks[mir::START_BLOCK];
    if statements.len() > limits.statements {
        return Err(LeafError::Limit);
    }
    if *is_cleanup || !after_last_stmt_debuginfos.is_empty() {
        return Err(LeafError::Body);
    }
    if statements.is_empty() {
        return Err(LeafError::Statement);
    }
    for mir::Statement { source_info, kind, debuginfos } in statements {
        if !debuginfos.is_empty() {
            return Err(LeafError::Statement);
        }
        if source_scopes.get(source_info.scope).is_none() {
            return Err(LeafError::Scope);
        }
        let mir::StatementKind::Assign(assign) = kind else {
            return Err(LeafError::Statement);
        };
        let (place, value) = &**assign;
        let destination = plain_place_ty(body, *place)?;
        check_rvalue(body, value, destination)?;
    }
    let Some(mir::Terminator { source_info, kind: mir::TerminatorKind::Return, loop_hint_attrs }) =
        terminator
    else {
        return Err(LeafError::Terminator);
    };
    if !loop_hint_attrs.is_empty() {
        return Err(LeafError::Terminator);
    }
    if source_scopes.get(source_info.scope).is_none() {
        return Err(LeafError::Scope);
    }
    Ok(())
}

fn plain_place_ty<'tcx>(
    body: &mir::Body<'tcx>,
    place: mir::Place<'tcx>,
) -> Result<Ty<'tcx>, LeafError> {
    if !place.projection.is_empty() {
        return Err(LeafError::Local);
    }
    let local = body.local_decls.get(place.local).ok_or(LeafError::Local)?;
    if !primitive(local.ty) {
        return Err(LeafError::Local);
    }
    Ok(local.ty)
}

fn operand_ty<'tcx>(
    body: &mir::Body<'tcx>,
    operand: &mir::Operand<'tcx>,
) -> Result<Ty<'tcx>, LeafError> {
    match operand {
        mir::Operand::Copy(place) | mir::Operand::Move(place) => plain_place_ty(body, *place),
        mir::Operand::Constant(constant) => {
            let mir::ConstOperand { span: _, user_ty, const_ } = &**constant;
            if user_ty.is_some() {
                return Err(LeafError::Constant);
            }
            match const_ {
                mir::Const::Val(mir::ConstValue::Scalar(Scalar::Int(_)), ty) if primitive(*ty) => {
                    Ok(*ty)
                }
                _ => Err(LeafError::Constant),
            }
        }
        _ => Err(LeafError::Statement),
    }
}

// This is a closure proof over valid optimized MIR, not a second MIR validator
// or an evaluator. No accepted operation can carry a hidden DefId/allocation.
// Keep the whitelist finite: overflow tuple results, division/remainder guards,
// unchecked operations, comparisons, casts and pointer metadata remain excluded.
fn check_rvalue<'tcx>(
    body: &mir::Body<'tcx>,
    value: &mir::Rvalue<'tcx>,
    destination: Ty<'tcx>,
) -> Result<(), LeafError> {
    let integer = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Int(_) | ty::Uint(_));
    let result = match value {
        mir::Rvalue::Use(operand, mir::WithRetag::No | mir::WithRetag::Yes) => {
            operand_ty(body, operand)?
        }
        mir::Rvalue::BinaryOp(op, operands) => {
            let left = operand_ty(body, &operands.0)?;
            let right = operand_ty(body, &operands.1)?;
            if !integer(left) || !integer(right) {
                return Err(LeafError::Statement);
            }
            match op {
                mir::BinOp::Add
                | mir::BinOp::Sub
                | mir::BinOp::Mul
                | mir::BinOp::BitXor
                | mir::BinOp::BitAnd
                | mir::BinOp::BitOr
                    if left == right => {}
                mir::BinOp::Shl | mir::BinOp::Shr => {}
                _ => return Err(LeafError::Statement),
            }
            left
        }
        mir::Rvalue::UnaryOp(op, operand) => {
            let ty = operand_ty(body, operand)?;
            match op {
                mir::UnOp::Not if integer(ty) => {}
                mir::UnOp::Neg if matches!(ty.kind(), ty::Int(_)) => {}
                _ => return Err(LeafError::Statement),
            }
            ty
        }
        _ => return Err(LeafError::Statement),
    };
    if result != destination {
        return Err(LeafError::Constant);
    }
    Ok(())
}

struct SpanClosure<'a> {
    source_map: &'a SourceMap,
    owner: LocalDefId,
    files: &'a [(usize, Arc<SourceFile>)],
    limit: usize,
    spans: Vec<Span>,
    error: Option<LeafError>,
}

impl SpanClosure<'_> {
    fn check(&mut self, span: Span) -> Result<(), LeafError> {
        if self.spans.len() >= self.limit {
            return Err(LeafError::Limit);
        }
        // Incremental HIR lowering attaches the current item as Span::parent.
        // It is dependency bookkeeping, not hygiene expansion. Only the same
        // already-reviewed source Item is allowed, never another definition.
        if span.is_dummy()
            || span.parent().is_some_and(|parent| parent != self.owner)
            || span.from_expansion()
            || span.ctxt() != SyntaxContext::root()
            || span.lo() > span.hi()
        {
            return Err(LeafError::Span);
        }
        let history = self.source_map.files();
        // Avoid lookup_source_file_idx's underflow for malformed positions.
        if history.is_empty() || span.lo() < history[0].start_pos {
            return Err(LeafError::Span);
        }
        let lo = history.partition_point(|file| file.start_pos <= span.lo()) - 1;
        let hi = history.partition_point(|file| file.start_pos <= span.hi()) - 1;
        if lo != hi {
            return Err(LeafError::Span);
        }
        let file = &history[lo];
        // Inclusive end is intentional: an empty span at EOF is valid.
        if span.lo() < file.start_pos || span.hi() > file.end_position() {
            return Err(LeafError::Span);
        }
        if !self.files.iter().any(|(index, active)| *index == lo && Arc::ptr_eq(active, file)) {
            return Err(LeafError::InactiveSpan);
        }
        self.spans.push(span);
        Ok(())
    }
}

impl<'tcx> Visitor<'tcx> for SpanClosure<'_> {
    fn visit_span(&mut self, span: Span) {
        if self.error.is_none() {
            self.error = self.check(span).err();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::num::NonZero;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use rustc_data_structures::intern::Interned;
    use rustc_data_structures::sync::{Registry, set_dyn_thread_safe_mode};
    use rustc_index::IndexVec;
    use rustc_span::hygiene::{LocalExpnId, Transparency};
    use rustc_span::source_map::{FileLoader, FilePathMapping, SourceMapInputs};
    use rustc_span::{SourceFileHashAlgorithm, create_default_session_globals_then};

    use super::*;

    fn native_test(f: impl FnOnce() + Send) {
        // Match the other rustc_middle native identity tests' one-way mode and
        // isolate TLS globals/registry rather than replacing another test's.
        set_dyn_thread_safe_mode(true);
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    Registry::new(NonZero::new(1).unwrap()).register();
                    create_default_session_globals_then(f);
                })
                .join()
                .unwrap();
        });
    }

    struct Loader(Arc<Mutex<String>>);
    impl FileLoader for Loader {
        fn file_exists(&self, _: &Path) -> bool {
            true
        }
        fn read_file(&self, _: &Path) -> io::Result<String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn read_binary_file(&self, _: &Path) -> io::Result<Arc<[u8]>> {
            Ok(self.0.lock().unwrap().as_bytes().into())
        }
        fn current_directory(&self) -> io::Result<PathBuf> {
            std::env::current_dir()
        }
    }

    fn setup() -> (SourceMap, Arc<Mutex<String>>) {
        let text = Arc::new(Mutex::new("fn leaf() -> bool { true }".to_string()));
        let source_map = SourceMap::with_inputs(SourceMapInputs {
            file_loader: Box::new(Loader(Arc::clone(&text))),
            path_mapping: FilePathMapping::empty(),
            hash_kind: SourceFileHashAlgorithm::Md5,
            checksum_hash_kind: None,
        });
        (source_map, text)
    }

    // A single cached primitive, borrowed for the duration of each test. No arena
    // fabrication, leaked allocation, unsafe code, or lifetime extension. Types
    // never enter a compiler interner or query in these focused body-only tests.
    fn cached(kind: ty::TyKind<'_>) -> ty::WithCachedTypeInfo<ty::TyKind<'_>> {
        ty::WithCachedTypeInfo {
            internee: kind,
            flags: ty::TypeFlags::empty(),
            outer_exclusive_binder: ty::INNERMOST,
        }
    }

    fn leaf<'tcx>(ty: Ty<'tcx>, span: Span) -> mir::Body<'tcx> {
        // Match incremental SpanLowerer::lower's native owner-parent metadata.
        let span = span.with_parent(Some(rustc_hir::def_id::CRATE_DEF_ID));
        let info = mir::SourceInfo::outermost(span);
        let constant = mir::Operand::Constant(Box::new(mir::ConstOperand {
            span,
            user_ty: None,
            const_: mir::Const::Val(mir::ConstValue::from_bool(true), ty),
        }));
        let mut block = mir::BasicBlockData::new(
            Some(mir::Terminator {
                source_info: info,
                kind: mir::TerminatorKind::Return,
                loop_hint_attrs: Default::default(),
            }),
            false,
        );
        block.statements.push(mir::Statement::new(
            info,
            mir::StatementKind::Assign(Box::new((
                mir::Place::return_place(),
                mir::Rvalue::Use(constant, mir::WithRetag::Yes),
            ))),
        ));
        let mut blocks = IndexVec::new();
        blocks.push(block);
        let mut scopes = IndexVec::new();
        scopes.push(mir::SourceScopeData {
            span,
            parent_scope: None,
            inlined: None,
            inlined_parent_scope: None,
            local_data: mir::ClearCrossCrate::Clear,
        });
        let mut locals = IndexVec::new();
        let mut return_decl = mir::LocalDecl::new(ty, span);
        // Match real optimized_mir after run_runtime_cleanup_passes, not the
        // pre-cleanup constructor's borrowck-only LocalInfo::Boring metadata.
        return_decl.local_info = mir::ClearCrossCrate::Clear;
        locals.push(return_decl);
        let mut body = mir::Body::new(
            mir::MirSource::item(rustc_hir::def_id::CRATE_DEF_ID.to_def_id()),
            blocks,
            scopes,
            locals,
            IndexVec::new(),
            0,
            Vec::new(),
            span,
            None,
            None,
        );
        body.phase = mir::MirPhase::Runtime(mir::RuntimePhase::Optimized);
        body.required_consts = Some(Vec::new());
        body.mentioned_items = Some(Vec::new());
        body
    }

    fn span(file: &SourceFile) -> Span {
        Span::new(file.start_pos, file.end_position(), SyntaxContext::root(), None)
    }

    #[test]
    fn actual_leaf_binds_body_and_file_and_accepts_eof() {
        native_test(|| {
            let (sm, _) = setup();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let cached = cached(ty::Bool);
            let ty = ty::Ty(Interned::new_unchecked(&cached));
            let body = leaf(ty, span(&file));
            let proof = inspect_body_closure(&body, &sm, LeafLimits::default()).unwrap();
            assert!(std::ptr::eq(proof.body(), &body));
            assert!(std::ptr::eq(proof.source_map(), &sm));
            assert!(Arc::ptr_eq(&proof.active_files()[0].1, &file));
            assert!(!proof.spans().is_empty());
            let mut visitor = SpanClosure {
                source_map: &sm,
                owner: rustc_hir::def_id::CRATE_DEF_ID,
                files: proof.active_files(),
                limit: 1,
                spans: Vec::new(),
                error: None,
            };
            assert_eq!(
                visitor.check(Span::new(
                    file.end_position(),
                    file.end_position(),
                    SyntaxContext::root(),
                    None
                )),
                Ok(())
            );
            visitor.limit = 8;
            assert_eq!(visitor.check(rustc_span::DUMMY_SP), Err(LeafError::Span));
            assert_eq!(
                visitor
                    .check(rustc_span::DUMMY_SP.with_parent(Some(rustc_hir::def_id::CRATE_DEF_ID))),
                Err(LeafError::Span)
            );
            let past_eof = rustc_span::BytePos(file.end_position().0 + 1);
            assert_eq!(
                visitor.check(Span::new(file.start_pos, past_eof, SyntaxContext::root(), None)),
                Err(LeafError::Span)
            );
            // Same historical index but a distinct SourceFile allocation is not
            // interchangeable, even when filename and contents match.
            let impostor = sm
                .new_source_file_version(file.name.clone(), "fn leaf() -> bool { true }".into())
                .unwrap();
            let wrong = vec![(proof.active_files()[0].0, impostor)];
            let mut wrong_view = SpanClosure {
                source_map: &sm,
                owner: rustc_hir::def_id::CRATE_DEF_ID,
                files: &wrong,
                limit: 8,
                spans: Vec::new(),
                error: None,
            };
            assert_eq!(wrong_view.check(span(&file)), Err(LeafError::InactiveSpan));
        });
    }

    #[test]
    fn proof_currentness_rechecks_body_map_and_active_revision_pointers() {
        native_test(|| {
            let (sm, text) = setup();
            let revision = sm.begin_resident_source_revision(2, 1024, 3).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let bool_cached = cached(ty::Bool);
            let ty = ty::Ty(Interned::new_unchecked(&bool_cached));
            let body = leaf(ty, span(&file));
            let other_body = leaf(ty, span(&file));
            let proof = inspect_body_closure(&body, &sm, LeafLimits::default()).unwrap();
            assert!(proof.is_for(&body));
            assert!(!proof.is_for(&other_body));
            assert!(proof.is_current_for(&sm));
            let (other_map, _) = setup();
            assert!(!proof.is_current_for(&other_map));
            drop(revision);
            assert!(!proof.is_current_for(&sm));
            let revision = sm.begin_resident_source_revision(2, 1024, 3).unwrap();
            assert!(!proof.is_current_for(&sm));
            let unchanged = sm.load_file(Path::new("leaf.rs")).unwrap();
            assert!(Arc::ptr_eq(&file, &unchanged));
            assert!(proof.is_current_for(&sm));
            drop(revision);
            *text.lock().unwrap() = "fn leaf() -> bool { false }".into();
            let _revision = sm.begin_resident_source_revision(2, 1024, 3).unwrap();
            let changed = sm.load_file(Path::new("leaf.rs")).unwrap();
            assert!(!Arc::ptr_eq(&file, &changed));
            assert!(!proof.is_current_for(&sm));
        });
    }

    #[test]
    fn scope_lint_root_must_be_absent_or_exactly_the_current_item_root() {
        native_test(|| {
            let (sm, _) = setup();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let bool_cached = cached(ty::Bool);
            let ty = ty::Ty(Interned::new_unchecked(&bool_cached));
            let mut body = leaf(ty, span(&file));
            let expected = rustc_hir::CRATE_HIR_ID;
            assert_eq!(check_lint_root(&body, expected), Ok(()));
            body.source_scopes[mir::OUTERMOST_SOURCE_SCOPE].local_data =
                mir::ClearCrossCrate::Set(mir::SourceScopeLocalData { lint_root: expected });
            assert_eq!(check_lint_root(&body, expected), Ok(()));
            let other_node = rustc_hir::HirId {
                owner: expected.owner,
                local_id: rustc_hir::ItemLocalId::from_u32(1),
            };
            assert_eq!(check_lint_root(&body, other_node), Err(LeafError::Scope));
        });
    }

    #[test]
    fn optimized_native_u8_leaf_accepts_cleared_metadata_and_literal_retag() {
        native_test(|| {
            let (sm, text) = setup();
            *text.lock().unwrap() = "#[unsafe(no_mangle)] pub fn leaf_a() -> u8 { 7 }".into();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let u8_cached = cached(ty::Uint(ty::UintTy::U8));
            let ty = Ty(Interned::new_unchecked(&u8_cached));
            let mut body = leaf(ty, span(&file));
            let mir::StatementKind::Assign(assign) =
                &mut body.basic_blocks.as_mut()[mir::START_BLOCK].statements[0].kind
            else {
                unreachable!()
            };
            let mir::Rvalue::Use(mir::Operand::Constant(operand), mir::WithRetag::Yes) =
                &mut assign.1
            else {
                unreachable!()
            };
            operand.const_ = mir::Const::Val(mir::ConstValue::Scalar(Scalar::from_u8(7)), ty);
            assert!(matches!(
                body.local_decls[mir::RETURN_PLACE].local_info,
                mir::ClearCrossCrate::Clear
            ));
            assert!(inspect_body_closure(&body, &sm, LeafLimits::default()).is_ok());
            let proof = inspect_body_closure(&body, &sm, LeafLimits::default()).unwrap();
            assert!(
                proof
                    .spans()
                    .iter()
                    .all(|span| span.parent() == Some(rustc_hir::def_id::CRATE_DEF_ID))
            );
            drop(proof);
            // Foreign definition parents are not merely discarded as metadata.
            let original_span = body.local_decls[mir::RETURN_PLACE].source_info.span;
            let foreign = LocalDefId { local_def_index: rustc_hir::def_id::DefIndex::from_u32(1) };
            body.local_decls[mir::RETURN_PLACE].source_info.span =
                original_span.with_parent(Some(foreign));
            assert!(matches!(
                inspect_body_closure(&body, &sm, LeafLimits::default()),
                Err(LeafError::Span)
            ));
            body.local_decls[mir::RETURN_PLACE].source_info.span = original_span;
            // Preserve the already reviewed pre-cleanup metadata and explicit
            // no-retag forms, but never admit arbitrary retained local metadata.
            body.local_decls[mir::RETURN_PLACE].local_info =
                mir::ClearCrossCrate::Set(Box::new(mir::LocalInfo::Boring));
            let mir::StatementKind::Assign(assign) =
                &mut body.basic_blocks.as_mut()[mir::START_BLOCK].statements[0].kind
            else {
                unreachable!()
            };
            let mir::Rvalue::Use(_, retag) = &mut assign.1 else { unreachable!() };
            *retag = mir::WithRetag::No;
            assert!(inspect_body_closure(&body, &sm, LeafLimits::default()).is_ok());
            body.local_decls[mir::RETURN_PLACE].local_info =
                mir::ClearCrossCrate::Set(Box::new(mir::LocalInfo::AggregateTemp));
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Local));
        });
    }

    #[test]
    fn actual_body_rejects_nonprimitive_calls_and_unreviewed_metadata() {
        native_test(|| {
            let (sm, _) = setup();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let bool_cached = cached(ty::Bool);
            let bool_ty = ty::Ty(Interned::new_unchecked(&bool_cached));
            let str_cached = cached(ty::Str);
            let str_ty = ty::Ty(Interned::new_unchecked(&str_cached));
            assert_eq!(
                inspect_shape(&leaf(str_ty, span(&file)), LeafLimits::default()),
                Err(LeafError::Local)
            );
            let mut body = leaf(bool_ty, span(&file));
            let operand = mir::Operand::Constant(Box::new(mir::ConstOperand {
                span: span(&file),
                user_ty: None,
                const_: mir::Const::Val(mir::ConstValue::from_bool(true), bool_ty),
            }));
            body.basic_blocks.as_mut()[mir::START_BLOCK].terminator.as_mut().unwrap().kind =
                mir::TerminatorKind::Call {
                    func: operand,
                    args: Box::new([]),
                    destination: mir::Place::return_place(),
                    target: None,
                    unwind: mir::UnwindAction::Unreachable,
                    call_source: mir::CallSource::Normal,
                    fn_span: span(&file),
                };
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Terminator));
            let mut body = leaf(bool_ty, span(&file));
            body.required_consts = None;
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Body));
            let mut body = leaf(bool_ty, span(&file));
            body.is_polymorphic = true;
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Body));
        });
    }

    #[test]
    fn straight_line_native_arguments_temporaries_and_debug_are_closed() {
        native_test(|| {
            let (sm, _) = setup();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            let int_cached = cached(ty::Int(ty::IntTy::I32));
            let ty = Ty(Interned::new_unchecked(&int_cached));
            let mut body = leaf(ty, span(&file));
            let info = body.local_decls[mir::RETURN_PLACE].source_info;
            let argument = body.local_decls.push(body.local_decls[mir::RETURN_PLACE].clone());
            let temporary = body.local_decls.push(body.local_decls[mir::RETURN_PLACE].clone());
            body.arg_count = 1;
            let scope = body.source_scopes.push(mir::SourceScopeData {
                span: info.span,
                parent_scope: Some(mir::OUTERMOST_SOURCE_SCOPE),
                inlined: None,
                inlined_parent_scope: None,
                local_data: mir::ClearCrossCrate::Clear,
            });
            body.var_debug_info.push(mir::VarDebugInfo {
                name: rustc_span::Symbol::intern("x"),
                source_info: info,
                composite: None,
                value: mir::VarDebugInfoContents::Place(argument.into()),
                argument_index: Some(1),
            });
            let copy = || mir::Operand::Copy(argument.into());
            let moved = || mir::Operand::Move(temporary.into());
            let assign = |place, value| {
                mir::Statement::new(info, mir::StatementKind::Assign(Box::new((place, value))))
            };
            body.basic_blocks.as_mut()[mir::START_BLOCK].statements = vec![
                assign(
                    temporary.into(),
                    mir::Rvalue::BinaryOp(mir::BinOp::BitXor, Box::new((copy(), copy()))),
                ),
                assign(mir::Place::return_place(), mir::Rvalue::UnaryOp(mir::UnOp::Not, moved())),
            ];
            body.local_decls[temporary].source_info.scope = scope;
            assert!(inspect_body_closure(&body, &sm, LeafLimits::default()).is_ok());
            for op in [
                mir::BinOp::Add,
                mir::BinOp::Sub,
                mir::BinOp::Mul,
                mir::BinOp::BitXor,
                mir::BinOp::BitAnd,
                mir::BinOp::BitOr,
                mir::BinOp::Shl,
                mir::BinOp::Shr,
            ] {
                assert_eq!(
                    check_rvalue(&body, &mir::Rvalue::BinaryOp(op, Box::new((copy(), copy()))), ty),
                    Ok(())
                );
            }
            assert_eq!(
                check_rvalue(&body, &mir::Rvalue::UnaryOp(mir::UnOp::Neg, copy()), ty),
                Ok(())
            );
            assert_eq!(
                check_rvalue(&body, &mir::Rvalue::Use(copy(), mir::WithRetag::No), ty),
                Ok(())
            );
            for op in [
                mir::BinOp::Div,
                mir::BinOp::Rem,
                mir::BinOp::AddWithOverflow,
                mir::BinOp::AddUnchecked,
                mir::BinOp::Cmp,
                mir::BinOp::Offset,
                mir::BinOp::Eq,
            ] {
                assert_eq!(
                    check_rvalue(&body, &mir::Rvalue::BinaryOp(op, Box::new((copy(), copy()))), ty),
                    Err(LeafError::Statement)
                );
            }
            assert_eq!(
                check_rvalue(&body, &mir::Rvalue::UnaryOp(mir::UnOp::PtrMetadata, copy()), ty),
                Err(LeafError::Statement)
            );
            for limits in [
                LeafLimits { locals: 2, ..LeafLimits::default() },
                LeafLimits { scopes: 1, ..LeafLimits::default() },
                LeafLimits { debug_entries: 0, ..LeafLimits::default() },
                LeafLimits { statements: 1, ..LeafLimits::default() },
            ] {
                assert!(matches!(inspect_body_closure(&body, &sm, limits), Err(LeafError::Limit)));
            }
            body.var_debug_info[0].argument_index = Some(2);
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Local));
            body.var_debug_info[0].argument_index = Some(1);
            body.var_debug_info[0].name = rustc_span::Symbol::intern("not a plain name");
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Local));
            body.var_debug_info[0].name = rustc_span::Symbol::intern("x");
            body.var_debug_info[0].value = mir::VarDebugInfoContents::Const(mir::ConstOperand {
                span: info.span,
                user_ty: None,
                const_: mir::Const::Val(mir::ConstValue::Scalar(Scalar::from_u32(7)), ty),
            });
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Local));
            body.var_debug_info[0].value = mir::VarDebugInfoContents::Place(argument.into());
            let expected = rustc_hir::CRATE_HIR_ID;
            body.source_scopes[scope].local_data =
                mir::ClearCrossCrate::Set(mir::SourceScopeLocalData { lint_root: expected });
            assert_eq!(check_lint_root(&body, expected), Ok(()));
            let foreign_root = rustc_hir::HirId {
                owner: expected.owner,
                local_id: rustc_hir::ItemLocalId::from_u32(1),
            };
            body.source_scopes[scope].local_data =
                mir::ClearCrossCrate::Set(mir::SourceScopeLocalData { lint_root: foreign_root });
            assert_eq!(check_lint_root(&body, expected), Err(LeafError::Scope));
            body.source_scopes[scope].local_data = mir::ClearCrossCrate::Clear;
            let invalid = mir::Local::from_usize(body.local_decls.len());
            assert_eq!(
                operand_ty(&body, &mir::Operand::Copy(invalid.into())),
                Err(LeafError::Local)
            );
            body.source_scopes[scope].parent_scope = Some(scope);
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Scope));
            body.source_scopes[scope].parent_scope = Some(mir::OUTERMOST_SOURCE_SCOPE);
            body.basic_blocks.as_mut()[mir::START_BLOCK].statements[0].kind =
                mir::StatementKind::Nop;
            assert_eq!(inspect_shape(&body, LeafLimits::default()), Err(LeafError::Statement));
        });
    }

    #[test]
    fn actual_body_visitor_rejects_expansion_stale_versions_and_limits() {
        native_test(|| {
            let (sm, text) = setup();
            let bool_cached = cached(ty::Bool);
            let ty = ty::Ty(Interned::new_unchecked(&bool_cached));
            let old = {
                let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
                sm.load_file(Path::new("leaf.rs")).unwrap()
            };
            *text.lock().unwrap() = "fn leaf() -> bool { false }".into();
            let _revision = sm.begin_resident_source_revision(2, 1024, 2).unwrap();
            let file = sm.load_file(Path::new("leaf.rs")).unwrap();
            assert!(!Arc::ptr_eq(&old, &file));
            let old_body = leaf(ty, span(&old));
            assert!(matches!(
                inspect_body_closure(&old_body, &sm, LeafLimits::default()),
                Err(LeafError::InactiveSpan)
            ));
            let mut body = leaf(ty, span(&file));
            let expanded = SyntaxContext::root()
                .apply_mark(LocalExpnId::fresh_empty().to_expn_id(), Transparency::Opaque);
            // Span only on the operand exercises MIR visitation, not body.span.
            let mir::StatementKind::Assign(assign) =
                &mut body.basic_blocks.as_mut()[mir::START_BLOCK].statements[0].kind
            else {
                unreachable!()
            };
            let mir::Rvalue::Use(mir::Operand::Constant(operand), _) = &mut assign.1 else {
                unreachable!()
            };
            operand.span = span(&file).with_ctxt(expanded);
            assert!(matches!(
                inspect_body_closure(&body, &sm, LeafLimits::default()),
                Err(LeafError::Span)
            ));
            let body = leaf(ty, span(&file));
            for limits in [
                LeafLimits { spans: 0, ..LeafLimits::default() },
                LeafLimits { statements: 0, ..LeafLimits::default() },
                LeafLimits { active_files: 0, ..LeafLimits::default() },
                LeafLimits { historical_files: 1, ..LeafLimits::default() },
            ] {
                assert!(matches!(inspect_body_closure(&body, &sm, limits), Err(LeafError::Limit)));
            }
            // A current body may still retain a historical local declaration span.
            let mut body = leaf(ty, span(&file));
            body.local_decls[mir::RETURN_PLACE].source_info.span = span(&old);
            assert!(matches!(
                inspect_body_closure(&body, &sm, LeafLimits::default()),
                Err(LeafError::InactiveSpan)
            ));
        });
    }
}
