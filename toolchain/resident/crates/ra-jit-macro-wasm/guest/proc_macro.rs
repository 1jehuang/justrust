//! The `proc_macro` crate of a proc macro ra-jit compiles to WebAssembly, as dtolnay's watt
//! runs one: ra-jit puts this text in place of the standard library's `library/proc_macro`
//! root when it compiles a proc-macro crate for `wasm32-wasip1`
//! (`crates/ra-jit-macro-wasm/src/lib.rs` says how the module is run). `proc-macro2`, `syn`
//! and `quote` compile against it unchanged.
//!
//! Token trees live in the module's memory: a stream is a vector of them, an identifier or a
//! literal holds its text. A span is a number the host gave out, and what can only be known of
//! it there (joining two, its source text, its file) is asked of the host by an import. The
//! input of a macro arrives, and its output leaves, encoded as [`wire`] says. Displaying a
//! stream follows the proc-macro server ra-jit expands native macros with
//! (`vendor/proc-macro-srv/crates/proc-macro-srv/src/token_stream.rs`), so a macro that
//! compares `to_string()`s sees the same text either way.

#![allow(clippy::all, unused, missing_docs)]

use std::{
    cell::RefCell,
    error, fmt,
    ffi::CStr,
    iter::FromIterator,
    ops::{Bound, Range, RangeBounds},
    path::PathBuf,
    rc::Rc,
    str::FromStr,
};

mod sys {
    #[link(wasm_import_module = "blaze_proc_macro")]
    unsafe extern "C" {
        pub fn def_site() -> u32;
        pub fn call_site() -> u32;
        pub fn mixed_site() -> u32;
        pub fn span_join(a: u32, b: u32) -> u32;
        pub fn span_resolved_at(a: u32, b: u32) -> u32;
        pub fn span_located_at(a: u32, b: u32) -> u32;
        pub fn span_parent(a: u32) -> u32;
        pub fn span_source(a: u32) -> u32;
        pub fn span_start(a: u32) -> u32;
        pub fn span_end(a: u32) -> u32;
        pub fn span_subspan(a: u32, start: u64, end: u64) -> u32;
        pub fn span_byte_range(a: u32, out: *mut u32);
        pub fn span_line(a: u32) -> u32;
        pub fn span_column(a: u32) -> u32;
        /// The length of the text the host put in its buffer for [`take`], or -1 for none.
        pub fn span_debug(a: u32) -> i32;
        pub fn span_file(a: u32) -> i32;
        pub fn span_local_file(a: u32) -> i32;
        pub fn span_source_text(a: u32) -> i32;
        /// A stream lexed from the text: its encoding's length in the buffer, or `-1 - len`
        /// with the error's text of `len` bytes there.
        pub fn ts_from_str(ptr: *const u8, len: usize) -> i32;
        pub fn literal_from_str(ptr: *const u8, len: usize) -> i32;
        /// Copies the host's buffer to `to`.
        pub fn take(to: *mut u8);
        pub fn track_path(ptr: *const u8, len: usize);
        pub fn track_env_var(ptr: *const u8, len: usize);
        pub fn diagnostic(level: u32, ptr: *const u8, len: usize, span: u32);
        /// The macro's output, encoded.
        pub fn output(ptr: *const u8, len: usize);
        /// What a panic said, before the module traps.
        pub fn panicked(ptr: *const u8, len: usize);
    }
}

fn taken(len: i32) -> Option<Vec<u8>> {
    let len = usize::try_from(len).ok()?;
    let mut bytes = vec![0u8; len];
    unsafe { sys::take(bytes.as_mut_ptr()) };
    Some(bytes)
}

fn taken_string(len: i32) -> Option<String> {
    taken(len).map(|it| String::from_utf8(it).unwrap_or_default())
}

pub fn is_available() -> bool {
    true
}

#[derive(Clone)]
pub struct TokenStream(Rc<Vec<TokenTree>>);

#[derive(Debug)]
pub struct LexError(String);

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for LexError {}

#[derive(Debug)]
pub struct ExpandError;

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("macro expansion failed")
    }
}

impl error::Error for ExpandError {}

