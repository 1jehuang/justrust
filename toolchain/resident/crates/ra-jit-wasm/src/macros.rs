//! Proc macros that are part of the compiler, for the calls a pack has no expansion of: an
//! edit of `rsx!` is one. A proc macro is a native library the page cannot load, but one
//! whose work is done by an ordinary crate can be linked in, and so can the code of one that
//! only needs `syn` and `proc_macro2`.

use std::str::FromStr;

use core_macro::component::{ComponentBody, ComponentMacroOptions};
use dioxus_rsx::CallBody;
use proc_macro2::TokenStream;
use quote::ToTokens;

use crate::pack::Call;

/// The expansion of `call` by the macro of that name here, when there is one.
pub fn expand(call: &Call) -> Option<Result<String, String>> {
    match (call.krate.as_str(), call.name.as_str()) {
        ("dioxus_core_macro", "rsx") => Some(rsx(&call.input)),
        ("dioxus_core_macro", "component") => Some(component(call)),
        ("dioxus_core_macro", "Props" | "derive_props") => Some(props(&call.input)),
        _ => None,
    }
}

/// `dioxus_core_macro::rsx`.
fn rsx(input: &str) -> Result<String, String> {
    let tokens = TokenStream::from_str(input).map_err(|e| e.to_string())?;
    Ok(match syn::parse2::<CallBody>(tokens) {
        Ok(body) => body.into_token_stream().to_string(),
        Err(e) => e.to_compile_error().to_string(),
    })
}

/// `#[dioxus_core_macro::component]`.
fn component(call: &Call) -> Result<String, String> {
    let tokens = |text: &str| TokenStream::from_str(text).map_err(|e| e.to_string());
    let options = syn::parse2::<ComponentMacroOptions>(tokens(call.attr.as_deref().unwrap_or(""))?);
    let body = syn::parse2::<ComponentBody>(tokens(&call.input)?);
    Ok(match (body, options) {
        (Ok(body), Ok(options)) => body.with_options(options).into_token_stream().to_string(),
        (Err(e), _) | (_, Err(e)) => e.to_compile_error().to_string(),
    })
}

/// `#[derive(dioxus_core_macro::Props)]`.
fn props(input: &str) -> Result<String, String> {
    let tokens = TokenStream::from_str(input).map_err(|e| e.to_string())?;
    let output = syn::parse2::<syn::DeriveInput>(tokens)
        .and_then(|input| core_macro::props::impl_my_derive(&input));
    Ok(output.unwrap_or_else(|e| e.to_compile_error()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, attr: Option<&str>, input: &str) -> String {
        let call = Call {
            krate: "dioxus_core_macro".to_owned(),
            name: name.to_owned(),
            attr: attr.map(str::to_owned),
            input: input.to_owned(),
        };
        expand(&call).expect("a macro this has").expect("an expansion")
    }

    #[test]
    fn component_with_props() {
        let expansion = call(
            "component",
            Some(""),
            "fn Badge(label: String, n: i32) -> Element { rsx! { p { \"{label}: {n}\" } } }",
        );
        assert!(expansion.contains("struct BadgeProps"), "{expansion}");
        assert!(!expansion.contains("compile_error"), "{expansion}");
    }

    #[test]
    fn component_that_is_not_a_function() {
        let expansion = call("component", Some(""), "struct Badge;");
        assert!(expansion.contains("compile_error"), "{expansion}");
    }

    #[test]
    fn derive_props() {
        let expansion = call("Props", None, "#[derive(Clone, PartialEq)] struct P { n: i32 }");
        assert!(expansion.contains("impl"), "{expansion}");
        assert!(!expansion.contains("compile_error"), "{expansion}");
    }
}
