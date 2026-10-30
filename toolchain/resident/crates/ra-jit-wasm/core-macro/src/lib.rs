//! The code of `dioxus-core-macro` as an ordinary library. A proc macro crate cannot be linked
//! as one, but this one does its work with `syn` and `proc_macro2`, so its modules compile
//! outside of it and its macros can run where there is no rustc to load them, like in a page.
#![allow(warnings, clippy::all)]

include!(concat!(env!("OUT_DIR"), "/core_macro/mod.rs"));
