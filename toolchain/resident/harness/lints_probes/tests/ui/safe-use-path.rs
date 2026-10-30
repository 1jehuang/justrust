#![deny(deprecated_safe_2024)]
#![allow(dead_code, unused)]
//@ edition: 2021
use std::env::set_var;
pub fn f() { set_var("A", "1"); }