impl TokenStream {
    pub fn new() -> TokenStream {
        TokenStream(Rc::new(Vec::new()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// As rust-analyzer's server does: an expression is not expanded early.
    pub fn expand_expr(&self) -> Result<TokenStream, ExpandError> {
        Ok(self.clone())
    }

    fn trees(&mut self) -> &mut Vec<TokenTree> {
        Rc::make_mut(&mut self.0)
    }
}

impl FromStr for TokenStream {
    type Err = LexError;

    fn from_str(src: &str) -> Result<TokenStream, LexError> {
        let len = unsafe { sys::ts_from_str(src.as_ptr(), src.len()) };
        match taken(len) {
            Some(bytes) => Ok(wire::Reader::new(&bytes).stream()),
            None => Err(LexError(taken_string(-1 - len).unwrap_or_default())),
        }
    }
}

impl fmt::Display for TokenStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut emit_whitespace = false;
        for tt in self.0.iter() {
            display_tree(tt, &mut emit_whitespace, f)?;
        }
        Ok(())
    }
}

fn display_tree(
    tt: &TokenTree,
    emit_whitespace: &mut bool,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    if std::mem::take(emit_whitespace) {
        f.write_str(" ")?;
    }
    match tt {
        TokenTree::Group(group) => {
            let (open, close) = group.delimiter.text();
            f.write_str(open)?;
            fmt::Display::fmt(&group.stream, f)?;
            f.write_str(close)?;
        }
        TokenTree::Punct(punct) => {
            *emit_whitespace = !punct.joint;
            write!(f, "{}", punct.ch)?;
        }
        TokenTree::Ident(ident) => {
            fmt::Display::fmt(ident, f)?;
            *emit_whitespace = true;
        }
        TokenTree::Literal(literal) => {
            fmt::Display::fmt(literal, f)?;
            *emit_whitespace = !matches!(
                literal.kind,
                LitKind::Str
                    | LitKind::StrRaw(_)
                    | LitKind::ByteStr
                    | LitKind::ByteStrRaw(_)
                    | LitKind::CStr
                    | LitKind::CStrRaw(_)
            );
        }
    }
    Ok(())
}

impl fmt::Debug for TokenStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenStream ")?;
        f.debug_list().entries(self.0.iter()).finish()
    }
}

impl Default for TokenStream {
    fn default() -> Self {
        TokenStream::new()
    }
}

impl From<TokenTree> for TokenStream {
    fn from(tree: TokenTree) -> TokenStream {
        TokenStream(Rc::new(vec![tree]))
    }
}

impl FromIterator<TokenTree> for TokenStream {
    fn from_iter<I: IntoIterator<Item = TokenTree>>(trees: I) -> Self {
        TokenStream(Rc::new(trees.into_iter().collect()))
    }
}

impl FromIterator<TokenStream> for TokenStream {
    fn from_iter<I: IntoIterator<Item = TokenStream>>(streams: I) -> Self {
        let mut all = TokenStream::new();
        all.extend(streams);
        all
    }
}

impl Extend<TokenTree> for TokenStream {
    fn extend<I: IntoIterator<Item = TokenTree>>(&mut self, trees: I) {
        self.trees().extend(trees);
    }
}

impl Extend<TokenStream> for TokenStream {
    fn extend<I: IntoIterator<Item = TokenStream>>(&mut self, streams: I) {
        for stream in streams {
            if self.0.is_empty() {
                *self = stream;
            } else {
                self.trees().extend(stream.0.iter().cloned());
            }
        }
    }
}

macro_rules! extend_items {
    ($($item:ident)*) => {
        $(
            impl Extend<$item> for TokenStream {
                fn extend<T: IntoIterator<Item = $item>>(&mut self, iter: T) {
                    self.extend(iter.into_iter().map(TokenTree::$item));
                }
            }
        )*
    };
}

extend_items!(Group Literal Punct Ident);

pub mod token_stream {
    use super::{TokenStream, TokenTree};

    #[derive(Clone)]
    pub struct IntoIter {
        stream: TokenStream,
        at: usize,
    }

    impl Iterator for IntoIter {
        type Item = TokenTree;

        fn next(&mut self) -> Option<TokenTree> {
            let tree = self.stream.0.get(self.at)?.clone();
            self.at += 1;
            Some(tree)
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            let left = self.stream.0.len() - self.at;
            (left, Some(left))
        }
    }

    impl IntoIterator for TokenStream {
        type Item = TokenTree;
        type IntoIter = IntoIter;

