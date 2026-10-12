use proc_macro::TokenStream;
/// #[repeat(n)] fn f() {..}: emits the fn plus `fn f_times() -> u32 { n }`.
#[proc_macro_attribute]
pub fn repeat(attr: TokenStream, item: TokenStream) -> TokenStream {
    let n: u32 = attr.to_string().trim().parse().unwrap_or(1);
    let s = item.to_string();
    let name = s.split("fn ").nth(1).unwrap().split('(').next().unwrap().trim().to_string();
    format!("{s} pub fn {name}_times() -> u32 {{ {n} }}").parse().unwrap()
}
/// derive(Fields): `fn field_count() -> usize`.
#[proc_macro_derive(Fields)]
pub fn fields(item: TokenStream) -> TokenStream {
    let s = item.to_string();
    let name = s.split("struct ").nth(1).unwrap().split(|c: char| !c.is_alphanumeric()).next().unwrap().to_string();
    let n = s.matches(':').count();
    format!("impl {name} {{ pub fn field_count() -> usize {{ {n} }} }}").parse().unwrap()
}
/// Reads an env var at expansion time (the documented caveat).
#[proc_macro_attribute]
pub fn env_tag(_a: TokenStream, item: TokenStream) -> TokenStream {
    let v = std::env::var("E2E_TAG").unwrap_or_default();
    format!("{item} pub fn tag() -> &'static str {{ {v:?} }}").parse().unwrap()
}
