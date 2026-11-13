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
// Float printing works on exact decimal expansions. A finite float is
// `m * 2^e`. For `e < 0` that equals `m * 5^-e / 10^-e`, so the decimal
// digits of one big integer plus a decimal point position describe the value
// exactly. Fixed and exponent precision round that expansion half-to-even,
// and the shortest form searches the digit prefixes of the value between the
// exact midpoints to the neighbouring floats of the same width.
// Everything lives in fixed stack buffers: the longest f64 expansion
// (`(4m + 2) * 5^1076`) has 770 digits and 80 limbs.

/// Decimal digits (most significant first, values 0-9) with `point` digits
/// before the decimal point (`point <= 0` means leading fractional zeros).
struct Decimal {
    digits: [u8; 800],
    len: usize,
    point: i64,
}

fn big_mul_small(b: &mut [u32], len: usize, m: u32) -> usize {
    let mut carry = 0u64;
    let mut i = 0usize;
    while i < len {
        let v = (b[i] as u64) * (m as u64) + carry;
        b[i] = v as u32;
        carry = v >> 32u32;
        i += 1usize;
    }
    if carry != 0u64 {
        b[len] = carry as u32;
        return len + 1usize;
    }
    len
}

/// Writes the decimal digits of `x * 2^e` for `e >= 0`, or of `x * 5^-e`
/// for `e < 0` (the value scaled by `10^-e`), most significant first.
/// Returns the digit count, zero for a zero value.
fn scaled_digits(x: u64, e: i32, out: &mut [u8]) -> usize {
    let mut b = [0u32; 84];
    let mut len = 0usize;
    if e >= 0i32 {
        let words = (e / 32i32) as usize;
        let bits = (e % 32i32) as u32;
        // x < 2^64 shifted by under 32 bits spans three limbs.
        let wide = (x as u128) << bits;
        b[words] = wide as u32;
        b[words + 1usize] = (wide >> 32u32) as u32;
        b[words + 2usize] = (wide >> 64u32) as u32;
        len = words + 3usize;
    } else {
        b[0usize] = x as u32;
        b[1usize] = (x >> 32u32) as u32;
        len = 2usize;
        let mut n = (0i32 - e) as usize;
        while n >= 13usize {
            len = big_mul_small(&mut b, len, 1220703125u32);
            n -= 13usize;
        }
        let mut small = 1u32;
        while n > 0usize {
            small = small * 5u32;
            n -= 1usize;
        }
        len = big_mul_small(&mut b, len, small);
    }
    while len > 0usize && b[len - 1usize] == 0u32 {
        len -= 1usize;
    }
    // Nine decimal digits per division, least significant first.
    let mut count = 0usize;
    while len > 0usize {
        let mut rem = 0u64;
        let mut i = len;
        while i > 0usize {
            i -= 1usize;
            let v = (rem << 32u32) | (b[i] as u64);
            b[i] = (v / 1000000000u64) as u32;
            rem = v % 1000000000u64;
        }
        while len > 0usize && b[len - 1usize] == 0u32 {
            len -= 1usize;
        }
        let mut chunk = rem as u32;
        let mut j = 0usize;
        while j < 9usize {
            out[count] = (chunk % 10u32) as u8;
            count += 1usize;
            chunk = chunk / 10u32;
            j += 1usize;
        }
    }
    while count > 0usize && out[count - 1usize] == 0u8 {
        count -= 1usize;
    }
    let mut lo = 0usize;
    let mut hi = count;
    while lo + 1usize < hi {
        hi -= 1usize;
        let t = out[lo];
        out[lo] = out[hi];
        out[hi] = t;
        lo += 1usize;
    }
    count
}

/// Right-aligns `len` digits to `width` digits with leading zeros.
fn align_right(d: &mut [u8], len: usize, width: usize) {
    let shift = width - len;
    if shift == 0usize {
        return;
    }
    let mut i = len;
    while i > 0usize {
        i -= 1usize;
        d[i + shift] = d[i];
    }
    let mut j = 0usize;
    while j < shift {
        d[j] = 0u8;
        j += 1usize;
    }
}

