//! Reissuance of a static loan requires the previous dynamic loan to be dead.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn loan_renewal_matches_stock() {
    oracle::check(
        "loan-renewal",
        &[
            Case {
                name: "guard_failure_rebinds_payload",
                source: include_str!("fixtures/patterns/guarded_borrowed_or.rs"),
                expect: Expect::Run("true 2\n"),
            },
            Case {
                name: "nested_loop_reborrows",
                source: r#"
fn main() { let mut value = 0; let mut i = 0;
    'outer: while i < 3 { i += 1; let mut j = 0;
        while j < 4 { j += 1; let r = &mut value; *r += 1;
            if j == 1 { continue; }
            if i == 2 { continue 'outer; }
            if j == 3 { break; }
        }
    }
    println!("{}", value);
}"#,
                expect: Expect::Run("8\n"),
            },
            Case {
                name: "escaped_shared_loan_conflicts",
                source: r#"
fn main() { let mut value = 0; let mut old = &value; let mut i = 0;
    while i < 2 { value += 1; println!("{}", *old); old = &value; i += 1; }
    println!("{}", *old);
}"#,
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