        fn into_iter(self) -> IntoIter {
            IntoIter { stream: self, at: 0 }
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Span(u32);

impl Span {
    pub fn def_site() -> Span {
        Span(unsafe { sys::def_site() })
    }

    pub fn call_site() -> Span {
        Span(unsafe { sys::call_site() })
    }

    pub fn mixed_site() -> Span {
        Span(unsafe { sys::mixed_site() })
    }

    fn some(handle: u32) -> Option<Span> {
        (handle != 0).then_some(Span(handle))
    }

    pub fn parent(&self) -> Option<Span> {
        Span::some(unsafe { sys::span_parent(self.0) })
    }

    pub fn source(&self) -> Span {
        Span(unsafe { sys::span_source(self.0) })
    }

    pub fn byte_range(&self) -> Range<usize> {
        let mut out = [0u32; 2];
        unsafe { sys::span_byte_range(self.0, out.as_mut_ptr()) };
        out[0] as usize..out[1] as usize
    }

    pub fn start(&self) -> Span {
        Span(unsafe { sys::span_start(self.0) })
    }

    pub fn end(&self) -> Span {
        Span(unsafe { sys::span_end(self.0) })
    }

    pub fn line(&self) -> usize {
        unsafe { sys::span_line(self.0) as usize }
    }

    pub fn column(&self) -> usize {
        unsafe { sys::span_column(self.0) as usize }
    }

    pub fn file(&self) -> String {
        taken_string(unsafe { sys::span_file(self.0) }).unwrap_or_default()
    }

    pub fn local_file(&self) -> Option<PathBuf> {
        taken_string(unsafe { sys::span_local_file(self.0) }).map(PathBuf::from)
    }

    pub fn join(&self, other: Span) -> Option<Span> {
        Span::some(unsafe { sys::span_join(self.0, other.0) })
    }

    pub fn resolved_at(&self, other: Span) -> Span {
        Span(unsafe { sys::span_resolved_at(self.0, other.0) })
    }

    pub fn located_at(&self, other: Span) -> Span {
        Span(unsafe { sys::span_located_at(self.0, other.0) })
    }

    pub fn eq(&self, other: &Span) -> bool {
        self.0 == other.0
    }

    pub fn source_text(&self) -> Option<String> {
        taken_string(unsafe { sys::span_source_text(self.0) })
    }

    pub fn save_span(&self) -> usize {
        0
    }

    pub fn recover_proc_macro_span(_id: usize) -> Span {
        Span::call_site()
    }

    pub fn error<T: Into<String>>(self, message: T) -> Diagnostic {
        Diagnostic::spanned(self, Level::Error, message)
    }

    pub fn warning<T: Into<String>>(self, message: T) -> Diagnostic {
        Diagnostic::spanned(self, Level::Warning, message)
    }

    pub fn note<T: Into<String>>(self, message: T) -> Diagnostic {
        Diagnostic::spanned(self, Level::Note, message)
    }

    pub fn help<T: Into<String>>(self, message: T) -> Diagnostic {
        Diagnostic::spanned(self, Level::Help, message)
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = taken_string(unsafe { sys::span_debug(self.0) }).unwrap_or_default();
        f.write_str(&text)
    }
}

#[derive(Clone)]
pub enum TokenTree {
    Group(Group),
    Ident(Ident),
    Punct(Punct),
    Literal(Literal),
}

impl TokenTree {
    pub fn span(&self) -> Span {
        match self {
            TokenTree::Group(it) => it.span(),
            TokenTree::Ident(it) => it.span(),
            TokenTree::Punct(it) => it.span(),
            TokenTree::Literal(it) => it.span(),
        }
    }

    pub fn set_span(&mut self, span: Span) {
        match self {
            TokenTree::Group(it) => it.set_span(span),
            TokenTree::Ident(it) => it.set_span(span),
            TokenTree::Punct(it) => it.set_span(span),
            TokenTree::Literal(it) => it.set_span(span),
        }
    }
}

impl fmt::Debug for TokenTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenTree::Group(it) => fmt::Debug::fmt(it, f),
            TokenTree::Ident(it) => fmt::Debug::fmt(it, f),
            TokenTree::Punct(it) => fmt::Debug::fmt(it, f),
            TokenTree::Literal(it) => fmt::Debug::fmt(it, f),
        }
    }
}

impl From<Group> for TokenTree {
    fn from(it: Group) -> TokenTree {
        TokenTree::Group(it)
    }
}

impl From<Ident> for TokenTree {
    fn from(it: Ident) -> TokenTree {
        TokenTree::Ident(it)
    }
}

impl From<Punct> for TokenTree {
    fn from(it: Punct) -> TokenTree {
        TokenTree::Punct(it)
    }
}

impl From<Literal> for TokenTree {
    fn from(it: Literal) -> TokenTree {
        TokenTree::Literal(it)
    }
}

impl fmt::Display for TokenTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&TokenStream::from(self.clone()), f)
    }
}

#[derive(Clone)]
pub struct Group {
    delimiter: Delimiter,
    stream: TokenStream,
    open: Span,
    close: Span,
    entire: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Delimiter {
    Parenthesis,
    Brace,
    Bracket,
    None,
}

impl Delimiter {
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Delimiter::Parenthesis => ("(", ")"),
            Delimiter::Brace => ("{", "}"),
            Delimiter::Bracket => ("[", "]"),
            Delimiter::None => ("", ""),
        }
    }
}

