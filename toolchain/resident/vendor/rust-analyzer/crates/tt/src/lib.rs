//! `tt` crate defines a `TokenTree` data structure: this is the interface (both
//! input and output) of macros.
//!
//! The `TokenTree` is semantically a tree, but for performance reasons it is stored as a flat structure.

#![cfg_attr(feature = "in-rust-tree", feature(rustc_private))]

#[cfg(feature = "in-rust-tree")]
extern crate rustc_driver as _;

#[cfg(not(feature = "in-rust-tree"))]
extern crate ra_ap_rustc_lexer as rustc_lexer;
#[cfg(feature = "in-rust-tree")]
extern crate rustc_lexer;

pub mod buffer;
pub mod iter;
mod storage;

use std::fmt;

use arrayvec::ArrayString;
use buffer::Cursor;
use intern::Symbol;
use stdx::{impl_from, itertools::Itertools as _};

pub use span::Span;
pub use text_size::{TextRange, TextSize};

use crate::storage::TokenTreesSlice;

pub use self::iter::{TtElement, TtIter};
pub use self::storage::{TopSubtree, TopSubtreeBuilder};

pub const MAX_GLUED_PUNCT_LEN: usize = 3;

#[derive(Clone, PartialEq, Debug)]
pub struct Lit {
    pub kind: LitKind,
    pub symbol: Symbol,
    pub suffix: Option<Symbol>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
// The discriminants are important for `storage.rs` decoding.
pub enum IdentIsRaw {
    No = 0,
    Yes = 1,
}
impl IdentIsRaw {
    pub fn yes(self) -> bool {
        matches!(self, IdentIsRaw::Yes)
    }
    pub fn no(&self) -> bool {
        matches!(self, IdentIsRaw::No)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            IdentIsRaw::No => "",
            IdentIsRaw::Yes => "r#",
        }
    }
    pub fn split_from_symbol(sym: &str) -> (Self, &str) {
        if let Some(sym) = sym.strip_prefix("r#") {
            (IdentIsRaw::Yes, sym)
        } else {
            (IdentIsRaw::No, sym)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum LitKind {
    Byte,
    Char,
    Integer, // e.g. `1`, `1u8`, `1f32`
    Float,   // e.g. `1.`, `1.0`, `1e3f32`
    Str,
    StrRaw(u8), // raw string delimited by `n` hash symbols
    ByteStr,
    ByteStrRaw(u8), // raw byte string delimited by `n` hash symbols
    CStr,
    CStrRaw(u8),
    Err(()),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenTree {
    Leaf(Leaf),
    Subtree(Subtree),
}
impl_from!(Leaf, Subtree for TokenTree);
impl TokenTree {
    pub fn first_span(&self) -> Span {
        match self {
            TokenTree::Leaf(l) => *l.span(),
            TokenTree::Subtree(s) => s.delimiter.open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Leaf {
    Literal(Literal),
    Punct(Punct),
    Ident(Ident),
}

impl Leaf {
    pub fn span(&self) -> &Span {
        match self {
            Leaf::Literal(it) => &it.span,
            Leaf::Punct(it) => &it.span,
            Leaf::Ident(it) => &it.span,
        }
    }

    fn symbol(&self) -> Option<&Symbol> {
        match self {
            Leaf::Literal(Literal { text_and_suffix: symbol, .. })
            | Leaf::Ident(Ident { sym: symbol, .. }) => Some(symbol),
            Leaf::Punct(_) => None,
        }
    }
}
impl_from!(Literal, Punct, Ident for Leaf);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Subtree {
    pub delimiter: Delimiter,
    /// Number of following token trees that belong to this subtree, excluding this subtree.
    pub len: u32,
}

impl Subtree {
    pub fn usize_len(&self) -> usize {
        self.len as usize
    }
}

#[derive(Clone, Copy)]
pub struct TokenTreesView<'a> {
    slice: TokenTreesSlice<'a>,
    len: usize,
}

impl<'a> TokenTreesView<'a> {
    #[inline]
    pub fn empty() -> Self {
        Self { slice: TokenTreesSlice::empty(), len: 0 }
    }

    pub fn iter(&self) -> TtIter<'a> {
        TtIter::new(*self)
    }

    pub fn cursor(&self) -> Cursor<'a> {
        Cursor::new(*self)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn try_into_subtree(self) -> Option<SubtreeView<'a>> {
        let is_subtree = self.iter_flat_tokens().next().is_some_and(
            |it| matches!(it, TokenTree::Subtree(subtree) if subtree.usize_len() == self.len - 1),
        );
        if is_subtree { Some(SubtreeView(self)) } else { None }
    }

    pub fn strip_invisible(self) -> TokenTreesView<'a> {
        self.try_into_subtree().map(|subtree| subtree.strip_invisible()).unwrap_or(self)
    }

    pub fn split(
        self,
        mut split_fn: impl FnMut(TtElement<'a>) -> bool,
    ) -> impl Iterator<Item = TokenTreesView<'a>> {
        let mut subtree_iter = self.iter();
        let mut need_to_yield_even_if_empty = true;

        std::iter::from_fn(move || {
            if subtree_iter.is_empty() && !need_to_yield_even_if_empty {
                return None;
            };

            need_to_yield_even_if_empty = false;
            let savepoint = subtree_iter.savepoint();
            let mut result = subtree_iter.from_savepoint(savepoint);
            while let Some(tt) = subtree_iter.next() {
                if split_fn(tt) {
                    need_to_yield_even_if_empty = true;
                    break;
                }
                result = subtree_iter.from_savepoint(savepoint);
            }
            Some(result)
        })
    }

    pub fn first_span(&self) -> Option<Span> {
        self.iter_flat_tokens().next().map(|it| it.first_span())
    }

    /// Note: this is quite expensive, this needs to decode the whole view,
    /// although it "tricks" by skipping subtrees (since we know their byte length).
    pub fn last_span(&self) -> Option<Span> {
        let mut iter = self.iter();
        loop {
            match iter.last()? {
                TtElement::Leaf(leaf) => return Some(*leaf.span()),
                TtElement::Subtree(subtree, tt_iter) => {
                    if subtree.len == 0 {
                        return Some(subtree.delimiter.close);
                    } else {
                        iter = tt_iter;
                    }
                }
            }
        }
    }

    pub fn iter_flat_tokens(&self) -> impl Iterator<Item = TokenTree> + use<'a> {
        self.slice.iter().take(self.len)
    }
}

impl fmt::Debug for TokenTreesView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut iter = self.iter();
        while let Some(tt) = iter.next() {
            print_debug_token(f, 0, tt)?;
            if !iter.is_empty() {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for TokenTreesView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        return token_trees_display(f, self.iter());

        fn subtree_display(
            subtree: &Subtree,
            f: &mut fmt::Formatter<'_>,
            iter: TtIter<'_>,
        ) -> fmt::Result {
            let (l, r) = match subtree.delimiter.kind {
                DelimiterKind::Parenthesis => ("(", ")"),
                DelimiterKind::Brace => ("{", "}"),
                DelimiterKind::Bracket => ("[", "]"),
                DelimiterKind::Invisible => ("", ""),
            };
            f.write_str(l)?;
            token_trees_display(f, iter)?;
            f.write_str(r)?;
            Ok(())
        }

        fn token_trees_display(f: &mut fmt::Formatter<'_>, iter: TtIter<'_>) -> fmt::Result {
            let mut needs_space = false;
            for child in iter {
                if needs_space {
                    f.write_str(" ")?;
                }
                needs_space = true;

                match child {
                    TtElement::Leaf(Leaf::Punct(p)) => {
                        needs_space = p.spacing == Spacing::Alone;
                        fmt::Display::fmt(&p, f)?;
                    }
                    TtElement::Leaf(leaf) => fmt::Display::fmt(&leaf, f)?,
                    TtElement::Subtree(subtree, subtree_iter) => {
                        subtree_display(&subtree, f, subtree_iter)?
                    }
                }
            }
            Ok(())
        }
    }
}

#[derive(Clone, Copy)]
// Invariant: always starts with `Subtree` that covers the entire thing.
pub struct SubtreeView<'a>(TokenTreesView<'a>);

impl<'a> SubtreeView<'a> {
    pub fn as_token_trees(self) -> TokenTreesView<'a> {
        self.0
    }

