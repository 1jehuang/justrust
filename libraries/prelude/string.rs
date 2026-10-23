// Native String: owned UTF-8 bytes in a Vec<u8>, ordinary Rust source.
use super::option::Option;
use super::mem::raw_realloc;

impl super::iter::FromIterator<char> for String {
    fn from_iter<I: super::iter::IntoIterator<Item = char>>(source: I) -> String {
        let mut iterator = source.into_iter();
        let mut text = String::new();
        loop {
            match iterator.next() {
                Option::Some(value) => text.push(value),
                Option::None => return text,
            }
        }
    }
}

impl<'a> super::iter::FromIterator<&'a char> for String {
    fn from_iter<I: super::iter::IntoIterator<Item = &'a char>>(source: I) -> String {
        let mut iterator = source.into_iter();
        let mut text = String::new();
        loop {
            match iterator.next() {
                Option::Some(value) => text.push(*value),
                Option::None => return text,
            }
        }
    }
}

pub struct String {
    ptr: usize,
    len: usize,
    cap: usize,
}

// &str is a (data, len) pair. These helpers reinterpret it through raw
// pointers; they do not check UTF-8 validity, callers must preserve it.
pub fn str_parts(s: &str) -> (usize, usize) {
    let p = &s as *const &str as *const (usize, usize);
    unsafe { *p }
}

pub fn read_u8(address: usize) -> u8 {
    let p = address as *const u8;
    unsafe { *p }
}

pub fn str_from_raw_parts(ptr: usize, len: usize) -> &'static str {
    // Even an empty Rust reference needs a non-null, aligned data pointer.
    let data = if ptr == 0usize && len == 0usize { 1usize } else { ptr };
    let pair = (data, len);
    let r = &pair;
    let p = r as *const (usize, usize) as *const &'static str;
    unsafe { *p }
}

pub fn str_byte(s: &str, index: usize) -> u8 {
    let (p, n) = str_parts(s);
    if index >= n {
        panic!("byte index out of bounds");
    }
    read_u8(p + index)
}

// Bytes are packed (one byte per address) in a raw heap buffer. Vec<u8>
// uses the private 8-byte scalar slot layout, so String manages its own.
fn write_u8(address: usize, value: u8) {
    let p = address as *mut u8;
    unsafe {
        *p = value;
    }
}

impl String {
    pub fn new() -> String {
        String {
            // Non-null dangling storage, aligned for u8. Zero capacity owns no mapping.
            ptr: 1usize,
            len: 0usize,
            cap: 0usize,
        }
    }

    pub fn with_capacity(capacity: usize) -> String {
        let mut s = String::new();
        s.reserve(capacity);
        s
    }

    pub fn from(s: &str) -> String {
        let mut r = String::new();
        r.push_str(s);
        r
    }

    pub fn reserve(&mut self, additional: usize) {
        let needed = self.len + additional;
        if needed <= self.cap {
            return;
        }
        let mut cap = if self.cap == 0usize { 16usize } else { self.cap * 2usize };
        if cap < needed {
            cap = needed;
        }
        let fresh = raw_realloc(self.ptr, self.cap, cap);
        if fresh == 0usize {
            panic!("memory allocation failed");
        }
        self.ptr = fresh;
        self.cap = cap;
    }

    pub fn push_str(&mut self, s: &str) {
        let (p, n) = str_parts(s);
        self.reserve(n);
        let mut i = 0usize;
        while i < n {
            write_u8(self.ptr + self.len + i, read_u8(p + i));
            i += 1usize;
        }
        self.len += n;
    }

    pub fn push_byte(&mut self, b: u8) {
        self.reserve(1usize);
        write_u8(self.ptr + self.len, b);
        self.len += 1usize;
    }

