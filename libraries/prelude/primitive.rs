// Inherent methods of the primitive types that are not compiler intrinsics,
// as ordinary Rust source. They are trait methods (traits injected by the
// prelude) because primitive inherent impls are reserved to the compiler.
use super::option::Option;
use super::result::Result;

// ---- char ----

pub trait CharMethods {
    fn to_digit(self, radix: u32) -> Option<u32>;
    fn is_digit(self, radix: u32) -> bool;
    fn is_alphabetic(self) -> bool;
    fn is_numeric(self) -> bool;
    fn is_alphanumeric(self) -> bool;
    fn is_lowercase(self) -> bool;
    fn is_uppercase(self) -> bool;
    fn len_utf8(self) -> usize;
    fn eq_ignore_ascii_case(&self, other: &char) -> bool;
}

impl CharMethods for char {
    fn to_digit(self, radix: u32) -> Option<u32> {
        if radix < 2u32 || radix > 36u32 {
            panic!("to_digit: invalid radix -- radix must be in the range 2 to 36 inclusive");
        }
        let c = self as u32;
        let d = if c >= 48u32 && c <= 57u32 {
            c - 48u32
        } else if c >= 97u32 && c <= 122u32 {
            c - 97u32 + 10u32
        } else if c >= 65u32 && c <= 90u32 {
            c - 65u32 + 10u32
        } else {
            return Option::None;
        };
        if d < radix { Option::Some(d) } else { Option::None }
    }
    fn is_digit(self, radix: u32) -> bool {
        self.to_digit(radix).is_some()
    }
    // ASCII ranges plus the common Latin-1/Latin Extended letters; full
    // Unicode tables are follow-up work.
    fn is_alphabetic(self) -> bool {
        let c = self as u32;
        (c >= 65u32 && c <= 90u32)
            || (c >= 97u32 && c <= 122u32)
            || c == 170u32
            || c == 181u32
            || c == 186u32
            || (c >= 192u32 && c <= 591u32 && c != 215u32 && c != 247u32)
            || (c >= 880u32 && c <= 1327u32)
    }
    fn is_numeric(self) -> bool {
        let c = self as u32;
        (c >= 48u32 && c <= 57u32) || c == 178u32 || c == 179u32 || c == 185u32
            || (c >= 188u32 && c <= 190u32)
    }
    fn is_alphanumeric(self) -> bool {
        self.is_alphabetic() || self.is_numeric()
    }
    fn is_lowercase(self) -> bool {
        let c = self as u32;
        (c >= 97u32 && c <= 122u32) || (c >= 223u32 && c <= 255u32 && c != 247u32)
    }
    fn is_uppercase(self) -> bool {
        let c = self as u32;
        (c >= 65u32 && c <= 90u32) || (c >= 192u32 && c <= 222u32 && c != 215u32)
    }
    fn len_utf8(self) -> usize {
        let c = self as u32;
        if c < 128u32 {
            1usize
        } else if c < 2048u32 {
            2usize
        } else if c < 65536u32 {
            3usize
        } else {
            4usize
        }
    }
    fn eq_ignore_ascii_case(&self, other: &char) -> bool {
        self.to_ascii_lowercase() == other.to_ascii_lowercase()
    }
}

/// `char::from_digit`.
pub fn char_from_digit(num: u32, radix: u32) -> Option<char> {
    if radix < 2u32 || radix > 36u32 {
        panic!("from_digit: radix is too high (maximum 36)");
    }
    if num >= radix {
        return Option::None;
    }
    let code = if num < 10u32 { 48u32 + num } else { 87u32 + num };
    Option::Some(code as u8 as char)
}

// ---- integers ----

pub trait IntMethods: Sized {
    fn checked_add(self, rhs: Self) -> Option<Self>;
    fn checked_sub(self, rhs: Self) -> Option<Self>;
    fn checked_mul(self, rhs: Self) -> Option<Self>;
    fn checked_div(self, rhs: Self) -> Option<Self>;
    fn checked_rem(self, rhs: Self) -> Option<Self>;
    fn rem_euclid(self, rhs: Self) -> Self;
    fn div_euclid(self, rhs: Self) -> Self;
    fn saturating_mul(self, rhs: Self) -> Self;
    fn abs_diff_u(self, rhs: Self) -> Self;
}