    pub fn iter(&self) -> TtIter<'a> {
        self.token_trees().iter()
    }

    pub fn top_subtree(&self) -> Subtree {
        let Some(TokenTree::Subtree(subtree)) = self.0.iter_flat_tokens().next() else {
            unreachable!("the first token tree is always the top subtree");
        };
        subtree
    }

    pub fn strip_invisible(&self) -> TokenTreesView<'a> {
        if self.top_subtree().delimiter.kind == DelimiterKind::Invisible {
            self.token_trees()
        } else {
            self.0
        }
    }

    pub fn token_trees(&self) -> TokenTreesView<'a> {
        let mut result = self.0;
        result.slice.advance();
        result.len -= 1;
        result
    }
}

impl fmt::Debug for SubtreeView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for SubtreeView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DelimSpan {
    pub open: Span,
    pub close: Span,
}

impl DelimSpan {
    pub fn from_single(sp: Span) -> Self {
        DelimSpan { open: sp, close: sp }
    }

    pub fn from_pair(open: Span, close: Span) -> Self {
        DelimSpan { open, close }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Delimiter {
    pub open: Span,
    pub close: Span,
    pub kind: DelimiterKind,
}

impl Delimiter {
    pub const fn invisible_spanned(span: Span) -> Self {
        Delimiter { open: span, close: span, kind: DelimiterKind::Invisible }
    }

    pub const fn invisible_delim_spanned(span: DelimSpan) -> Self {
        Delimiter { open: span.open, close: span.close, kind: DelimiterKind::Invisible }
    }

    pub fn delim_span(&self) -> DelimSpan {
        DelimSpan { open: self.open, close: self.close }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
// The discriminants are important for decoding for `storage.rs`.
pub enum DelimiterKind {
    Parenthesis = 0,
    Brace = 1,
    Bracket = 2,
    Invisible = 3,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Literal {
    /// Escaped, text then suffix concatenated.
    pub text_and_suffix: Symbol,
    pub span: Span,
    pub kind: LitKind,
    pub suffix_len: u8,
}

impl Literal {
    #[inline]
    pub fn text_and_suffix(&self) -> (&str, &str) {
        let text_and_suffix = self.text_and_suffix.as_str();
        text_and_suffix.split_at(text_and_suffix.len() - usize::from(self.suffix_len))
    }

    #[inline]
    pub fn text(&self) -> &str {
        self.text_and_suffix().0
    }

    #[inline]
    pub fn suffix(&self) -> &str {
        self.text_and_suffix().1
    }

    pub fn new(text: &str, span: Span, kind: LitKind, suffix: &str) -> Self {
        const MAX_INLINE_CAPACITY: usize = 30;
        let text_and_suffix = if suffix.is_empty() {
            Symbol::intern(text)
        } else if (text.len() + suffix.len()) < MAX_INLINE_CAPACITY {
            let mut text_and_suffix = ArrayString::<MAX_INLINE_CAPACITY>::new();
            text_and_suffix.push_str(text);
            text_and_suffix.push_str(suffix);
            Symbol::intern(&text_and_suffix)
        } else {
            let mut text_and_suffix = String::with_capacity(text.len() + suffix.len());
            text_and_suffix.push_str(text);
            text_and_suffix.push_str(suffix);
            Symbol::intern(&text_and_suffix)
        };

        Self { text_and_suffix, span, kind, suffix_len: suffix.len().try_into().unwrap() }
    }

    #[inline]
    pub fn new_no_suffix(text: &str, span: Span, kind: LitKind) -> Self {
        Self { text_and_suffix: Symbol::intern(text), span, kind, suffix_len: 0 }
    }
}

pub fn token_to_literal(text: &str, span: Span) -> Literal {
    use rustc_lexer::LiteralKind;

    let token = rustc_lexer::tokenize(text, rustc_lexer::FrontmatterAllowed::No).next_tuple();
    let Some((rustc_lexer::Token {
        kind: rustc_lexer::TokenKind::Literal { kind, suffix_start },
        ..
    },)) = token
    else {
        return Literal::new_no_suffix(text, span, LitKind::Err(()));
    };

    let (kind, start_offset, end_offset) = match kind {
        LiteralKind::Int { .. } => (LitKind::Integer, 0, 0),
        LiteralKind::Float { .. } => (LitKind::Float, 0, 0),
        LiteralKind::Char { terminated } => (LitKind::Char, 1, terminated as usize),
        LiteralKind::Byte { terminated } => (LitKind::Byte, 2, terminated as usize),
        LiteralKind::Str { terminated } => (LitKind::Str, 1, terminated as usize),
        LiteralKind::ByteStr { terminated } => (LitKind::ByteStr, 2, terminated as usize),
        LiteralKind::CStr { terminated } => (LitKind::CStr, 2, terminated as usize),
        LiteralKind::RawStr { n_hashes } => (
            LitKind::StrRaw(n_hashes.unwrap_or_default()),
            2 + n_hashes.unwrap_or_default() as usize,
            1 + n_hashes.unwrap_or_default() as usize,
        ),
        LiteralKind::RawByteStr { n_hashes } => (
            LitKind::ByteStrRaw(n_hashes.unwrap_or_default()),
            3 + n_hashes.unwrap_or_default() as usize,
            1 + n_hashes.unwrap_or_default() as usize,
        ),
        LiteralKind::RawCStr { n_hashes } => (
            LitKind::CStrRaw(n_hashes.unwrap_or_default()),
            3 + n_hashes.unwrap_or_default() as usize,
            1 + n_hashes.unwrap_or_default() as usize,
        ),
    };

    let (lit, suffix) = text.split_at(suffix_start as usize);
    let lit = &lit[start_offset..lit.len() - end_offset];
    let suffix = match suffix {
        "" | "_" => "",
        // ill-suffixed literals
        _ if !matches!(kind, LitKind::Integer | LitKind::Float | LitKind::Err(_)) => {
            return Literal::new_no_suffix(text, span, LitKind::Err(()));
        }
        suffix => suffix,
    };

    Literal::new(lit, span, kind, suffix)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Punct {
    pub char: char,
    pub spacing: Spacing,
    pub span: Span,
}

/// Indicates whether a token can join with the following token to form a
/// compound token. Used for conversions to `proc_macro::Spacing`. Also used to
/// guide pretty-printing, which is where the `JointHidden` value (which isn't
/// part of `proc_macro::Spacing`) comes in useful.
// The discriminants are important for decoding for `storage.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Spacing {
    /// The token cannot join with the following token to form a compound
    /// token.
    ///
    /// In token streams parsed from source code, the compiler will use `Alone`
    /// for any token immediately followed by whitespace, a non-doc comment, or
    /// EOF.
    ///
    /// When constructing token streams within the compiler, use this for each
    /// token that (a) should be pretty-printed with a space after it, or (b)
    /// is the last token in the stream. (In the latter case the choice of
    /// spacing doesn't matter because it is never used for the last token. We
    /// arbitrarily use `Alone`.)
    ///
    /// Converts to `proc_macro::Spacing::Alone`, and
    /// `proc_macro::Spacing::Alone` converts back to this.
    Alone = 0,

    /// The token can join with the following token to form a compound token.
    ///
    /// In token streams parsed from source code, the compiler will use `Joint`
    /// for any token immediately followed by punctuation (as determined by
    /// `Token::is_punct`).
    ///
    /// When constructing token streams within the compiler, use this for each
    /// token that (a) should be pretty-printed without a space after it, and
    /// (b) is followed by a punctuation token.
    ///
    /// Converts to `proc_macro::Spacing::Joint`, and
    /// `proc_macro::Spacing::Joint` converts back to this.
    Joint = 1,

    /// The token can join with the following token to form a compound token,
    /// but this will not be visible at the proc macro level. (This is what the
    /// `Hidden` means; see below.)
    ///
    /// In token streams parsed from source code, the compiler will use
    /// `JointHidden` for any token immediately followed by anything not
    /// covered by the `Alone` and `Joint` cases: an identifier, lifetime,
    /// literal, delimiter, doc comment.
    ///
    /// When constructing token streams, use this for each token that (a)
    /// should be pretty-printed without a space after it, and (b) is followed
    /// by a non-punctuation token.
    ///
    /// Converts to `proc_macro::Spacing::Alone`, but
    /// `proc_macro::Spacing::Alone` converts back to `token::Spacing::Alone`.
    /// Because of that, pretty-printing of `TokenStream`s produced by proc
    /// macros is unavoidably uglier (with more whitespace between tokens) than
    /// pretty-printing of `TokenStream`'s produced by other means (i.e. parsed
    /// source code, internally constructed token streams, and token streams
    /// produced by declarative macros).
    JointHidden = 2,
}

/// Identifier or keyword.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub sym: Symbol,
    pub span: Span,
    pub is_raw: IdentIsRaw,
}

impl Ident {
    pub fn new(text: &str, span: Span) -> Self {
        // let raw_stripped = IdentIsRaw::split_from_symbol(text.as_ref());
        let (is_raw, text) = IdentIsRaw::split_from_symbol(text);
        Ident { sym: Symbol::intern(text), span, is_raw }
    }
}

fn print_debug_subtree(
    f: &mut fmt::Formatter<'_>,
    subtree: &Subtree,
    level: usize,
    iter: TtIter<'_>,
) -> fmt::Result {
    let align = "  ".repeat(level);

    let Delimiter { kind, open, close } = &subtree.delimiter;
    let delim = match kind {
        DelimiterKind::Invisible => "$$",
        DelimiterKind::Parenthesis => "()",
        DelimiterKind::Brace => "{}",
        DelimiterKind::Bracket => "[]",
    };

    write!(f, "{align}SUBTREE {delim} ",)?;
    write!(f, "{open:#?}")?;
    write!(f, " ")?;
    write!(f, "{close:#?}")?;
    for child in iter {
        writeln!(f)?;
        print_debug_token(f, level + 1, child)?;
    }

    Ok(())
}

fn print_debug_token(f: &mut fmt::Formatter<'_>, level: usize, tt: TtElement<'_>) -> fmt::Result {
    let align = "  ".repeat(level);

    match tt {
        TtElement::Leaf(leaf) => match leaf {
            Leaf::Literal(lit) => {
                let (text, suffix) = lit.text_and_suffix();
                write!(f, "{}LITERAL {:?} {}{} {:#?}", align, lit.kind, text, suffix, lit.span)?;
            }
            Leaf::Punct(punct) => {
                write!(
                    f,
                    "{}PUNCH   {} [{}] {:#?}",
                    align,
                    punct.char,
                    if punct.spacing == Spacing::Alone { "alone" } else { "joint" },
                    punct.span
                )?;
            }
            Leaf::Ident(ident) => {
                write!(
                    f,
                    "{}IDENT   {}{} {:#?}",
                    align,
                    ident.is_raw.as_str(),
                    ident.sym,
                    ident.span
                )?;
            }
        },
        TtElement::Subtree(subtree, subtree_iter) => {
            print_debug_subtree(f, &subtree, level, subtree_iter)?;
        }
    }

    Ok(())
}

impl fmt::Debug for TopSubtree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.view(), f)
    }
}

impl fmt::Display for TopSubtree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.view(), f)
    }
}

