// std::fmt as ordinary Rust source. A Formatter owns its output buffer (the
// compiler has no lifetime parameters on nominal types yet), and builders
// address it through a raw pointer. format!/write!/derive(Debug) lower to
// calls on this API (see SWARM-PLAN.md, macros stream).
use super::option::Option;
use super::string::String;

pub struct Error;

pub type Result = super::result::Result<(), Error>;

pub struct Formatter<'a> {
    // std's Formatter borrows its output sink for 'a. This one owns its
    // buffer, so the marker only carries 'a for source compatibility
    // (`fmt::Formatter<'_>` in user signatures).
    buf: String,
    fill: char,
    // 0 none, 1 left, 2 right, 3 center
    align: u8,
    width: Option<usize>,
    precision: Option<usize>,
    alternate: bool,
    plus: bool,
    zero: bool,
    // Pretty-printing nesting depth for {:#?}.
    indent: usize,
}

pub trait Display {
    fn fmt(&self, f: &mut Formatter) -> Result;
}

pub trait Debug {
    fn fmt(&self, f: &mut Formatter) -> Result;
}

pub trait Write {
    fn write_str(&mut self, s: &str) -> Result;
    /// write!/writeln! target: append the macro's formatted buffer.
    fn __jr_write_buf(&mut self, buf: String) -> Result {
        self.write_str(buf.as_str())
    }
}

/// Saved spec returned by `push_spec`, restored by `pop_spec`.
pub struct Spec {
    fill: char,
    align: u8,
    width: Option<usize>,
    precision: Option<usize>,
    flags: u32,
}

