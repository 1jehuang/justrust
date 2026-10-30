//! The procedural macro of the `fragment` case.

use proc_macro::{Delimiter, TokenStream, TokenTree};

/// The expression inside the group without delimiters that rustc passes for an `$e:expr` of a
/// `macro_rules!` macro, and a string saying so when the argument is not such a group. Crates
/// rely on the group being there (const_format's `parse_unwrap_group`).
#[proc_macro]
pub fn fragment(input: TokenStream) -> TokenStream {
    match input.into_iter().next() {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::None => group.stream(),
        _ => "\"not a group\"".parse().unwrap_or_default(),
    }
}
