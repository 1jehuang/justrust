//! Defines `Body`: a lowered representation of functions, statics and
//! consts.
use std::ops;

use base_db::SourceDatabase;
use hir_expand::{HirFileId, InFile, Lookup, span_map::SpanMap};
use span::{AstIdMap, Edition, ROOT_ERASED_FILE_AST_ID, RealSpanMap};
use syntax::{
    AstNode, NodeOrToken, SyntaxKind, SyntaxNode, SyntaxNodePtr, TextRange, TextSize, ast,
};
use triomphe::Arc;

use crate::{
    BlockId, DefWithBodyId, ExpressionStoreOwnerId, HasModule,
    expr_store::{
        ExpressionStore, ExpressionStoreSourceMap, Move, SelfParamPtr,
        lower::{lower_body, lower_body_scopes},
        pretty,
    },
    hir::{BindingId, ExprId, PatId},
    src::HasSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param<Id> {
    /// The id of the formal parameter. In analysis, you want to use this: it has a special status as a parameter,
    /// while [`user_written`][Self::user_written] is just a local variable.
    pub formal: Id,
    /// The id that corresponds to the pattern the user wrote. In IDE, you usually want to use this.
    ///
    /// It will be different than [`formal`][Self::formal] for coroutine fns (async fn etc.), because we apply
    /// a desugaring for those that copies the parameter into a local variable.
    pub user_written: Id,
}

impl<Id: Copy> Param<Id> {
    pub(crate) fn new(id: Id) -> Self {
        Self { formal: id, user_written: id }
    }
}

/// The body of an item (function, const etc.).
#[derive(Debug, Eq, PartialEq)]
pub struct Body {
    pub store: ExpressionStore,
    /// The patterns for the function's parameters. While the parameter types are
    /// part of the function signature, the patterns are not (they don't change
    /// the external type of the function).
    ///
    /// If this `Body` is for the body of a constant, this will just be
    /// empty.
    pub params: Box<[Param<PatId>]>,
    pub self_param: Option<Param<BindingId>>,
}

impl ops::Deref for Body {
    type Target = ExpressionStore;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

/// An item body together with the mapping from syntax nodes to HIR expression
/// IDs. This is needed to go from e.g. a position in a file to the HIR
/// expression containing it; but for type inference etc., we want to operate on
/// a structure that is agnostic to the actual positions of expressions in the
/// file, so that we don't recompute types whenever some whitespace is typed.
///
/// One complication here is that, due to macro expansion, a single `Body` might
/// be spread across several files. So, for each ExprId and PatId, we record
/// both the HirFileId and the position inside the file. However, we only store
/// AST -> ExprId mapping for non-macro files, as it is not clear how to handle
/// this properly for macros.
#[derive(Default, Debug, Eq, PartialEq)]
pub struct BodySourceMap {
    pub self_param: Option<InFile<SelfParamPtr>>,
    pub store: ExpressionStoreSourceMap,
}

impl ops::Deref for BodySourceMap {
    type Target = ExpressionStoreSourceMap;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

#[salsa::tracked]
impl Body {
    /// The body with where each part of it is written. For a body written in a file, this is
    /// [`Body::relative`] with its source map moved to where the body is in the file: the body
    /// is lowered once, and an edit elsewhere in the file only moves the map.
    #[salsa::tracked(lru = 512, returns(ref))]
    pub fn with_source_map(
        db: &dyn SourceDatabase,
        def: DefWithBodyId,
    ) -> (Arc<Body>, BodySourceMap) {
        let _p = tracing::info_span!("body_with_source_map_query").entered();
        let Some((body, source_map)) = Self::relative(db, def) else {
            let (body, source_map) = lower_in_file(db, def);
            return (Arc::new(body), source_map);
        };
        let syntax = body_syntax(db, def).as_ref().expect("lowered without its syntax");
        let item = def_syntax(db, def).value;
        let start = item.text_range().start() - syntax.item.text_range().start();
        let anchor = item
            .ancestors()
            .find(|it| {
                let range = it.text_range();
                it.kind() == syntax.kind
                    && range.start() == start
                    && range.len() == syntax.range.len()
                    && kept(it).0 == syntax.tree
            })
            .expect("the anchor is not around the item");
        debug_assert_eq!(Some(start), Self::anchor_offset(db, def));
        let to = |ptr: SyntaxNodePtr| {
            let range = ptr.text_range() + start;
            anchor
                .descendant_at_range(range, ptr.kind())
                .map(|it| SyntaxNodePtr::new(&it))
                .expect("no node where the body has one")
        };
        let mv = Move { file: syntax.file_id, to: &to };
        let source_map = BodySourceMap {
            self_param: source_map.self_param.map(|it| mv.ptr(it)),
            store: source_map.store.moved(&mv),
        };
        if check_relative() {
            let (whole, in_file) = lower_in_file(db, def);
            assert_eq!(in_file, source_map, "{def:?}: the source map lowered alone, moved");
            assert_eq!(whole.params.len(), body.params.len());
        }
        (body.clone(), source_map)
    }

    /// The body lowered from [`body_syntax`] rather than from the whole file: what it is made of
    /// stays the same while edits elsewhere in the file move it, so it is lowered again only
    /// when its own text changes. The source map is of the tree of [`BodySyntax::root`]. `None`
    /// when the body is not written in a file, but by a macro. Kept for more bodies than
    /// [`Body::with_source_map`] is: what is lowered again after it is let go of has changed
    /// to all that reads it.
    #[salsa::tracked(lru = 4096, returns(ref))]
    pub fn relative(
        db: &dyn SourceDatabase,
        def: DefWithBodyId,
    ) -> Option<(Arc<Body>, BodySourceMap)> {
        let _p = tracing::info_span!("body_relative_query").entered();
        let input = relative_input(db, def)?;
        let (body, source_map) = lower_body(
            db,
            def,
            input.syntax_node,
            input.file_id,
            def.module(db),
            input.parts.params,
            input.parts.body,
            input.parts.is_async_fn,
            input.parts.is_gen_fn,
            input.maps,
        );
        Some((Arc::new(body), source_map))
    }

    #[salsa::tracked(returns(deref))]
    pub fn of(db: &dyn SourceDatabase, def: DefWithBodyId) -> Arc<Body> {
        match Self::relative(db, def) {
            Some((body, _)) => body.clone(),
            None => Self::with_source_map(db, def).0.clone(),
        }
    }

    /// Current item-bearing blocks, in the original lowering visit order. This query uses
    /// the authoritative collector with an output sink, not `Body::of` or an AstId guess.
    #[salsa::tracked(returns(ref))]
    pub(crate) fn block_scopes(db: &dyn SourceDatabase, def: DefWithBodyId) -> Vec<BlockId> {
        let input = relative_input(db, def).unwrap_or_else(|| file_input(db, def));
        lower_body_scopes(
            db,
            def,
            input.syntax_node,
            input.file_id,
            def.module(db),
            input.parts.params,
            input.parts.body,
            input.parts.is_async_fn,
            input.parts.is_gen_fn,
            input.maps,
        )
    }

    /// Independent original whole-file input selection and full materializing traversal.
    /// Do not replace this with the candidate's input helper or scope-query output.
    #[cfg(test)]
    #[salsa::tracked(returns(ref))]
    pub(crate) fn original_fullwalk_blocks(
        db: &dyn SourceDatabase,
        def: DefWithBodyId,
    ) -> Vec<BlockId> {
        let mut params = None;
        let mut is_async_fn = false;
        let mut is_gen_fn = false;
        let (InFile { file_id, value: body }, syntax_node) = match def {
            DefWithBodyId::FunctionId(f) => {
                let src = f.lookup(db).source(db);
                params = src.value.param_list();
                is_async_fn = src.value.async_token().is_some();
                is_gen_fn = src.value.gen_token().is_some();
                let syntax_node = SyntaxNodePtr::new(src.syntax().value);
                (src.map(|it| it.body().map(ast::Expr::from)), syntax_node)
            }
            DefWithBodyId::ConstId(c) => {
                let src = c.lookup(db).source(db);
                let syntax_node = SyntaxNodePtr::new(src.syntax().value);
                (src.map(|it| it.body()), syntax_node)
            }
            DefWithBodyId::StaticId(s) => {
                let src = s.lookup(db).source(db);
                let syntax_node = SyntaxNodePtr::new(src.syntax().value);
                (src.map(|it| it.body()), syntax_node)
            }
            DefWithBodyId::VariantId(v) => {
                let src = v.lookup(db).source(db);
                let syntax_node = SyntaxNodePtr::new(src.syntax().value);
                (src.map(|it| it.const_arg()?.expr()), syntax_node)
            }
        };
        let (body, _) = lower_body(
            db,
            def,
            syntax_node,
            file_id,
            def.module(db),
            params,
            body,
            is_async_fn,
            is_gen_fn,
            None,
        );
        body.blocks(db).map(|(block, _)| block).collect()
    }
}

impl Body {
    /// Where in its file the anchor of [`body_syntax`] begins, which the source map of
    /// [`Body::relative`] is relative to. `None` for a body a macro writes.
    pub fn anchor_offset(db: &dyn SourceDatabase, def: DefWithBodyId) -> Option<TextSize> {
        let syntax = body_syntax(db, def).as_ref()?;
        Some(def_syntax(db, def).value.text_range().start() - syntax.item.text_range().start())
    }

    /// The source map of [`Body::relative`] when there is one, and of
    /// [`Body::with_source_map`] when there is not: for what needs of it only the file or the
    /// expansion each part is written in, and the macro calls, and not where in the file.
    /// It stays the same while edits elsewhere in the file move the body.
    pub fn unmoved_source_map(db: &dyn SourceDatabase, def: DefWithBodyId) -> &BodySourceMap {
        match Self::relative(db, def) {
            Some((_, source_map)) => source_map,
            None => &Self::with_source_map(db, def).1,
        }
    }
}

/// The syntax of the item `def` is the body of.
fn def_syntax(db: &dyn SourceDatabase, def: DefWithBodyId) -> InFile<SyntaxNode> {
    match def {
        DefWithBodyId::FunctionId(it) => it.lookup(db).source(db).map(|it| it.syntax().clone()),
        DefWithBodyId::ConstId(it) => it.lookup(db).source(db).map(|it| it.syntax().clone()),
        DefWithBodyId::StaticId(it) => it.lookup(db).source(db).map(|it| it.syntax().clone()),
        DefWithBodyId::VariantId(it) => it.lookup(db).source(db).map(|it| it.syntax().clone()),
    }
}

/// Lowers the body of `def` from the whole file it is in. What a body written by a macro is
/// lowered with, and what [`check_relative`] checks the others against.
fn lower_in_file(db: &dyn SourceDatabase, def: DefWithBodyId) -> (Body, BodySourceMap) {
    let input = file_input(db, def);
    lower_body(
        db,
        def,
        input.syntax_node,
        input.file_id,
        def.module(db),
        input.parts.params,
        input.parts.body,
        input.parts.is_async_fn,
        input.parts.is_gen_fn,
        input.maps,
    )
}

struct BodyParts {
    params: Option<ast::ParamList>,
    body: Option<ast::Expr>,
    is_async_fn: bool,
    is_gen_fn: bool,
}

/// Selection only: parameter *types* are not Body inputs. Closure types are visited later
/// by the collector. Keeping this shared prevents scope discovery from scanning signatures.
fn body_parts(def: DefWithBodyId, node: SyntaxNode) -> Option<BodyParts> {
    let mut parts = BodyParts { params: None, body: None, is_async_fn: false, is_gen_fn: false };
    parts.body = match def {
        DefWithBodyId::FunctionId(_) => {
            let it = ast::Fn::cast(node)?;
            parts.params = it.param_list();
            parts.is_async_fn = it.async_token().is_some();
            parts.is_gen_fn = it.gen_token().is_some();
            it.body().map(ast::Expr::from)
        }
        DefWithBodyId::ConstId(_) => ast::Const::cast(node)?.body(),
        DefWithBodyId::StaticId(_) => ast::Static::cast(node)?.body(),
        DefWithBodyId::VariantId(_) => ast::Variant::cast(node)?.const_arg()?.expr(),
    };
    Some(parts)
}

struct BodyInput<'db> {
    syntax_node: SyntaxNodePtr,
    file_id: HirFileId,
    parts: BodyParts,
    maps: Option<(&'db RealSpanMap, &'db AstIdMap)>,
}

fn relative_input(db: &dyn SourceDatabase, def: DefWithBodyId) -> Option<BodyInput<'_>> {
    let syntax = body_syntax(db, def).as_ref()?;
    Some(BodyInput {
        syntax_node: syntax.item,
        file_id: syntax.file_id,
        parts: body_parts(def, syntax.item.to_node(&syntax.root(db)))?,
        maps: Some((&syntax.span_map, &syntax.ast_id_map)),
    })
}

fn file_input(db: &dyn SourceDatabase, def: DefWithBodyId) -> BodyInput<'_> {
    let src = def_syntax(db, def);
    // A variant without a const argument has no body. The relative route falls back here.
    let parts = body_parts(def, src.value.clone()).unwrap_or_else(|| {
        assert!(matches!(def, DefWithBodyId::VariantId(_)), "body source has the wrong kind");
        BodyParts { params: None, body: None, is_async_fn: false, is_gen_fn: false }
    });
    BodyInput {
        syntax_node: SyntaxNodePtr::new(&src.value),
        file_id: src.file_id,
        parts,
        maps: None,
    }
}