// Generated per integer type (macro expansions cannot yet bind `self`).
impl IntMethods for i8 {
    fn checked_add(self, rhs: i8) -> Option<i8> {
        let r = (self as i128) + (rhs as i128);
        if r < (i8::MIN as i128) || r > (i8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i8)
        }
    }
    fn checked_sub(self, rhs: i8) -> Option<i8> {
        let r = (self as i128) - (rhs as i128);
        if r < (i8::MIN as i128) || r > (i8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i8)
        }
    }
    fn checked_mul(self, rhs: i8) -> Option<i8> {
        let r = (self as i128) * (rhs as i128);
        if r < (i8::MIN as i128) || r > (i8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i8)
        }
    }
    fn checked_div(self, rhs: i8) -> Option<i8> {
        if rhs == (0 as i8) || (self == i8::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: i8) -> Option<i8> {
        if rhs == (0 as i8) || (self == i8::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: i8) -> i8 {
        let r = self % rhs;
        if r < (0 as i8) {
            if rhs < (0 as i8) { r - rhs } else { r + rhs }
        } else {
            r
        }
    }
    fn div_euclid(self, rhs: i8) -> i8 {
        let q = self / rhs;
        if self % rhs < (0 as i8) {
            if rhs > (0 as i8) { q - (1 as i8) } else { q + (1 as i8) }
        } else {
            q
        }
    }
    fn saturating_mul(self, rhs: i8) -> i8 {
        let r = (self as i128) * (rhs as i128);
        if r < (i8::MIN as i128) {
            i8::MIN
        } else if r > (i8::MAX as i128) {
            i8::MAX
        } else {
            r as i8
        }
    }
    fn abs_diff_u(self, rhs: i8) -> i8 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for i16 {
    fn checked_add(self, rhs: i16) -> Option<i16> {
        let r = (self as i128) + (rhs as i128);
        if r < (i16::MIN as i128) || r > (i16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i16)
        }
    }
    fn checked_sub(self, rhs: i16) -> Option<i16> {
        let r = (self as i128) - (rhs as i128);
        if r < (i16::MIN as i128) || r > (i16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i16)
        }
    }
    fn checked_mul(self, rhs: i16) -> Option<i16> {
        let r = (self as i128) * (rhs as i128);
        if r < (i16::MIN as i128) || r > (i16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i16)
        }
    }
    fn checked_div(self, rhs: i16) -> Option<i16> {
        if rhs == (0 as i16) || (self == i16::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: i16) -> Option<i16> {
        if rhs == (0 as i16) || (self == i16::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: i16) -> i16 {
        let r = self % rhs;
        if r < (0 as i16) {
            if rhs < (0 as i16) { r - rhs } else { r + rhs }
        } else {
            r
        }
    }
    fn div_euclid(self, rhs: i16) -> i16 {
        let q = self / rhs;
        if self % rhs < (0 as i16) {
            if rhs > (0 as i16) { q - (1 as i16) } else { q + (1 as i16) }
        } else {
            q
        }
    }
    fn saturating_mul(self, rhs: i16) -> i16 {
        let r = (self as i128) * (rhs as i128);
        if r < (i16::MIN as i128) {
            i16::MIN
        } else if r > (i16::MAX as i128) {
            i16::MAX
        } else {
            r as i16
        }
    }
    fn abs_diff_u(self, rhs: i16) -> i16 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for i32 {
    fn checked_add(self, rhs: i32) -> Option<i32> {
        let r = (self as i128) + (rhs as i128);
        if r < (i32::MIN as i128) || r > (i32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i32)
        }
    }
    fn checked_sub(self, rhs: i32) -> Option<i32> {
        let r = (self as i128) - (rhs as i128);
        if r < (i32::MIN as i128) || r > (i32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i32)
        }
    }
    fn checked_mul(self, rhs: i32) -> Option<i32> {
        let r = (self as i128) * (rhs as i128);
        if r < (i32::MIN as i128) || r > (i32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i32)
        }
    }
    fn checked_div(self, rhs: i32) -> Option<i32> {
        if rhs == (0 as i32) || (self == i32::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: i32) -> Option<i32> {
        if rhs == (0 as i32) || (self == i32::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: i32) -> i32 {
        let r = self % rhs;
        if r < (0 as i32) {
            if rhs < (0 as i32) { r - rhs } else { r + rhs }
        } else {
            r
        }
    }
    fn div_euclid(self, rhs: i32) -> i32 {
        let q = self / rhs;
        if self % rhs < (0 as i32) {
            if rhs > (0 as i32) { q - (1 as i32) } else { q + (1 as i32) }
        } else {
            q
        }
    }
    fn saturating_mul(self, rhs: i32) -> i32 {
        let r = (self as i128) * (rhs as i128);
        if r < (i32::MIN as i128) {
            i32::MIN
        } else if r > (i32::MAX as i128) {
            i32::MAX
        } else {
            r as i32
        }
    }
    fn abs_diff_u(self, rhs: i32) -> i32 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for i64 {
    fn checked_add(self, rhs: i64) -> Option<i64> {
        let r = (self as i128) + (rhs as i128);
        if r < (i64::MIN as i128) || r > (i64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i64)
        }
    }
    fn checked_sub(self, rhs: i64) -> Option<i64> {
        let r = (self as i128) - (rhs as i128);
        if r < (i64::MIN as i128) || r > (i64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i64)
        }
    }
    fn checked_mul(self, rhs: i64) -> Option<i64> {
        let r = (self as i128) * (rhs as i128);
        if r < (i64::MIN as i128) || r > (i64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as i64)
        }
    }
    fn checked_div(self, rhs: i64) -> Option<i64> {
        if rhs == (0 as i64) || (self == i64::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: i64) -> Option<i64> {
        if rhs == (0 as i64) || (self == i64::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: i64) -> i64 {
        let r = self % rhs;
        if r < (0 as i64) {
            if rhs < (0 as i64) { r - rhs } else { r + rhs }
        } else {
            r
        }
    }
    fn div_euclid(self, rhs: i64) -> i64 {
        let q = self / rhs;
        if self % rhs < (0 as i64) {
            if rhs > (0 as i64) { q - (1 as i64) } else { q + (1 as i64) }
        } else {
            q
        }
    }
    fn saturating_mul(self, rhs: i64) -> i64 {
        let r = (self as i128) * (rhs as i128);
        if r < (i64::MIN as i128) {
            i64::MIN
        } else if r > (i64::MAX as i128) {
            i64::MAX
        } else {
            r as i64
        }
    }
    fn abs_diff_u(self, rhs: i64) -> i64 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for isize {
    fn checked_add(self, rhs: isize) -> Option<isize> {
        let r = (self as i128) + (rhs as i128);
        if r < (isize::MIN as i128) || r > (isize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as isize)
        }
    }
    fn checked_sub(self, rhs: isize) -> Option<isize> {
        let r = (self as i128) - (rhs as i128);
        if r < (isize::MIN as i128) || r > (isize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as isize)
        }
    }
    fn checked_mul(self, rhs: isize) -> Option<isize> {
        let r = (self as i128) * (rhs as i128);
        if r < (isize::MIN as i128) || r > (isize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as isize)
        }
    }
    fn checked_div(self, rhs: isize) -> Option<isize> {
        if rhs == (0 as isize) || (self == isize::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: isize) -> Option<isize> {
        if rhs == (0 as isize) || (self == isize::MIN && rhs == -1) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: isize) -> isize {
        let r = self % rhs;
        if r < (0 as isize) {
            if rhs < (0 as isize) { r - rhs } else { r + rhs }
        } else {
            r
        }
    }
    fn div_euclid(self, rhs: isize) -> isize {
        let q = self / rhs;
        if self % rhs < (0 as isize) {
            if rhs > (0 as isize) { q - (1 as isize) } else { q + (1 as isize) }
        } else {
            q
        }
    }
    fn saturating_mul(self, rhs: isize) -> isize {
        let r = (self as i128) * (rhs as i128);
        if r < (isize::MIN as i128) {
            isize::MIN
        } else if r > (isize::MAX as i128) {
            isize::MAX
        } else {
            r as isize
        }
    }
    fn abs_diff_u(self, rhs: isize) -> isize {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for u8 {
    fn checked_add(self, rhs: u8) -> Option<u8> {
        let r = (self as i128) + (rhs as i128);
        if r < (u8::MIN as i128) || r > (u8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u8)
        }
    }
    fn checked_sub(self, rhs: u8) -> Option<u8> {
        let r = (self as i128) - (rhs as i128);
        if r < (u8::MIN as i128) || r > (u8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u8)
        }
    }
    fn checked_mul(self, rhs: u8) -> Option<u8> {
        let r = (self as i128) * (rhs as i128);
        if r < (u8::MIN as i128) || r > (u8::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u8)
        }
    }
    fn checked_div(self, rhs: u8) -> Option<u8> {
        if rhs == (0 as u8) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: u8) -> Option<u8> {
        if rhs == (0 as u8) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: u8) -> u8 {
        self % rhs
    }
    fn div_euclid(self, rhs: u8) -> u8 {
        self / rhs
    }
    fn saturating_mul(self, rhs: u8) -> u8 {
        let r = (self as i128) * (rhs as i128);
        if r < (u8::MIN as i128) {
            u8::MIN
        } else if r > (u8::MAX as i128) {
            u8::MAX
        } else {
            r as u8
        }
    }
    fn abs_diff_u(self, rhs: u8) -> u8 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for u16 {
    fn checked_add(self, rhs: u16) -> Option<u16> {
        let r = (self as i128) + (rhs as i128);
        if r < (u16::MIN as i128) || r > (u16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u16)
        }
    }
    fn checked_sub(self, rhs: u16) -> Option<u16> {
        let r = (self as i128) - (rhs as i128);
        if r < (u16::MIN as i128) || r > (u16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u16)
        }
    }
    fn checked_mul(self, rhs: u16) -> Option<u16> {
        let r = (self as i128) * (rhs as i128);
        if r < (u16::MIN as i128) || r > (u16::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u16)
        }
    }
    fn checked_div(self, rhs: u16) -> Option<u16> {
        if rhs == (0 as u16) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: u16) -> Option<u16> {
        if rhs == (0 as u16) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: u16) -> u16 {
        self % rhs
    }
    fn div_euclid(self, rhs: u16) -> u16 {
        self / rhs
    }
    fn saturating_mul(self, rhs: u16) -> u16 {
        let r = (self as i128) * (rhs as i128);
        if r < (u16::MIN as i128) {
            u16::MIN
        } else if r > (u16::MAX as i128) {
            u16::MAX
        } else {
            r as u16
        }
    }
    fn abs_diff_u(self, rhs: u16) -> u16 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for u32 {
    fn checked_add(self, rhs: u32) -> Option<u32> {
        let r = (self as i128) + (rhs as i128);
        if r < (u32::MIN as i128) || r > (u32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u32)
        }
    }
    fn checked_sub(self, rhs: u32) -> Option<u32> {
        let r = (self as i128) - (rhs as i128);
        if r < (u32::MIN as i128) || r > (u32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u32)
        }
    }
    fn checked_mul(self, rhs: u32) -> Option<u32> {
        let r = (self as i128) * (rhs as i128);
        if r < (u32::MIN as i128) || r > (u32::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u32)
        }
    }
    fn checked_div(self, rhs: u32) -> Option<u32> {
        if rhs == (0 as u32) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: u32) -> Option<u32> {
        if rhs == (0 as u32) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: u32) -> u32 {
        self % rhs
    }
    fn div_euclid(self, rhs: u32) -> u32 {
        self / rhs
    }
    fn saturating_mul(self, rhs: u32) -> u32 {
        let r = (self as i128) * (rhs as i128);
        if r < (u32::MIN as i128) {
            u32::MIN
        } else if r > (u32::MAX as i128) {
            u32::MAX
        } else {
            r as u32
        }
    }
    fn abs_diff_u(self, rhs: u32) -> u32 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for u64 {
    fn checked_add(self, rhs: u64) -> Option<u64> {
        let r = (self as i128) + (rhs as i128);
        if r < (u64::MIN as i128) || r > (u64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u64)
        }
    }
    fn checked_sub(self, rhs: u64) -> Option<u64> {
        let r = (self as i128) - (rhs as i128);
        if r < (u64::MIN as i128) || r > (u64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u64)
        }
    }
    fn checked_mul(self, rhs: u64) -> Option<u64> {
        let r = (self as i128) * (rhs as i128);
        if r < (u64::MIN as i128) || r > (u64::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as u64)
        }
    }
    fn checked_div(self, rhs: u64) -> Option<u64> {
        if rhs == (0 as u64) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: u64) -> Option<u64> {
        if rhs == (0 as u64) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: u64) -> u64 {
        self % rhs
    }
    fn div_euclid(self, rhs: u64) -> u64 {
        self / rhs
    }
    fn saturating_mul(self, rhs: u64) -> u64 {
        let r = (self as i128) * (rhs as i128);
        if r < (u64::MIN as i128) {
            u64::MIN
        } else if r > (u64::MAX as i128) {
            u64::MAX
        } else {
            r as u64
        }
    }
    fn abs_diff_u(self, rhs: u64) -> u64 {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

impl IntMethods for usize {
    fn checked_add(self, rhs: usize) -> Option<usize> {
        let r = (self as i128) + (rhs as i128);
        if r < (usize::MIN as i128) || r > (usize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as usize)
        }
    }
    fn checked_sub(self, rhs: usize) -> Option<usize> {
        let r = (self as i128) - (rhs as i128);
        if r < (usize::MIN as i128) || r > (usize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as usize)
        }
    }
    fn checked_mul(self, rhs: usize) -> Option<usize> {
        let r = (self as i128) * (rhs as i128);
        if r < (usize::MIN as i128) || r > (usize::MAX as i128) {
            Option::None
        } else {
            Option::Some(r as usize)
        }
    }
    fn checked_div(self, rhs: usize) -> Option<usize> {
        if rhs == (0 as usize) {
            Option::None
        } else {
            Option::Some(self / rhs)
        }
    }
    fn checked_rem(self, rhs: usize) -> Option<usize> {
        if rhs == (0 as usize) {
            Option::None
        } else {
            Option::Some(self % rhs)
        }
    }
    fn rem_euclid(self, rhs: usize) -> usize {
        self % rhs
    }
    fn div_euclid(self, rhs: usize) -> usize {
        self / rhs
    }
    fn saturating_mul(self, rhs: usize) -> usize {
        let r = (self as i128) * (rhs as i128);
        if r < (usize::MIN as i128) {
            usize::MIN
        } else if r > (usize::MAX as i128) {
            usize::MAX
        } else {
            r as usize
        }
    }
    fn abs_diff_u(self, rhs: usize) -> usize {
        if self > rhs { self - rhs } else { rhs - self }
    }
}