    pub fn push(&mut self, c: char) {
        let code = c as u32;
        if code < 128u32 {
            self.push_byte(code as u8);
        } else if code < 2048u32 {
            self.push_byte((192u32 | (code >> 6u32)) as u8);
            self.push_byte((128u32 | (code & 63u32)) as u8);
        } else if code < 65536u32 {
            self.push_byte((224u32 | (code >> 12u32)) as u8);
            self.push_byte((128u32 | ((code >> 6u32) & 63u32)) as u8);
            self.push_byte((128u32 | (code & 63u32)) as u8);
        } else {
            self.push_byte((240u32 | (code >> 18u32)) as u8);
            self.push_byte((128u32 | ((code >> 12u32) & 63u32)) as u8);
            self.push_byte((128u32 | ((code >> 6u32) & 63u32)) as u8);
            self.push_byte((128u32 | (code & 63u32)) as u8);
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0usize
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn clear(&mut self) {
        self.len = 0usize;
    }

    pub fn truncate(&mut self, len: usize) {
        if len < self.len {
            self.len = len;
        }
    }

    pub fn as_str(&self) -> &str {
        str_from_raw_parts(self.ptr, self.len)
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr as *const u8
    }

    pub fn push_string(&mut self, other: &String) {
        self.push_str(other.as_str());
    }

    pub fn insert(&mut self, index: usize, c: char) {
        let mut tail = String::new();
        tail.push_str(str_from_raw_parts(self.ptr + index, self.len - index));
        self.len = index;
        self.push(c);
        self.push_str(tail.as_str());
    }

    pub fn to_uppercase(&self) -> String {
        let mut r = String::with_capacity(self.len);
        let mut i = 0usize;
        while i < self.len {
            let c = decode_utf8_at(self.ptr, i);
            super::unicode_uppercase::push_uppercase(&mut r, c);
            i += utf8_width(read_u8(self.ptr + i));
        }
        r
    }

    pub fn to_lowercase(&self) -> String {
        let mut r = String::with_capacity(self.len);
        let mut i = 0usize;
        while i < self.len {
            let b = read_u8(self.ptr + i);
            r.push_byte(if b >= 65u8 && b <= 90u8 { b + 32u8 } else { b });
            i += 1usize;
        }
        r
    }

    pub fn eq_str(&self, other: &str) -> bool {
        str_eq(self.as_str(), other)
    }

    pub fn contains(&self, pattern: &str) -> bool {
        str_find(self.as_str(), pattern).is_some()
    }

    pub fn starts_with(&self, pattern: &str) -> bool {
        str_starts_with(self.as_str(), pattern)
    }

    pub fn ends_with(&self, pattern: &str) -> bool {
        str_ends_with(self.as_str(), pattern)
    }

    pub fn find(&self, pattern: &str) -> Option<usize> {
        str_find(self.as_str(), pattern)
    }

    pub fn trim(&self) -> &str {
        str_trim(self.as_str())
    }

    pub fn repeat(&self, count: usize) -> String {
        let mut r = String::with_capacity(self.len * count);
        let mut i = 0usize;
        while i < count {
            r.push_str(self.as_str());
            i += 1usize;
        }
        r
    }

    pub fn pop(&mut self) -> Option<char> {
        if self.len == 0usize {
            return Option::None;
        }
        // Walk back over continuation bytes to the start of the last char.
        let mut start = self.len - 1usize;
        while start > 0usize {
            if (read_u8(self.ptr + start) & 192u8) != 128u8 {
                break;
            }
            start -= 1usize;
        }
        let c = decode_utf8_at(self.ptr, start);
        self.len = start;
        Option::Some(c)
    }
}

// Decode the scalar starting at byte `at` of valid UTF-8 at `base`.
pub fn decode_utf8_at(base: usize, at: usize) -> char {
    let b0 = read_u8(base + at) as u32;
    if b0 < 128u32 {
        return unsafe_char(b0);
    }
    let b1 = (read_u8(base + at + 1usize) as u32) & 63u32;
    if b0 < 224u32 {
        return unsafe_char(((b0 & 31u32) << 6u32) | b1);
    }
    let b2 = (read_u8(base + at + 2usize) as u32) & 63u32;
    if b0 < 240u32 {
        return unsafe_char(((b0 & 15u32) << 12u32) | (b1 << 6u32) | b2);
    }
    let b3 = (read_u8(base + at + 3usize) as u32) & 63u32;
    unsafe_char(((b0 & 7u32) << 18u32) | (b1 << 12u32) | (b2 << 6u32) | b3)
}

pub fn utf8_width(first: u8) -> usize {
    if first < 128u8 {
        1usize
    } else if first < 224u8 {
        2usize
    } else if first < 240u8 {
        3usize
    } else {
        4usize
    }
}

// Reinterpret a known-valid scalar value as char.
pub fn unsafe_char(code: u32) -> char {
    let r = &code;
    let p = r as *const u32 as *const char;
    unsafe { *p }
}

pub fn str_eq(a: &str, b: &str) -> bool {
    let (pa, na) = str_parts(a);
    let (pb, nb) = str_parts(b);
    if na != nb {
        return false;
    }
    let mut i = 0usize;
    while i < na {
        if read_u8(pa + i) != read_u8(pb + i) {
            return false;
        }
        i += 1usize;
    }
    true
}

pub fn str_find(haystack: &str, needle: &str) -> Option<usize> {
    let (ph, nh) = str_parts(haystack);
    let (pn, nn) = str_parts(needle);
    if nn > nh {
        return Option::None;
    }
    let mut i = 0usize;
    while i + nn <= nh {
        let mut j = 0usize;
        while j < nn && read_u8(ph + i + j) == read_u8(pn + j) {
            j += 1usize;
        }
        if j == nn {
            return Option::Some(i);
        }
        i += 1usize;
    }
    Option::None
}

pub(super) fn str_starts_with(text: &str, pattern: &str) -> bool {
    let (base, len) = str_parts(text);
    let (needle, count) = str_parts(pattern);
    if count > len { return false; }
    let mut i = 0usize;
    while i < count {
        if read_u8(base + i) != read_u8(needle + i) { return false; }
        i += 1usize;
    }
    true
}

pub(super) fn str_ends_with(text: &str, pattern: &str) -> bool {
    let (base, len) = str_parts(text);
    let (needle, count) = str_parts(pattern);
    if count > len { return false; }
    let start = len - count;
    let mut i = 0usize;
    while i < count {
        if read_u8(base + start + i) != read_u8(needle + i) { return false; }
        i += 1usize;
    }
    true
}

// Unicode White_Space, the property used by Rust's char::is_whitespace.
pub(super) fn char_is_whitespace(ch: char) -> bool {
    let c = ch as u32;
    (c >= 9u32 && c <= 13u32) || c == 32u32 || c == 133u32
        || c == 160u32 || c == 5760u32 || (c >= 8192u32 && c <= 8202u32)
        || c == 8232u32 || c == 8233u32 || c == 8239u32
        || c == 8287u32 || c == 12288u32
}

pub fn str_trim(s: &str) -> &str {
    let (p, n) = str_parts(s);
    let mut start = 0usize;
    while start < n && char_is_whitespace(decode_utf8_at(p, start)) {
        start += utf8_width(read_u8(p + start));
    }
    let mut end = n;
    while end > start {
        let mut at = end - 1usize;
        while at > start && (read_u8(p + at) & 192u8) == 128u8 { at -= 1usize; }
        if !char_is_whitespace(decode_utf8_at(p, at)) { break; }
        end = at;
    }
    str_from_raw_parts(p + start, end - start)
}

impl Clone for String {
    fn clone(&self) -> String {
        String::from(self.as_str())
    }
}

/// `std::string::ToString`, blanket-implemented for every `Display` type.
pub trait ToString {
    fn to_string(&self) -> String;
}
// `T: Sized` here plus a direct `str` impl: the compiler does not yet
// instantiate generic impls with unsized actuals.
// Stopgap until unsized blanket instantiation: `str::to_string` comes from
// this separate prelude trait (same method name, disjoint Self types).
pub trait StrToString {
    fn to_string(&self) -> String;
}
impl StrToString for str {
    fn to_string(&self) -> String {
        String::from(self)
    }
}
impl<T: super::fmt::Display> ToString for T {
    fn to_string(&self) -> String {
        let mut f = super::fmt::Formatter::new();
        let _ = self.fmt(&mut f);
        f.into_string()
    }
}

impl super::ops::Deref for String {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl super::cmp::PartialEq for String {
    fn eq(&self, other: &String) -> bool {
        str_eq(self.as_str(), other.as_str())
    }
}

impl super::cmp::PartialEq<&str> for String {
    fn eq(&self, other: &&str) -> bool {
        str_eq(self.as_str(), *other)
    }
}

impl super::cmp::PartialEq<String> for &str {
    fn eq(&self, other: &String) -> bool {
        str_eq(*self, other.as_str())
    }
}

impl super::cmp::Eq for String {}

// std's str hashing: the bytes, then a 0xff terminator.
impl super::hash::Hash for String {
    fn hash<H: super::hash::Hasher>(&self, state: &mut H) {
        let mut i = 0usize;
        while i < self.len {
            state.write_u8(read_u8(self.ptr + i));
            i += 1usize;
        }
        state.write_u8(255u8);
    }
}

/// `str::chars`: UTF-8 decoding iterator over `char`s. Exposed through a
/// prelude trait (like `StrToString`) because `str` has no inherent impl here.
pub trait StrChars {
    fn chars(&self) -> Chars<'_, ()>;
}
impl StrChars for str {
    fn chars(&self) -> Chars<'_, ()> {
        let (ptr, len): (usize, usize) = str_parts(self);
        Chars { text: self, unit: (), ptr, front: 0usize, back: len }
    }
}

/// `front..back` is the undecoded byte range of valid UTF-8 at `ptr`.
pub struct Chars<'a, U> {
    text: &'a str,
    // Always `()`: an open parameter keeps projected `IntoIterator` methods
    // on the generic path (closed reference-bearing ones are a boundary).
    unit: U,
    ptr: usize,
    front: usize,
    back: usize,
}

impl<'a, U> Chars<'a, U> {
    pub fn count(self) -> usize {
        let mut n: usize = 0usize;
        let mut at: usize = self.front;
        while at < self.back {
            at += utf8_width(read_u8(self.ptr + at));
            n += 1usize;
        }
        n
    }
}

impl<'a, U> super::iter::Iterator for Chars<'a, U> {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        if self.front >= self.back {
            return Option::None;
        }
        let at: usize = self.front;
        self.front += utf8_width(read_u8(self.ptr + at));
        Option::Some(decode_utf8_at(self.ptr, at))
    }
}