/// Whether [`Body::with_source_map`] checks the source map of each body lowered alone, moved,
/// against that of the same body lowered in its file, and [`BodySyntax::root`] parses every
/// anchor again and prints the ones that parse differently alone (`RA_CHECK_RELATIVE_BODIES`).
fn check_relative() -> bool {
    static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECK.get_or_init(|| std::env::var_os("RA_CHECK_RELATIVE_BODIES").is_some())
}

/// What lowering a body reads of the file it is written in, as a tree of its own: the anchor
/// the body is written in ([`RealSpanMap::anchor_at`]), which is the item itself unless that
/// is an item in a body, or the item of an `impl` or a module with attributes. Its ranges begin
/// at the anchor, and so stay as they are while edits elsewhere move it in the file. A body
/// lowered from this is lowered again only when this changes, not on every edit of the file.
///
/// It holds no tree, so that a file's tree is let go of once nothing is parsing or lowering
/// it: the tree of [`BodySyntax::root`] is parsed again from the text of the anchor when it
/// is not among the last few made. It is the same as the one in the file, which it is
/// compared by ([`tree_hash`]).
pub struct BodySyntax {
    pub file_id: HirFileId,
    tree: u128,
    /// The text of the file when this was made, with where the anchor is in it, and what kind
    /// of node it is. Only what is there is read: where it is changes without this changing.
    text: Arc<str>,
    range: TextRange,
    kind: SyntaxKind,
    edition: Edition,
    /// The item in [`BodySyntax::root`].
    pub item: SyntaxNodePtr,
    pub ast_id_map: AstIdMap,
    pub span_map: RealSpanMap,
}

