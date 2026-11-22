// tidy-alphabetical-start
#![feature(decl_macro)]
#![feature(file_buffered)]
#![feature(iter_intersperse)]
#![feature(try_blocks)]
// tidy-alphabetical-end

mod callbacks;
pub mod diagnostics;
pub mod interface;
mod limits;
pub mod passes;
mod queries;
pub mod resident;
pub mod util;

pub use callbacks::setup_callbacks;
pub use interface::{Config, run_compiler};
pub use passes::{DEFAULT_QUERY_PROVIDERS, create_and_enter_global_ctxt, parse};
pub use queries::{
    Linker, finalize_after_overlap, link_before_persist_and_reply, link_before_persist_enabled,
    overlap_persist_and_link, overlap_persist_enabled,
};

#[cfg(test)]
mod tests;
