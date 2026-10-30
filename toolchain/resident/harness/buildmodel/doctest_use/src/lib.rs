/// On a use.
///
/// ```
/// assert!(true);
/// ```
use core::fmt;

/// ```
/// assert!(true);
/// ```
pub use core::{mem, ptr as p};

/// ```
/// assert!(true);
/// ```
pub use core::cell::*;

/// ```
/// assert!(true);
/// ```
extern crate core as c2;

pub mod m {
    /// ```
    /// assert!(true);
    /// ```
    use super::S as T;
    /// ```
    /// assert!(true);
    /// ```
    #[allow(unused)]
    use super::{S as _, self as q};
}
pub struct S;
impl fmt::Debug for S { fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result { let _ = (p::null::<u8>(), mem::size_of::<u8>()); Ok(()) } }
pub mod n {
    /// ```
    /// assert!(true);
    /// ```
    #[allow(unused)]
    pub use core::{fmt::{self, Debug as D, Write}, cell::{Cell, RefCell as _}, mem::*, ptr::{self}};
    /// ```
    /// assert!(true);
    /// ```
    #[allow(unused)]
    extern crate core as _;
    /// ```
    /// assert!(true);
    /// ```
    #[allow(unused)]
    use crate as krate;
}

/// A constant without a name is `_`.
///
/// ```
/// assert!(true);
/// ```
const _: () = ();

/// An example of attributes alone still has a `main`.
///
/// ```
/// #![allow(dead_code)]
/// ```
pub fn attributes_alone() {}