impl PartialEq for BodySyntax {
    fn eq(&self, other: &Self) -> bool {
        self.file_id == other.file_id
            && self.tree == other.tree
            && self.item == other.item
            && self.ast_id_map == other.ast_id_map
            && self.span_map == other.span_map
    }
}

impl Eq for BodySyntax {}

impl std::fmt::Debug for BodySyntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodySyntax")
            .field("file_id", &self.file_id)
            .field("range", &self.range)
            .field("item", &self.item)
            .finish_non_exhaustive()
    }
}

impl BodySyntax {
    /// The anchor as the root of a tree of its own.
    pub fn root(&self, db: &dyn SourceDatabase) -> SyntaxNode {
        let kept = trees::get(self.tree).filter(|_| !check_relative());
        kept.unwrap_or_else(|| {
            let root = self.reparse().unwrap_or_else(|| self.from_file(db));
            trees::put(self.tree, root.clone(), None);
            root
        })
    }

    /// The text of the anchor.
    pub fn text(&self) -> &str {
        &self.text[self.range]
    }

    /// The anchor parsed alone, when that makes the tree it is in the file.
    fn reparse(&self) -> Option<SyntaxNode> {
        let file = ast::SourceFile::parse(self.text(), self.edition).syntax_node();
        let root = match self.kind == SyntaxKind::SOURCE_FILE {
            true => file,
            false => {
                let range = TextRange::up_to(self.range.len());
                let mut items = file.children();
                let item = items.next().filter(|_| items.next().is_none())?;
                (item.kind() == self.kind && item.text_range() == range)
                    .then(|| item.clone_subtree())?
            }
        };
        (tree_hash(&root) == self.tree).then_some(root)
    }

