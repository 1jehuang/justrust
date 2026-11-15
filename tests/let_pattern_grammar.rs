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

#[test]
fn let_else_initializer_temporaries_end_before_the_pattern_test() {
    oracle::check(
        "let-else-temporaries",
        &[
            Case {
                name: "borrowed_and_owned_temporaries_in_initializers",
                source: r#"struct Noisy(&'static str);
impl Drop for Noisy {
    fn drop(&mut self) {
        println!("drop {}", self.0);
    }
}
impl Noisy {
    fn first(&self) -> Option<char> {
        self.0.chars().next()
    }
}
struct Cursor {
    text: &'static str,
    pos: usize,
}
impl Cursor {
    fn bump(&mut self) -> Result<char, String> {
        let Some(c) = self.text[self.pos..].chars().next() else {
            return Err(format!("end at {}", self.pos));
        };
        self.pos += c.len_utf8();
        Ok(c)
    }
}
fn head(name: &'static str) -> char {
    let Some(c) = Noisy(name).first() else {
        println!("else {}", name);
        return '?';
    };
    println!("bound {}", c);
    c
}
fn parse(text: &str) -> i32 {
    let Ok(n) = text.trim().to_string().parse::<i32>() else {
        return -1;
    };
    n
}
fn main() {
    let mut c = Cursor { text: "h\u{e9}!", pos: 0 };
    println!("{:?} {:?} {:?} {:?}", c.bump(), c.bump(), c.bump(), c.bump());
    println!("{} {}", head("abc"), head(""));
    println!("{} {}", parse(" 42 "), parse("x"));
    let v = vec![String::from("k=v"), String::from("bare")];
    for item in &v {
        let Some((k, val)) = item.as_str().split_once('=') else {
            println!("no pair in {}", item);
            continue;
        };
        println!("{} -> {}", k, val);
    }
}"#,
                expect: Expect::Run(
                    "Ok('h') Ok('é') Ok('!') Err(\"end at 4\")\ndrop abc\nbound a\ndrop \nelse \na ?\n42 -1\nk -> v\nno pair in bare\n",
                ),
            },
            Case {
                name: "binding_cannot_borrow_a_dropped_temporary",
                source: r#"fn unused() -> usize {
    let Some(s) = Some(String::from("temp").as_str()) else { return 0; };
    s.len()
}
fn main() {}"#,
                expect: Expect::Invalid("E0716"),
            },
            Case {
                name: "else_block_must_diverge",
                source: r#"fn unused(text: &str) -> char {
    let Some(c) = text[1..].chars().next() else { 0u8 };
    c
}
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
