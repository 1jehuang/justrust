// Names injected into every user module scope. User items and explicit imports
// shadow these. Glob imports remain unsupported.
pub mod core_v1 {
    pub use super::super::iter::{Iterator, IntoIterator, FromIterator, DoubleEndedIterator, Extend};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::mem::drop;
    pub use super::super::convert::{From, Into};
    pub use super::super::primitive::{ByteMethods, CharMethods, CheckedShiftMethods, FloatBits, IntMethods, RadixParse};
    pub use super::super::float_math::{FloatMath, FloatTrig};
    // Allocation-free slice methods are core `[T]` inherent methods.
    pub use super::super::slice::{SliceChunks, SliceContains, SliceIter, SliceIterMut, SliceMethods, SliceSort, SliceMutRefMethods, SliceRefMethods};
}

pub mod v1 {
    pub use super::super::iter::{Iterator, IntoIterator, FromIterator, DoubleEndedIterator, Extend};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::vec::Vec;
    pub use super::super::boxed::Box;
    pub use super::super::string::{SliceJoin, StrChars, StrMethods, StrToString, String, ToString};
    pub use super::super::slice::{SliceChunks, SliceContains, SliceIter, SliceIterMut, SliceMethods, SliceSort, SliceMutRefMethods, SliceRefMethods, SliceToVec};
    pub use super::super::ptr::{ConstPtr, MutPtr};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::mem::drop;
    pub use super::super::convert::{From, Into};
    pub use super::super::primitive::{ByteMethods, CharMethods, CheckedShiftMethods, FloatBits, IntMethods, RadixParse};
    pub use super::super::float_math::{FloatMath, FloatTrig};
}

// Editions 2015/2018: the 2021 additions (FromIterator) are absent.
pub mod core_v1_2018 {
    pub use super::super::iter::{Iterator, IntoIterator, DoubleEndedIterator};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::mem::drop;
    pub use super::super::convert::{From, Into};
    pub use super::super::primitive::{ByteMethods, CharMethods, CheckedShiftMethods, FloatBits, IntMethods, RadixParse};
    pub use super::super::float_math::{FloatMath, FloatTrig};
    // Allocation-free slice methods are core `[T]` inherent methods.
    pub use super::super::slice::{SliceChunks, SliceContains, SliceIter, SliceIterMut, SliceMethods, SliceSort, SliceMutRefMethods, SliceRefMethods};
}

// Editions 2015/2018: the 2021 additions (FromIterator) are absent.
pub mod v1_2018 {
    pub use super::super::iter::{Iterator, IntoIterator, DoubleEndedIterator};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::vec::Vec;
    pub use super::super::boxed::Box;
    pub use super::super::string::{SliceJoin, StrChars, StrMethods, StrToString, String, ToString};
    pub use super::super::slice::{SliceChunks, SliceContains, SliceIter, SliceIterMut, SliceMethods, SliceSort, SliceMutRefMethods, SliceRefMethods, SliceToVec};
    pub use super::super::ptr::{ConstPtr, MutPtr};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::mem::drop;
    pub use super::super::convert::{From, Into};
    pub use super::super::primitive::{ByteMethods, CharMethods, CheckedShiftMethods, FloatBits, IntMethods, RadixParse};
    pub use super::super::float_math::{FloatMath, FloatTrig};
}