// ---- parsing (str::parse) ----

pub trait FromStr: Sized {
    type Err;
    fn from_str(s: &str) -> Result<Self, Self::Err>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IntErrorKind {
    Empty,
    InvalidDigit,
    PosOverflow,
    NegOverflow,
    Zero,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIntError {
    kind: IntErrorKind,
}

impl ParseIntError {
    pub fn kind(&self) -> &IntErrorKind {
        &self.kind
    }
}

impl super::fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad(match self.kind {
            IntErrorKind::Empty => "cannot parse integer from empty string",
            IntErrorKind::InvalidDigit => "invalid digit found in string",
            IntErrorKind::PosOverflow => "number too large to fit in target type",
            IntErrorKind::NegOverflow => "number too small to fit in target type",
            IntErrorKind::Zero => "number would be zero for non-zero type",
        })
    }
}

/// Parse a decimal integer into the range [min, max] (as i128).
pub fn parse_i128(s: &str, min: i128, max: i128) -> Result<i128, ParseIntError> {
    let (p, n) = super::string::str_parts(s);
    if n == 0usize {
        return Result::Err(ParseIntError { kind: IntErrorKind::Empty });
    }
    let mut i = 0usize;
    let mut negative = false;
    let first = super::string::read_u8(p);
    if first == 43u8 || first == 45u8 {
        negative = first == 45u8;
        i = 1usize;
        if n == 1usize {
            return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit });
        }
        if negative && min == 0i128 {
            return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit });
        }
    }
    let mut value = 0i128;
    while i < n {
        let b = super::string::read_u8(p + i);
        if b < 48u8 || b > 57u8 {
            return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit });
        }
        let d = (b - 48u8) as i128;
        if negative {
            if value < (min + d) / 10i128 {
                return Result::Err(ParseIntError { kind: IntErrorKind::NegOverflow });
            }
            value = value * 10i128 - d;
            if value < min {
                return Result::Err(ParseIntError { kind: IntErrorKind::NegOverflow });
            }
        } else {
            if value > (max - d) / 10i128 {
                return Result::Err(ParseIntError { kind: IntErrorKind::PosOverflow });
            }
            value = value * 10i128 + d;
            if value > max {
                return Result::Err(ParseIntError { kind: IntErrorKind::PosOverflow });
            }
        }
        i += 1usize;
    }
    Result::Ok(value)
}