impl<'a, U> super::iter::DoubleEndedIterator for Chars<'a, U> {
    fn next_back(&mut self) -> Option<char> {
        if self.front >= self.back {
            return Option::None;
        }
        let mut at = self.back - 1usize;
        while at > self.front && (read_u8(self.ptr + at) & 192u8) == 128u8 {
            at -= 1usize;
        }
        self.back = at;
        Option::Some(decode_utf8_at(self.ptr, at))
    }
}

impl<'a, U> super::iter::IntoIterator for Chars<'a, U> {
    type Item = char;
    type IntoIter = Chars<'a, U>;
    fn into_iter(self) -> Chars<'a, U> {
        self
    }
}

/// `str` methods that return borrowed iterators. A separate prelude trait,
/// like `StrToString`, until inherent `impl str` blocks are supported.
pub trait StrPattern {
    fn is_contained_in(self, text: &str) -> bool;
}

impl StrPattern for char {
    fn is_contained_in(self, text: &str) -> bool {
        let (ptr, len) = str_parts(text);
        let mut at = 0usize;
        while at < len {
            if decode_utf8_at(ptr, at) == self {
                return true;
            }
            at += utf8_width(read_u8(ptr + at));
        }
        false
    }
}

impl StrPattern for &str {
    fn is_contained_in(self, text: &str) -> bool {
        str_find(text, self).is_some()
    }
}

