//! Macros that hand back literals built from values, not parsed from text.

use proc_macro::{Literal, TokenStream, TokenTree};

#[proc_macro]
pub fn minus_one(_input: TokenStream) -> TokenStream {
    TokenTree::Literal(Literal::i32_suffixed(-1)).into()
}

#[proc_macro]
pub fn minus_half(_input: TokenStream) -> TokenStream {
    TokenTree::Literal(Literal::f32_suffixed(-0.5)).into()
}

#[proc_macro]
pub fn dashed(_input: TokenStream) -> TokenStream {
    TokenTree::Literal(Literal::string("-1")).into()
}

/// Names the unit struct it is on, and takes nothing but `struct Unit;` spelled so.
#[proc_macro_derive(NamedByText)]
pub fn named_by_text(input: TokenStream) -> TokenStream {
    assert_eq!(input.to_string(), "struct Unit;");
    "impl Unit { pub fn name() -> &'static str { \"Unit\" } }".parse().unwrap()
}
