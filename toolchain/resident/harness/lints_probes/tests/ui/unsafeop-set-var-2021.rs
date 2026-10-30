//@ edition: 2021
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(dead_code, unused)]
pub unsafe fn danger() {}
pub static mut S: u8 = 0;
pub union U { pub a: u8, pub b: i8 }
pub unsafe fn f() { std::env::set_var("A", "1"); }