impl<'a> Formatter<'a> {
    pub fn new() -> Formatter<'a> {
        Formatter {
            buf: String::new(),
            fill: ' ',
            align: 0u8,
            width: Option::None,
            precision: Option::None,
            alternate: false,
            plus: false,
            zero: false,
            indent: 0usize,
        }
    }

    pub fn into_string(self) -> String {
        self.buf
    }

    pub fn as_str(&self) -> &str {
        self.buf.as_str()
    }

    pub fn alternate(&self) -> bool {
        self.alternate
    }

    pub fn width(&self) -> Option<usize> {
        self.width
    }

    pub fn precision(&self) -> Option<usize> {
        self.precision
    }

    pub fn fill(&self) -> char {
        self.fill
    }

    pub fn sign_plus(&self) -> bool {
        self.plus
    }

    /// flags: bit0 alternate, bit1 plus, bit2 zero-pad.
    pub fn push_spec(
        &mut self,
        fill: char,
        align: u8,
        width: Option<usize>,
        precision: Option<usize>,
        flags: u32,
    ) -> Spec {
        let old = Spec {
            fill: self.fill,
            align: self.align,
            width: self.width,
            precision: self.precision,
            flags: (if self.alternate { 1u32 } else { 0u32 })
                | (if self.plus { 2u32 } else { 0u32 })
                | (if self.zero { 4u32 } else { 0u32 }),
        };
        self.fill = fill;
        self.align = align;
        self.width = width;
        self.precision = precision;
        self.alternate = (flags & 1u32) != 0u32;
        self.plus = (flags & 2u32) != 0u32;
        self.zero = (flags & 4u32) != 0u32;
        old
    }

    pub fn pop_spec(&mut self, old: Spec) {
        self.fill = old.fill;
        self.align = old.align;
        self.width = old.width;
        self.precision = old.precision;
        self.alternate = (old.flags & 1u32) != 0u32;
        self.plus = (old.flags & 2u32) != 0u32;
        self.zero = (old.flags & 4u32) != 0u32;
    }

    fn clear_spec(&mut self) -> Spec {
        let alternate = self.alternate;
        self.push_spec(' ', 0u8, Option::None, Option::None, if alternate { 1u32 } else { 0u32 })
    }

    /// write!/writeln! on a Formatter (inherent, like std's write_fmt).
    pub fn __jr_write_buf(&mut self, buf: String) -> Result {
        self.write_str(buf.as_str())
    }

    pub fn write_str(&mut self, s: &str) -> Result {
        if self.indent > 0usize {
            // Pretty mode: indent after every newline.
            let (p, n) = super::string::str_parts(s);
            let mut i = 0usize;
            while i < n {
                let b = super::string::read_u8(p + i);
                self.buf.push_byte(b);
                if b == 10u8 {
                    self.write_indent();
                }
                i += 1usize;
            }
        } else {
            self.buf.push_str(s);
        }
        super::result::Result::Ok(())
    }

    fn write_indent(&mut self) {
        let mut k = 0usize;
        while k < self.indent {
            self.buf.push_str("    ");
            k += 1usize;
        }
    }

    pub fn write_char(&mut self, c: char) -> Result {
        self.buf.push(c);
        super::result::Result::Ok(())
    }

    fn padding(&mut self, count: usize) {
        let mut i = 0usize;
        while i < count {
            self.buf.push(self.fill);
            i += 1usize;
        }
    }

    /// Writes `s` honoring width, fill, alignment (default left) and
    /// precision (truncates to that many chars).
    pub fn pad(&mut self, s: &str) -> Result {
        let mut chars = char_count(s);
        let mut text = s;
        if let Option::Some(p) = self.precision {
            if p < chars {
                text = prefix_chars(s, p);
                chars = p;
            }
        }
        self.pad_aligned(text, chars, 1u8)
    }

    fn pad_aligned(&mut self, text: &str, chars: usize, default_align: u8) -> Result {
        let width = match self.width {
            Option::Some(w) => w,
            Option::None => 0usize,
        };
        if chars >= width {
            return self.write_str(text);
        }
        let total = width - chars;
        let align = if self.align == 0u8 { default_align } else { self.align };
        let (before, after) = if align == 1u8 {
            (0usize, total)
        } else if align == 2u8 {
            (total, 0usize)
        } else {
            (total / 2usize, total - total / 2usize)
        };
        self.padding(before);
        let _ = self.write_str(text);
        self.padding(after);
        super::result::Result::Ok(())
    }

    /// Writes digits with an optional sign/prefix, honoring width, `+`,
    /// zero padding and alignment (default right).
    pub fn pad_integral(&mut self, nonneg: bool, prefix: &str, digits: &str) -> Result {
        let mut sign = "";
        if !nonneg {
            sign = "-";
        } else if self.plus {
            sign = "+";
        }
        let pre = if self.alternate { prefix } else { "" };
        let len = char_count(sign) + char_count(pre) + char_count(digits);
        let width = match self.width {
            Option::Some(w) => w,
            Option::None => 0usize,
        };
        if self.zero && len < width {
            let _ = self.write_str(sign);
            let _ = self.write_str(pre);
            let mut i = len;
            while i < width {
                self.buf.push('0');
                i += 1usize;
            }
            return self.write_str(digits);
        }
        let mut s = String::new();
        s.push_str(sign);
        s.push_str(pre);
        s.push_str(digits);
        self.pad_aligned(s.as_str(), len, 2u8)
    }

    pub fn debug_struct(&mut self, name: &str) -> DebugStruct {
        let _ = self.write_str(name);
        DebugStruct {
            f: self as *mut Formatter as usize,
            fields: 0usize,
        }
    }

    pub fn debug_tuple(&mut self, name: &str) -> DebugTuple {
        let _ = self.write_str(name);
        DebugTuple {
            f: self as *mut Formatter as usize,
            fields: 0usize,
            empty_name: name_is_empty(name),
        }
    }

    pub fn debug_list(&mut self) -> DebugList {
        let _ = self.write_str("[");
        DebugList {
            f: self as *mut Formatter as usize,
            entries: 0usize,
            set: false,
        }
    }

    pub fn debug_set(&mut self) -> DebugList {
        let _ = self.write_str("{");
        DebugList {
            f: self as *mut Formatter as usize,
            entries: 0usize,
            set: true,
        }
    }

    pub fn debug_map(&mut self) -> DebugMap {
        let _ = self.write_str("{");
        DebugMap {
            f: self as *mut Formatter as usize,
            entries: 0usize,
        }
    }
}