/// Compares the first `n` digits: 0 less, 1 equal, 2 greater.
fn cmp_prefix(a: &[u8], b: &[u8], n: usize) -> u8 {
    let mut i = 0usize;
    while i < n {
        if a[i] != b[i] {
            return if a[i] < b[i] { 0u8 } else { 2u8 };
        }
        i += 1usize;
    }
    1u8
}

fn rest_is_zero(d: &[u8], from: usize, len: usize) -> bool {
    let mut i = from;
    while i < len {
        if d[i] != 0u8 {
            return false;
        }
        i += 1usize;
    }
    true
}

fn zero_decimal() -> Decimal {
    Decimal { digits: [0u8; 800], len: 1usize, point: 1i64 }
}

/// Exact decimal expansion of `m * 2^e`.
fn exact_decimal(m: u64, e: i32) -> Decimal {
    let mut d = zero_decimal();
    if m == 0u64 {
        return d;
    }
    d.len = scaled_digits(m, e, &mut d.digits);
    let k: i64 = if e < 0i32 { 0i64 - (e as i64) } else { 0i64 };
    d.point = (d.len as i64) - k;
    d
}

/// Shortest decimal that reads back as `m * 2^e` in a float format whose
/// neighbours are one unit of `m` away (half a unit below when
/// `lower_closer`), choosing the candidate closest to the exact value.
fn shortest_decimal(m: u64, e: i32, lower_closer: bool) -> Decimal {
    let mut out = zero_decimal();
    if m == 0u64 {
        return out;
    }
    // Interval bounds are inclusive when the mantissa is even, because
    // reading a midpoint rounds to the even neighbour.
    let even = m % 2u64 == 0u64;
    let e2 = e - 2i32;
    let k: i64 = if e2 < 0i32 { 0i64 - (e2 as i64) } else { 0i64 };
    let below = if lower_closer { 1u64 } else { 2u64 };
    let mut high = [0u8; 800];
    let mut value = [0u8; 800];
    let mut low = [0u8; 800];
    let high_len = scaled_digits(m * 4u64 + 2u64, e2, &mut high);
    let width = high_len + 1usize;
    align_right(&mut high, high_len, width);
    let value_len = scaled_digits(m * 4u64, e2, &mut value);
    align_right(&mut value, value_len, width);
    let low_len = scaled_digits(m * 4u64 - below, e2, &mut low);
    align_right(&mut low, low_len, width);
    let mut up = [0u8; 800];
    let mut n = 1usize;
    loop {
        let c = cmp_prefix(&value, &low, n);
        let down_ok = c == 2u8 || (c == 1u8 && even && rest_is_zero(&low, n, width));
        // `up` is the n digit prefix of the value plus one in its last place.
        let mut i = 0usize;
        while i < n {
            up[i] = value[i];
            i += 1usize;
        }
        let mut j = n;
        while j > 0usize {
            j -= 1usize;
            if up[j] == 9u8 {
                up[j] = 0u8;
            } else {
                up[j] += 1u8;
                break;
            }
        }
        let c = cmp_prefix(&up, &high, n);
        let up_ok = c == 0u8 || (c == 1u8 && (even || !rest_is_zero(&high, n, width)));
        if down_ok || up_ok {
            let take_up = !down_ok || (up_ok && n < width && value[n] >= 5u8);
            let mut start = 0usize;
            let mut len = 0usize;
            let mut i = 0usize;
            while i < n {
                let d = if take_up { up[i] } else { value[i] };
                if d != 0u8 && len == 0usize {
                    start = i;
                }
                if d != 0u8 || len > 0usize {
                    out.digits[len] = d;
                    len += 1usize;
                }
                i += 1usize;
            }
            while len > 1usize && out.digits[len - 1usize] == 0u8 {
                len -= 1usize;
            }
            out.len = len;
            out.point = (width as i64) - (start as i64) - k;
            return out;
        }
        n += 1usize;
    }
}

