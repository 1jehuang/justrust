//! Lossless conversions expressed as ordinary checked library source.
//!
//! This initial implementation supplies the standard `From` declaration and
//! the byte-to-Unicode-scalar conversion. Other conversion impls are separate
//! library work, not inferred from the spelling of a call.
pub trait From<T>: Sized {
    fn from(value: T) -> Self;
}

impl From<u8> for char {
    fn from(value: u8) -> char {
        // Every byte is a Unicode scalar, including 0x80..=0xff. This is not
        // UTF-8 decoding and must never sign-extend the input.
        value as char
    }
}