pub trait StrMethods {
    fn to_uppercase(&self) -> String;
    fn contains<P: StrPattern>(&self, pattern: P) -> bool;
    fn split_whitespace<'a>(&'a self) -> SplitWhitespace<&'a str>;
    fn split<'a>(&'a self, separator: char) -> Split<&'a str>;
}

impl StrMethods for str {
    fn to_uppercase(&self) -> String {
        String::from(self).to_uppercase()
    }

    fn contains<P: StrPattern>(&self, pattern: P) -> bool {
        pattern.is_contained_in(self)
    }

    fn split<'a>(&'a self, separator: char) -> Split<&'a str> {
        let (ptr, len) = str_parts(self);
        Split { ptr, len, at: 0usize, separator, finished: false, marker: self }
    }

    fn split_whitespace<'a>(&'a self) -> SplitWhitespace<&'a str> {
        let (ptr, len) = str_parts(self);
        SplitWhitespace { ptr, len, at: 0usize, marker: self }
    }
}

/// Iterator over the non-empty, ASCII-whitespace-separated words of a `str`.
/// `S` is always `&'a str`: a generic carrier, because closed
/// `impl<'a> Iterator for X<'a>` impls with `&'a str` items do not yet
/// normalize. Words are rebuilt as `S` from (pointer, length) pairs.
pub struct SplitWhitespace<S> {
    ptr: usize,
    len: usize,
    at: usize,
    marker: S,
}