/// Rounds `len` digits to their first `cut >= 1` digits (`cut < len`),
/// half-to-even on the dropped tail. Returns true when the carry ran off
/// the top (the kept digits are then all zero and stand for a one followed
/// by `cut` zeros).
fn round_digits(d: &mut [u8], len: usize, cut: usize) -> bool {
    let first = d[cut];
    let up = first > 5u8
        || (first == 5u8 && (!rest_is_zero(d, cut + 1usize, len) || d[cut - 1usize] % 2u8 == 1u8));
    if !up {
        return false;
    }
    let mut j = cut;
    while j > 0usize {
        j -= 1usize;
        if d[j] == 9u8 {
            d[j] = 0u8;
        } else {
            d[j] += 1u8;
            return false;
        }
    }
    true
}

fn push_zeros(s: &mut String, n: usize) {
    let mut i = 0usize;
    while i < n {
        s.push_byte(48u8);
        i += 1usize;
    }
}

fn push_digit_range(s: &mut String, d: &[u8], from: usize, to: usize) {
    let mut i = from;
    while i < to {
        s.push_byte(48u8 + d[i]);
        i += 1usize;
    }
}

/// `{:.N}`: exactly `precision` fractional digits.
fn push_fixed(s: &mut String, mut value: Decimal, precision: usize) {
    // Index of the first digit that does not fit.
    let cut = value.point + (precision as i64);
    if cut <= 0i64 {
        // Every digit is dropped. Only a value above half of the last place
        // rounds up (a tie rounds to the even zero).
        let up = cut == 0i64
            && (value.digits[0usize] > 5u8
                || (value.digits[0usize] == 5u8
                    && !rest_is_zero(&value.digits, 1usize, value.len)));
        value.digits[0usize] = if up { 1u8 } else { 0u8 };
        value.len = 1usize;
        value.point = if up { 1i64 - (precision as i64) } else { 1i64 };
    } else if (cut as usize) < value.len {
        let cut = cut as usize;
        if round_digits(&mut value.digits, value.len, cut) {
            value.digits[0usize] = 1u8;
            value.point += 1i64;
        }
        value.len = cut;
    }
    let len = value.len;
    if value.point <= 0i64 {
        s.push_byte(48u8);
    } else {
        let whole = value.point as usize;
        if len <= whole {
            push_digit_range(s, &value.digits, 0usize, len);
            push_zeros(s, whole - len);
        } else {
            push_digit_range(s, &value.digits, 0usize, whole);
        }
    }
    if precision == 0usize {
        return;
    }
    s.push('.');
    let mut written = 0usize;
    if value.point < 0i64 {
        let mut zeros = (0i64 - value.point) as usize;
        if zeros > precision {
            zeros = precision;
        }
        push_zeros(s, zeros);
        written = zeros;
    }
    let from = if value.point > 0i64 { value.point as usize } else { 0usize };
    if from < len && written < precision {
        let mut to = from + (precision - written);
        if to > len {
            to = len;
        }
        push_digit_range(s, &value.digits, from, to);
        written += to - from;
    }
    push_zeros(s, precision - written);
}

/// Positional form of every digit, `.0` appended to integers for Debug.
fn push_positional(s: &mut String, value: Decimal, debug: bool) {
    let len = value.len;
    if value.point <= 0i64 {
        s.push_str("0.");
        push_zeros(s, (0i64 - value.point) as usize);
        push_digit_range(s, &value.digits, 0usize, len);
        return;
    }
    let whole = value.point as usize;
    if len <= whole {
        push_digit_range(s, &value.digits, 0usize, len);
        push_zeros(s, whole - len);
        if debug {
            s.push_str(".0");
        }
        return;
    }
    push_digit_range(s, &value.digits, 0usize, whole);
    s.push('.');
    push_digit_range(s, &value.digits, whole, len);
}

