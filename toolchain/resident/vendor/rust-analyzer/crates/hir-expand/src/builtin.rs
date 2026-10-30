//! Builtin macros and attributes
#[macro_use]
pub mod quote;

mod attr_macro;
mod derive_macro;
mod fn_macro;

pub use self::{
    attr_macro::{BuiltinAttrExpander, find_builtin_attr, pseudo_derive_attr_expansion},
    derive_macro::{BuiltinDeriveExpander, find_builtin_derive},
    fn_macro::{
        BuiltinFnLikeExpander, EagerExpander, MODULE_PATH_PLACEHOLDER, find_builtin_macro,
        include_input_to_file_id,
    },
};
