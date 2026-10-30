//! An impl is an impl of its crate wherever it is written. One in a function was known only
//! if its type or its trait was declared in that function too: zerocopy's tests `impl
//! KnownLayout for elain::Align<8>` in a test, and `to_string` of the error that names it did
//! not resolve.

use std::fmt;
pub trait Known { const ALIGN: usize; }
pub struct Misaligned<S, D>(pub S, pub std::marker::PhantomData<D>);
impl<S, D> Misaligned<S, D> { pub fn new(s: S) -> Self { Misaligned(s, std::marker::PhantomData) } }
impl<S: fmt::Debug, D: Known> fmt::Display for Misaligned<S, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{:?} not at {}", self.0, D::ALIGN) }
}
pub struct Eight;
macro_rules! known { ($ty:ty) => { impl Known for $ty { const ALIGN: usize = 8; } }; }

fn in_macro() -> String {
    known!(Eight);
    Misaligned::<_, Eight>::new(3u8).to_string()
}

fn by_hand() -> String {
    impl Known for u16 {
        const ALIGN: usize = 2;
    }
    Misaligned::<_, u16>::new(4u8).to_string()
}

pub fn run() -> (String, String) {
    (in_macro(), by_hand())
}
