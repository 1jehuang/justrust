//! Explicit method arguments retain canonical outer impl substitutions.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn explicit_method_arguments_preserve_generic_owner() {
    oracle::check(
        "explicit-method-owner",
        &[Case {
            name: "concrete_method_actual_with_generic_owner",
            source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> {
    fn new(value: T) -> Self { Self { value } }
    fn relay(self) -> Self { self }
    fn pair<U>(self, extra: U) -> (T, U) { (self.value, extra) }
}
fn main() {
    let small = Packet::<u64>::new(42u64).pair::<bool>(true);
    let large = Packet::<[u64;9]>::new([7u64;9]).relay().pair::<i32>(9);
    println!("{} {} {} {}", small.0, small.1, large.0[8], large.1);
}
"#,
            expect: Expect::Run("42 true 7 9\n"),
        }],
    );
}