fn name_is_empty(s: &str) -> bool {
    let (_, n) = super::string::str_parts(s);
    n == 0usize
}

pub fn char_count(s: &str) -> usize {
    let (p, n) = super::string::str_parts(s);
    let mut i = 0usize;
    let mut count = 0usize;
    while i < n {
        if (super::string::read_u8(p + i) & 192u8) != 128u8 {
            count += 1usize;
        }
        i += 1usize;
    }
    count
}

fn prefix_chars(s: &str, chars: usize) -> &str {
    let (p, n) = super::string::str_parts(s);
    let mut i = 0usize;
    let mut count = 0usize;
    while i < n {
        if (super::string::read_u8(p + i) & 192u8) != 128u8 {
            if count == chars {
                break;
            }
            count += 1usize;
        }
        i += 1usize;
    }
    super::string::str_from_raw_parts(p, i)
}

pub struct DebugStruct {
    f: usize,
    fields: usize,
}

impl DebugStruct {
    pub fn field<T: Debug>(self, name: &str, value: &T) -> DebugStruct {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate {
            if self.fields == 0usize {
                let _ = f.write_str(" {");
            }
            f.indent += 1usize;
            let _ = f.write_str("\n");
            let _ = f.write_str(name);
            let _ = f.write_str(": ");
            let old = f.clear_spec();
            let _ = Debug::fmt(value, f);
            f.pop_spec(old);
            let _ = f.write_str(",");
            f.indent -= 1usize;
        } else {
            let _ = f.write_str(if self.fields == 0usize { " { " } else { ", " });
            let _ = f.write_str(name);
            let _ = f.write_str(": ");
            let _ = Debug::fmt(value, f);
        }
        DebugStruct {
            f: self.f,
            fields: self.fields + 1usize,
        }
    }

    pub fn finish(self) -> Result {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if self.fields == 0usize {
            return super::result::Result::Ok(());
        }
        if f.alternate {
            let _ = f.write_str("\n");
            f.write_str("}")
        } else {
            f.write_str(" }")
        }
    }
}

pub struct DebugTuple {
    f: usize,
    fields: usize,
    empty_name: bool,
}

impl DebugTuple {
    pub fn field<T: Debug>(self, value: &T) -> DebugTuple {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate {
            if self.fields == 0usize {
                let _ = f.write_str("(");
            }
            f.indent += 1usize;
            let _ = f.write_str("\n");
            let old = f.clear_spec();
            let _ = Debug::fmt(value, f);
            f.pop_spec(old);
            let _ = f.write_str(",");
            f.indent -= 1usize;
        } else {
            let _ = f.write_str(if self.fields == 0usize { "(" } else { ", " });
            let _ = Debug::fmt(value, f);
        }
        DebugTuple {
            f: self.f,
            fields: self.fields + 1usize,
            empty_name: self.empty_name,
        }
    }

    pub fn finish(self) -> Result {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if self.fields == 0usize {
            return super::result::Result::Ok(());
        }
        if f.alternate {
            let _ = f.write_str("\n");
        } else if self.fields == 1usize && self.empty_name {
            let _ = f.write_str(",");
        }
        f.write_str(")")
    }
}

pub struct DebugList {
    f: usize,
    entries: usize,
    set: bool,
}

impl DebugList {
    pub fn entry<T: Debug>(self, value: &T) -> DebugList {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate {
            f.indent += 1usize;
            let _ = f.write_str("\n");
            let _ = Debug::fmt(value, f);
            let _ = f.write_str(",");
            f.indent -= 1usize;
        } else {
            if self.entries > 0usize {
                let _ = f.write_str(", ");
            }
            let _ = Debug::fmt(value, f);
        }
        DebugList {
            f: self.f,
            entries: self.entries + 1usize,
            set: self.set,
        }
    }

    pub fn finish(self) -> Result {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate && self.entries > 0usize {
            let _ = f.write_str("\n");
        }
        f.write_str(if self.set { "}" } else { "]" })
    }
}

pub struct DebugMap {
    f: usize,
    entries: usize,
}

