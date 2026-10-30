// Inherent methods of the primitive types that are not compiler intrinsics,
// as ordinary Rust source. They are trait methods (traits injected by the
// prelude) because primitive inherent impls are reserved to the compiler.
use super::iter::Iterator;
use super::option::Option;
use super::result::Result;
use super::string::StrChars;

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

// Checked shift counts are u32 for every primitive, independently of value width.
// These source methods guard before evaluating a shift, so invalid counts return
// None without reaching either the static count rejection or runtime trap.
pub trait CheckedShiftMethods: Sized {
    fn checked_shl(self, rhs: u32) -> Option<Self>;
    fn checked_shr(self, rhs: u32) -> Option<Self>;
}

impl CheckedShiftMethods for i8 {
    fn checked_shl(self, rhs: u32) -> Option<i8> {
        if rhs < 8u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<i8> {
        if rhs < 8u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for i16 {
    fn checked_shl(self, rhs: u32) -> Option<i16> {
        if rhs < 16u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<i16> {
        if rhs < 16u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for i32 {
    fn checked_shl(self, rhs: u32) -> Option<i32> {
        if rhs < 32u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<i32> {
        if rhs < 32u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for i64 {
    fn checked_shl(self, rhs: u32) -> Option<i64> {
        if rhs < 64u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<i64> {
        if rhs < 64u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for i128 {
    fn checked_shl(self, rhs: u32) -> Option<i128> {
        if rhs < 128u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<i128> {
        if rhs < 128u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for isize {
    fn checked_shl(self, rhs: u32) -> Option<isize> {
        if rhs < 64u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<isize> {
        if rhs < 64u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for u8 {
    fn checked_shl(self, rhs: u32) -> Option<u8> {
        if rhs < 8u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<u8> {
        if rhs < 8u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for u16 {
    fn checked_shl(self, rhs: u32) -> Option<u16> {
        if rhs < 16u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<u16> {
        if rhs < 16u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for u32 {
    fn checked_shl(self, rhs: u32) -> Option<u32> {
        if rhs < 32u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<u32> {
        if rhs < 32u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for u64 {
    fn checked_shl(self, rhs: u32) -> Option<u64> {
        if rhs < 64u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<u64> {
        if rhs < 64u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for u128 {
    fn checked_shl(self, rhs: u32) -> Option<u128> {
        if rhs < 128u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<u128> {
        if rhs < 128u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

impl CheckedShiftMethods for usize {
    fn checked_shl(self, rhs: u32) -> Option<usize> {
        if rhs < 64u32 { Option::Some(self << rhs) } else { Option::None }
    }
    fn checked_shr(self, rhs: u32) -> Option<usize> {
        if rhs < 64u32 { Option::Some(self >> rhs) } else { Option::None }
    }
}

// ---- parsing (str::parse) ----

pub trait FromStr: Sized {
    type Err;
    fn from_str(s: &str) -> Result<Self, Self::Err>;
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct ParseBoolError {
    private: (),
}

impl super::fmt::Debug for ParseBoolError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("ParseBoolError")
    }
}

impl super::fmt::Display for ParseBoolError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad("provided string was not `true` or `false`")
    }
}

impl FromStr for bool {
    type Err = ParseBoolError;
    fn from_str(s: &str) -> Result<bool, ParseBoolError> {
        let (p, n) = super::string::str_parts(s);
        if n == 4usize
            && super::string::read_u8(p) == b't'
            && super::string::read_u8(p + 1usize) == b'r'
            && super::string::read_u8(p + 2usize) == b'u'
            && super::string::read_u8(p + 3usize) == b'e'
        {
            Result::Ok(true)
        } else if n == 5usize
            && super::string::read_u8(p) == b'f'
            && super::string::read_u8(p + 1usize) == b'a'
            && super::string::read_u8(p + 2usize) == b'l'
            && super::string::read_u8(p + 3usize) == b's'
            && super::string::read_u8(p + 4usize) == b'e'
        {
            Result::Ok(false)
        } else {
            Result::Err(ParseBoolError { private: () })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CharErrorKind {
    EmptyString,
    TooManyChars,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCharError {
    kind: CharErrorKind,
}

impl super::fmt::Display for ParseCharError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad(match self.kind {
            CharErrorKind::EmptyString => "cannot parse char from empty string",
            CharErrorKind::TooManyChars => "too many characters in string",
        })
    }
}

impl FromStr for char {
    type Err = ParseCharError;
    fn from_str(s: &str) -> Result<char, ParseCharError> {
        let mut chars = s.chars();
        match chars.next() {
            Option::None => Result::Err(ParseCharError { kind: CharErrorKind::EmptyString }),
            Option::Some(value) => match chars.next() {
                Option::None => Result::Ok(value),
                Option::Some(_) => Result::Err(ParseCharError { kind: CharErrorKind::TooManyChars }),
            },
        }
    }
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
    parse_i128_radix(s, 10u32, min, max)
}

fn radix_digit(byte: u8, radix: u32) -> Option<u32> {
    let digit = if byte >= 48u8 && byte <= 57u8 {
        (byte - 48u8) as u32
    } else if byte >= 65u8 && byte <= 90u8 {
        (byte - 65u8) as u32 + 10u32
    } else if byte >= 97u8 && byte <= 122u8 {
        (byte - 97u8) as u32 + 10u32
    } else {
        return Option::None;
    };
    if digit < radix { Option::Some(digit) } else { Option::None }
}

fn check_radix(radix: u32) {
    if radix < 2u32 || radix > 36u32 {
        panic!("from_ascii_radix: radix must lie in the range `[2, 36]` - found {}", radix);
    }
}

fn parse_i128_radix(s: &str, radix: u32, min: i128, max: i128) -> Result<i128, ParseIntError> {
    check_radix(radix);
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
        let d = match radix_digit(super::string::read_u8(p + i), radix) {
            Option::Some(digit) => digit as i128,
            Option::None => return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit }),
        };
        if negative {
            if value < (min + d) / (radix as i128) {
                return Result::Err(ParseIntError { kind: IntErrorKind::NegOverflow });
            }
            value = value * (radix as i128) - d;
            if value < min {
                return Result::Err(ParseIntError { kind: IntErrorKind::NegOverflow });
            }
        } else {
            if value > (max - d) / (radix as i128) {
                return Result::Err(ParseIntError { kind: IntErrorKind::PosOverflow });
            }
            value = value * (radix as i128) + d;
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
    parse_u128_radix(s, 10u32)
}

fn parse_u128_radix(s: &str, radix: u32) -> Result<u128, ParseIntError> {
    check_radix(radix);
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
        let digit = match radix_digit(super::string::read_u8(p + i), radix) {
            Option::Some(digit) => digit as u128,
            Option::None => return Result::Err(ParseIntError { kind: IntErrorKind::InvalidDigit }),
        };
        if value > (u128::MAX - digit) / (radix as u128) {
            return Result::Err(ParseIntError { kind: IntErrorKind::PosOverflow });
        }
        value = value * (radix as u128) + digit;
        i += 1usize;
    }
    Result::Ok(value)
}

pub trait RadixParse: Sized {
    fn from_str_radix(src: &str, radix: u32) -> Result<Self, ParseIntError>;
}

impl RadixParse for i8 {
    fn from_str_radix(src: &str, radix: u32) -> Result<i8, ParseIntError> {
        match parse_i128_radix(src, radix, i8::MIN as i128, i8::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i8),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for i16 {
    fn from_str_radix(src: &str, radix: u32) -> Result<i16, ParseIntError> {
        match parse_i128_radix(src, radix, i16::MIN as i128, i16::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i16),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for i32 {
    fn from_str_radix(src: &str, radix: u32) -> Result<i32, ParseIntError> {
        match parse_i128_radix(src, radix, i32::MIN as i128, i32::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i32),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for i64 {
    fn from_str_radix(src: &str, radix: u32) -> Result<i64, ParseIntError> {
        match parse_i128_radix(src, radix, i64::MIN as i128, i64::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as i64),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for i128 {
    fn from_str_radix(src: &str, radix: u32) -> Result<i128, ParseIntError> {
        parse_i128_radix(src, radix, i128::MIN, i128::MAX)
    }
}
impl RadixParse for isize {
    fn from_str_radix(src: &str, radix: u32) -> Result<isize, ParseIntError> {
        match parse_i128_radix(src, radix, isize::MIN as i128, isize::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as isize),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for u8 {
    fn from_str_radix(src: &str, radix: u32) -> Result<u8, ParseIntError> {
        match parse_i128_radix(src, radix, 0i128, u8::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u8),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for u16 {
    fn from_str_radix(src: &str, radix: u32) -> Result<u16, ParseIntError> {
        match parse_i128_radix(src, radix, 0i128, u16::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u16),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for u32 {
    fn from_str_radix(src: &str, radix: u32) -> Result<u32, ParseIntError> {
        match parse_i128_radix(src, radix, 0i128, u32::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u32),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for u64 {
    fn from_str_radix(src: &str, radix: u32) -> Result<u64, ParseIntError> {
        match parse_i128_radix(src, radix, 0i128, u64::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as u64),
            Result::Err(error) => Result::Err(error),
        }
    }
}
impl RadixParse for u128 {
    fn from_str_radix(src: &str, radix: u32) -> Result<u128, ParseIntError> {
        parse_u128_radix(src, radix)
    }
}
impl RadixParse for usize {
    fn from_str_radix(src: &str, radix: u32) -> Result<usize, ParseIntError> {
        match parse_i128_radix(src, radix, 0i128, usize::MAX as i128) {
            Result::Ok(value) => Result::Ok(value as usize),
            Result::Err(error) => Result::Err(error),
        }
    }
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

// ---- float parsing ----
//
// Exact decimal-to-binary conversion: the decimal significand is kept as an
// arbitrary-precision integer (little-endian u32 limbs) and every candidate
// is compared exactly, so results are correctly rounded (ties to even) for
// any input length, matching stock `f64::from_str`/`f32::from_str`.

#[derive(Debug, Clone, PartialEq, Eq)]
enum FloatErrorKind {
    Empty,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFloatError {
    kind: FloatErrorKind,
}

impl super::fmt::Display for ParseFloatError {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.pad(match self.kind {
            FloatErrorKind::Empty => "cannot parse float from empty string",
            FloatErrorKind::Invalid => "invalid float literal",
        })
    }
}

fn float_invalid() -> ParseFloatError {
    ParseFloatError { kind: FloatErrorKind::Invalid }
}

fn ascii_lower(b: u8) -> u8 {
    if b >= 65u8 && b <= 90u8 { b + 32u8 } else { b }
}

/// Case-insensitive comparison of bytes [start, start+len) against `word`.
fn bytes_equal_ignore_case(p: usize, start: usize, n: usize, word: &str) -> bool {
    let (wp, wn) = super::string::str_parts(word);
    if n - start != wn {
        return false;
    }
    let mut i = 0usize;
    while i < wn {
        if ascii_lower(super::string::read_u8(p + start + i)) != super::string::read_u8(wp + i) {
            return false;
        }
        i += 1usize;
    }
    true
}

/// Arbitrary-precision unsigned integer, little-endian base 2^32 limbs.
struct Big {
    limbs: super::vec::Vec<u32>,
}

impl Big {
    fn from_u64(v: u64) -> Big {
        let mut limbs = super::vec::Vec::new();
        limbs.push(v as u32);
        limbs.push((v >> 32u32) as u32);
        let mut b = Big { limbs };
        b.trim();
        b
    }
    fn trim(&mut self) {
        while self.limbs.len() > 0usize && self.limbs[self.limbs.len() - 1usize] == 0u32 {
            let _ = self.limbs.pop();
        }
    }
    fn mul_small_add(&mut self, m: u32, a: u32) {
        let mut carry = a as u64;
        let mut i = 0usize;
        while i < self.limbs.len() {
            let v = (self.limbs[i] as u64) * (m as u64) + carry;
            self.limbs[i] = v as u32;
            carry = v >> 32u32;
            i += 1usize;
        }
        if carry != 0u64 {
            self.limbs.push(carry as u32);
        }
    }
    fn mul_pow10(&mut self, mut e: usize) {
        while e >= 9usize {
            self.mul_small_add(1000000000u32, 0u32);
            e -= 9usize;
        }
        let mut m = 1u32;
        while e > 0usize {
            m *= 10u32;
            e -= 1usize;
        }
        if m != 1u32 {
            self.mul_small_add(m, 0u32);
        }
    }
    fn shl(&mut self, bits: usize) {
        if self.limbs.len() == 0usize {
            return;
        }
        let words = bits / 32usize;
        let rem = (bits % 32usize) as u32;
        if rem != 0u32 {
            let mut carry = 0u32;
            let mut i = 0usize;
            while i < self.limbs.len() {
                let v = self.limbs[i];
                self.limbs[i] = (v << rem) | carry;
                carry = v >> (32u32 - rem);
                i += 1usize;
            }
            if carry != 0u32 {
                self.limbs.push(carry);
            }
        }
        if words > 0usize {
            let old = self.limbs.len();
            let mut k = 0usize;
            while k < words {
                self.limbs.push(0u32);
                k += 1usize;
            }
            let mut i = old;
            while i > 0usize {
                i -= 1usize;
                self.limbs[i + words] = self.limbs[i];
            }
            let mut j = 0usize;
            while j < words {
                self.limbs[j] = 0u32;
                j += 1usize;
            }
        }
    }
    fn bit_len(&self) -> usize {
        let n = self.limbs.len();
        if n == 0usize {
            return 0usize;
        }
        let top = self.limbs[n - 1usize];
        let mut bits = 0usize;
        let mut t = top;
        while t != 0u32 {
            bits += 1usize;
            t >>= 1u32;
        }
        (n - 1usize) * 32usize + bits
    }
    fn cmp(&self, other: &Big) -> super::cmp::Ordering {
        if self.limbs.len() != other.limbs.len() {
            return if self.limbs.len() < other.limbs.len() {
                super::cmp::Ordering::Less
            } else {
                super::cmp::Ordering::Greater
            };
        }
        let mut i = self.limbs.len();
        while i > 0usize {
            i -= 1usize;
            if self.limbs[i] != other.limbs[i] {
                return if self.limbs[i] < other.limbs[i] {
                    super::cmp::Ordering::Less
                } else {
                    super::cmp::Ordering::Greater
                };
            }
        }
        super::cmp::Ordering::Equal
    }
    fn clone_big(&self) -> Big {
        let mut limbs = super::vec::Vec::new();
        let mut i = 0usize;
        while i < self.limbs.len() {
            limbs.push(self.limbs[i]);
            i += 1usize;
        }
        Big { limbs }
    }
}

/// Parsed decimal: value = digits * 10^exp10, digits as a Big integer.
struct Decimal {
    negative: bool,
    digits: Big,
    zero: bool,
    exp10: i64,
}

enum Special {
    Finite(Decimal),
    Infinity(bool),
    NaN(bool),
}

fn parse_decimal(s: &str) -> Result<Special, ParseFloatError> {
    let (p, n) = super::string::str_parts(s);
    if n == 0usize {
        return Result::Err(ParseFloatError { kind: FloatErrorKind::Empty });
    }
    let mut i = 0usize;
    let mut negative = false;
    let first = super::string::read_u8(p);
    if first == 43u8 || first == 45u8 {
        negative = first == 45u8;
        i = 1usize;
    }
    if bytes_equal_ignore_case(p, i, n, "inf") || bytes_equal_ignore_case(p, i, n, "infinity") {
        return Result::Ok(Special::Infinity(negative));
    }
    if bytes_equal_ignore_case(p, i, n, "nan") {
        return Result::Ok(Special::NaN(negative));
    }
    let mut digits = Big { limbs: super::vec::Vec::new() };
    let mut any_digit = false;
    let mut zero = true;
    let mut exp10: i64 = 0i64;
    // Leading zeros are dropped; stored digits are significant, but every
    // digit (including trailing zeros) is kept so the value stays exact.
    while i < n {
        let b = super::string::read_u8(p + i);
        if b >= 48u8 && b <= 57u8 {
            any_digit = true;
            if !(zero && b == 48u8) {
                zero = false;
                digits.mul_small_add(10u32, (b - 48u8) as u32);
            }
            i += 1usize;
        } else {
            break;
        }
    }
    if i < n && super::string::read_u8(p + i) == 46u8 {
        i += 1usize;
        while i < n {
            let b = super::string::read_u8(p + i);
            if b >= 48u8 && b <= 57u8 {
                any_digit = true;
                if !(zero && b == 48u8) {
                    zero = false;
                    digits.mul_small_add(10u32, (b - 48u8) as u32);
                }
                // Each fractional digit, leading zeros included, scales by 10^-1.
                exp10 -= 1i64;
                i += 1usize;
            } else {
                break;
            }
        }
    }
    if !any_digit {
        return Result::Err(float_invalid());
    }
    if i < n {
        let b = super::string::read_u8(p + i);
        if b == 101u8 || b == 69u8 {
            i += 1usize;
            let mut exp_negative = false;
            if i < n {
                let s = super::string::read_u8(p + i);
                if s == 43u8 || s == 45u8 {
                    exp_negative = s == 45u8;
                    i += 1usize;
                }
            }
            if i >= n {
                return Result::Err(float_invalid());
            }
            // Saturate huge exponents; they only decide overflow or zero.
            let mut e: i64 = 0i64;
            while i < n {
                let d = super::string::read_u8(p + i);
                if d < 48u8 || d > 57u8 {
                    return Result::Err(float_invalid());
                }
                if e < 100000000000i64 {
                    e = e * 10i64 + (d - 48u8) as i64;
                }
                i += 1usize;
            }
            exp10 = if exp_negative { exp10 - e } else { exp10 + e };
        } else {
            return Result::Err(float_invalid());
        }
    }
    Result::Ok(Special::Finite(Decimal { negative, digits, zero, exp10 }))
}

/// Correctly rounded binary value of a positive decimal for a format with
/// `mant_bits` explicit mantissa bits and exponent bias `bias`.
/// Returns the raw (unsigned) bit pattern without the sign.
fn decimal_to_bits(d: &Decimal, mant_bits: u32, exp_bits: u32) -> u64 {
    let bias: i64 = (1i64 << (exp_bits - 1u32)) - 1i64;
    let max_exp: i64 = bias;
    let min_exp: i64 = 1i64 - bias;
    let precision: i64 = mant_bits as i64 + 1i64;
    let inf_bits: u64 = ((1u64 << exp_bits) - 1u64) << mant_bits;
    if d.zero {
        return 0u64;
    }
    let ndigits = (d.digits.bit_len() as i64) * 30103i64 / 100000i64 + 1i64;
    // Quick out-of-range decisions keep huge exponents from building bignums.
    if d.exp10 + ndigits > 400i64 {
        return inf_bits;
    }
    if d.exp10 + ndigits < -400i64 {
        return 0u64;
    }
    // value = num / den exactly, as big integers.
    let mut num = d.digits.clone_big();
    let mut den = Big::from_u64(1u64);
    if d.exp10 >= 0i64 {
        num.mul_pow10(d.exp10 as usize);
    } else {
        den.mul_pow10((0i64 - d.exp10) as usize);
    }
    // Binary exponent estimate e2 with 2^e2 <= value < 2^(e2+1).
    let mut e2: i64 = num.bit_len() as i64 - den.bit_len() as i64;
    loop {
        // Compare value against 2^e2.
        let mut a = num.clone_big();
        let mut b = den.clone_big();
        if e2 >= 0i64 { b.shl(e2 as usize); } else { a.shl((0i64 - e2) as usize); }
        match a.cmp(&b) {
            super::cmp::Ordering::Less => { e2 -= 1i64; }
            _ => {
                let mut a2 = num.clone_big();
                let mut b2 = den.clone_big();
                let up = e2 + 1i64;
                if up >= 0i64 { b2.shl(up as usize); } else { a2.shl((0i64 - up) as usize); }
                match a2.cmp(&b2) {
                    super::cmp::Ordering::Less => break,
                    _ => { e2 += 1i64; }
                }
            }
        }
    }
    if e2 > max_exp {
        return inf_bits;
    }
    // Scale so the integer part holds `precision` bits (fewer when subnormal).
    let exponent = if e2 < min_exp { min_exp } else { e2 };
    let shift: i64 = precision - 1i64 - exponent;
    let mut a = num.clone_big();
    let mut b = den.clone_big();
    if shift >= 0i64 { a.shl(shift as usize); } else { b.shl((0i64 - shift) as usize); }
    // q = floor(a / b) by binary long division on a bounded quotient width.
    let mut q: u64 = 0u64;
    let mut bit: i64 = precision + 1i64;
    let mut rem = a.clone_big();
    while bit >= 0i64 {
        let mut t = b.clone_big();
        t.shl(bit as usize);
        match rem.cmp(&t) {
            super::cmp::Ordering::Less => {}
            _ => {
                rem = big_sub(&rem, &t);
                q |= 1u64 << (bit as u32);
            }
        }
        bit -= 1i64;
    }
    // Round half to even using the exact remainder.
    let mut twice = rem.clone_big();
    twice.shl(1usize);
    let round_up = match twice.cmp(&b) {
        super::cmp::Ordering::Greater => true,
        super::cmp::Ordering::Equal => (q & 1u64) == 1u64,
        super::cmp::Ordering::Less => false,
    };
    if round_up {
        q += 1u64;
    }
    let mut biased = exponent + bias;
    if q >= (1u64 << (precision as u32)) {
        q >>= 1u32;
        biased += 1i64;
    }
    if biased >= (1i64 << exp_bits) - 1i64 {
        return inf_bits;
    }
    if q < (1u64 << mant_bits) {
        // Subnormal (or rounded up into the smallest normal, handled above).
        return q;
    }
    ((biased as u64) << mant_bits) | (q & ((1u64 << mant_bits) - 1u64))
}

fn big_sub(a: &Big, b: &Big) -> Big {
    let mut limbs = super::vec::Vec::new();
    let mut borrow = 0i64;
    let mut i = 0usize;
    while i < a.limbs.len() {
        let rhs = if i < b.limbs.len() { b.limbs[i] as i64 } else { 0i64 };
        let mut v = a.limbs[i] as i64 - rhs - borrow;
        if v < 0i64 {
            v += 4294967296i64;
            borrow = 1i64;
        } else {
            borrow = 0i64;
        }
        limbs.push(v as u32);
        i += 1usize;
    }
    let mut r = Big { limbs };
    r.trim();
    r
}

fn f64_from_bits(b: u64) -> f64 {
    let p = &b as *const u64 as *const f64;
    unsafe { *p }
}

fn f32_from_bits(b: u32) -> f32 {
    let p = &b as *const u32 as *const f32;
    unsafe { *p }
}

impl FromStr for f64 {
    type Err = ParseFloatError;
    fn from_str(s: &str) -> Result<f64, ParseFloatError> {
        let sign = |negative: bool| if negative { 1u64 << 63u32 } else { 0u64 };
        match parse_decimal(s) {
            Result::Err(error) => Result::Err(error),
            Result::Ok(Special::Infinity(negative)) => {
                Result::Ok(f64_from_bits(sign(negative) | 0x7ff0000000000000u64))
            }
            Result::Ok(Special::NaN(negative)) => {
                Result::Ok(f64_from_bits(sign(negative) | 0x7ff8000000000000u64))
            }
            Result::Ok(Special::Finite(d)) => {
                Result::Ok(f64_from_bits(sign(d.negative) | decimal_to_bits(&d, 52u32, 11u32)))
            }
        }
    }
}

impl FromStr for f32 {
    type Err = ParseFloatError;
    fn from_str(s: &str) -> Result<f32, ParseFloatError> {
        let sign = |negative: bool| if negative { 1u32 << 31u32 } else { 0u32 };
        match parse_decimal(s) {
            Result::Err(error) => Result::Err(error),
            Result::Ok(Special::Infinity(negative)) => {
                Result::Ok(f32_from_bits(sign(negative) | 0x7f800000u32))
            }
            Result::Ok(Special::NaN(negative)) => {
                Result::Ok(f32_from_bits(sign(negative) | 0x7fc00000u32))
            }
            Result::Ok(Special::Finite(d)) => {
                let bits = decimal_to_bits(&d, 23u32, 8u32) as u32;
                Result::Ok(f32_from_bits(sign(d.negative) | bits))
            }
        }
    }
}

/// Bit-exact float/integer reinterpretation (`to_bits`/`from_bits`).
pub trait FloatBits {
    type Bits;
    fn to_bits(self) -> Self::Bits;
    fn from_bits(bits: Self::Bits) -> Self;
}

impl FloatBits for f64 {
    type Bits = u64;
    fn to_bits(self) -> u64 {
        let p = &self as *const f64 as *const u64;
        unsafe { *p }
    }
    fn from_bits(bits: u64) -> f64 {
        f64_from_bits(bits)
    }
}

impl FloatBits for f32 {
    type Bits = u32;
    fn to_bits(self) -> u32 {
        let p = &self as *const f32 as *const u32;
        unsafe { *p }
    }
    fn from_bits(bits: u32) -> f32 {
        f32_from_bits(bits)
    }
}