    /// The anchor from the tree of its file, for one that parses differently alone.
    fn from_file(&self, db: &dyn SourceDatabase) -> SyntaxNode {
        if check_relative() {
            eprintln!(
                "{:?}: the anchor at {:?} parses differently alone",
                self.file_id, self.range
            );
        }
        let file = self.file_id.file_id().expect("a body written in a file").parse(db);
        file.syntax_node()
            .descendants()
            .filter(|it| it.text_range() == self.range && it.kind() == self.kind)
            .map(|it| kept(&it))
            .find(|it| it.0 == self.tree)
            .expect("the anchor is not in its file")
            .1
    }
}

/// What the subtree of `node` is made of, as the root of a tree of its own, so that two trees
/// made of the same are the same. A flat node's is read in place.
fn tree_hash(node: &SyntaxNode) -> u128 {
    use std::hash::{Hash, Hasher};
    let alone;
    let node = match node.flat() {
        Some(_) => node,
        None => {
            alone = node.clone_subtree().to_flat();
            &alone
        }
    };
    let (tree, at) = node.flat().expect("a flat tree");
    let half = |seed: u8| {
        let mut hasher = std::hash::DefaultHasher::new();
        seed.hash(&mut hasher);
        tree.hash_subtree(at, &mut hasher);
        hasher.finish() as u128
    };
    half(0) << 64 | half(1)
}

