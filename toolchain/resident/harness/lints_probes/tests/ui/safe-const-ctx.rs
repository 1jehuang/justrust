#![deny(deprecated_safe_2024)]
#![allow(dead_code, unused)]
//@ edition: 2021
pub fn f() -> usize { let _ = [0u8; 1]; std::env::var("A").map(|_| 1).unwrap_or(0) }
