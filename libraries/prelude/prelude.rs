// Names injected into every user module scope. User items and explicit imports
// shadow these. Glob imports remain unsupported.
pub mod core_v1 {
    pub use super::super::iter::{Iterator, IntoIterator};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::convert::From;
    pub use super::super::primitive::{CharMethods, IntMethods};
}

pub mod v1 {
    pub use super::super::iter::{Iterator, IntoIterator};
    pub use super::super::option::Option;
    pub use super::super::option::Option::{None, Some};
    pub use super::super::result::Result;
    pub use super::super::result::Result::{Err, Ok};
    pub use super::super::vec::Vec;
    pub use super::super::boxed::Box;
    pub use super::super::string::{StrChars, StrMethods, StrToString, String, ToString};
    pub use super::super::slice::{SliceContains, SliceMethods, SliceRefMethods, SliceToVec};
    pub use super::super::ptr::{ConstPtr, MutPtr};
    pub use super::super::ops::{Fn, FnMut, FnOnce};
    pub use super::super::cmp::{Eq, Ord, PartialEq, PartialOrd};
    pub use super::super::default::Default;
    pub use super::super::convert::From;
    pub use super::super::primitive::{CharMethods, IntMethods};
}