impl DebugMap {
    pub fn entry<K: Debug, V: Debug>(self, key: &K, value: &V) -> DebugMap {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate {
            f.indent += 1usize;
            let _ = f.write_str("\n");
            let _ = Debug::fmt(key, f);
            let _ = f.write_str(": ");
            let _ = Debug::fmt(value, f);
            let _ = f.write_str(",");
            f.indent -= 1usize;
        } else {
            if self.entries > 0usize {
                let _ = f.write_str(", ");
            }
            let _ = Debug::fmt(key, f);
            let _ = f.write_str(": ");
            let _ = Debug::fmt(value, f);
        }
        DebugMap {
            f: self.f,
            entries: self.entries + 1usize,
        }
    }

    pub fn finish(self) -> Result {
        let fp = self.f as *mut Formatter;
        let f = unsafe { &mut *fp };
        if f.alternate && self.entries > 0usize {
            let _ = f.write_str("\n");
        }
        f.write_str("}")
    }
}

// ---- integers ----

fn fmt_u128(f: &mut Formatter, nonneg: bool, mut n: u128) -> Result {
    let mut s = String::new();
    let mut digits = [0u8; 40];
    let mut i = 0usize;
    loop {
        digits[i] = (n % 10u128) as u8;
        i += 1usize;
        n = n / 10u128;
        if n == 0u128 {
            break;
        }
    }
    while i > 0usize {
        i -= 1usize;
        s.push_byte(48u8 + digits[i]);
    }
    f.pad_integral(nonneg, "", s.as_str())
}

fn fmt_radix(f: &mut Formatter, mut n: u128, radix: u128, upper: bool, prefix: &str) -> Result {
    let mut s = String::new();
    let mut digits = [0u8; 130];
    let mut i = 0usize;
    loop {
        digits[i] = (n % radix) as u8;
        i += 1usize;
        n = n / radix;
        if n == 0u128 {
            break;
        }
    }
    while i > 0usize {
        i -= 1usize;
        let d = digits[i];
        let c = if d < 10u8 {
            48u8 + d
        } else if upper {
            55u8 + d
        } else {
            87u8 + d
        };
        s.push_byte(c);
    }
    f.pad_integral(true, prefix, s.as_str())
}

pub fn fmt_unsigned(f: &mut Formatter, n: u128) -> Result {
    fmt_u128(f, true, n)
}

pub fn fmt_signed(f: &mut Formatter, n: i128) -> Result {
    if n < 0i128 {
        // Magnitude without overflow, correct for i128::MIN.
        let magnitude = ((-(n + 1i128)) as u128) + 1u128;
        fmt_u128(f, false, magnitude)
    } else {
        fmt_u128(f, true, n as u128)
    }
}

pub fn fmt_lower_hex(f: &mut Formatter, n: u128) -> Result {
    fmt_radix(f, n, 16u128, false, "0x")
}

pub fn fmt_upper_hex(f: &mut Formatter, n: u128) -> Result {
    fmt_radix(f, n, 16u128, true, "0x")
}

pub fn fmt_binary(f: &mut Formatter, n: u128) -> Result {
    fmt_radix(f, n, 2u128, false, "0b")
}

pub fn fmt_octal(f: &mut Formatter, n: u128) -> Result {
    fmt_radix(f, n, 8u128, false, "0o")
}

impl Display for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Display for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Display for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Display for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Display for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self)
    }
}
impl Display for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Display for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Display for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Display for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Display for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Display for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self)
    }
}
impl Display for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}

impl Debug for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Debug for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Debug for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Debug for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Debug for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self)
    }
}
impl Debug for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_unsigned(f, *self as u128)
    }
}
impl Debug for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Debug for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Debug for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Debug for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}
impl Debug for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self)
    }
}
impl Debug for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed(f, *self as i128)
    }
}

// ---- bool, char, unit, String ----

impl Display for bool {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad(if *self { "true" } else { "false" })
    }
}
impl Debug for bool {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad(if *self { "true" } else { "false" })
    }
}

impl Display for char {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut s = String::new();
        s.push(*self);
        f.pad(s.as_str())
    }
}

