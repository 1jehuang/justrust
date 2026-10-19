//! Let patterns require parentheses around a top-level or-pattern.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn let_pattern_grammar_matches_stock() {
    oracle::check(
        "let-pattern-grammar",
        &[
            Case {
                name: "parenthesized_at_binding",
                source: r#"fn main() { let (n @ _) = 42; println!("{}", n); }"#,
                expect: Expect::Run("42\n"),
            },
            Case {
                name: "unparenthesized_or_let",
                source: r#"fn main() { let x | x = 42; println!("{}", x); }"#,
                expect: Expect::Invalid("top-level or-patterns"),
            },
            Case {
                name: "unparenthesized_or_let_else",
                source: r#"enum E { A(i32), B(i32) }
fn main() { let E::A(x) | E::B(x) = E::A(42) else { return; }; println!("{}", x); }"#,
                expect: Expect::Invalid("top-level or-patterns"),
            },
        ],
    );
}
