//! Nested nominal inference must finish without recursively locking PlanState.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_nominal_inference_checks_builds_and_runs_like_stock() {
    oracle::check(
        "nominal-intern-liveness",
        &[Case {
            name: "nested_actual_in_generic_field",
            source: r#"struct Inner<T> { value: T }
struct Outer<T> { data: Inner<T> }
fn main() {
    let nested = Inner { value: Inner { value: 3i32 } };
    let outer = Outer { data: nested };
    println!("{}", outer.data.value.value);
}"#,
            expect: Expect::Run("3\n"),
        }],
    );
}