pub fn escape_char_into(s: &mut String, c: char, quote: char) {
    if c == '\n' {
        s.push_str("\\n");
    } else if c == '\r' {
        s.push_str("\\r");
    } else if c == '\t' {
        s.push_str("\\t");
    } else if c == '\\' {
        s.push_str("\\\\");
    } else if c == '\0' {
        s.push_str("\\0");
    } else if c == quote {
        s.push('\\');
        s.push(c);
    } else {
        s.push(c);
    }
}

impl Debug for char {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut s = String::new();
        s.push('\'');
        escape_char_into(&mut s, *self, '\'');
        s.push('\'');
        f.write_str(s.as_str())
    }
}

pub fn debug_str(f: &mut Formatter, text: &str) -> Result {
    let mut s = String::new();
    s.push('"');
    let (p, n) = super::string::str_parts(text);
    let mut i = 0usize;
    while i < n {
        let c = super::string::decode_utf8_at(p, i);
        escape_char_into(&mut s, c, '"');
        i += super::string::utf8_width(super::string::read_u8(p + i));
    }
    s.push('"');
    f.write_str(s.as_str())
}

impl Display for () {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad("()")
    }
}
impl Debug for () {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad("()")
    }
}

impl Display for String {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad(self.as_str())
    }
}
impl Debug for String {
    fn fmt(&self, f: &mut Formatter) -> Result {
        debug_str(f, self.as_str())
    }
}

// ---- floats ----

fn push_u64(s: &mut String, mut n: u64) {
    let mut digits = [0u8; 20];
    let mut i = 0usize;
    loop {
        digits[i] = (n % 10u64) as u8;
        i += 1usize;
        n = n / 10u64;
        if n == 0u64 {
            break;
        }
    }
    while i > 0usize {
        i -= 1usize;
        s.push_byte(48u8 + digits[i]);
    }
}

fn pow10(n: usize) -> f64 {
    let mut r = 1.0f64;
    let mut i = 0usize;
    while i < n {
        r = r * 10.0f64;
        i += 1usize;
    }
    r
}

/// Fixed-precision decimal with round-half-to-even on the scaled value.
fn push_fixed(s: &mut String, x: f64, precision: usize) {
    let scale = pow10(precision);
    let scaled = x * scale;
    let mut whole = scaled as u64;
    let frac = scaled - (whole as f64);
    if frac > 0.5f64 || (frac == 0.5f64 && (whole % 2u64) == 1u64) {
        whole += 1u64;
    }
    let unit = scale as u64;
    let int_part = whole / unit;
    push_u64(s, int_part);
    if precision > 0usize {
        s.push('.');
        let mut rest = whole % unit;
        let mut digits = [0u8; 20];
        let mut i = 0usize;
        while i < precision {
            digits[i] = (rest % 10u64) as u8;
            rest = rest / 10u64;
            i += 1usize;
        }
        while i > 0usize {
            i -= 1usize;
            s.push_byte(48u8 + digits[i]);
        }
    }
}

/// Shortest decimal that round-trips through f64 (Rust's `{}` for floats),
/// limited to 17 significant digits.
fn push_shortest(s: &mut String, x: f64) {
    if x == 0.0f64 {
        s.push('0');
        return;
    }
    let mut precision = 0usize;
    while precision < 17usize {
        let scale = pow10(precision);
        let scaled = x * scale;
        let rounded = round_half_even(scaled);
        if rounded / scale == x {
            break;
        }
        precision += 1usize;
    }
    push_fixed(s, x, precision);
}

fn round_half_even(x: f64) -> f64 {
    let whole = x as u64;
    let frac = x - (whole as f64);
    let mut r = whole;
    if frac > 0.5f64 || (frac == 0.5f64 && (whole % 2u64) == 1u64) {
        r += 1u64;
    }
    r as f64
}

