#![deny(deprecated_safe_2024)]
#![allow(dead_code, unused)]
//@ edition: 2021
pub fn f() { let g = std::env::set_var::<&str, &str>; g("A", "1"); }
