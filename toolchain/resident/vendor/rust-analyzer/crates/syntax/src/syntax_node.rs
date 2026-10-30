//! This module defines Concrete Syntax Tree (CST), used by rust-analyzer.
//!
//! The CST includes comments and whitespace, provides a single node type,
//! `SyntaxNode`, and a basic traversal API (parent, children, siblings).
//!
//! The *real* implementation is in the (language-agnostic) `rowan` crate, this
//! module just wraps its API.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rowan::{GreenNodeBuilder, Language};

use crate::{Parse, SyntaxError, SyntaxKind, TextSize};

pub(crate) use rowan::{GreenNode, GreenToken, NodeOrToken};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RustLanguage {}
impl Language for RustLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.into())
    }
}

pub type SyntaxNode = rowan::SyntaxNode<RustLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<RustLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<RustLanguage>;
pub type SyntaxNodeChildren = rowan::SyntaxNodeChildren<RustLanguage>;
pub type SyntaxElementChildren = rowan::SyntaxElementChildren<RustLanguage>;
pub type PreorderWithTokens = rowan::api::PreorderWithTokens<RustLanguage>;

/// The tree of a parse: green, or flat when nobody edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tree {
    Green(GreenNode),
    Flat(Arc<rowan::FlatTree>),
}

impl Tree {
    pub fn root(&self) -> SyntaxNode {
        match self {
            Tree::Green(green) => SyntaxNode::new_root(green.clone()),
            Tree::Flat(tree) => SyntaxNode::new_flat_root(tree.clone()),
        }
    }

    pub fn into_green(self) -> GreenNode {
        match self {
            Tree::Green(green) => green,
            Tree::Flat(_) => self.root().green().to_owned(),
        }
    }
}

static GREEN_TREES: AtomicBool = AtomicBool::new(false);

/// Whether parses make green trees, which edit cheaply, instead of flat ones, which are
/// made and walked cheaply. Flat unless asked for green.
pub fn set_green_trees(green: bool) {
    GREEN_TREES.store(green, Ordering::Relaxed);
}

enum Builder {
    Green(GreenNodeBuilder<'static>),
    Flat(rowan::FlatTreeBuilder),
}

pub struct SyntaxTreeBuilder {
    errors: Vec<SyntaxError>,
    inner: Builder,
}

impl Default for SyntaxTreeBuilder {
    fn default() -> SyntaxTreeBuilder {
        let inner = match GREEN_TREES.load(Ordering::Relaxed) {
            true => Builder::Green(GreenNodeBuilder::new()),
            false => Builder::Flat(rowan::FlatTreeBuilder::new()),
        };
        SyntaxTreeBuilder { errors: Vec::new(), inner }
    }
}

impl SyntaxTreeBuilder {
    pub(crate) fn finish_raw(self) -> (Tree, Vec<SyntaxError>) {
        let tree = match self.inner {
            Builder::Green(it) => Tree::Green(it.finish()),
            Builder::Flat(it) => Tree::Flat(Arc::new(it.finish())),
        };
        (tree, self.errors)
    }

    pub fn finish(self) -> Parse<SyntaxNode> {
        let (tree, errors) = self.finish_raw();
        // Disable block validation, see https://github.com/rust-lang/rust-analyzer/pull/10357
        #[allow(clippy::overly_complex_bool_expr)]
        if cfg!(debug_assertions) && false {
            crate::validation::validate_block_structure(&tree.root());
        }
        Parse::new(tree, errors)
    }

    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let kind = RustLanguage::kind_to_raw(kind);
        match &mut self.inner {
            Builder::Green(it) => it.token(kind, text),
            Builder::Flat(it) => it.token(kind, text),
        }
    }

    pub fn start_node(&mut self, kind: SyntaxKind) {
        let kind = RustLanguage::kind_to_raw(kind);
        match &mut self.inner {
            Builder::Green(it) => it.start_node(kind),
            Builder::Flat(it) => it.start_node(kind),
        }
    }

    pub fn finish_node(&mut self) {
        match &mut self.inner {
            Builder::Green(it) => it.finish_node(),
            Builder::Flat(it) => it.finish_node(),
        }
    }

    pub fn error(&mut self, error: String, text_pos: TextSize) {
        self.errors.push(SyntaxError::new_at_offset(error, text_pos));
    }
}
