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
                name: "if_let_temporary_scrutinee_else_break_and_return",
                source: r#"fn segments<T: std::fmt::Debug>(word: &str, tag: T) -> usize {
    let mut it = word.char_indices().peekable();
    let mut init = 0;
    let mut n = 0;
    while let Some((i, c)) = it.next() {
        if let Some(&(next_i, next)) = it.peek() {
            if c.is_lowercase() && next.is_uppercase() {
                println!("{:?} {}", tag, &word[init..next_i]);
                init = next_i;
                n += 1;
            } else if i > 100 {
                return 0;
            }
        } else {
            println!("{:?} {}", tag, &word[init..]);
            break;
        }
    }
    n
}
fn first(v: &[u8]) -> u8 {
    let mut it = v.iter();
    loop {
        let x = it.next();
        if let Some(_) = Some(3) { if let Some(&b) = x { if b > 1 { return b; } } else { return 0; } } else { return 9; }
    }
}
fn main() {
    println!("{}", segments("helloWorldFooBar", 'x'));
    println!("{} {}", first(&[1, 2]), first(&[]));
}"#,
                expect: Expect::Run("'x' hello\n'x' World\n'x' Foo\n'x' Bar\n3\n2 0\n"),
            },
            Case {
                name: "if_let_else_break_still_rejects_moved_binding",
                source: r#"fn main() { let v = vec![String::new()]; let mut it = v.into_iter(); while let Some(s) = it.next() { if let Some(_) = Some(1) { drop(s); } else { break; } println!("{}", s); } }"#,
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "if_let_else_break_still_rejects_maybe_uninit",
                source: r#"fn main() { let mut x: i32; loop { if let Some(_) = Some(1).filter(|v| *v > 5) { x = 1; } else { break; } } println!("{}", x); }"#,
                expect: Expect::Invalid("E0381"),
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
