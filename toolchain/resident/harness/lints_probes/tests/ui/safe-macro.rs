#![deny(deprecated_safe_2024)]
#![allow(dead_code, unused)]
//@ edition: 2021
macro_rules! m { () => { std::env::set_var("A", "1") } }
pub fn f() { m!(); }