impl fmt::Display for Leaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Leaf::Ident(it) => fmt::Display::fmt(it, f),
            Leaf::Literal(it) => fmt::Display::fmt(it, f),
            Leaf::Punct(it) => fmt::Display::fmt(it, f),
        }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.is_raw.as_str(), f)?;
        fmt::Display::fmt(&self.sym, f)
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (text, suffix) = self.text_and_suffix();
        match self.kind {
            LitKind::Byte => write!(f, "b'{}'", text),
            LitKind::Char => write!(f, "'{}'", text),
            LitKind::Integer | LitKind::Float | LitKind::Err(_) => write!(f, "{}", text),
            LitKind::Str => write!(f, "\"{}\"", text),
            LitKind::ByteStr => write!(f, "b\"{}\"", text),
            LitKind::CStr => write!(f, "c\"{}\"", text),
            LitKind::StrRaw(num_of_hashes) => {
                let num_of_hashes = num_of_hashes as usize;
                write!(f, r#"r{0:#<num_of_hashes$}"{text}"{0:#<num_of_hashes$}"#, "", text = text)
            }
            LitKind::ByteStrRaw(num_of_hashes) => {
                let num_of_hashes = num_of_hashes as usize;
                write!(f, r#"br{0:#<num_of_hashes$}"{text}"{0:#<num_of_hashes$}"#, "", text = text)
            }
            LitKind::CStrRaw(num_of_hashes) => {
                let num_of_hashes = num_of_hashes as usize;
                write!(f, r#"cr{0:#<num_of_hashes$}"{text}"{0:#<num_of_hashes$}"#, "", text = text)
            }
        }?;
        write!(f, "{suffix}")?;
        Ok(())
    }
}

impl fmt::Display for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.char, f)
    }
}