impl Group {
    pub fn new(delimiter: Delimiter, stream: TokenStream) -> Group {
        let span = Span::call_site();
        Group { delimiter, stream, open: span, close: span, entire: span }
    }

    pub fn delimiter(&self) -> Delimiter {
        self.delimiter
    }

    pub fn stream(&self) -> TokenStream {
        self.stream.clone()
    }

    pub fn span(&self) -> Span {
        self.entire
    }

    pub fn span_open(&self) -> Span {
        self.open
    }

    pub fn span_close(&self) -> Span {
        self.close
    }

    pub fn set_span(&mut self, span: Span) {
        (self.open, self.close, self.entire) = (span, span, span);
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&TokenStream::from(TokenTree::Group(self.clone())), f)
    }
}

impl fmt::Debug for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Group")
            .field("delimiter", &self.delimiter)
            .field("stream", &self.stream)
            .field("span", &self.entire)
            .finish()
    }
}

#[derive(Clone)]
pub struct Punct {
    ch: char,
    joint: bool,
    span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Spacing {
    Joint,
    Alone,
}

impl Punct {
    pub fn new(ch: char, spacing: Spacing) -> Punct {
        const LEGAL_CHARS: &[char] = &[
            '=', '<', '>', '!', '~', '+', '-', '*', '/', '%', '^', '&', '|', '@', '.', ',', ';',
            ':', '#', '$', '?', '\'',
        ];
        if !LEGAL_CHARS.contains(&ch) {
            panic!("unsupported character `{:?}`", ch);
        }
        Punct { ch, joint: spacing == Spacing::Joint, span: Span::call_site() }
    }

    pub fn as_char(&self) -> char {
        self.ch
    }

    pub fn spacing(&self) -> Spacing {
        if self.joint { Spacing::Joint } else { Spacing::Alone }
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn set_span(&mut self, span: Span) {
        self.span = span;
    }
}

impl fmt::Display for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.ch)
    }
}

impl fmt::Debug for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Punct")
            .field("ch", &self.ch)
            .field("spacing", &self.spacing())
            .field("span", &self.span)
            .finish()
    }
}

impl PartialEq<char> for Punct {
    fn eq(&self, rhs: &char) -> bool {
        self.ch == *rhs
    }
}

impl PartialEq<Punct> for char {
    fn eq(&self, rhs: &Punct) -> bool {
        *self == rhs.ch
    }
}

/// How an identifier is written: `x`, `r#x` or `k#x`.
#[derive(Copy, Clone, PartialEq, Eq)]
enum IdentKind {
    Normal,
    Raw,
    ForcedKeyword,
}

#[derive(Clone)]
pub struct Ident {
    sym: Rc<str>,
    kind: IdentKind,
    span: Span,
}

impl Ident {
    pub fn new(string: &str, span: Span) -> Ident {
        Ident { sym: string.into(), kind: IdentKind::Normal, span }
    }

    pub fn new_raw(string: &str, span: Span) -> Ident {
        Ident { sym: string.into(), kind: IdentKind::Raw, span }
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn set_span(&mut self, span: Span) {
        self.span = span;
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            IdentKind::Normal => {}
            IdentKind::Raw => f.write_str("r#")?,
            IdentKind::ForcedKeyword => f.write_str("k#")?,
        }
        f.write_str(&self.sym)
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ident").field("ident", &self.to_string()).field("span", &self.span).finish()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LitKind {
    Byte,
    Char,
    Integer,
    Float,
    Str,
    StrRaw(u8),
    ByteStr,
    ByteStrRaw(u8),
    CStr,
    CStrRaw(u8),
    ErrWithGuar,
}

#[derive(Clone)]
pub struct Literal {
    kind: LitKind,
    symbol: Rc<str>,
    suffix: Option<Rc<str>>,
    span: Span,
}

macro_rules! suffixed_int_literals {
    ($($name:ident => $kind:ident,)*) => ($(
        pub fn $name(n: $kind) -> Literal {
            Literal::new(LitKind::Integer, &n.to_string(), Some(stringify!($kind)))
        }
    )*)
}

macro_rules! unsuffixed_int_literals {
    ($($name:ident => $kind:ident,)*) => ($(
        pub fn $name(n: $kind) -> Literal {
            Literal::new(LitKind::Integer, &n.to_string(), None)
        }
    )*)
}

impl Literal {
    fn new(kind: LitKind, value: &str, suffix: Option<&str>) -> Literal {
        Literal { kind, symbol: value.into(), suffix: suffix.map(Into::into), span: Span::call_site() }
    }