/// `d.ddde±x`. With a precision the digits are rounded or zero-extended to
/// `precision + 1` places. `value` has no leading zero unless it is zero.
fn push_exponent(s: &mut String, mut value: Decimal, precision: Option<usize>, upper: bool) {
    let mut exp = value.point - 1i64;
    let mut len = value.len;
    match precision {
        Option::Some(p) => {
            if p + 1usize < len {
                if round_digits(&mut value.digits, len, p + 1usize) {
                    value.digits[0usize] = 1u8;
                    exp += 1i64;
                }
                len = p + 1usize;
            }
        }
        Option::None => {
            while len > 1usize && value.digits[len - 1usize] == 0u8 {
                len -= 1usize;
            }
        }
    }
    s.push_byte(48u8 + value.digits[0usize]);
    let places = match precision {
        Option::Some(p) => p,
        Option::None => len - 1usize,
    };
    if places > 0usize {
        s.push('.');
        push_digit_range(s, &value.digits, 1usize, len);
        push_zeros(s, places - (len - 1usize));
    }
    s.push_byte(if upper { 69u8 } else { 101u8 });
    if exp < 0i64 {
        s.push('-');
        exp = 0i64 - exp;
    }
    let mut buf = [0u8; 20];
    let mut i = 0usize;
    let mut n = exp as u64;
    loop {
        buf[i] = (n % 10u64) as u8;
        i += 1usize;
        n = n / 10u64;
        if n == 0u64 {
            break;
        }
    }
    while i > 0usize {
        i -= 1usize;
        s.push_byte(48u8 + buf[i]);
    }
}

/// Formats the finite float `m * 2^e` (or an infinity or NaN).
/// `class`: 0 finite, 1 infinite, 2 NaN. `mode`: 0 Display, 1 Debug,
/// 2 LowerExp, 3 UpperExp.
fn fmt_float_parts(
    f: &mut Formatter,
    negative: bool,
    class: u8,
    m: u64,
    e: i32,
    lower_closer: bool,
    mode: u8,
) -> Result {
    if class == 2u8 {
        // NaN never shows a sign, but still honors width and zero padding.
        let plus = f.plus;
        f.plus = false;
        let r = f.pad_integral(true, "", "NaN");
        f.plus = plus;
        return r;
    }
    if class == 1u8 {
        return f.pad_integral(!negative, "", "inf");
    }
    let mut s = String::new();
    let precision = f.precision;
    if mode >= 2u8 {
        let value = match precision {
            Option::Some(_) => exact_decimal(m, e),
            Option::None => shortest_decimal(m, e, lower_closer),
        };
        push_exponent(&mut s, value, precision, mode == 3u8);
    } else {
        match precision {
            Option::Some(p) => {
                push_fixed(&mut s, exact_decimal(m, e), p);
            }
            Option::None => {
                let value = shortest_decimal(m, e, lower_closer);
                // Debug switches to exponent form outside [1e-4, 1e16).
                let exp = value.point - 1i64;
                if mode == 1u8 && m != 0u64 && (exp < -4i64 || exp >= 16i64) {
                    push_exponent(&mut s, value, Option::None, false);
                } else {
                    push_positional(&mut s, value, mode == 1u8);
                }
            }
        }
    }
    f.pad_integral(!negative, "", s.as_str())
}

fn fmt_f64(f: &mut Formatter, x: f64, mode: u8) -> Result {
    let p = &x as *const f64 as *const u64;
    let bits = unsafe { *p };
    let negative = (bits >> 63u32) != 0u64;
    let field = ((bits >> 52u32) & 2047u64) as i32;
    let fraction = bits & 4503599627370495u64;
    if field == 2047i32 {
        let class = if fraction == 0u64 { 1u8 } else { 2u8 };
        return fmt_float_parts(f, negative, class, 0u64, 0i32, false, mode);
    }
    if field == 0i32 {
        return fmt_float_parts(f, negative, 0u8, fraction, -1074i32, false, mode);
    }
    let m = fraction | 4503599627370496u64;
    fmt_float_parts(f, negative, 0u8, m, field - 1075i32, fraction == 0u64 && field > 1i32, mode)
}

fn fmt_f32(f: &mut Formatter, x: f32, mode: u8) -> Result {
    let p = &x as *const f32 as *const u32;
    let bits = unsafe { *p };
    let negative = (bits >> 31u32) != 0u32;
    let field = ((bits >> 23u32) & 255u32) as i32;
    let fraction = (bits & 8388607u32) as u64;
    if field == 255i32 {
        let class = if fraction == 0u64 { 1u8 } else { 2u8 };
        return fmt_float_parts(f, negative, class, 0u64, 0i32, false, mode);
    }
    if field == 0i32 {
        return fmt_float_parts(f, negative, 0u8, fraction, -149i32, false, mode);
    }
    let m = fraction | 8388608u64;
    fmt_float_parts(f, negative, 0u8, m, field - 150i32, fraction == 0u64 && field > 1i32, mode)
}