/// `anchor` as the root of a tree of its own, with its [`tree_hash`]: the one kept among the
/// trees of this thread when that is of this anchor or made of the same, else a copy, kept.
/// The bodies of an anchor are made one after another, and hashing or copying it again for
/// each is long when it is a whole `impl` or module.
fn kept(anchor: &SyntaxNode) -> (u128, SyntaxNode) {
    if let Some(found) = trees::of_anchor(anchor) {
        return found;
    }
    let tree = tree_hash(anchor);
    let root = trees::get(tree).unwrap_or_else(|| anchor.clone_subtree());
    trees::put(tree, root.clone(), Some(anchor));
    (tree, root)
}

/// The trees of the anchors last parsed or lowered on this thread: what a body is lowered from
/// right after its [`BodySyntax`] is made, and where its places are found while it is compiled.
mod trees {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        sync::{Arc, Weak},
    };

    use syntax::{FlatTree, SyntaxNode};

    const KEPT: usize = 32;

    /// A tree, its root, and the anchor in a file's flat tree it was made of: that tree, which
    /// is not let go of while this holds it, and the anchor's number in it.
    type Kept = (u128, SyntaxNode, Option<(Weak<FlatTree>, u32)>);

    thread_local! {
        static TREES: RefCell<VecDeque<Kept>> = const { RefCell::new(VecDeque::new()) };
    }

    pub(super) fn get(tree: u128) -> Option<SyntaxNode> {
        TREES.with_borrow(|trees| trees.iter().find(|it| it.0 == tree).map(|it| it.1.clone()))
    }

    /// The tree kept of `anchor`, when it is one of these.
    pub(super) fn of_anchor(anchor: &SyntaxNode) -> Option<(u128, SyntaxNode)> {
        let (file, at) = anchor.flat()?;
        TREES.with_borrow(|trees| {
            let of = |it: &&Kept| {
                it.2.as_ref().is_some_and(|(of, of_at)| {
                    *of_at == at && std::ptr::eq(of.as_ptr(), Arc::as_ptr(file))
                })
            };
            trees.iter().find(of).map(|it| (it.0, it.1.clone()))
        })
    }

    pub(super) fn put(tree: u128, root: SyntaxNode, anchor: Option<&SyntaxNode>) {
        let anchor = anchor.and_then(|it| it.flat()).map(|(file, at)| (Arc::downgrade(file), at));
        TREES.with_borrow_mut(|trees| {
            trees.retain(|it| it.0 != tree);
            if trees.len() == KEPT {
                trees.pop_front();
            }
            trees.push_back((tree, root, anchor));
        })
    }
}

