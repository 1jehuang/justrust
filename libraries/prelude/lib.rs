// justrust native prelude library (our own code, not Rust's std).
//
// The compiler loads this crate root as the hidden module behind the `std::`
// and `core::` paths and injects `prelude::v1` names into every user module.
// See PRELUDE.md. Add a file with `pub mod name;` here plus one entry in
// src/compiler/source_prelude.rs.

pub mod alloc_surface {
    pub use super::boxed;
    pub use super::string;
    pub use super::vec;
}

pub mod core_surface {
    pub use super::option;
    pub use super::result;
    pub mod mem {
        pub use super::super::mem::{size_of, align_of, swap, replace, drop};
    }
    pub use super::ops;
    pub use super::cmp;
    pub use super::default;
    pub use super::convert;
    pub use super::str;
    pub use super::num;
    pub use super::f32;
    pub use super::f64;
    pub use super::char;
    pub mod hash {
        pub use super::super::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
    }
    pub mod ptr {
        pub use super::super::ptr::{null, null_mut, read, write, eq};
    }
    pub use super::slice;
    pub use super::iter;
    // core::fmt's traits and Formatter. This native Formatter buffers into
    // a heap String internally; that is an implementation detail of the
    // library, not a no_std-visible allocation API.
    pub mod fmt {
        pub use super::super::fmt::{Debug, Display, Error, Formatter, LowerExp, Result, UpperExp, Write};
    }
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
pub mod borrow;
pub mod rc;
pub mod cell {
    pub use super::rc::{BorrowError, BorrowMutError, Ref, RefCell, RefMut};
}
pub mod string;
mod unicode_uppercase;
pub mod fmt;
pub mod primitive;
pub mod str {
    pub use super::primitive::{FromStr, ParseBoolError};
    pub use super::string::Utf8Error;
    pub fn from_utf8(v: &[u8]) -> Result<&str, Utf8Error> {
        super::string::str_from_utf8(v)
    }
}
pub mod char {
    pub use super::primitive::ParseCharError;
}
pub mod num {
    pub use super::primitive::{IntErrorKind, ParseFloatError, ParseIntError};
}
pub mod primitive_constants;
pub mod float_math;
pub mod f64 {
    pub use super::float_math::f64_items::{RADIX, MANTISSA_DIGITS, DIGITS, EPSILON, MIN, MIN_POSITIVE, MAX, MIN_EXP, MAX_EXP, MIN_10_EXP, MAX_10_EXP};
    pub mod consts {
        pub use super::super::float_math::f64_consts::{PI, TAU, FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, FRAC_PI_8, FRAC_1_PI, FRAC_2_PI, FRAC_2_SQRT_PI, SQRT_2, FRAC_1_SQRT_2, E, LOG2_E, LOG2_10, LOG10_E, LOG10_2, LN_2, LN_10};
    }
}
pub mod f32 {
    pub use super::float_math::f32_items::{RADIX, MANTISSA_DIGITS, DIGITS, EPSILON, MIN, MIN_POSITIVE, MAX, MIN_EXP, MAX_EXP, MIN_10_EXP, MAX_10_EXP};
    pub mod consts {
        pub use super::super::float_math::f32_consts::{PI, TAU, FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, FRAC_PI_8, FRAC_1_PI, FRAC_2_PI, FRAC_2_SQRT_PI, SQRT_2, FRAC_1_SQRT_2, E, LOG2_E, LOG2_10, LOG10_E, LOG10_2, LN_2, LN_10};
    }
}
pub mod ptr;
pub mod slice;
pub mod iter;
pub mod test;
pub mod io;
pub mod error;
pub mod sets;
