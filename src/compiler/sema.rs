//! General semantic core: inference, trait solving and body checking.
//! Pure library. See SEMANTIC-CORE.md.

pub mod check;
pub mod hir;
pub mod infer;
pub mod solve;
pub mod ty;

#[cfg(test)]
mod tests;