/// The [`BodySyntax`] of a body written in a file, `None` for one a macro writes.
#[salsa::tracked(returns(ref))]
pub fn body_syntax(db: &dyn SourceDatabase, def: DefWithBodyId) -> Option<BodySyntax> {
    let src = def_syntax(db, def);
    let SpanMap::RealSpanMap(span_map) = src.file_id.span_map(db) else { return None };
    let item = src.value;
    let (start, anchor) = span_map.anchor_at(item.text_range().start());
    let anchor = match anchor == ROOT_ERASED_FILE_AST_ID {
        true => item.ancestors().last()?,
        false => {
            let ptr = src.file_id.ast_id_map(db).get_erased(anchor);
            item.ancestors().find(|it| SyntaxNodePtr::new(it) == ptr)?
        }
    };
    let (tree, root) = kept(&anchor);
    let range = item.text_range() - start;
    let relative = match root.covering_element(range) {
        NodeOrToken::Node(it) => it,
        NodeOrToken::Token(it) => it.parent()?,
    };
    let relative =
        relative.ancestors().find(|it| it.text_range() == range && it.kind() == item.kind())?;
    let (file, edition) = src.file_id.file_id()?.unpack(db);
    Some(BodySyntax {
        file_id: src.file_id,
        tree,
        text: db.file_text(file).text(db).clone(),
        range: anchor.text_range(),
        kind: anchor.kind(),
        edition,
        item: SyntaxNodePtr::new(&relative),
        ast_id_map: src.file_id.ast_id_map(db).relative_to(&anchor, &root),
        span_map: span_map.within(anchor.text_range()),
    })
}

impl Body {
    pub fn root_expr(&self) -> ExprId {
        // A `Body` can also contain root expressions that aren't the body (in the param patterns),
        // but the body always come last.
        self.store.expr_roots().next_back().unwrap()
    }

    /// Returns `true` if this is the formal or user-written self param.
    #[inline]
    pub fn is_any_self_param(&self, binding: BindingId) -> bool {
        self.self_param.is_some_and(|self_param| {
            self_param.formal == binding || self_param.user_written == binding
        })
    }

    pub fn pretty_print(
        &self,
        db: &dyn SourceDatabase,
        owner: DefWithBodyId,
        edition: Edition,
    ) -> String {
        pretty::print_body_hir(db, self, owner, edition)
    }

    pub fn pretty_print_expr(
        &self,
        db: &dyn SourceDatabase,
        owner: DefWithBodyId,
        expr: ExprId,
        edition: Edition,
    ) -> String {
        pretty::print_expr_hir(db, self, owner.into(), expr, edition)
    }

    pub fn pretty_print_pat(
        &self,
        db: &dyn SourceDatabase,
        owner: ExpressionStoreOwnerId,
        pat: PatId,
        oneline: bool,
        edition: Edition,
    ) -> String {
        pretty::print_pat_hir(db, self, owner, pat, oneline, edition)
    }
}

impl BodySourceMap {
    pub fn self_param_syntax(&self) -> Option<InFile<SelfParamPtr>> {
        self.self_param
    }
}
