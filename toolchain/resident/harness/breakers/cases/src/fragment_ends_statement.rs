//! tests/ui/parser/macro/statement-boundaries.rs: an `if` passed as `$e:expr` still ends a
//! statement where it is written out, so `$e |x| x` is the `if` and then a closure. ra-jit
//! writes an expression fragment out in parentheses to keep its precedence, which makes it an
//! operand of `|`; that does not type, and the function is left to rustc.

pub fn run() -> u32 {
    breakers2021::fragment_ends_statement::closure_after_fragment()
}
