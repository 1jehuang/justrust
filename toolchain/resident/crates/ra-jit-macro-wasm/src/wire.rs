//! The host's side of the encoding a stream crosses into and out of the module in
//! (`guest/proc_macro.rs`, `wire`), from and to rust-analyzer's token trees, with each span
//! a number of the [`Spans`] of the expansion.

use intern::Symbol;
use rustc_hash::FxHashMap;
use span::Span;
use tt::{
    DelimiterKind, IdentIsRaw, Leaf, LitKind, Spacing, Subtree, TokenTree, TopSubtree,
    iter::{TtElement, TtIter},
};

/// The spans of an expansion, numbered from 1 in the order the host first gave them out: 0
/// is none.
#[derive(Default)]
pub(crate) struct Spans {
    all: Vec<Span>,
    numbers: FxHashMap<Span, u32>,
}

impl Spans {
    pub(crate) fn number(&mut self, span: Span) -> u32 {
        *self.numbers.entry(span).or_insert_with(|| {
            self.all.push(span);
            self.all.len() as u32
        })
    }

    /// Numbers `span` next, as a kept output's table has it, the same as an earlier or not.
    pub(crate) fn push(&mut self, span: Span) {
        self.all.push(span);
        self.numbers.entry(span).or_insert(self.all.len() as u32);
    }

    pub(crate) fn get(&self, number: u32) -> Option<Span> {
        self.all.get((number as usize).checked_sub(1)?).copied()
    }

    pub(crate) fn all(&self) -> &[Span] {
        &self.all
    }
}

pub(crate) fn lit_kind_number(kind: LitKind) -> (u8, u8) {
    match kind {
        LitKind::Byte => (0, 0),
        LitKind::Char => (1, 0),
        LitKind::Integer => (2, 0),
        LitKind::Float => (3, 0),
        LitKind::Str => (4, 0),
        LitKind::StrRaw(n) => (5, n),
        LitKind::ByteStr => (6, 0),
        LitKind::ByteStrRaw(n) => (7, n),
        LitKind::CStr => (8, 0),
        LitKind::CStrRaw(n) => (9, n),
        LitKind::Err(()) => (10, 0),
    }
}

fn lit_kind(kind: u8, hashes: u8) -> LitKind {
    match kind {
        0 => LitKind::Byte,
        1 => LitKind::Char,
        2 => LitKind::Integer,
        3 => LitKind::Float,
        4 => LitKind::Str,
        5 => LitKind::StrRaw(hashes),
        6 => LitKind::ByteStr,
        7 => LitKind::ByteStrRaw(hashes),
        8 => LitKind::CStr,
        9 => LitKind::CStrRaw(hashes),
        _ => LitKind::Err(()),
    }
}

/// Writes streams as the module reads them.
pub(crate) struct Writer<'a> {
    pub(crate) out: Vec<u8>,
    pub(crate) spans: &'a mut Spans,
}

