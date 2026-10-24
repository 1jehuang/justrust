fn equal(a: &str, b: &str) -> bool { a == b }
fn main() { assert!(equal("same", "same")); assert!(!equal("same", "other")); }
