#![deny(deprecated_safe_2024)]
#![allow(dead_code, unused)]
//@ edition: 2021
pub fn f() { std::env::set_var("A", "1"); }