/// `{:e}` / `{:E}` of an integer magnitude: every digit is significant,
/// trailing zeros are dropped unless a precision asks for them.
fn fmt_int_exp(f: &mut Formatter, nonneg: bool, mut n: u128, upper: bool) -> Result {
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
    let mut value = zero_decimal();
    value.len = i;
    value.point = i as i64;
    let mut k = 0usize;
    while i > 0usize {
        i -= 1usize;
        value.digits[k] = buf[i];
        k += 1usize;
    }
    let mut s = String::new();
    let precision = f.precision;
    push_exponent(&mut s, value, precision, upper);
    f.pad_integral(nonneg, "", s.as_str())
}

fn fmt_signed_exp(f: &mut Formatter, n: i128, upper: bool) -> Result {
    if n < 0i128 {
        fmt_int_exp(f, false, ((-(n + 1i128)) as u128) + 1u128, upper)
    } else {
        fmt_int_exp(f, true, n as u128, upper)
    }
}

pub trait LowerExp {
    fn fmt(&self, f: &mut Formatter) -> Result;
}

pub trait UpperExp {
    fn fmt(&self, f: &mut Formatter) -> Result;
}

impl Display for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f64(f, *self, 0u8)
    }
}
impl Debug for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f64(f, *self, 1u8)
    }
}
impl LowerExp for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f64(f, *self, 2u8)
    }
}
impl UpperExp for f64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f64(f, *self, 3u8)
    }
}
impl Display for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f32(f, *self, 0u8)
    }
}
impl Debug for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f32(f, *self, 1u8)
    }
}
impl LowerExp for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f32(f, *self, 2u8)
    }
}
impl UpperExp for f32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_f32(f, *self, 3u8)
    }
}
impl LowerExp for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, false)
    }
}
impl UpperExp for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_int_exp(f, true, *self as u128, true)
    }
}
impl LowerExp for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl LowerExp for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl LowerExp for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl LowerExp for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl LowerExp for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl LowerExp for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, false)
    }
}
impl UpperExp for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_signed_exp(f, *self as i128, true)
    }
}
impl<T: LowerExp + ?Sized> LowerExp for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        LowerExp::fmt(&**self, f)
    }
}
impl<T: UpperExp + ?Sized> UpperExp for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        UpperExp::fmt(&**self, f)
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

impl<T: Debug, const N: usize> Debug for [T; N] {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut list = f.debug_list();
        let mut i = 0usize;
        while i < N {
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

/// write!/writeln! private buffer. Reachable from no_std crates through
/// core::fmt; it never exposes an allocation API by a public name.
pub fn __jr_write_buffer() -> String {
    String::new()
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

// ---- `{:e}` / `{:E}` entry points (same spec encoding as *Arg / *Into) ----

pub trait LowerExpArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: LowerExp + ?Sized> LowerExpArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        print_formatter(f, fd);
    }
}

pub trait UpperExpArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: UpperExp + ?Sized> UpperExpArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        print_formatter(f, fd);
    }
}

pub trait LowerExpInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: LowerExp + ?Sized> LowerExpInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}

pub trait UpperExpInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: UpperExp + ?Sized> UpperExpInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = self.fmt(&mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}
// ---- radix formatting traits ----

