// tidy-alphabetical-start
#![allow(internal_features)]
#![feature(associated_type_defaults)]
#![feature(default_field_values)]
#![feature(deref_patterns)]
#![feature(macro_metavar_expr)]
#![feature(proc_macro_diagnostic)]
#![feature(proc_macro_internals)]
#![feature(try_blocks)]
#![feature(yeet_expr)]
// tidy-alphabetical-end

mod build;
mod diagnostics;
mod mbe;
mod placeholders;
mod proc_macro_server;
mod stats;

pub use mbe::macro_rules::{MacroRulesMacroExpander, compile_declarative_macro};
pub mod base;
pub mod config;
pub mod declarative_cache;
pub mod expand;
pub mod module;
pub mod proc_macro;
pub mod proc_macro_client;

use rustc_data_structures::profiling::{JrPhaseGuard, SelfProfilerRef, TimingGuard, jr_phase};

/// Static labels keep disabled instrumentation off the allocation and clock paths.
/// Unlike verbose activities these high-frequency phases never print per invocation.
fn expansion_phase<'a>(
    prof: &'a SelfProfilerRef,
    label: &'static str,
) -> (TimingGuard<'a>, JrPhaseGuard) {
    (prof.generic_activity(label), jr_phase(label))
}