    suffixed_int_literals! {
        u8_suffixed => u8,
        u16_suffixed => u16,
        u32_suffixed => u32,
        u64_suffixed => u64,
        u128_suffixed => u128,
        usize_suffixed => usize,
        i8_suffixed => i8,
        i16_suffixed => i16,
        i32_suffixed => i32,
        i64_suffixed => i64,
        i128_suffixed => i128,
        isize_suffixed => isize,
    }

    unsuffixed_int_literals! {
        u8_unsuffixed => u8,
        u16_unsuffixed => u16,
        u32_unsuffixed => u32,
        u64_unsuffixed => u64,
        u128_unsuffixed => u128,
        usize_unsuffixed => usize,
        i8_unsuffixed => i8,
        i16_unsuffixed => i16,
        i32_unsuffixed => i32,
        i64_unsuffixed => i64,
        i128_unsuffixed => i128,
        isize_unsuffixed => isize,
    }

    pub fn f32_unsuffixed(n: f32) -> Literal {
        if !n.is_finite() {
            panic!("Invalid float literal {n}");
        }
        let mut repr = n.to_string();
        if !repr.contains('.') {
            repr.push_str(".0");
        }
        Literal::new(LitKind::Float, &repr, None)
    }

    pub fn f32_suffixed(n: f32) -> Literal {
        if !n.is_finite() {
            panic!("Invalid float literal {n}");
        }
        Literal::new(LitKind::Float, &n.to_string(), Some("f32"))
    }

    pub fn f64_unsuffixed(n: f64) -> Literal {
        if !n.is_finite() {
            panic!("Invalid float literal {n}");
        }
        let mut repr = n.to_string();
        if !repr.contains('.') {
            repr.push_str(".0");
        }
        Literal::new(LitKind::Float, &repr, None)
    }

    pub fn f64_suffixed(n: f64) -> Literal {
        if !n.is_finite() {
            panic!("Invalid float literal {n}");
        }
        Literal::new(LitKind::Float, &n.to_string(), Some("f64"))
    }

    pub fn string(string: &str) -> Literal {
        let escape = EscapeOptions {
            escape_single_quote: false,
            escape_double_quote: true,
            escape_nonascii: false,
        };
        Literal::new(LitKind::Str, &escape_bytes(string.as_bytes(), escape), None)
    }

    pub fn character(ch: char) -> Literal {
        let escape = EscapeOptions {
            escape_single_quote: true,
            escape_double_quote: false,
            escape_nonascii: false,
        };
        let repr = escape_bytes(ch.encode_utf8(&mut [0u8; 4]).as_bytes(), escape);
        Literal::new(LitKind::Char, &repr, None)
    }

    pub fn byte_character(byte: u8) -> Literal {
        let escape = EscapeOptions {
            escape_single_quote: true,
            escape_double_quote: false,
            escape_nonascii: true,
        };
        Literal::new(LitKind::Byte, &escape_bytes(&[byte], escape), None)
    }

    pub fn byte_string(bytes: &[u8]) -> Literal {
        let escape = EscapeOptions {
            escape_single_quote: false,
            escape_double_quote: true,
            escape_nonascii: true,
        };
        Literal::new(LitKind::ByteStr, &escape_bytes(bytes, escape), None)
    }

    pub fn c_string(string: &CStr) -> Literal {
        let escape = EscapeOptions {
            escape_single_quote: false,
            escape_double_quote: true,
            escape_nonascii: false,
        };
        Literal::new(LitKind::CStr, &escape_bytes(string.to_bytes(), escape), None)
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn set_span(&mut self, span: Span) {
        self.span = span;
    }

    pub fn subspan<R: RangeBounds<usize>>(&self, range: R) -> Option<Span> {
        // Each bound as 0 for none, else 1 + 2 * whether it is excluded + 4 * its value.
        let bound = |it: Bound<&usize>| match it {
            Bound::Unbounded => 0u64,
            Bound::Included(at) => 1 + 4 * *at as u64,
            Bound::Excluded(at) => 3 + 4 * *at as u64,
        };
        let (start, end) = (bound(range.start_bound()), bound(range.end_bound()));
        Span::some(unsafe { sys::span_subspan(self.span.0, start, end) })
    }

    fn parts(&self, f: &mut dyn FnMut(&str) -> fmt::Result) -> fmt::Result {
        let hashes = |n: u8| "#".repeat(n as usize);
        let symbol = &*self.symbol;
        match self.kind {
            LitKind::Byte => [f("b'"), f(symbol), f("'")].into_iter().collect::<fmt::Result>()?,
            LitKind::Char => [f("'"), f(symbol), f("'")].into_iter().collect::<fmt::Result>()?,
            LitKind::Str => [f("\""), f(symbol), f("\"")].into_iter().collect::<fmt::Result>()?,
            LitKind::StrRaw(n) => {
                let h = hashes(n);
                [f("r"), f(&h), f("\""), f(symbol), f("\""), f(&h)]
                    .into_iter()
                    .collect::<fmt::Result>()?
            }
            LitKind::ByteStr => {
                [f("b\""), f(symbol), f("\"")].into_iter().collect::<fmt::Result>()?
            }
            LitKind::ByteStrRaw(n) => {
                let h = hashes(n);
                [f("br"), f(&h), f("\""), f(symbol), f("\""), f(&h)]
                    .into_iter()
                    .collect::<fmt::Result>()?
            }
            LitKind::CStr => [f("c\""), f(symbol), f("\"")].into_iter().collect::<fmt::Result>()?,
            LitKind::CStrRaw(n) => {
                let h = hashes(n);
                [f("cr"), f(&h), f("\""), f(symbol), f("\""), f(&h)]
                    .into_iter()
                    .collect::<fmt::Result>()?
            }
            LitKind::Integer | LitKind::Float | LitKind::ErrWithGuar => f(symbol)?,
        }
        match &self.suffix {
            Some(suffix) => f(suffix),
            None => Ok(()),
        }
    }
}

impl FromStr for Literal {
    type Err = LexError;

