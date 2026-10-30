//! tests/ui/proc-macro/negative-token.rs: a negative number a procedural macro makes with
//! `Literal::i32_suffixed` is one token whose text starts with a minus sign. It was read as
//! a literal of that text, which is no number, and came out as zero.

pub fn run() -> (i32, f32, &'static str) {
    (literals::minus_one!(), literals::minus_half!(), literals::dashed!())
}