impl Subtree {
    /// Count the number of tokens recursively
    pub fn count(&self) -> usize {
        self.usize_len()
    }
}

pub fn pretty(tkns: TokenTreesView<'_>) -> String {
    return pretty_impl(tkns.iter());

    fn tokentree_to_text(tkn: TtElement<'_>) -> String {
        match tkn {
            TtElement::Leaf(leaf) => {
                format!("{}", leaf)
            }
            TtElement::Subtree(Subtree { delimiter, .. }, subtree_content) => {
                let content = pretty_impl(subtree_content);
                let (open, close) = match delimiter.kind {
                    DelimiterKind::Brace => ("{", "}"),
                    DelimiterKind::Bracket => ("[", "]"),
                    DelimiterKind::Parenthesis => ("(", ")"),
                    DelimiterKind::Invisible => ("", ""),
                };
                format!("{open}{content}{close}")
            }
        }
    }

    fn pretty_impl(tkns: TtIter<'_>) -> String {
        let mut last = String::new();
        let mut last_to_joint = true;

        for tkn in tkns {
            last =
                [last, tokentree_to_text(tkn.clone())].join(if last_to_joint { "" } else { " " });
            last_to_joint = false;
            if let TtElement::Leaf(Leaf::Punct(Punct { spacing, .. })) = tkn
                && spacing == Spacing::Joint
            {
                last_to_joint = true;
            }
        }
        last
    }
}