pub trait LowerHex {
    fn fmt(&self, f: &mut Formatter) -> Result;
}
impl LowerHex for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128)
    }
}
impl LowerHex for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u8 as u128)
    }
}
impl LowerHex for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u16 as u128)
    }
}
impl LowerHex for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u32 as u128)
    }
}
impl LowerHex for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u64 as u128)
    }
}
impl LowerHex for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as u128 as u128)
    }
}
impl LowerHex for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_lower_hex(f, *self as usize as u128)
    }
}
impl<T: LowerHex + ?Sized> LowerHex for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        LowerHex::fmt(&**self, f)
    }
}
impl<T: LowerHex + ?Sized> LowerHex for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        LowerHex::fmt(&**self, f)
    }
}
pub trait LowerHexArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: LowerHex + ?Sized> LowerHexArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = LowerHex::fmt(self, &mut f);
        print_formatter(f, fd);
    }
}
pub trait LowerHexInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: LowerHex + ?Sized> LowerHexInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = LowerHex::fmt(self, &mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}
pub trait UpperHex {
    fn fmt(&self, f: &mut Formatter) -> Result;
}
impl UpperHex for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128)
    }
}
impl UpperHex for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u8 as u128)
    }
}
impl UpperHex for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u16 as u128)
    }
}
impl UpperHex for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u32 as u128)
    }
}
impl UpperHex for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u64 as u128)
    }
}
impl UpperHex for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as u128 as u128)
    }
}
impl UpperHex for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_upper_hex(f, *self as usize as u128)
    }
}
impl<T: UpperHex + ?Sized> UpperHex for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        UpperHex::fmt(&**self, f)
    }
}
impl<T: UpperHex + ?Sized> UpperHex for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        UpperHex::fmt(&**self, f)
    }
}
pub trait UpperHexArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: UpperHex + ?Sized> UpperHexArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = UpperHex::fmt(self, &mut f);
        print_formatter(f, fd);
    }
}
pub trait UpperHexInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: UpperHex + ?Sized> UpperHexInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = UpperHex::fmt(self, &mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}
pub trait Binary {
    fn fmt(&self, f: &mut Formatter) -> Result;
}
impl Binary for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128)
    }
}
impl Binary for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u8 as u128)
    }
}
impl Binary for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u16 as u128)
    }
}
impl Binary for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u32 as u128)
    }
}
impl Binary for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u64 as u128)
    }
}
impl Binary for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as u128 as u128)
    }
}
impl Binary for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_binary(f, *self as usize as u128)
    }
}
impl<T: Binary + ?Sized> Binary for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Binary::fmt(&**self, f)
    }
}
impl<T: Binary + ?Sized> Binary for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Binary::fmt(&**self, f)
    }
}
pub trait BinaryArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Binary + ?Sized> BinaryArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = Binary::fmt(self, &mut f);
        print_formatter(f, fd);
    }
}
pub trait BinaryInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Binary + ?Sized> BinaryInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = Binary::fmt(self, &mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}
pub trait Octal {
    fn fmt(&self, f: &mut Formatter) -> Result;
}
impl Octal for u8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for u16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for u32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for u64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for u128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for usize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128)
    }
}
impl Octal for i8 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u8 as u128)
    }
}
impl Octal for i16 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u16 as u128)
    }
}
impl Octal for i32 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u32 as u128)
    }
}
impl Octal for i64 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u64 as u128)
    }
}
impl Octal for i128 {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as u128 as u128)
    }
}
impl Octal for isize {
    fn fmt(&self, f: &mut Formatter) -> Result {
        fmt_octal(f, *self as usize as u128)
    }
}
impl<T: Octal + ?Sized> Octal for &T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Octal::fmt(&**self, f)
    }
}
impl<T: Octal + ?Sized> Octal for &mut T {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Octal::fmt(&**self, f)
    }
}
pub trait OctalArg {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Octal + ?Sized> OctalArg for T {
    fn __jr_fmt(&self, fd: usize, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = Octal::fmt(self, &mut f);
        print_formatter(f, fd);
    }
}
pub trait OctalInto {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32);
}
impl<T: Octal + ?Sized> OctalInto for T {
    fn __jr_fmt_into(&self, buf: &mut String, prefix: &str, suffix: &str, fill: char, align: u8, width: usize, precision: usize, flags: u32) {
        let mut f = arg_formatter(fill, align, width, precision, flags);
        let _ = Octal::fmt(self, &mut f);
        buf.push_str(prefix);
        buf.push_str(f.as_str());
        buf.push_str(suffix);
    }
}