pub fn fmt_float(f: &mut Formatter, x: f64, debug: bool) -> Result {
    if x != x {
        return f.pad("NaN");
    }
    let negative = x < 0.0f64 || (x == 0.0f64 && 1.0f64 / x < 0.0f64);
    let magnitude = if negative { -x } else { x };
    let mut s = String::new();
    if magnitude == 1.0f64 / 0.0f64 {
        s.push_str("inf");
    } else if magnitude >= 1e16f64 {
        // Large values: integer digits through u128.
        let mut digits = String::new();
        let mut n = magnitude as u128;
        let mut buf = [0u8; 40];
        let mut i = 0usize;
        loop {
            buf[i] = (n % 10u128) as u8;
            i += 1usize;
            n = n / 10u128;
            if n == 0u128 {
                break;
            }
        }
        while i > 0usize {
            i -= 1usize;
            digits.push_byte(48u8 + buf[i]);
        }
        s.push_str(digits.as_str());
        match f.precision {
            Option::Some(p) => {
                if p > 0usize {
                    s.push('.');
                    let mut k = 0usize;
                    while k < p {
                        s.push('0');
                        k += 1usize;
                    }
                }
            }
            Option::None => {
                if debug {
                    s.push_str(".0");
                }
            }
        }
    } else {
        match f.precision {
            Option::Some(p) => push_fixed(&mut s, magnitude, p),
            Option::None => {
                push_shortest(&mut s, magnitude);
                if debug && magnitude == ((magnitude as u64) as f64) {
                    s.push_str(".0");
                }
            }
        }
    }
    f.pad_integral(!negative, "", s.as_str())
}

impl Display for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_float(f, *self, false)
    }
}
impl Debug for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_float(f, *self, true)
    }
}
impl Display for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_float(f, *self as f64, false)
    }
}
impl Debug for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_float(f, *self as f64, true)
    }
}

// ---- containers ----

// Tuples format like an anonymous tuple struct: `(a, b)`, with a trailing
// comma for one element, matching std (arity 1-4, as cmp.rs).
impl<A: Debug> Debug for (A,) {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.debug_tuple("").field(&(*self).0).finish()
    }
}
impl<A: Debug, B: Debug> Debug for (A, B) {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.debug_tuple("")
            .field(&(*self).0)
            .field(&(*self).1)
            .finish()
    }
}
impl<A: Debug, B: Debug, C: Debug> Debug for (A, B, C) {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.debug_tuple("")
            .field(&(*self).0)
            .field(&(*self).1)
            .field(&(*self).2)
            .finish()
    }
}
impl<A: Debug, B: Debug, C: Debug, D: Debug> Debug for (A, B, C, D) {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.debug_tuple("")
            .field(&(*self).0)
            .field(&(*self).1)
            .field(&(*self).2)
            .field(&(*self).3)
            .finish()
    }
}

impl<T: Debug> Debug for super::vec::Vec<T> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut list = f.debug_list();
        let mut i = 0usize;
        while i < self.len() {
            list = list.entry(self.get_ref(i));
            i += 1usize;
        }
        list.finish()
    }
}

impl<T: Debug> Debug for [T] {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut list = f.debug_list();
        let mut i = 0usize;
        while i < self.len() {
            list = list.entry::<T>(&self[i]);
            i += 1usize;
        }
        list.finish()
    }
}

impl<T: Debug> Debug for Option<T> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            Option::Some(v) => f.debug_tuple("Some").field(v).finish(),
            Option::None => f.write_str("None"),
        }
    }
}

impl<T: Debug, E: Debug> Debug for super::result::Result<T, E> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            super::result::Result::Ok(v) => f.debug_tuple("Ok").field(v).finish(),
            super::result::Result::Err(e) => f.debug_tuple("Err").field(e).finish(),
        }
    }
}

// ---- ToString ----

pub fn to_string_display<T: Display>(value: &T) -> String {
    let mut f = Formatter::new();
    let _ = Display::fmt(value, &mut f);
    f.into_string()
}

// ---- output (println!/print!/eprintln!/eprint! lowering target) ----

