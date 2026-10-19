//! Trait method equality bounds retain callable source ownership.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn default_method_callable_equality_executes() {
    oracle::stock_native(
        "trait Apply { fn apply<F: Fn(i32) -> i32>(&self, f: F) -> i32 { f(3) } } struct S; impl Apply for S {} fn main() { let f = |x| x + 1; println!(\"{}\", S.apply(f)); }",
    );
}

#[test]
fn required_method_callable_equality_executes() {
    oracle::stock_native(
        "trait Apply { fn apply<F: Fn(i32) -> i32>(&self, f: F) -> i32; } struct S; impl Apply for S { fn apply<F: Fn(i32) -> i32>(&self, f: F) -> i32 { f(3) } } fn main() { let f = |x| x + 1; println!(\"{}\", S.apply(f)); }",
    );
}

#[test]
fn invalid_unused_method_callable_body_is_rejected() {
    oracle::stock_and_native_reject(
        "trait Apply { fn apply<F: Fn(i32) -> i32>(&self, f: F) -> i32 { f(true) } } fn main() {}",
    );
}
