// justrust native prelude library (our own code, not Rust's std).
//
// The compiler loads this crate root as the hidden module behind the `std::`
// and `core::` paths and injects `prelude::v1` names into every user module.
// See PRELUDE.md. Add a file with `pub mod name;` here plus one entry in
// src/compiler/source_prelude.rs.

pub mod core_surface {
    pub use super::option;
    pub use super::result;
    pub mod mem {
        pub use super::super::mem::{size_of, align_of, swap, replace};
    }
    pub use super::ops;
    pub use super::cmp;
    pub use super::default;
    pub use super::convert;
    pub use super::str;
    pub use super::num;
    pub mod hash {
        pub use super::super::hash::{Hash, Hasher};
    }
    pub mod ptr {
        pub use super::super::ptr::{null, null_mut, read, write, eq};
    }
    pub use super::slice;
    pub use super::iter;
    pub mod prelude {
        pub use super::super::prelude::core_v1 as v1;
    }
}

pub mod option;
pub mod result;
pub mod prelude;
pub mod native_heap;
pub mod mem;
pub mod process;
pub mod boxed;
pub mod vec;
pub mod ops;
pub mod cmp;
pub mod default;
pub mod convert;
pub mod hash;
pub mod collections;
pub mod string;
mod unicode_uppercase;
pub mod fmt;
pub mod primitive;
pub mod str {
    pub use super::primitive::FromStr;
}
pub mod num {
    pub use super::primitive::{IntErrorKind, ParseIntError};
}
pub mod primitive_constants;
pub mod ptr;
pub mod slice;
pub mod iter;
pub mod test;