/// Write the formatted buffer to `fd` (1 stdout, 2 stderr).
pub fn print_formatter(f: Formatter, fd: usize) {
    let (p, n) = super::string::str_parts(f.as_str());
    let mut done = 0usize;
    while done < n {
        let wrote = unsafe {
            __justrust_linux_syscall6!(1usize, fd, p + done, n - done, 0usize, 0usize, 0usize)
        };
        if wrote == 0usize || wrote > n {
            break;
        }
        done += wrote;
    }
}

impl Display for str {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.pad(self)
    }
}
impl Debug for str {
    fn fmt(&self, f: &mut Formatter) -> Result {
        debug_str(f, self)
    }
}
impl<T: Display + ?Sized> Display for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Display::fmt(*self, f)
    }
}
impl<T: Debug + ?Sized> Debug for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Debug::fmt(*self, f)
    }
}
impl<T: Display + ?Sized> Display for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Display::fmt(&**self, f)
    }
}
impl<T: Debug + ?Sized> Debug for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Debug::fmt(&**self, f)
    }
}

// ---- format-argument entry points (println!/print!/eprint! lowering) ----
// The compiler lowers each `{}` / `{:?}` placeholder whose type has no native
// formatter (or that carries a spec) to one call of these traits. `width` and
// `precision` use usize::MAX for "absent". `align`: 0 none, 1 <, 2 >, 3 ^.
// `flags`: bit0 '#', bit1 '+', bit2 '0'.

fn arg_formatter(fill: char, align: u8, width: usize, precision: usize, flags: u32) -> Formatter {
    let mut f = Formatter::new();
    let mut w: Option<usize> = Option::None;
    if width != 18446744073709551615usize {
        w = Option::Some(width);
    }
    let mut p: Option<usize> = Option::None;
    if precision != 18446744073709551615usize {
        p = Option::Some(precision);
    }
    let _ = f.push_spec(fill, align, w, p, flags);
    f
}

pub trait DisplayArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Display + ?Sized> DisplayArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        print_formatter(f, fd);
    }
}

pub trait SliceDebugArg {
    fn __jr_fmt_slice(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Debug> SliceDebugArg for [T] {
    fn __jr_fmt_slice(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        print_formatter(f, fd);
    }
}

pub trait DebugArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Debug + ?Sized> DebugArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        print_formatter(f, fd);
    }
}

// `str` cannot yet instantiate the blanket impls (no unsized generic
// arguments), so string slices use these concrete entry points.
pub trait StrDisplayArg {
    fn __jr_fmt_str(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl StrDisplayArg for str {
    fn __jr_fmt_str(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = f.pad(self);
        print_formatter(f, fd);
    }
}
pub trait StrDebugArg {
    fn __jr_fmt_str(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl StrDebugArg for str {
    fn __jr_fmt_str(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = debug_str(&mut f, self);
        print_formatter(f, fd);
    }
}

impl Debug for super::cmp::Ordering {
    fn fmt(&self, f: &mut Formatter) -> Result {
        f.write_str(match self {
            super::cmp::Ordering::Less => "Less",
            super::cmp::Ordering::Equal => "Equal",
            super::cmp::Ordering::Greater => "Greater",
        })
    }
}

// ---- format!/write!/writeln! entry points ----
// Each placeholder of format!/write! appends `prefix`, the formatted value,
// then `suffix` to the macro's private String. Spec encoding matches *Arg.

pub trait DisplayInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Display + ?Sized> DisplayInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}

pub trait DebugInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Debug + ?Sized> DebugInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}

pub trait StrDisplayInto {
    fn __jr_fmt_into_str(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl StrDisplayInto for str {
    fn __jr_fmt_into_str(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = f.pad(self);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}

pub trait StrDebugInto {
    fn __jr_fmt_into_str(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl StrDebugInto for str {
    fn __jr_fmt_into_str(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = debug_str(&mut f, self);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}

/// format! result: a call (not a block) so method chains type normally.
pub fn __jr_format(buf: String) -> String {
    buf
}

impl Write for Formatter {
    fn write_str(&mut self, s: &str) -> Result {
        Formatter::write_str(self, s)
    }
}

impl Write for String {
    fn write_str(&mut self, s: &str) -> Result {
        self.push_str(s);
        Result::Ok(())
    }
}