pub struct Split<S> {
    ptr: usize,
    len: usize,
    at: usize,
    separator: char,
    finished: bool,
    marker: S,
}

impl<S: StrCarrier> super::iter::Iterator for Split<S> {
    type Item = S;
    fn next(&mut self) -> Option<S> {
        if self.finished {
            return Option::None;
        }
        let start = self.at;
        while self.at < self.len {
            let at = self.at;
            let character = decode_utf8_at(self.ptr, at);
            self.at += utf8_width(read_u8(self.ptr + at));
            if character == self.separator {
                return Option::Some(S::from_parts(self.ptr + start, at - start));
            }
        }
        self.finished = true;
        Option::Some(S::from_parts(self.ptr + start, self.len - start))
    }
}

impl<S: StrCarrier> super::iter::IntoIterator for Split<S> {
    type Item = S;
    type IntoIter = Self;
    fn into_iter(self) -> Self {
        self
    }
}

fn is_space(b: u8) -> bool {
    b == 32u8 || (b >= 9u8 && b <= 13u8)
}

/// Rebuilds a borrowed `str` carrier from a (pointer, length) pair.
pub trait StrCarrier {
    fn from_parts(ptr: usize, len: usize) -> Self;
}

impl<'a> StrCarrier for &'a str {
    fn from_parts(ptr: usize, len: usize) -> &'a str {
        str_from_raw_parts(ptr, len)
    }
}

impl<S: StrCarrier> super::iter::Iterator for SplitWhitespace<S> {
    type Item = S;
    fn next(&mut self) -> Option<S> {
        while self.at < self.len && is_space(read_u8(self.ptr + self.at)) {
            self.at += 1usize;
        }
        if self.at >= self.len {
            return Option::None;
        }
        let start = self.at;
        while self.at < self.len && !is_space(read_u8(self.ptr + self.at)) {
            self.at += 1usize;
        }
        Option::Some(S::from_parts(self.ptr + start, self.at - start))
    }
}

impl<S: StrCarrier> super::iter::IntoIterator for SplitWhitespace<S> {
    type Item = S;
    type IntoIter = SplitWhitespace<S>;
    fn into_iter(self) -> SplitWhitespace<S> {
        self
    }
}

/// Element views accepted by `join` (stock `Borrow<str>` for `String`/`&str`).
pub trait JoinElement {
    fn push_onto(&self, out: &mut String);
}

impl JoinElement for String {
    fn push_onto(&self, out: &mut String) {
        out.push_str(self.as_str());
    }
}

impl<'a> JoinElement for &'a str {
    fn push_onto(&self, out: &mut String) {
        out.push_str(*self);
    }
}

/// `[S].join(sep)` for `String` and `&str` elements (stock `Join<&str>`).
/// Implemented on `Vec<S>` until `Vec<T>: Deref<Target = [T]>` lands; stock
/// reaches the same slice method by autoderef.
pub trait SliceJoin {
    fn join(&self, separator: &str) -> String;
}

impl<S: JoinElement> SliceJoin for super::vec::Vec<S> {
    fn join(&self, separator: &str) -> String {
        let mut out = String::new();
        let mut i = 0usize;
        while i < self.len() {
            if i > 0usize {
                out.push_str(separator);
            }
            self.get_ref(i).push_onto(&mut out);
            i += 1usize;
        }
        out
    }
}
