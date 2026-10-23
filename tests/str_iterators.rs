//! Borrowed `str` iterators from the native prelude, checked against stock
//! rustc at workers 1 and 8 (CHECK and BUILD).
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::{Case, Expect};

#[test]
fn string_emptiness_and_nominal_error_parser_match_stock() {
    closure_oracle::check(
        "str-emptiness-parser",
        &[
            Case {
                name: "empty_unicode_nul_and_borrowed_strings",
                source: r#"fn empty(s: &str) -> bool { s.is_empty() }
                    fn main() {
                        for s in ["", "é", "日🦀", "\0", "a\0b"] {
                            println!("{}", empty(s));
                        }
                        let owned = String::from("");
                        println!("{}", empty(&owned));
                    }"#,
                expect: Expect::Run("true\nfalse\nfalse\nfalse\nfalse\ntrue\n"),
            },
            Case {
                name: "nominal_error_parser",
                source: r#"#[derive(Debug)]
                    enum ParseError { Empty, BadDigit(char), Overflow }
                    fn parse_u32(s: &str) -> Result<u32, ParseError> {
                        if s.is_empty() { return Err(ParseError::Empty); }
                        let mut n: u32 = 0;
                        for c in s.chars() {
                            let d = c.to_digit(10).ok_or(ParseError::BadDigit(c))?;
                            n = n.checked_mul(10).ok_or(ParseError::Overflow)?;
                            n = n.checked_add(d).ok_or(ParseError::Overflow)?;
                        }
                        Ok(n)
                    }
                    fn main() {
                        for s in ["123", "", "12a", "99999999999", "4294967295", "4294967296", "0"] {
                            println!("{:?}", parse_u32(s));
                        }
                    }"#,
                expect: Expect::Run(
                    "Ok(123)\nErr(Empty)\nErr(BadDigit('a'))\nErr(Overflow)\nOk(4294967295)\nErr(Overflow)\nOk(0)\n",
                ),
            },
            Case {
                name: "unused_wrong_argument_count",
                source: "fn unused(s: &str) -> bool { s.is_empty(1) } fn main() {}",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "unused_wrong_generic_count",
                source: "fn unused(s: &str) -> bool { s.is_empty::<u8>() } fn main() {}",
                expect: Expect::Invalid("E0107"),
            },
            Case {
                name: "unused_wrong_result_type",
                source: "fn unused(s: &str) -> u32 { s.is_empty() } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

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
