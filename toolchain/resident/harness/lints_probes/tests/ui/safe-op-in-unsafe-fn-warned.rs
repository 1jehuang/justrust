#![deny(deprecated_safe_2024)]
#![warn(unsafe_op_in_unsafe_fn)]
#![allow(dead_code, unused)]
//@ edition: 2021
pub unsafe fn f() { std::env::set_var("A", "1"); }
