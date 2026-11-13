//! Argument impl Trait parameters have compiler-owned, unspellable identities.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn anonymous_impl_parameters_do_not_alias_source_names() {
    oracle::check(
        "impl-trait-hygiene",
        &[
            Case {
                name: "ordinary_impl_0_is_distinct",
                source: r#"trait V { fn value(&self)->i32; }
impl V for u8 { fn value(&self)->i32 { *self as i32 } }
fn f<impl_0>(x:impl_0,y:impl V)->impl_0 { x }
fn main() { println!("{}",f::<i32>(7,2u8)); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "anonymous_parameter_is_not_source_addressable",
                source: r#"trait V {} fn f(x:impl V)->impl_0 { x } fn main() {}"#,
                expect: Expect::Invalid("E0425"),
            },
        ],
    );
}

#[test]
fn opaque_owner_reuses_its_published_template_cfg() {
    oracle::check(
        "impl-trait-owner-template",
        &[
            Case {
                name: "opaque_iterator_owner_with_cleanup",
                source: r#"use std::collections::BTreeMap;
fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').filter(|w| !w.is_empty()).map(|w| w.to_lowercase()) }
fn main() {
    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (ln, line) in "A b a\nb C".lines().enumerate() { for w in tokens(line) { index.entry(w).or_default().push(ln + 1); } }
    let all: Vec<String> = tokens("x Y  z").collect();
    println!("{:?} {:?}", index, all);
}"#,
                expect: Expect::Run(
                    "{\"a\": [1, 1], \"b\": [1, 2], \"c\": [2]} [\"x\", \"y\", \"z\"]\n",
                ),
            },
            Case {
                name: "opaque_item_is_not_its_hidden_type",
                source: r#"fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').map(|w| w.to_lowercase()) }
fn main() { let v: Vec<&str> = tokens("a b").collect(); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "opaque_owner_moved_value_reuse",
                source: r#"fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').map(|w| w.to_lowercase()) }
fn main() { let t = tokens("a b"); let a: Vec<String> = t.collect(); let b: Vec<String> = t.collect(); }"#,
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}
