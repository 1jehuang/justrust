//! tests/ui/proc-macro/derive-same-struct.rs and five more: a procedural macro that asserts
//! how its input is spelled. rustc spells `struct Unit;` as it is written; the macro server
//! of the stable toolchain, which is what expands macros for ra-jit, spells it `struct Unit ;`
//! with every token apart. The macro panics, what it would have written is missing, and the
//! function that uses it is left to rustc, with the panic in the reason.

pub fn run() -> &'static str {
    breakers2021::macro_reads_its_spelling::name()
}
