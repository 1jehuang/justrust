// Native String: owned UTF-8 bytes in a Vec<u8>, ordinary Rust source.
use super::option::Option;
use super::mem::raw_realloc;

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
    let pair = (ptr, len);
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
            ptr: 0usize,
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
            let b = read_u8(self.ptr + i);
            r.push_byte(if b >= 97u8 && b <= 122u8 { b - 32u8 } else { b });
            i += 1usize;
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
        let (_, n) = str_parts(pattern);
        n <= self.len && str_eq(str_from_raw_parts(self.ptr, n), pattern)
    }

    pub fn ends_with(&self, pattern: &str) -> bool {
        let (_, n) = str_parts(pattern);
        n <= self.len && str_eq(str_from_raw_parts(self.ptr + self.len - n, n), pattern)
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

fn ascii_space(b: u8) -> bool {
    b == 32u8 || b == 9u8 || b == 10u8 || b == 13u8 || b == 12u8 || b == 11u8
}

pub fn str_trim(s: &str) -> &str {
    let (p, n) = str_parts(s);
    let mut start = 0usize;
    while start < n && ascii_space(read_u8(p + start)) {
        start += 1usize;
    }
    let mut end = n;
    while end > start && ascii_space(read_u8(p + end - 1usize)) {
        end -= 1usize;
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
