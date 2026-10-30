#![deny(unused_braces, unused_import_braces)]
#![allow(dead_code, unused)]
pub mod a { pub fn b() {} pub fn c() {} pub mod d { pub fn e() {} pub fn g() {} } }
macro_rules! m { () => { use a::{b}; } }
m!();
