//! Conservative closure evidence for one optimized constant-return MIR leaf.
//!
//! This is NOT changed-source authorization, query reuse permission, or evidence
//! about callers/HIR/attributes/ABI. It does not consult or modify reuse gates.
//! Evidence borrows the inspected body and owns exact active-file Arcs. Activation
//! of another revision invalidates its *meaning* as a current-source snapshot;
//! consumers must recheck source revision identity before any future use.

use std::sync::Arc;

use rustc_hir::def::DefKind;
use rustc_span::source_map::SourceMap;
use rustc_span::{SourceFile, Span, SyntaxContext};

use crate::mir::interpret::Scalar;
use crate::mir::visit::Visitor;
use crate::{mir, ty};

#[derive(Clone, Copy, Debug)]
pub struct LeafLimits {
    pub historical_files: usize,
    pub active_files: usize,
    pub spans: usize,
    pub statements: usize,
}

impl Default for LeafLimits {
    fn default() -> Self {
        Self { historical_files: 4096, active_files: 64, spans: 32, statements: 8 }
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
}

/// `tcx` establishes ordinary local item kind and absence of inherited generics,
/// facts not represented by `Body::is_polymorphic`. The active source view is
/// obtained natively from `source_map`, never supplied as an assertion by callers.
/// Traversal and copying are budgeted. The owner must keep source loading and
/// revision activation quiescent during inspection: SourceMap's snapshot API
/// does not provide a bounded atomic snapshot with its historical-file count.
/// No claim of concurrent or subsequent revision freshness is made.
pub fn inspect<'body, 'tcx>(
    tcx: ty::TyCtxt<'tcx>,
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
    inspect_body_closure(body, source_map, limits)
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
    let mut visitor = SpanClosure {
        source_map,
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
    Ok(LeafProof { body, source_map, files, spans })
}

fn primitive(ty: ty::Ty<'_>) -> bool {
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
        || *arg_count != 0
        || spread_arg.is_some()
        || !var_debug_info.is_empty()
        || *is_polymorphic
        || injection_phase.is_some()
        || tainted_by_errors.is_some()
        || coverage_early_info.is_some()
        || coverage_mir_info.is_some()
        || !matches!(required_consts, Some(c) if c.is_empty())
        || !matches!(mentioned_items, Some(c) if c.is_empty())
        || basic_blocks.len() != 1
        || local_decls.len() != 1
        || source_scopes.len() != 1
    {
        return Err(LeafError::Body);
    }
    let local = &local_decls[mir::RETURN_PLACE];
    let mir::LocalDecl { mutability: _, local_info, ty, user_ty, source_info } = local;
    if !primitive(*ty)
        || user_ty.is_some()
        || !matches!(local_info, mir::ClearCrossCrate::Set(info) if matches!(**info, mir::LocalInfo::Boring))
        || source_info.scope != mir::OUTERMOST_SOURCE_SCOPE
    {
        return Err(LeafError::Local);
    }
    let mir::SourceScopeData {
        span: _,
        parent_scope,
        inlined,
        inlined_parent_scope,
        local_data: _,
    } = &source_scopes[mir::OUTERMOST_SOURCE_SCOPE];
    if parent_scope.is_some() || inlined.is_some() || inlined_parent_scope.is_some() {
        return Err(LeafError::Scope);
    }
    let mir::BasicBlockData { statements, after_last_stmt_debuginfos, terminator, is_cleanup } =
        &basic_blocks[mir::START_BLOCK];
    if statements.len() > limits.statements {
        return Err(LeafError::Limit);
    }
    if *is_cleanup || !after_last_stmt_debuginfos.is_empty() {
        return Err(LeafError::Body);
    }
    // Exactly one write. Even storage/noop statements are excluded initially:
    // admitting them is unnecessary for optimized constant leaves.
    if statements.len() != 1 {
        return Err(LeafError::Statement);
    }
    let mir::Statement { source_info, kind, debuginfos } = &statements[0];
    if !debuginfos.is_empty() {
        return Err(LeafError::Statement);
    }
    if source_info.scope != mir::OUTERMOST_SOURCE_SCOPE {
        return Err(LeafError::Scope);
    }
    let mir::StatementKind::Assign(assign) = kind else {
        return Err(LeafError::Statement);
    };
    let (place, value) = &**assign;
    if place.local != mir::RETURN_PLACE || !place.projection.is_empty() {
        return Err(LeafError::Statement);
    }
    let mir::Rvalue::Use(mir::Operand::Constant(constant), mir::WithRetag::No) = value else {
        return Err(LeafError::Constant);
    };
    let mir::ConstOperand { span: _, user_ty, const_ } = &**constant;
    if user_ty.is_some() {
        return Err(LeafError::Constant);
    }
    let mir::Const::Val(mir::ConstValue::Scalar(Scalar::Int(_)), const_ty) = const_ else {
        return Err(LeafError::Constant);
    };
    if const_ty != ty {
        return Err(LeafError::Constant);
    }
    let Some(mir::Terminator { source_info, kind: mir::TerminatorKind::Return, loop_hint_attrs }) =
        terminator
    else {
        return Err(LeafError::Terminator);
    };
    if !loop_hint_attrs.is_empty() {
        return Err(LeafError::Terminator);
    }
    if source_info.scope != mir::OUTERMOST_SOURCE_SCOPE {
        return Err(LeafError::Scope);
    }
    Ok(())
}

struct SpanClosure<'a> {
    source_map: &'a SourceMap,
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
        if span.is_dummy()
            || span.parent().is_some()
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

    fn leaf<'tcx>(ty: ty::Ty<'tcx>, span: Span) -> mir::Body<'tcx> {
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
                mir::Rvalue::Use(constant, mir::WithRetag::No),
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
        locals.push(mir::LocalDecl::new(ty, span));
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
                files: &wrong,
                limit: 8,
                spans: Vec::new(),
                error: None,
            };
            assert_eq!(wrong_view.check(span(&file)), Err(LeafError::InactiveSpan));
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