    fn from_str(src: &str) -> Result<Literal, LexError> {
        let len = unsafe { sys::literal_from_str(src.as_ptr(), src.len()) };
        let Some(bytes) = taken(len) else {
            return Err(LexError("cannot parse string into literal".to_owned()));
        };
        match wire::Reader::new(&bytes).stream().0.first() {
            Some(TokenTree::Literal(literal)) => Ok(literal.clone()),
            _ => Err(LexError("cannot parse string into literal".to_owned())),
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.parts(&mut |part| f.write_str(part))
    }
}

impl fmt::Debug for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Literal")
            .field("kind", &format_args!("{:?}", self.kind))
            .field("symbol", &self.symbol)
            .field("suffix", &format_args!("{:?}", self.suffix))
            .field("span", &self.span)
            .finish()
    }
}

#[derive(Copy, Clone)]
struct EscapeOptions {
    escape_single_quote: bool,
    escape_double_quote: bool,
    escape_nonascii: bool,
}

fn escape_bytes(bytes: &[u8], opt: EscapeOptions) -> String {
    let mut repr = String::new();
    if opt.escape_nonascii {
        for &byte in bytes {
            escape_single_byte(byte, opt, &mut repr);
        }
    } else {
        for chunk in bytes.utf8_chunks() {
            for ch in chunk.valid().chars() {
                if (ch == '\'' && !opt.escape_single_quote)
                    || (ch == '"' && !opt.escape_double_quote)
                {
                    repr.push(ch);
                } else {
                    repr.extend(ch.escape_debug());
                }
            }
            for &byte in chunk.invalid() {
                escape_single_byte(byte, opt, &mut repr);
            }
        }
    }
    repr
}

fn escape_single_byte(byte: u8, opt: EscapeOptions, repr: &mut String) {
    if byte == b'\0' {
        repr.push_str("\\0");
    } else if (byte == b'\'' && !opt.escape_single_quote)
        || (byte == b'"' && !opt.escape_double_quote)
    {
        repr.push(byte as char);
    } else {
        repr.extend(byte.escape_ascii().map(char::from));
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Level {
    Error,
    Warning,
    Note,
    Help,
}

pub trait MultiSpan {
    fn into_spans(self) -> Vec<Span>;
}

impl MultiSpan for Span {
    fn into_spans(self) -> Vec<Span> {
        vec![self]
    }
}

impl MultiSpan for Vec<Span> {
    fn into_spans(self) -> Vec<Span> {
        self
    }
}

impl<'a> MultiSpan for &'a [Span] {
    fn into_spans(self) -> Vec<Span> {
        self.to_vec()
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    level: Level,
    message: String,
    spans: Vec<Span>,
    children: Vec<Diagnostic>,
}

macro_rules! diagnostic_child_methods {
    ($spanned:ident, $regular:ident, $level:expr) => {
        pub fn $spanned<S, T>(mut self, spans: S, message: T) -> Diagnostic
        where
            S: MultiSpan,
            T: Into<String>,
        {
            self.children.push(Diagnostic::spanned(spans, $level, message));
            self
        }

        pub fn $regular<T: Into<String>>(mut self, message: T) -> Diagnostic {
            self.children.push(Diagnostic::new($level, message));
            self
        }
    };
}

impl Diagnostic {
    pub fn new<T: Into<String>>(level: Level, message: T) -> Diagnostic {
        Diagnostic { level, message: message.into(), spans: vec![], children: vec![] }
    }

    pub fn spanned<S, T>(spans: S, level: Level, message: T) -> Diagnostic
    where
        S: MultiSpan,
        T: Into<String>,
    {
        Diagnostic { level, message: message.into(), spans: spans.into_spans(), children: vec![] }
    }

    diagnostic_child_methods!(span_error, error, Level::Error);
    diagnostic_child_methods!(span_warning, warning, Level::Warning);
    diagnostic_child_methods!(span_note, note, Level::Note);
    diagnostic_child_methods!(span_help, help, Level::Help);

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn set_message<T: Into<String>>(&mut self, message: T) {
        self.message = message.into();
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn set_spans<S: MultiSpan>(&mut self, spans: S) {
        self.spans = spans.into_spans();
    }

    pub fn children(&self) -> impl Iterator<Item = &Diagnostic> {
        self.children.iter()
    }

    pub fn emit(self) {
        let level = match self.level {
            Level::Error => 0,
            Level::Warning => 1,
            Level::Note => 2,
            Level::Help => 3,
        };
        let span = self.spans.first().map_or(0, |it| it.0);
        unsafe { sys::diagnostic(level, self.message.as_ptr(), self.message.len(), span) };
        for child in self.children {
            child.emit();
        }
    }
}

pub mod tracked {
    use std::{
        env::{self, VarError},
        ffi::OsStr,
        path::Path,
    };

    pub fn env_var<K: AsRef<OsStr> + AsRef<str>>(key: K) -> Result<String, VarError> {
        let key: &str = key.as_ref();
        unsafe { super::sys::track_env_var(key.as_ptr(), key.len()) };
        env::var(key)
    }

    pub fn path<P: AsRef<Path>>(path: P) {
        let path: &str = path.as_ref().to_str().unwrap();
        unsafe { super::sys::track_path(path.as_ptr(), path.len()) };
    }
}

/// The encoding of a stream between the module and the host, little-endian:
///
/// ```text
/// stream  := count:u32 tree*
/// tree    := 0 delimiter:u8 open:u32 close:u32 entire:u32 stream
///          | 1 span:u32 kind:u8 text            (kind: 0 plain, 1 r#, 2 k#)
///          | 2 span:u32 ch:u32 joint:u8
///          | 3 span:u32 kind:u8 hashes:u8 text suffix
/// text    := len:u32 bytes
/// suffix  := len:u32 bytes, len u32::MAX for none
/// ```
///
/// Delimiters are 0 `()`, 1 `{}`, 2 `[]`, 3 none; literal kinds are numbered as [`LitKind`]
/// lists them.
pub mod wire {
    use std::rc::Rc;

    use super::*;

    pub struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl<'a> Reader<'a> {
        pub fn new(bytes: &'a [u8]) -> Reader<'a> {
            Reader { bytes, at: 0 }
        }

        fn u8(&mut self) -> u8 {
            let it = self.bytes[self.at];
            self.at += 1;
            it
        }

        fn u32(&mut self) -> u32 {
            let it = u32::from_le_bytes(self.bytes[self.at..self.at + 4].try_into().unwrap());
            self.at += 4;
            it
        }

        fn text(&mut self) -> Rc<str> {
            let len = self.u32() as usize;
            let text = std::str::from_utf8(&self.bytes[self.at..self.at + len]).unwrap();
            self.at += len;
            text.into()
        }

        pub fn stream(&mut self) -> TokenStream {
            let count = self.u32() as usize;
            let mut trees = Vec::with_capacity(count);
            for _ in 0..count {
                trees.push(self.tree());
            }
            TokenStream(Rc::new(trees))
        }

        fn tree(&mut self) -> TokenTree {
            match self.u8() {
                0 => {
                    let delimiter = match self.u8() {
                        0 => Delimiter::Parenthesis,
                        1 => Delimiter::Brace,
                        2 => Delimiter::Bracket,
                        _ => Delimiter::None,
                    };
                    let (open, close, entire) =
                        (Span(self.u32()), Span(self.u32()), Span(self.u32()));
                    let stream = self.stream();
                    TokenTree::Group(Group { delimiter, stream, open, close, entire })
                }
                1 => {
                    let span = Span(self.u32());
                    let kind = match self.u8() {
                        1 => IdentKind::Raw,
                        2 => IdentKind::ForcedKeyword,
                        _ => IdentKind::Normal,
                    };
                    TokenTree::Ident(Ident { sym: self.text(), kind, span })
                }
                2 => {
                    let span = Span(self.u32());
                    let ch = char::from_u32(self.u32()).unwrap_or('?');
                    TokenTree::Punct(Punct { ch, joint: self.u8() != 0, span })
                }
                _ => {
                    let span = Span(self.u32());
                    let (kind, hashes) = (self.u8(), self.u8());
                    let kind = match kind {
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
                        _ => LitKind::ErrWithGuar,
                    };
                    let symbol = self.text();
                    let suffix = match u32::from_le_bytes(
                        self.bytes[self.at..self.at + 4].try_into().unwrap(),
                    ) {
                        u32::MAX => {
                            self.at += 4;
                            None
                        }
                        _ => Some(self.text()),
                    };
                    TokenTree::Literal(Literal { kind, symbol, suffix, span })
                }
            }
        }
    }

    pub fn write(stream: &TokenStream, out: &mut Vec<u8>) {
        out.extend_from_slice(&(stream.0.len() as u32).to_le_bytes());
        for tree in stream.0.iter() {
            write_tree(tree, out);
        }
    }

    fn text(text: &str, out: &mut Vec<u8>) {
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    fn write_tree(tree: &TokenTree, out: &mut Vec<u8>) {
        match tree {
            TokenTree::Group(group) => {
                out.push(0);
                out.push(match group.delimiter {
                    Delimiter::Parenthesis => 0,
                    Delimiter::Brace => 1,
                    Delimiter::Bracket => 2,
                    Delimiter::None => 3,
                });
                for span in [group.open, group.close, group.entire] {
                    out.extend_from_slice(&span.0.to_le_bytes());
                }
                write(&group.stream, out);
            }
            TokenTree::Ident(ident) => {
                out.push(1);
                out.extend_from_slice(&ident.span.0.to_le_bytes());
                out.push(match ident.kind {
                    IdentKind::Normal => 0,
                    IdentKind::Raw => 1,
                    IdentKind::ForcedKeyword => 2,
                });
                text(&ident.sym, out);
            }
            TokenTree::Punct(punct) => {
                out.push(2);
                out.extend_from_slice(&punct.span.0.to_le_bytes());
                out.extend_from_slice(&(punct.ch as u32).to_le_bytes());
                out.push(punct.joint as u8);
            }
            TokenTree::Literal(literal) => {
                out.push(3);
                out.extend_from_slice(&literal.span.0.to_le_bytes());
                let (kind, hashes) = match literal.kind {
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
                    LitKind::ErrWithGuar => (10, 0),
                };
                out.extend_from_slice(&[kind, hashes]);
                text(&literal.symbol, out);
                match &literal.suffix {
                    Some(suffix) => text(suffix, out),
                    None => out.extend_from_slice(&u32::MAX.to_le_bytes()),
                }
            }
        }
    }
}

/// What ra-jit's list of a crate's macros calls (`crates/ra-jit/src/macro_list.rs`): the
/// module's exports run a macro through these.
#[doc(hidden)]
pub mod __blaze {
    use super::*;

    /// Room for the host to write an input into, kept until the expansion ends.
    pub fn alloc(len: usize) -> *mut u8 {
        let mut room = vec![0u8; len];
        let at = room.as_mut_ptr();
        ROOM.with(|it| it.borrow_mut().push(room));
        at
    }

    thread_local! {
        static ROOM: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    }

    fn input(ptr: *const u8, len: usize) -> TokenStream {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        wire::Reader::new(bytes).stream()
    }

    fn hook() {
        std::panic::set_hook(Box::new(|info| {
            let payload = info.payload();
            let message = match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
                (Some(it), _) => (*it).to_owned(),
                (_, Some(it)) => it.clone(),
                _ => "Box<dyn Any>".to_owned(),
            };
            unsafe { sys::panicked(message.as_ptr(), message.len()) };
        }));
    }

    fn give(output: TokenStream) {
        let mut out = Vec::new();
        wire::write(&output, &mut out);
        unsafe { sys::output(out.as_ptr(), out.len()) };
    }

    /// A derive or a function-like macro.
    pub fn expand1(f: fn(TokenStream) -> TokenStream, ptr: *const u8, len: usize) {
        hook();
        give(f(input(ptr, len)));
    }

    /// An attribute.
    pub fn expand2(
        f: fn(TokenStream, TokenStream) -> TokenStream,
        attr: *const u8,
        attr_len: usize,
        ptr: *const u8,
        len: usize,
    ) {
        hook();
        give(f(input(attr, attr_len), input(ptr, len)));
    }

    /// The list of macros, a line each: kind (0 derive, 1 attribute, 2 function-like), name
    /// and the derive's helper attributes, tab-separated.
    pub fn list(text: &str) {
        unsafe { sys::output(text.as_ptr(), text.len()) };
    }

    /// As [`list`], of the bytes of the static the module also carries in a custom section.
    pub fn list_bytes(text: &[u8]) {
        unsafe { sys::output(text.as_ptr(), text.len()) };
    }
}
