//! Static-owner source authority rejection controls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn static_owner_constraints_reject_wrong_arguments_and_unused_bad_items() {
    oracle::check(
        "static-owner-invalid",
        &[
            Case {
                name: "wrong_owner_argument",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> { fn wrap(value: T) -> Option<Self> { Some(Self { value }) } }
fn main() { let value = Packet::<u8>::wrap(300u16); }
"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_ordered_method_argument",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> { fn pair<U>(value: T, extra: U) -> Option<(Self,U)> { Some((Self { value },extra)) } }
fn main() { let value = Packet::<u8>::pair::<u16>(1u8,2u32); }
"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_bad_body",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> { fn wrap(value: T) -> Option<Self> { Some(Self { value }) } }
fn unused_bad() -> u8 { 300u16 }
fn main() { let value = Packet::<u64>::wrap(42u64); }
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