/// The text `stringify!` gives for tokens. This follows rustc's `print_tts`: a space goes after
/// a token that had one in the source, unless it is one of the places rustc never puts one.
/// A token written in the body of a macro always counts as having one.
///
/// `source_joined` says whether the source has another token right after the one at a span, or
/// `None` if the token is not from the source.
///
/// One difference from rustc is left: what an `ident` or `lifetime` fragment matched counts as
/// followed by a space there, and here it is treated like a `tt` fragment.
///
/// `through_proc_macro`: the tokens of the root context came out of a proc macro, and so have
/// the spacing rustc's proc macro server gives them back: `Joint` for a punctuation the macro
/// said is joint, `Alone` for everything else, whatever the source they are from had. That is
/// what rustc does with tokens the macro took apart and made again, as syn and quote do; a
/// stream it hands on whole keeps the spacing of its source there, and is spaced here.
pub fn stringify(
    tkns: TokenTreesView<'_>,
    source_joined: &dyn Fn(Span, &str) -> Option<bool>,
    through_proc_macro: bool,
) -> String {
    let mut text = String::new();
    let given = |span, text: &str| match through_proc_macro {
        true => Some(false),
        false => source_joined(span, text),
    };
    write_stringified(tkns.iter(), &given, through_proc_macro, &mut text);
    return text;

    /// The operators that are one token to rustc and several here.
    const OPERATORS: &[&str] = &[
        "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "..", "...", "..=", "<<", ">>",
        "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "^=", "&=", "|=", "<-",
    ];

    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "for",
        "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "ref", "return", "self",
        "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
        "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override",
        "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
    ];

    fn start(tt: &TtElement<'_>) -> Span {
        match tt {
            TtElement::Leaf(leaf) => *leaf.span(),
            TtElement::Subtree(subtree, _) => subtree.delimiter.open,
        }
    }

    fn end(tt: &TtElement<'_>) -> Span {
        match tt {
            TtElement::Leaf(leaf) => *leaf.span(),
            TtElement::Subtree(subtree, _) => subtree.delimiter.close,
        }
    }

    fn punct(tt: &TtElement<'_>) -> Option<char> {
        match tt {
            TtElement::Leaf(Leaf::Punct(punct)) => Some(punct.char),
            _ => None,
        }
    }

    /// Whether rustc's token for `prev` has a spacing other than `Alone`. `operator` is the
    /// part of an operator that ends with `prev`.
    fn joined(
        prev: &TtElement<'_>,
        next: &TtElement<'_>,
        operator: &str,
        source_joined: &dyn Fn(Span, &str) -> Option<bool>,
        through_proc_macro: bool,
    ) -> bool {
        // A lifetime is one token to rustc.
        if punct(prev) == Some('\'') && matches!(next, TtElement::Leaf(Leaf::Ident(_))) {
            return true;
        }
        let in_source = match prev {
            TtElement::Leaf(Leaf::Punct(it)) if !it.span.ctx.is_root() => {
                // A token of a macro's body is `Alone` to rustc.
                return matches!(it.spacing, Spacing::Joint)
                    && punct(next).is_some_and(|next| {
                        OPERATORS.iter().any(|it| {
                            it.strip_prefix(operator).is_some_and(|rest| rest.starts_with(next))
                        })
                    });
            }
            TtElement::Leaf(leaf) if !leaf.span().ctx.is_root() => return false,
            // A lifetime is one token to rustc, so what comes before it is no part of an
            // operator with it.
            TtElement::Leaf(Leaf::Punct(it)) if through_proc_macro => {
                return matches!(it.spacing, Spacing::Joint) && punct(next) != Some('\'');
            }
            TtElement::Leaf(leaf) => source_joined(*leaf.span(), &leaf.to_string()),
            TtElement::Subtree(subtree, _) => {
                let close = match subtree.delimiter.kind {
                    // What a macro puts around an expression it was passed.
                    DelimiterKind::Parenthesis if expression_parens(subtree) => return false,
                    DelimiterKind::Parenthesis => ")",
                    DelimiterKind::Brace => "}",
                    // rustc does the same, for the `hex-literal` crate.
                    DelimiterKind::Bracket if !subtree.delimiter.close.ctx.is_root() => {
                        return false;
                    }
                    DelimiterKind::Bracket => "]",
                    DelimiterKind::Invisible => return false,
                };
                source_joined(subtree.delimiter.close, close)
            }
        };
        if let Some(it) = in_source {
            return it;
        }
        // The spacing of a token made by a macro says what follows it now.
        matches!(prev, TtElement::Leaf(Leaf::Punct(Punct { spacing: Spacing::Joint, .. })))
            || adjacent(end(prev), start(next))
    }

    /// Whether `subtree` is the parentheses `macro_rules!` puts around an expression it was
    /// passed: both of the span of the fragment in the macro's body. A group a proc macro gave
    /// the span of a group of the source it was handed, with `Group::set_span`, has one span
    /// for both too, of the source's context, and its parentheses are the source's.
    fn expression_parens(subtree: &Subtree) -> bool {
        subtree.delimiter.open == subtree.delimiter.close && !subtree.delimiter.open.ctx.is_root()
    }

    /// rustc's `idents_would_merge`.
    fn idents_would_merge(prev: &TtElement<'_>, next: &TtElement<'_>) -> bool {
        matches!((prev, next), (TtElement::Leaf(Leaf::Ident(_)), TtElement::Leaf(Leaf::Ident(_))))
    }

    /// Whether `next` starts where `prev` ends.
    fn adjacent(prev: Span, next: Span) -> bool {
        prev.anchor == next.anchor
            && prev.range.end() == next.range.start()
            && prev.range != next.range
    }

    /// rustc's `space_between`.
    fn space_between(prev: &TtElement<'_>, next: &TtElement<'_>) -> bool {
        match (prev, next) {
            (_, _) if punct(prev) == Some('.') && punct(next).is_none() => false,
            (_, TtElement::Leaf(Leaf::Ident(_))) if punct(prev) == Some('$') => false,
            (_, _) if punct(prev).is_none() && matches!(punct(next), Some(',' | ';' | '.')) => {
                false
            }
            (TtElement::Leaf(Leaf::Ident(ident)), TtElement::Subtree(subtree, _))
                if subtree.delimiter.kind == DelimiterKind::Parenthesis =>
            {
                ident.is_raw.no() && KEYWORDS.contains(&ident.sym.as_str())
            }
            (_, TtElement::Subtree(subtree, _)) if punct(prev) == Some('#') => {
                subtree.delimiter.kind != DelimiterKind::Bracket
            }
            _ => true,
        }
    }

    fn write_stringified(
        tkns: TtIter<'_>,
        source_joined: &dyn Fn(Span, &str) -> Option<bool>,
        through_proc_macro: bool,
        text: &mut String,
    ) {
        let mut tkns = tkns.peekable();
        let mut operator = String::new();
        while let Some(tkn) = tkns.next() {
            match &tkn {
                TtElement::Leaf(Leaf::Punct(punct)) => {
                    operator.push(punct.char);
                    text.push(punct.char);
                }
                TtElement::Leaf(leaf) => text.push_str(&leaf.to_string()),
                TtElement::Subtree(subtree, content) => {
                    let (open, close) = match subtree.delimiter.kind {
                        // What a macro puts around an expression it was passed, which is not
                        // in the source.
                        DelimiterKind::Parenthesis if expression_parens(subtree) => ("", ""),
                        // rustc puts a space inside a brace where the source had one after
                        // the opening one.
                        DelimiterKind::Brace => match content.clone().next() {
                            Some(first)
                                if !source_joined(subtree.delimiter.open, "{").unwrap_or(
                                    subtree.delimiter.open != subtree.delimiter.close
                                        && adjacent(subtree.delimiter.open, start(&first)),
                                ) =>
                            {
                                ("{ ", " }")
                            }
                            _ => ("{", "}"),
                        },
                        DelimiterKind::Bracket => ("[", "]"),
                        DelimiterKind::Parenthesis => ("(", ")"),
                        DelimiterKind::Invisible => ("", ""),
                    };
                    text.push_str(open);
                    write_stringified(content.clone(), source_joined, through_proc_macro, text);
                    text.push_str(close);
                }
            }
            let Some(next) = tkns.peek() else { break };
            let is_joined = joined(&tkn, next, &operator, source_joined, through_proc_macro);
            if !is_joined || punct(&tkn).is_none() {
                operator.clear();
            }
            if if is_joined { idents_would_merge(&tkn, next) } else { space_between(&tkn, next) } {
                text.push(' ');
            }
        }
    }
}

