//! The examples of a derive are those of the function that is written, whatever the derive
//! is called.
use proc_macro::TokenStream;

/// ```
/// #[derive(app::Nothing)]
/// struct Unit;
/// assert_eq!(std::mem::size_of::<Unit>(), 0);
/// ```
#[proc_macro_derive(Nothing)]
pub fn derive_nothing(_: TokenStream) -> TokenStream {
    TokenStream::new()
}

/// ```
/// assert_eq!(app::same!(1 + 1), 2);
/// ```
#[proc_macro]
pub fn same(tokens: TokenStream) -> TokenStream {
    tokens
}
