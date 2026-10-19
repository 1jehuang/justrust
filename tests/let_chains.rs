//! Edition 2024 `if let` / `while let` chains, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn let_chains_match_stock() {
    oracle::check(
        "let-chains",
        &[
            Case {
                name: "if_chains",
                source: r#"fn f(o: Option<(i32, i32)>) -> i32 {
    if let Some((x, y)) = o && x < y && let Some(z) = Some(x + y) { z } else { -1 }
}
fn main() {
    println!("{} {} {}", f(Some((1, 2))), f(Some((3, 2))), f(None));
    let n: Option<i32> = None;
    if true && let Some(q) = n { println!("{}", q) } else if let Some(w) = Some(4) && w > 3 { println!("w {}", w) }
    let a = 5;
    if a > 2 && let 5 = a { println!("five"); }
}"#,
                expect: Expect::Run("3 -1 -1\nw 4\nfive\n"),
            },
            Case {
                name: "while_chains",
                source: r#"fn main() {
    let mut i = 0;
    while i < 5 && let Some(k) = Some(i * 2) { print!("{} ", k); i += 1; }
    println!();
    let mut it = 0;
    'o: while let Some(k) = if it < 9 { Some(it) } else { None } && k != 4 { it += 1; if k == 7 { break 'o; } print!("{} ", k); }
    println!("{}", it);
}"#,
                expect: Expect::Run("0 2 4 6 8 \n0 1 2 3 4\n"),
            },
            Case {
                name: "binding_out_of_scope_in_else",
                source: r#"fn main() { let o = Some(1); if let Some(x) = o && x > 0 { } else { println!("{}", x); } }"#,
                expect: Expect::Invalid("E0425"),
            },
            Case {
                name: "while_break_value",
                source: r#"fn main() { while let Some(x) = Some(1) && x > 0 { break (); } }"#,
                expect: Expect::Invalid("E0571"),
            },
        ],
    );
}