impl Writer<'_> {
    fn u32(&mut self, it: u32) {
        self.out.extend_from_slice(&it.to_le_bytes());
    }

    fn span(&mut self, span: Span) {
        let number = self.spans.number(span);
        self.u32(number);
    }

    fn text(&mut self, text: &str) {
        self.u32(text.len() as u32);
        self.out.extend_from_slice(text.as_bytes());
    }

    /// What a macro is given of `top`: what is in it when it is a group without delimiters,
    /// else it, as rust-analyzer's protocol hands a server the tree
    /// (`proc-macro-api/src/legacy_protocol/msg/flat.rs`).
    pub(crate) fn top(&mut self, top: &TopSubtree) {
        let view = top.view();
        let subtree = view.top_subtree();
        if subtree.delimiter.kind == DelimiterKind::Invisible {
            let contents = fragment_contents(&subtree, view.iter()).unwrap_or(view.iter());
            self.stream(contents);
        } else {
            self.u32(1);
            self.group(&subtree, view.iter());
        }
    }

    fn stream(&mut self, contents: TtIter<'_>) {
        self.u32(contents.clone().count() as u32);
        for element in contents {
            match element {
                TtElement::Leaf(leaf) => self.leaf(&leaf),
                TtElement::Subtree(subtree, contents) => self.group(&subtree, contents),
            }
        }
    }

    fn group(&mut self, subtree: &Subtree, contents: TtIter<'_>) {
        self.out.push(0);
        self.out.push(subtree.delimiter.kind as u8);
        let (open, close) = (subtree.delimiter.open, subtree.delimiter.close);
        self.span(open);
        self.span(close);
        self.span(entire(open, close));
        let contents = fragment_contents(subtree, contents.clone()).unwrap_or(contents);
        self.stream(contents);
    }

    /// Writes trees the host made, as [`Reader`] reads them.
    pub(crate) fn trees(&mut self, trees: &[Tree]) {
        self.u32(trees.len() as u32);
        for tree in trees {
            match tree {
                Tree::Leaf(leaf) => self.leaf(leaf),
                Tree::Group { kind, open, close, entire, trees } => {
                    self.out.push(0);
                    self.out.push(*kind as u8);
                    self.span(*open);
                    self.span(*close);
                    self.span(*entire);
                    self.trees(trees);
                }
            }
        }
    }

    pub(crate) fn leaf(&mut self, leaf: &Leaf) {
        match leaf {
            Leaf::Ident(ident) => {
                self.out.push(1);
                self.span(ident.span);
                self.out.push(ident.is_raw as u8);
                self.text(ident.sym.as_str());
            }
            Leaf::Punct(punct) => {
                self.out.push(2);
                self.span(punct.span);
                self.u32(punct.char as u32);
                self.out.push(u8::from(punct.spacing == Spacing::Joint));
            }
            Leaf::Literal(literal) => {
                self.out.push(3);
                self.span(literal.span);
                let (kind, hashes) = lit_kind_number(literal.kind);
                self.out.extend_from_slice(&[kind, hashes]);
                let (text, suffix) = literal.text_and_suffix();
                self.text(text);
                match suffix {
                    "" => self.u32(u32::MAX),
                    suffix => self.text(suffix),
                }
            }
        }
    }
}

/// What rustc calls the span of a whole group: rust-analyzer's protocol does not say, and
/// its server joins the delimiters' (`flat.rs`, `span_join`).
fn entire(open: Span, close: Span) -> Span {
    if open.anchor == close.anchor && open.ctx == close.ctx {
        Span { range: open.range.cover(close.range), ..close }
    } else {
        open
    }
}

/// What is in the parentheses of an expression a macro was passed and passed on, as
/// `flat.rs` has it: a proc macro is not to see them.
fn fragment_contents<'a>(subtree: &Subtree, mut contents: TtIter<'a>) -> Option<TtIter<'a>> {
    if subtree.delimiter.kind != DelimiterKind::Invisible {
        return None;
    }
    let Some(TtElement::Subtree(inner, inner_contents)) = contents.next() else {
        return None;
    };
    let wrapper = inner.delimiter.kind == DelimiterKind::Parenthesis
        && inner.delimiter.open == subtree.delimiter.open
        && inner.delimiter.close == subtree.delimiter.open;
    (wrapper && contents.next().is_none()).then_some(inner_contents)
}

/// Reads what the module wrote.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    spans: &'a Spans,
}