#[derive(Debug)]
pub enum TransformTtAction<'a> {
    Keep,
    ReplaceWith(TokenTreesView<'a>),
}

impl TransformTtAction<'_> {
    #[inline]
    pub fn remove() -> Self {
        Self::ReplaceWith(TokenTreesView::empty())
    }
}

/// This function takes a token tree, and calls `callback` with each token tree in it.
/// Then it does what the callback says: keeps the tt or replaces it with a (possibly empty)
/// tts view.
pub fn transform_tt<'b>(
    tt: &mut TopSubtree,
    mut callback: impl FnMut(&TokenTree) -> TransformTtAction<'b>,
) {
    let mut tt_vec = tt.as_token_trees().iter_flat_tokens().collect::<Vec<_>>();

    // We need to keep a stack of the currently open subtrees, because we need to update
    // them if we change the number of items in them.
    let mut subtrees_stack = Vec::new();
    let mut i = 0;
    while i < tt_vec.len() {
        'pop_finished_subtrees: while let Some(&subtree_idx) = subtrees_stack.last() {
            let TokenTree::Subtree(subtree) = &tt_vec[subtree_idx] else {
                unreachable!("non-subtree on subtrees stack");
            };
            if i >= subtree_idx + 1 + subtree.usize_len() {
                subtrees_stack.pop();
            } else {
                break 'pop_finished_subtrees;
            }
        }

        let current = &tt_vec[i];
        let action = callback(current);
        match action {
            TransformTtAction::Keep => {
                // This cannot be shared with the replaced case, because then we may push the same subtree
                // twice, and will update it twice which will lead to errors.
                if let TokenTree::Subtree(_) = current {
                    subtrees_stack.push(i);
                }

                i += 1;
            }
            TransformTtAction::ReplaceWith(replacement) => {
                let old_len = 1 + match current {
                    TokenTree::Leaf(_) => 0,
                    TokenTree::Subtree(subtree) => subtree.usize_len(),
                };
                let len_diff = replacement.len() as i64 - old_len as i64;
                tt_vec.splice(i..i + old_len, replacement.iter_flat_tokens());
                // Skip the newly inserted replacement, we don't want to visit it.
                i += replacement.len();

                for &subtree_idx in &subtrees_stack {
                    let TokenTree::Subtree(subtree) = &mut tt_vec[subtree_idx] else {
                        unreachable!("non-subtree on subtrees stack");
                    };
                    subtree.len = (i64::from(subtree.len) + len_diff).try_into().unwrap();
                }
            }
        }
    }

    *tt = TopSubtree::from_serialized(tt_vec);
}