// Accumulate unsigned values directly so the upper half of u128 is never
// transported through i128. Check both operations before executing them.
fn parse_u128(s: &str) -> Result<u128, ParseIntError> {
    let (p, n) = super::string::str_parts(s);
    if n == 0usize {
        return Result::Err(ParseIntError { kind: IntErrorKind::Empty });
    }
    let mut i = 0usize;
    if super::string::read_u8(p) == 43u8 {
        i = 1usize;
        if i == n {
            return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit });
        }
    }
    let mut value = 0u128;
    while i < n {
        let b = super::string::read_u8(p + i);
        if b < 48u8 || b > 57u8 {
            return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit });
        }
        let digit = (b - 48u8) as u128;
        if value > (u128::MAX - digit) / 10u128 {
            return Result::Err(ParseIntError { kind: IntErrorKind::PosOverflow });
        }
        value = value * 10u128 + digit;
        i += 1usize;
    }
    Result::Ok(value)
}

impl FromStr for i8 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<i8, ParseIntError> {
        match parse_i128(s, i8::MIN as i128, i8::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i8),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for i16 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<i16, ParseIntError> {
        match parse_i128(s, i16::MIN as i128, i16::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i16),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for i32 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<i32, ParseIntError> {
        match parse_i128(s, i32::MIN as i128, i32::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i32),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for i64 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<i64, ParseIntError> {
        match parse_i128(s, i64::MIN as i128, i64::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i64),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for i128 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<i128, ParseIntError> {
        parse_i128(s, i128::MIN, i128::MAX)
    }
}
impl FromStr for isize {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<isize, ParseIntError> {
        match parse_i128(s, isize::MIN as i128, isize::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as isize),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for u8 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<u8, ParseIntError> {
        match parse_i128(s, 0i128, u8::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u8),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for u16 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<u16, ParseIntError> {
        match parse_i128(s, 0i128, u16::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u16),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for u32 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<u32, ParseIntError> {
        match parse_i128(s, 0i128, u32::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u32),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for u64 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<u64, ParseIntError> {
        match parse_i128(s, 0i128, u64::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u64),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl FromStr for u128 {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<u128, ParseIntError> {
        parse_u128(s)
    }
}
impl FromStr for usize {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<usize, ParseIntError> {
        match parse_i128(s, 0i128, usize::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as usize),
            Result::Err(error) => Result::Err(error),
        }
    }
}
