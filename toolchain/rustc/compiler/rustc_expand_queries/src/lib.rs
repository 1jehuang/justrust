#![allow(internal_features, reason = "proc macro internals")]
#![feature(proc_macro_internals)]

mod attr;
mod declarative;
mod derive;

pub fn setup_callbacks() {
    rustc_expand::declarative_cache::EXPAND_DECLARATIVE_MACRO_CACHED
        .swap(&(declarative::expand_declarative_macro_cached as _));
    rustc_expand::proc_macro::EXPAND_DERIVE_MACRO_CACHED
        .swap(&(derive::expand_derive_macro_cached as _));
    rustc_expand::proc_macro::EXPAND_ATTR_MACRO_CACHED
        .swap(&(attr::expand_attr_macro_cached as _));
}

pub fn provide(providers: &mut rustc_middle::query::Providers) {
    providers.declarative_macro_expansion = declarative::declarative_macro_expansion;
    providers.derive_macro_expansion = derive::derive_macro_expansion;
    providers.attr_macro_expansion = attr::attr_macro_expansion;
}