/// A stream the module wrote, as trees: what [`Reader`] makes, and what is turned into
/// rust-analyzer's.
pub(crate) enum Tree {
    Group { kind: DelimiterKind, open: Span, close: Span, entire: Span, trees: Vec<Tree> },
    Leaf(Leaf),
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8], spans: &'a Spans) -> Reader<'a> {
        Reader { bytes, at: 0, spans }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(len).filter(|it| *it <= self.bytes.len());
        let end = end.ok_or("the module wrote a stream that ends early")?;
        let it = &self.bytes[self.at..end];
        self.at = end;
        Ok(it)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn span(&mut self) -> Result<Span, String> {
        let number = self.u32()?;
        self.spans.get(number).ok_or_else(|| format!("the module named span {number}, unknown"))
    }

    fn text(&mut self) -> Result<&'a str, String> {
        let len = self.u32()? as usize;
        std::str::from_utf8(self.take(len)?).map_err(|it| it.to_string())
    }

    pub(crate) fn stream(&mut self) -> Result<Vec<Tree>, String> {
        let count = self.u32()? as usize;
        let mut trees = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            trees.push(self.tree()?);
        }
        Ok(trees)
    }

    fn tree(&mut self) -> Result<Tree, String> {
        Ok(match self.u8()? {
            0 => {
                let kind = match self.u8()? {
                    0 => DelimiterKind::Parenthesis,
                    1 => DelimiterKind::Brace,
                    2 => DelimiterKind::Bracket,
                    _ => DelimiterKind::Invisible,
                };
                let (open, close, entire) = (self.span()?, self.span()?, self.span()?);
                Tree::Group { kind, open, close, entire, trees: self.stream()? }
            }
            1 => {
                let span = self.span()?;
                let is_raw = if self.u8()? == 1 { IdentIsRaw::Yes } else { IdentIsRaw::No };
                Tree::Leaf(Leaf::Ident(tt::Ident {
                    sym: Symbol::intern(self.text()?),
                    span,
                    is_raw,
                }))
            }
            2 => {
                let span = self.span()?;
                let char = char::from_u32(self.u32()?).ok_or("the module wrote no character")?;
                let spacing = if self.u8()? != 0 { Spacing::Joint } else { Spacing::Alone };
                Tree::Leaf(Leaf::Punct(tt::Punct { char, spacing, span }))
            }
            3 => {
                let span = self.span()?;
                let (kind, hashes) = (self.u8()?, self.u8()?);
                let text = self.text()?;
                let suffix = match u32::from_le_bytes(
                    self.bytes
                        .get(self.at..self.at + 4)
                        .ok_or("a literal ends early")?
                        .try_into()
                        .unwrap(),
                ) {
                    u32::MAX => {
                        self.at += 4;
                        ""
                    }
                    _ => self.text()?,
                };
                Tree::Leaf(Leaf::Literal(tt::Literal::new(
                    text,
                    span,
                    lit_kind(kind, hashes),
                    suffix,
                )))
            }
            other => return Err(format!("the module wrote a tree of kind {other}")),
        })
    }
}

/// rust-analyzer's tree of a macro's output: its one group, or the stream in a group without
/// delimiters at `call_site`, as its server writes it.
pub(crate) fn top_subtree(trees: Vec<Tree>, call_site: Span) -> TopSubtree {
    let mut flat = Vec::new();
    match <[Tree; 1]>::try_from(trees) {
        Ok([group @ Tree::Group { .. }]) => flatten(group, &mut flat),
        Ok([leaf]) => flatten(top_of(vec![leaf], call_site), &mut flat),
        Err(trees) => flatten(top_of(trees, call_site), &mut flat),
    }
    TopSubtree::from_serialized(flat)
}

fn top_of(trees: Vec<Tree>, call_site: Span) -> Tree {
    let (open, close, entire) = (call_site, call_site, call_site);
    Tree::Group { kind: DelimiterKind::Invisible, open, close, entire, trees }
}

fn flatten(tree: Tree, flat: &mut Vec<TokenTree>) {
    match tree {
        Tree::Leaf(leaf) => flat.push(TokenTree::Leaf(leaf)),
        Tree::Group { kind, open, close, entire: _, trees } => {
            let at = flat.len();
            flat.push(TokenTree::Subtree(Subtree {
                delimiter: tt::Delimiter { open, close, kind },
                len: 0,
            }));
            for tree in trees {
                flatten(tree, flat);
            }
            let len = (flat.len() - at - 1) as u32;
            if let TokenTree::Subtree(subtree) = &mut flat[at] {
                subtree.len = len;
            }
        }
    }
}
