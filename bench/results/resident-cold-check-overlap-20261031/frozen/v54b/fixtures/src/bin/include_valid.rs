fn main() { let value: u32 = include!("../included/valid.rs"); assert_eq!(value, 9); }
