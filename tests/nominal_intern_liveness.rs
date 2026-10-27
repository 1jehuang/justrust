//! Nested nominal inference must finish without recursively locking PlanState.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_nominal_inference_checks_builds_and_runs_like_stock() {
    oracle::check(
        "nominal-intern-liveness",
        &[
            Case {
                name: "nested_actual_in_generic_field",
                source: r#"struct Inner<T> { value: T }
struct Outer<T> { data: Inner<T> }
fn main() {
    let nested = Inner { value: Inner { value: 3i32 } };
    let outer = Outer { data: nested };
    println!("{}", outer.data.value.value);
}"#,
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "independent_deep_nominal_frontiers",
                source: r#"struct Wrap<T> { value: T }
fn deep(x: Wrap<Wrap<Wrap<Wrap<i32>>>>) -> i32 {
    x.value.value.value.value
}
fn shallow(x: Wrap<Wrap<bool>>) -> bool { x.value.value }
fn main() {
    let x = Wrap { value: Wrap { value: Wrap { value: Wrap { value: 17i32 } } } };
    let y = Wrap { value: Wrap { value: true } };
    println!("{} {}", deep(x), shallow(y));
}"#,
                expect: Expect::Run("17 true\n"),
            },
        ],
    );
}
