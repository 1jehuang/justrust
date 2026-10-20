//! Static associated calls bind the written owner before their nested result.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn static_owner_nested_returns_match_stock() {
    oracle::check(
        "static-owner-nested",
        &[
            Case {
                name: "nested_option_self_and_ordered_method_actuals",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> {
    fn wrap(value: T) -> Option<Self> { Some(Self { value }) }
    fn pair<U>(value: T, extra: U) -> Option<(Self, U)> {
        Some((Self { value }, extra))
    }
}
fn main() {
    let a = Packet::<u64>::wrap(42);
    let b = Packet::<i32>::pair::<u16>(-7, 9);
    match a { Some(p) => println!("{}", p.value), None => println!("missing") }
    match b { Some((p, extra)) => println!("{} {}", p.value, extra), None => println!("missing") }
}
"#,
                expect: Expect::Run("42\n-7 9\n"),
            },
            Case {
                name: "written_owner_constrains_generic_null_argument",
                source: r#"
struct Pointer<T> { pointer: *mut T }
impl<T> Pointer<T> {
    fn wrap(pointer: *mut T) -> Option<Self> { Some(Self { pointer }) }
}
fn main() {
    let value = Pointer::<i32>::wrap(core::ptr::null_mut());
    match value { Some(p) => println!("{}", p.pointer as usize == 0usize), None => println!("missing") }
}
"#,
                expect: Expect::Run("true\n"),
            },
        ],
    );
}
