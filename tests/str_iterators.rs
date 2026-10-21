//! Borrowed `str` iterators from the native prelude, checked against stock
//! rustc at workers 1 and 8 (CHECK and BUILD).
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::{Case, Expect};

#[test]
fn split_whitespace_matches_stock() {
    closure_oracle::check(
        "str-iterators",
        &[
            Case {
                name: "split_whitespace_words",
                source: "fn main(){ let mut n = 0; let mut it = \"the cat  sat\\ton\\nthe mat\".split_whitespace(); while let Some(x) = it.next() { println!(\"[{}]\", x); n += 1; } println!(\"{}\", n); }",
                expect: Expect::Run("[the]\n[cat]\n[sat]\n[on]\n[the]\n[mat]\n6\n"),
            },
            Case {
                name: "split_whitespace_edges",
                source: "fn main(){ let mut a = \"\".split_whitespace(); let mut b = \"  x  \".split_whitespace(); let e = match a.next() { Some(_) => 1, None => 0 }; let x = match b.next() { Some(w) => w.len(), None => 99 }; let y = match b.next() { Some(_) => 1, None => 0 }; println!(\"{} {} {}\", e, x, y); }",
                expect: Expect::Run("0 1 0\n"),
            },
            Case {
                name: "split_whitespace_on_string_in_unused_fn_is_not_mutable",
                source: "fn unused(){ let it = \"a b\".split_whitespace(); it.next(); } fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
        ],
    );
}
