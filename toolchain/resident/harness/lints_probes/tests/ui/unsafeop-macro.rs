#![deny(unsafe_op_in_unsafe_fn)]
#![allow(dead_code, unused)]
pub unsafe fn danger() {}
pub static mut S: u8 = 0;
pub union U { pub a: u8, pub b: i8 }
macro_rules! m { () => { danger() } }
pub unsafe fn f() { m!(); }
