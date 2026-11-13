//! Lossless conversions expressed as ordinary checked library source.
//!
//! This supplies the standard `From` declaration, the byte-to-Unicode-scalar
//! conversion, `bool` to integer, and every lossless primitive integer
//! widening that stock `core` provides on 64-bit targets. Each impl is
//! ordinary library source; nothing is inferred from the spelling of a call.
pub trait From<T>: Sized {
    fn from(value: T) -> Self;
}

impl<T> From<T> for T {
    fn from(value: T) -> T {
        value
    }
}

impl From<u8> for char {
    fn from(value: u8) -> char {
        // Every byte is a Unicode scalar, including 0x80..=0xff. This is not
        // UTF-8 decoding and must never sign-extend the input.
        value as char
    }
}

// Only the impls stock `core` declares. Notably `usize`/`isize` widen only
// from `u8`/`u16` (resp. `i8`/`i16`/`u8`), never from 32-bit types, since the
// standard library keeps pointer width portable.
macro_rules! lossless_from {
    ($($small:ty => $($large:ty),+;)+) => {
        $($(
            impl From<$small> for $large {
                fn from(value: $small) -> $large {
                    value as $large
                }
            }
        )+)+
    };
}

macro_rules! bool_from {
    ($($large:ty),+) => {
        $(
            impl From<bool> for $large {
                fn from(value: bool) -> $large {
                    if value { 1 } else { 0 }
                }
            }
        )+
    };
}

bool_from! { u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize }

lossless_from! {
    u8 => u16, u32, u64, u128, usize, i16, i32, i64, i128, isize;
    u16 => u32, u64, u128, usize, i32, i64, i128;
    u32 => u64, u128, i64, i128;
    u64 => u128, i128;
    i8 => i16, i32, i64, i128, isize;
    i16 => i32, i64, i128, isize;
    i32 => i64, i128;
    i64 => i128;
}

/// The reciprocal of `From`, provided by the standard blanket impl.
pub trait Into<T>: Sized {
    fn into(self) -> T;
}

impl<T, U: From<T>> Into<U> for T {
    fn into(self) -> U {
        U::from(self)
    }
}

impl<'a> From<&'a str> for super::string::String {
    fn from(value: &'a str) -> super::string::String {
        super::string::String::from(value)
    }
}

// Adapted from library/core/src/convert/mod.rs and the `AsRef`/`AsMut` impls in
// library/core/src/str, library/alloc/src/{string,vec,boxed,rc}.rs.
// MIT OR Apache-2.0, Rust Project Contributors.

/// Used to do a cheap reference-to-reference conversion.
pub trait AsRef<T: ?Sized> {
    fn as_ref(&self) -> &T;
}

/// Used to do a cheap mutable-to-mutable reference conversion.
pub trait AsMut<T: ?Sized> {
    fn as_mut(&mut self) -> &mut T;
}

// As lifts over &
impl<T: ?Sized, U: ?Sized> AsRef<U> for &T
where
    T: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        <T as AsRef<U>>::as_ref(*self)
    }
}

// As lifts over &mut
impl<T: ?Sized, U: ?Sized> AsRef<U> for &mut T
where
    T: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        <T as AsRef<U>>::as_ref(&**self)
    }
}

impl<T> AsRef<[T]> for [T] {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T> AsMut<[T]> for [T] {
    fn as_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T, const N: usize> AsRef<[T]> for [T; N] {
    fn as_ref(&self) -> &[T] {
        unsafe { super::slice::from_raw_parts(self as *const [T; N] as *const T, N) }
    }
}

impl<T, const N: usize> AsMut<[T]> for [T; N] {
    fn as_mut(&mut self) -> &mut [T] {
        unsafe { super::slice::from_raw_parts_mut(self as *mut [T; N] as *mut T, N) }
    }
}

impl AsRef<str> for str {
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsRef<[u8]> for str {
    fn as_ref(&self) -> &[u8] {
        super::string::StrMethods::as_bytes(self)
    }
}

impl AsRef<str> for super::string::String {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for super::string::String {
    fn as_ref(&self) -> &[u8] {
        super::string::StrMethods::as_bytes(self.as_str())
    }
}

impl<T> AsRef<super::vec::Vec<T>> for super::vec::Vec<T> {
    fn as_ref(&self) -> &super::vec::Vec<T> {
        self
    }
}

impl<T> AsMut<super::vec::Vec<T>> for super::vec::Vec<T> {
    fn as_mut(&mut self) -> &mut super::vec::Vec<T> {
        self
    }
}

impl<T> AsRef<[T]> for super::vec::Vec<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> AsMut<[T]> for super::vec::Vec<T> {
    fn as_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: ?Sized> AsRef<T> for super::boxed::Box<T> {
    fn as_ref(&self) -> &T {
        &**self
    }
}

impl<T: ?Sized> AsMut<T> for super::boxed::Box<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut **self
    }
}

impl<T: ?Sized> AsRef<T> for super::rc::Rc<T> {
    fn as_ref(&self) -> &T {
        &**self
    }
}
