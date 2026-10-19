// justrust native prelude library (our own code, not Rust's std).
//
// The compiler loads this crate root as the hidden module behind the `std::`
// and `core::` paths and injects `prelude::v1` names into every user module.
// See PRELUDE.md. Add a file with `pub mod name;` here plus one entry in
// src/compiler/source_prelude.rs.

pub mod option;
pub mod result;
pub mod prelude;
pub mod native_heap;
pub mod mem;
pub mod process;
pub mod boxed;
pub mod vec;
pub mod ops;
//pub mod cmp;
//pub mod default;
//pub mod hash;
pub mod string;
//pub mod fmt; // gated until per-task Work budget lands
pub mod ptr;
