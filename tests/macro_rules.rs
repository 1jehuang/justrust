//! Independent stock macro oracle and direct bounded token-utility tests.
//! These tests do NOT establish native source-macro, hygiene, or full Rust admission.
//! The stock subprocess is an oracle only, never a utility/compiler fallback.
use justrust_engine::compiler::macro_rules::{
    CallSite, DefinitionSite, ExpandedTokens, MacroError, RulesDef, compile_rules, expand_rules,
};
use justrust_engine::compiler::source::PreparedLexInput;
use justrust_engine::compiler::token_primitives::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Independently pinned semantic reference, not claimed source-matched to rustc:
// rust-lang/reference src/macros-by-example.md at
// f1e8ba23e46236877fe90c86e4cb7075ad37b776, 27548 bytes, SHA256
// 32e55cf4b4f62b969d1348b15d30b6f807501e32cfa04133e2b0bc5e944dcc67.
// Reference licensed MIT OR Apache-2.0, Copyright (c) 2010 The Rust Project Developers.
// No Reference or compiler implementation is vendored here.
const BASELINE_STOCK: &str = "rustc 1.98.1; 48a229ceaefd4985c50990b14116b6d856af0985; 2026-09-01; x86_64-unknown-linux-gnu; LLVM 22.1.8";

struct StockCase {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    group: usize,
}
fn stock_cases() -> Vec<StockCase> {
    vec![
        StockCase {
            name: "tt_forward_transparent",
            source: "macro_rules! sink {(3)=>{1};($x:tt)=>{2}} macro_rules! outer {($x:tt)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(3),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ident_forward_keyword",
            source: "macro_rules! sink {(fn)=>{1};($x:ident)=>{2}} macro_rules! outer {($x:ident)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(fn),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_forward_opaque",
            source: "macro_rules! sink {(3)=>{1};($x:literal)=>{2}} macro_rules! outer {($x:literal)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(3),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_forward_fixed_reject",
            source: "macro_rules! sink {(3)=>{1}} macro_rules! outer {($x:literal)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(3),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "expr_forward_opaque",
            source: "macro_rules! sink {(3)=>{1};($x:expr)=>{2}} macro_rules! outer {($x:expr)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(3),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "expr_forward_fixed_reject",
            source: "macro_rules! sink {(3)=>{1}} macro_rules! outer {($x:expr)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!(3),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "lifetime_forward_transparent",
            source: "macro_rules! sink {('a)=>{1};($x:lifetime)=>{2}} macro_rules! outer {($x:lifetime)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!('a),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "tt_group_forward",
            source: "macro_rules! sink {((x))=>{1}} macro_rules! outer {($x:tt)=>{sink!($x)}}\nfn main(){ assert_eq!(outer!((x)),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ordered_first_match",
            source: "macro_rules! m {($x:tt)=>{1};(tag)=>{2}}\nfn main(){ assert_eq!(m!(tag),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ordered_failed_match_fallback",
            source: "macro_rules! m {(a)=>{2};(b)=>{1}}\nfn main(){ assert_eq!(m!(b),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ordered_transcription_no_fallback",
            source: "macro_rules! m {(a)=>{$missing};($x:tt)=>{1}}\nfn main(){ assert_eq!(m!(a),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "ordered_semantic_no_fallback",
            source: "macro_rules! m {(a)=>{missing_function()};($x:tt)=>{1}}\nfn main(){ assert_eq!(m!(a),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "outer_delimiter_bracket",
            source: "macro_rules! m {(x)=>{1}}\nfn main(){ assert_eq!(m![x],1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "outer_delimiter_brace",
            source: "macro_rules! m {((x))=>{1}}\nfn main(){ assert_eq!(m!{(x)},1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "inner_delimiter_mismatch",
            source: "macro_rules! m {((x))=>{1}}\nfn main(){ assert_eq!(m![[x]],1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_negative_integer",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(-1),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_negative_spaced",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(- 1),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_negative_float",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(-1.5),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_negative_bool_ignored",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(-true),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_negative_string_ignored",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(-\"x\"),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_plus_reject",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(+1),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_double_minus_reject",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!(--1),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_group_reject",
            source: "macro_rules! m {($x:literal)=>{1}}\nfn main(){ assert_eq!(m!((-1)),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "tt_negative_two_tokens",
            source: "macro_rules! m {($x:tt)=>{1}}\nfn main(){ assert_eq!(m!(-1),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "ident_keyword",
            source: "macro_rules! m {($x:ident)=>{1}}\nfn main(){ assert_eq!(m!(self),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ident_raw_keyword",
            source: "macro_rules! m {($x:ident)=>{1}}\nfn main(){ assert_eq!(m!(r#fn),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "ident_underscore_reject",
            source: "macro_rules! m {($x:ident)=>{1}}\nfn main(){ assert_eq!(m!(_),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "raw_crate_lexical_reject",
            source: "macro_rules! m {($x:tt)=>{1}}\nfn main(){ assert_eq!(m!(r#crate),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_star_empty",
            source: "macro_rules! m {($($x:literal),*)=>{0 $(+$x)*}}\nfn main(){ assert_eq!(m!(),0); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_plus_empty_reject",
            source: "macro_rules! m {($($x:literal),+)=>{0 $(+$x)*}}\nfn main(){ assert_eq!(m!(),0); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_star_three",
            source: "macro_rules! m {($($x:literal),*)=>{0 $(+$x)*}}\nfn main(){ assert_eq!(m!(1,2,3),6); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_optional_zero",
            source: "macro_rules! m {($($x:literal)?)=>{0 $(+$x)?}}\nfn main(){ assert_eq!(m!(),0); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_optional_one",
            source: "macro_rules! m {($($x:literal)?)=>{0 $(+$x)?}}\nfn main(){ assert_eq!(m!(2),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_optional_two_reject",
            source: "macro_rules! m {($($x:literal)?)=>{0 $(+$x)?}}\nfn main(){ assert_eq!(m!(1 2),0); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_optional_separator_invalid",
            source: "macro_rules! m {($($x:literal),?)=>{1}}\nfn main(){ assert_eq!(1,1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_nested_zero_rows",
            source: "macro_rules! m {($( [$($x:literal),*] );*)=>{{let mut sum=0; $($(sum+=$x;)*)* sum}}}\nfn main(){ assert_eq!(m!([1,2];[];[3]),6); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_zip_equal",
            source: "macro_rules! m {($($a:literal),* ; $($b:literal),*)=>{0 $(+$a*$b)*}}\nfn main(){ assert_eq!(m!(1,2;3,4),11); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "repeat_zip_unequal_no_fallback",
            source: "macro_rules! m {($($a:literal),* ; $($b:literal),*)=>{0 $(+$a*$b)*};($($t:tt)*)=>{1}}\nfn main(){ assert_eq!(m!(1,2;3),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_depth_invalid",
            source: "macro_rules! m {($($x:ident),*)=>{$x}}\nfn main(){ assert_eq!(m!(a,b),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "repeat_no_variable_invalid",
            source: "macro_rules! m {()=>{$(1)*}}\nfn main(){ assert_eq!(m!(),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "local_ambiguity",
            source: "macro_rules! m {($($i:ident)* $j:ident)=>{1}}\nfn main(){ assert_eq!(m!(x),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "duplicate_binding_invalid",
            source: "macro_rules! m {($x:tt $x:tt)=>{1}}\nfn main(){ assert_eq!(1,1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "trailing_input_reject",
            source: "macro_rules! m {(a)=>{1}}\nfn main(){ assert_eq!(m!(a b),1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "unknown_fragment_invalid",
            source: "macro_rules! m {($x:bogus)=>{1}}\nfn main(){ assert_eq!(1,1); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "expr2024_const_first",
            source: "macro_rules! m {($x:expr)=>{1};(const $($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(const {0}),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "expr2024_underscore",
            source: "macro_rules! m {($x:expr)=>{1}}\nfn main(){ assert_eq!(m!(_),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "expr2021_const_fallback",
            source: "macro_rules! m {($x:expr_2021)=>{1};(const $($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(const {0}),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_word_fallback",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(word),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_plus_fallback",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(+1),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "literal_doubleminus_fallback_control",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(--1),2); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_negative_ident_fallback_control",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(-word),2); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_negative_group_fallback_control",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(-(1)),2); println!(\"macro ok\"); }",
            stdout: None,
            group: 0,
        },
        StockCase {
            name: "literal_trailing_partial_fallback",
            source: "macro_rules! m {($x:literal)=>{1};($($x:tt)*)=>{2}}\nfn main(){ assert_eq!(m!(1 x),2); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "unknown_metavar_unused_control",
            source: "macro_rules! m {(a)=>{$missing};($x:tt)=>{1}}\nfn main(){ assert_eq!(1,1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "unknown_metavar_other_arm_control",
            source: "macro_rules! m {(a)=>{$missing};($x:tt)=>{1}}\nfn main(){ assert_eq!(m!(b),1); println!(\"macro ok\"); }",
            stdout: Some("macro ok\n"),
            group: 0,
        },
        StockCase {
            name: "dollar_crate_generated_ident",
            source: "macro_rules! take {($i:ident)=>{1};($($t:tt)*)=>{2}} macro_rules! outer {()=>{take!($crate)}}\nfn main(){println!(\"{}\",outer!());}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "dollar_crate_source_not_ident",
            source: "macro_rules! take {($i:ident)=>{1};($($t:tt)*)=>{2}}\nfn main(){println!(\"{}\",take!($crate));}",
            stdout: Some("2\n"),
            group: 1,
        },
        StockCase {
            name: "dollar_crate_not_plain_crate",
            source: "macro_rules! take {(crate)=>{1};($i:ident)=>{2}} macro_rules! outer {()=>{take!($crate)}}\nfn main(){println!(\"{}\",outer!());}",
            stdout: Some("2\n"),
            group: 1,
        },
        StockCase {
            name: "underscore_ident_fallback",
            source: "macro_rules! take {($i:ident)=>{1};($t:tt)=>{2}}\nfn main(){println!(\"{}\",take!(_));}",
            stdout: Some("2\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_shift_assign",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(>>=));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_spaced_shift_assign",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(> > =));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "double_tt_shift_assign",
            source: "macro_rules! take {($a:tt $b:tt)=>{1}}\nfn main(){println!(\"{}\",take!(>>=));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_shift_assign_spaced",
            source: "macro_rules! take {(>>=)=>{1}}\nfn main(){println!(\"{}\",take!(> > =));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_spaced_shift_assign_joint",
            source: "macro_rules! take {(> > =)=>{1}}\nfn main(){println!(\"{}\",take!(>>=));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_shift_assign_joint",
            source: "macro_rules! take {(>>=)=>{1}}\nfn main(){println!(\"{}\",take!(>>=));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "fixed_colon_colon_spaced",
            source: "macro_rules! take {(::)=>{1}}\nfn main(){println!(\"{}\",take!(: :));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_equal_equal_spaced",
            source: "macro_rules! take {(==)=>{1}}\nfn main(){println!(\"{}\",take!(= =));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_spaced_equal_equal_joint",
            source: "macro_rules! take {(= =)=>{1}}\nfn main(){println!(\"{}\",take!(==));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_unknown_plus_plus_spaced",
            source: "macro_rules! take {(++)=>{1}}\nfn main(){println!(\"{}\",take!(+ +));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_plus_plus",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(++));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "single_tt_range_inclusive",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(..=));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_lifetime",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!('a));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_float",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(0.1));}",
            stdout: Some("1\n"),
            group: 1,
        },
        StockCase {
            name: "single_tt_tuple_dot_float",
            source: "macro_rules! take {($t:tt)=>{1}}\nfn main(){println!(\"{}\",take!(.0.1));}",
            stdout: None,
            group: 1,
        },
        StockCase {
            name: "fixed_plain_raw",
            source: "macro_rules! m {(foo)=>{1};($t:tt)=>{2}} fn main(){println!(\"{}\",m!(r#foo));}",
            stdout: Some("2\n"),
            group: 2,
        },
        StockCase {
            name: "fixed_raw_plain",
            source: "macro_rules! m {(r#foo)=>{1};($t:tt)=>{2}} fn main(){println!(\"{}\",m!(foo));}",
            stdout: Some("2\n"),
            group: 2,
        },
        StockCase {
            name: "logical_left_arrow",
            source: "macro_rules! m {($t:tt)=>{1}} fn main(){println!(\"{}\",m!(<-));}",
            stdout: Some("1\n"),
            group: 2,
        },
        StockCase {
            name: "opaque_int",
            source: "macro_rules! sink {($y:literal)=>{1}} macro_rules! pass {($x:literal)=>{sink!(-$x)}} fn main(){println!(\"{}\",pass!(1));}",
            stdout: Some("1\n"),
            group: 2,
        },
        StockCase {
            name: "opaque_negative",
            source: "macro_rules! sink {($y:literal)=>{1}} macro_rules! pass {($x:literal)=>{sink!(-$x)}} fn main(){println!(\"{}\",pass!(-1));}",
            stdout: None,
            group: 2,
        },
        StockCase {
            name: "opaque_bool",
            source: "macro_rules! sink {($y:literal)=>{1}} macro_rules! pass {($x:literal)=>{sink!(-$x)}} fn main(){println!(\"{}\",pass!(true));}",
            stdout: Some("1\n"),
            group: 2,
        },
        StockCase {
            name: "opaque_string",
            source: "macro_rules! sink {($y:literal)=>{1}} macro_rules! pass {($x:literal)=>{sink!(-$x)}} fn main(){println!(\"{}\",pass!(\"x\"));}",
            stdout: Some("1\n"),
            group: 2,
        },
    ]
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "jr-macro-oracle-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn bounded(command: &mut Command, seconds: u64) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if start.elapsed() > Duration::from_secs(seconds) {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "oracle timeout: {:?}: {}",
                command,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn stock_macro_semantics_edition2024_measured_before_native_utility() {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = bounded(Command::new(&rustc).arg("-vV"), 10);
    assert!(version.status.success());
    eprintln!(
        "MACRO BASELINE: {BASELINE_STOCK}\nACTUAL STOCK TOOLCHAIN (separate from Reference pin):\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let scratch = Scratch::new();
    let mut counts = [(0usize, 0usize); 3];
    let cases = stock_cases();
    assert_eq!(cases.len(), 81);
    for case in cases {
        let source = scratch.0.join(format!("{}.rs", case.name));
        let image = scratch.0.join(case.name);
        fs::write(&source, case.source).unwrap();
        let result = bounded(
            Command::new(&rustc)
                .args([
                    "--edition=2024",
                    "--crate-name=macro_oracle",
                    "-Cpanic=abort",
                ])
                .arg(&source)
                .arg("-o")
                .arg(&image),
            20,
        );
        match case.stdout {
            Some(expected) => {
                counts[case.group].0 += 1;
                assert!(
                    result.status.success(),
                    "{}: {}",
                    case.name,
                    String::from_utf8_lossy(&result.stderr)
                );
                let bytes = fs::read(&image).unwrap();
                assert_eq!(
                    bytes.get(..6),
                    Some(&b"\x7fELF\x02\x01"[..]),
                    "{} ELF64 little endian",
                    case.name
                );
                let runtime = bounded(&mut Command::new(&image), 5);
                assert!(
                    runtime.status.success(),
                    "{} runtime: {:?} {}",
                    case.name,
                    runtime.status,
                    String::from_utf8_lossy(&runtime.stderr)
                );
                assert_eq!(runtime.stdout, expected.as_bytes(), "{} stdout", case.name);
                assert!(
                    runtime.stderr.is_empty(),
                    "{} stderr: {}",
                    case.name,
                    String::from_utf8_lossy(&runtime.stderr)
                );
            }
            None => {
                counts[case.group].1 += 1;
                assert!(
                    !result.status.success(),
                    "{} unexpectedly accepted",
                    case.name
                );
                assert!(
                    !result.stderr.is_empty(),
                    "{} missing rejection diagnostic",
                    case.name
                );
                assert!(!image.exists(), "{} failure published artifact", case.name);
            }
        }
    }
    assert_eq!(counts, [(31, 24), (10, 9), (6, 1)]);
}

// Every utility test uses lexer+grouping directly. No Rust compiler subprocess,
// source macro admission, name resolution, or preexpanded compiler fixture.
struct Fixture {
    session: TokenSession,
    parts: Vec<TokenSlice>,
    span: SpanId,
    krate: CrateId,
}
impl Fixture {
    fn new(parts: &[&str]) -> Self {
        Self::limited(parts, ExpansionLimits::default())
    }
    fn limited(parts: &[&str], limits: ExpansionLimits) -> Self {
        let text = parts
            .iter()
            .map(|p| format!("{{{p}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let budget = ExpansionBudget::new(limits);
        let input = PreparedLexInput::identity(
            Path::new("utility-only.rs"),
            &text,
            &budget,
            justrust_engine::compiler::owner_path::standalone_issued(
                &budget,
                justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let mut session = TokenSession::standalone(input.snapshot().clone(), budget)
            .unwrap()
            .0;
        let lexed = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
        let tokens = lexed
            .events()
            .iter()
            .map(|event| event.token)
            .collect::<Vec<_>>();
        let stream = group(&mut session, &tokens, GroupPolicy::OriginalSource).unwrap();
        let root = TokenSlice::roots(&session, stream).unwrap();
        let slices = (0..parts.len())
            .map(|n| root.children(&session, n).unwrap().unwrap())
            .collect();
        let span = session.original_span(input.eof().unwrap()).unwrap();
        let krate = session
            .register_crate_root(session.sources().file(0).unwrap())
            .unwrap();
        Self {
            session,
            parts: slices,
            span,
            krate,
        }
    }
    fn definition(&mut self, at: usize) -> Result<RulesDef, MacroError> {
        compile_rules(
            self.parts[at].clone(),
            DefinitionSite {
                span: self.span,
                edition: Edition::E2024,
                defining_crate: self.krate,
            },
            &mut self.session,
        )
    }
    fn expand(&mut self, definition: &RulesDef, at: usize) -> Result<ExpandedTokens, MacroError> {
        expand_rules(
            definition,
            self.parts[at].clone(),
            CallSite {
                span: self.span,
                parent: None,
            },
            &mut self.session,
        )
    }
    fn spell(&self, slice: &TokenSlice) -> Vec<String> {
        let mut result = vec![];
        for i in 0..slice.len(&self.session).unwrap() {
            match slice.node(&self.session, i).unwrap() {
                Node::Leaf(Token {
                    kind: TokenKind::Ident(ident),
                    ..
                }) => result.push(self.session.symbol(ident.symbol).unwrap().to_owned()),
                Node::Leaf(Token {
                    kind: TokenKind::Literal { spelling, .. },
                    ..
                }) => result.push(self.session.symbol(*spelling).unwrap().to_owned()),
                Node::Leaf(Token {
                    kind: TokenKind::Punct { ch, .. },
                    ..
                }) => result.push(ch.to_string()),
                Node::Leaf(Token {
                    kind: TokenKind::Opaque(_),
                    ..
                }) => result.push("<opaque>".into()),
                Node::Leaf(Token {
                    kind: TokenKind::DollarCrate(_),
                    ..
                }) => result.push("<dollar-crate>".into()),
                Node::Delimited { kind, .. } => {
                    result.push(format!("open:{kind:?}"));
                    result.extend(self.spell(&slice.children(&self.session, i).unwrap().unwrap()));
                    result.push(format!("close:{kind:?}"));
                }
            }
        }
        result
    }
}

#[test]
fn utility_ordered_arms_and_lazy_selected_transcriber_commit() {
    for (rules, args, expected) in [
        ("($x:tt)=>{first};(tag)=>{second}", "tag", "first"),
        ("(a)=>{first};(b)=>{second}", "b", "second"),
        ("(a)=>{$missing};($x:tt)=>{fallback}", "b", "fallback"),
        (
            "($x:literal)=>{literal};($($t:tt)*)=>{fallback}",
            "word",
            "fallback",
        ),
        (
            "($x:literal)=>{literal};($($t:tt)*)=>{fallback}",
            "+1",
            "fallback",
        ),
        (
            "($x:literal)=>{literal};($($t:tt)*)=>{fallback}",
            "1 x",
            "fallback",
        ),
        ("(foo)=>{fixed};($t:tt)=>{fallback}", "r#foo", "fallback"),
        ("(r#foo)=>{fixed};($t:tt)=>{fallback}", "foo", "fallback"),
    ] {
        let mut f = Fixture::new(&[rules, args]);
        let def = f.definition(0).unwrap();
        let out = f.expand(&def, 1).unwrap();
        assert_eq!(
            f.spell(&out.tokens(&f.session).unwrap()),
            [expected],
            "{rules} / {args}"
        );
    }
    let mut f = Fixture::new(&["(a)=>{$missing};($x:tt)=>{fallback}", "a"]);
    let def = f
        .definition(0)
        .expect("unselected unknown metavariable is not a definition error");
    assert!(
        matches!(f.expand(&def, 1), Err(MacroError::Syntax { .. })),
        "selected transcriber failure cannot fall through"
    );
}

#[test]
fn utility_logical_tt_operators_lifetimes_groups_and_tuple_boundaries() {
    for (args, accepted) in [
        (">>=", true),
        ("> > =", false),
        ("..=", true),
        ("<-", true),
        ("++", false),
        ("'a", true),
        ("0.1", true),
        (".0.1", false),
        ("-1", false),
        ("(x)", true),
    ] {
        let mut f = Fixture::new(&["($t:tt)=>{matched}", args]);
        let def = f.definition(0).unwrap();
        match f.expand(&def, 1) {
            Ok(out) if accepted => {
                assert_eq!(f.spell(&out.tokens(&f.session).unwrap()), ["matched"])
            }
            Err(MacroError::NoMatch) if !accepted => {}
            Err(error) => panic!("{args}: {error:?}"),
            Ok(_) => panic!("{args}: unexpectedly accepted"),
        }
    }
    for (rules, args, accepted) in [
        ("(>>=)=>{matched}", ">>=", true),
        ("(>>=)=>{matched}", "> > =", false),
        ("(> > =)=>{matched}", ">>=", false),
        ("(::)=>{matched}", ": :", false),
        ("(==)=>{matched}", "= =", false),
        ("(= =)=>{matched}", "==", false),
        ("(++)=>{matched}", "+ +", true),
        ("((x))=>{matched}", "[x]", false),
    ] {
        let mut f = Fixture::new(&[rules, args]);
        let def = f.definition(0).unwrap();
        let result = f.expand(&def, 1);
        if accepted {
            result.unwrap_or_else(|e| panic!("{rules} / {args}: {e:?}"));
        } else {
            assert!(
                matches!(result, Err(MacroError::NoMatch)),
                "{rules} / {args}"
            );
        }
    }
}

#[test]
fn utility_literal_fragment_signs_are_syntax_not_negation_semantics() {
    for args in ["1", "-1", "- 1", "-1.5", "true", "-true", "-\"x\""] {
        let mut f = Fixture::new(&["($x:literal)=>{matched}", args]);
        let def = f.definition(0).unwrap();
        let out = f
            .expand(&def, 1)
            .unwrap_or_else(|e| panic!("{args}: {e:?}"));
        assert_eq!(f.spell(&out.tokens(&f.session).unwrap()), ["matched"]);
    }
    for args in ["--1", "-word", "-(1)"] {
        let mut f = Fixture::new(&["($x:literal)=>{literal};($($t:tt)*)=>{fallback}", args]);
        let def = f.definition(0).unwrap();
        assert!(
            matches!(f.expand(&def, 1), Err(MacroError::Syntax { .. })),
            "minus commits literal parsing: {args}"
        );
    }
    for args in ["+1", "(-1)"] {
        let mut f = Fixture::new(&["($x:literal)=>{literal}", args]);
        let def = f.definition(0).unwrap();
        assert!(
            matches!(f.expand(&def, 1), Err(MacroError::NoMatch)),
            "{args}"
        );
    }
}

#[test]
fn utility_forwarded_literal_is_opaque_but_tt_and_ident_are_transparent() {
    for (kind, args, sink, expected) in [
        (
            "literal",
            "3",
            "(3)=>{fixed};($x:literal)=>{opaque}",
            "opaque",
        ),
        ("tt", "3", "(3)=>{fixed};($x:tt)=>{other}", "fixed"),
        ("ident", "fn", "(fn)=>{fixed};($x:ident)=>{other}", "fixed"),
    ] {
        let forward = format!("($x:{kind})=>{{$x}}");
        let mut f = Fixture::new(&[&forward, args, sink]);
        let pass = f.definition(0).unwrap();
        let take = f.definition(2).unwrap();
        let out = f.expand(&pass, 1).unwrap();
        let tokens = out.tokens(&f.session).unwrap();
        if kind == "literal" {
            assert_eq!(f.spell(&tokens), ["<opaque>"]);
        }
        let parent = out.expansion(&f.session).unwrap();
        let next = expand_rules(
            &take,
            tokens,
            CallSite {
                span: f.span,
                parent: Some(parent),
            },
            &mut f.session,
        )
        .unwrap();
        assert_eq!(f.spell(&next.tokens(&f.session).unwrap()), [expected]);
        assert_eq!(
            f.session
                .expansion(next.expansion(&f.session).unwrap())
                .unwrap()
                .parent,
            Some(parent)
        );
    }
    let mut f = Fixture::new(&["($x:literal)=>{$x}", "3", "(3)=>{fixed}"]);
    let pass = f.definition(0).unwrap();
    let take = f.definition(2).unwrap();
    let out = f.expand(&pass, 1).unwrap();
    assert!(matches!(
        expand_rules(
            &take,
            out.tokens(&f.session).unwrap(),
            CallSite {
                span: f.span,
                parent: None
            },
            &mut f.session
        ),
        Err(MacroError::NoMatch)
    ));
}

#[test]
fn utility_prefix_minus_before_opaque_literal_checks_original_sign() {
    for (args, accepted) in [("1", true), ("true", true), ("\"x\"", true), ("-1", false)] {
        let mut f = Fixture::new(&["($x:literal)=>{-$x}", args, "($y:literal)=>{matched}"]);
        let pass = f.definition(0).unwrap();
        let take = f.definition(2).unwrap();
        let out = f.expand(&pass, 1).unwrap();
        let result = expand_rules(
            &take,
            out.tokens(&f.session).unwrap(),
            CallSite {
                span: f.span,
                parent: None,
            },
            &mut f.session,
        );
        if accepted {
            let next = result.unwrap_or_else(|e| panic!("{args}: {e:?}"));
            assert_eq!(f.spell(&next.tokens(&f.session).unwrap()), ["matched"]);
        } else {
            assert!(
                matches!(result, Err(MacroError::Syntax { .. })),
                "negative opaque literal cannot be negated again"
            );
        }
    }
}

#[test]
fn utility_ident_keywords_underscore_and_dollarcrate_identity() {
    for args in ["fn", "self", "r#fn"] {
        let mut f = Fixture::new(&["($i:ident)=>{matched}", args]);
        let def = f.definition(0).unwrap();
        assert!(f.expand(&def, 1).is_ok(), "{args}");
    }
    let mut f = Fixture::new(&["($i:ident)=>{ident};($t:tt)=>{other}", "_"]);
    let def = f.definition(0).unwrap();
    let out = f.expand(&def, 1).unwrap();
    assert_eq!(f.spell(&out.tokens(&f.session).unwrap()), ["other"]);
    let mut f = Fixture::new(&["()=>{$crate}", "", "(crate)=>{plain};($i:ident)=>{special}"]);
    let pass = f.definition(0).unwrap();
    let take = f.definition(2).unwrap();
    let out = f.expand(&pass, 1).unwrap();
    assert_eq!(
        f.spell(&out.tokens(&f.session).unwrap()),
        ["<dollar-crate>"]
    );
    let next = expand_rules(
        &take,
        out.tokens(&f.session).unwrap(),
        CallSite {
            span: f.span,
            parent: None,
        },
        &mut f.session,
    )
    .unwrap();
    assert_eq!(f.spell(&next.tokens(&f.session).unwrap()), ["special"]);
}

#[test]
fn utility_repetitions_preserve_empty_nested_and_zipped_cardinality() {
    for (rules, args, expected) in [
        ("($($x:ident),*)=>{$($x)*}", "", vec![]),
        ("($($x:ident),*)=>{$($x)*}", "a,b,c", vec!["a", "b", "c"]),
        ("($($x:ident)?)=>{$($x)?}", "", vec![]),
        ("($($x:ident)?)=>{$($x)?}", "a", vec!["a"]),
        (
            "($( [$($x:ident),*] );*)=>{$($($x)*)*}",
            "[a,b];[];[c]",
            vec!["a", "b", "c"],
        ),
        (
            "($($a:ident),* ; $($b:ident),*)=>{$($a $b)*}",
            "a,b;c,d",
            vec!["a", "c", "b", "d"],
        ),
    ] {
        let mut f = Fixture::new(&[rules, args]);
        let def = f.definition(0).unwrap();
        let out = f
            .expand(&def, 1)
            .unwrap_or_else(|e| panic!("{rules} / {args}: {e:?}"));
        assert_eq!(
            f.spell(&out.tokens(&f.session).unwrap()),
            expected,
            "{rules} / {args}"
        );
    }
}

#[test]
fn utility_repetition_errors_and_ambiguity_are_not_fallbacks() {
    for (rules, args) in [
        (
            "($($a:ident),* ; $($b:ident),*)=>{$($a $b)*};($($t:tt)*)=>{fallback}",
            "a,b;c",
        ),
        ("($($x:ident),*)=>{$x}", "a,b"),
        ("()=>{$(one)*}", ""),
    ] {
        let mut f = Fixture::new(&[rules, args]);
        let def = f.definition(0).unwrap();
        assert!(
            matches!(f.expand(&def, 1), Err(MacroError::Syntax { .. })),
            "{rules} / {args}"
        );
    }
    for (rules, args) in [
        ("($($x:ident),+)=>{yes}", ""),
        ("($($x:ident)?)=>{yes}", "a b"),
    ] {
        let mut f = Fixture::new(&[rules, args]);
        let def = f.definition(0).unwrap();
        assert!(matches!(f.expand(&def, 1), Err(MacroError::NoMatch)));
    }
    let mut f = Fixture::new(&[
        "($($i:ident)* $j:ident)=>{first};($($t:tt)*)=>{fallback}",
        "x",
    ]);
    let def = f.definition(0).unwrap();
    assert!(matches!(
        f.expand(&def, 1),
        Err(MacroError::Ambiguous { .. })
    ));
}

#[test]
fn utility_definition_syntax_and_valid_unsupported_fragments_are_distinct() {
    for rules in ["($x:tt $x:tt)=>{yes}", "($($x:ident),?)=>{yes}"] {
        let mut f = Fixture::new(&[rules]);
        assert!(
            matches!(f.definition(0), Err(MacroError::Syntax { .. })),
            "{rules}"
        );
    }
    // Every Rust fragment specifier is admitted; unknown specifiers stay unsupported.
    for fragment in [
        "expr",
        "expr_2021",
        "lifetime",
        "ty",
        "path",
        "pat",
        "pat_param",
        "item",
        "block",
        "stmt",
        "meta",
        "vis",
    ] {
        let rules = format!("($x:{fragment})=>{{yes}}");
        let mut f = Fixture::new(&[&rules]);
        assert!(f.definition(0).is_ok(), "{fragment}");
    }
    let mut f = Fixture::new(&["($x:bogus)=>{yes}"]);
    assert!(matches!(
        f.definition(0),
        Err(MacroError::Unsupported { .. })
    ));
}

#[test]
fn utility_foreign_definition_arguments_call_and_output_fail_closed() {
    let mut a = Fixture::new(&["($x:tt)=>{$x}", "a"]);
    let mut b = Fixture::new(&["($x:tt)=>{$x}", "b"]);
    let def = a.definition(0).unwrap();
    assert!(matches!(
        b.expand(&def, 1),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    let own = b.definition(0).unwrap();
    assert!(matches!(
        expand_rules(
            &own,
            a.parts[1].clone(),
            CallSite {
                span: b.span,
                parent: None
            },
            &mut b.session
        ),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        expand_rules(
            &own,
            b.parts[1].clone(),
            CallSite {
                span: a.span,
                parent: None
            },
            &mut b.session
        ),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        compile_rules(
            b.parts[0].clone(),
            DefinitionSite {
                span: a.span,
                edition: Edition::E2024,
                defining_crate: b.krate
            },
            &mut b.session
        ),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        compile_rules(
            b.parts[0].clone(),
            DefinitionSite {
                span: b.span,
                edition: Edition::E2024,
                defining_crate: a.krate
            },
            &mut b.session
        ),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    let out = a.expand(&def, 1).unwrap();
    assert!(matches!(
        out.tokens(&b.session),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        out.expansion(&b.session),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(matches!(
        expand_rules(
            &own,
            b.parts[1].clone(),
            CallSite {
                span: b.span,
                parent: Some(out.expansion(&a.session).unwrap())
            },
            &mut b.session
        ),
        Err(MacroError::Contract(TokenError::ForeignHandle))
    ));
    assert!(a.session.budget().check().is_ok());
    assert!(b.session.budget().check().is_ok());
}

#[test]
fn utility_cursor_marks_ranges_and_logical_capture_keep_physical_occurrences() {
    let f = Fixture::new(&[">>=", ">>="]);
    let mut cursor = MacroCursor::new(&f.session, f.parts[0].clone()).unwrap();
    let mark = cursor.mark(&f.session).unwrap();
    assert!(matches!(
        cursor.bump(&f.session).unwrap(),
        Some(LogicalToken::Operator { len: 3, .. })
    ));
    assert!(cursor.bump(&f.session).unwrap().is_none());
    let capture = cursor.capture_since(&f.session, mark.clone()).unwrap();
    assert_eq!(capture.len(&f.session).unwrap(), 3);
    assert_eq!(capture.physical_weight(&f.session).unwrap(), 3);
    let other = MacroCursor::new(&f.session, f.parts[1].clone()).unwrap();
    let before = cursor.mark(&f.session).unwrap();
    assert!(matches!(
        cursor.restore(&f.session, other.mark(&f.session).unwrap()),
        Err(TokenError::ForeignHandle)
    ));
    assert_eq!(cursor.mark(&f.session).unwrap(), before);
    assert!(matches!(
        capture.subslice(&f.session, 0..4),
        Err(TokenError::InvalidRange)
    ));
    cursor.restore(&f.session, mark).unwrap();
    assert!(cursor.bump(&f.session).unwrap().is_some());
}

#[test]
fn utility_budget_poison_and_output_cap_remain_typed_and_shared() {
    let limits = ExpansionLimits {
        invocation_output: 0,
        ..ExpansionLimits::default()
    };
    let mut f = Fixture::limited(&["()=>{one}", ""], limits);
    let def = f.definition(0).unwrap();
    let shared = f.session.budget().clone();
    let Err(MacroError::Resource(failure)) = f.expand(&def, 1) else {
        panic!("output cap must fail as Resource");
    };
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(failure.resource, BudgetResource::InvocationOutput);
    assert_eq!(shared.snapshot().failure, Some(failure.clone()));
    assert!(matches!(f.expand(&def,1),Err(MacroError::Resource(next)) if next==failure));
    assert_eq!(shared.snapshot().temporary_bytes, 0);
}

#[test]
fn utility_zero_invocations_and_matcher_states_cannot_be_bypassed() {
    for states in [false, true] {
        let mut limits = ExpansionLimits::default();
        let resource = if states {
            limits.matcher_states = 0;
            BudgetResource::MatcherStates
        } else {
            limits.counters[Counter::Invocations as usize] = 0;
            BudgetResource::Counter(Counter::Invocations)
        };
        let mut f = Fixture::limited(&["($x:tt)=>{$x}", "one"], limits);
        let def = f.definition(0).unwrap();
        let Err(MacroError::Resource(failure)) = f.expand(&def, 1) else {
            panic!("zero admission must fail as Resource");
        };
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.resource, resource);
        assert_eq!(f.session.budget().snapshot().temporary_bytes, 0);
    }
}

#[test]
fn utility_success_and_nomatch_release_temporary_and_owned_rules_storage() {
    let mut f = Fixture::new(&["(a)=>{one}", "a", "b"]);
    let before = f.session.budget().snapshot();
    let def = f.definition(0).unwrap();
    let compiled = f.session.budget().snapshot();
    assert!(compiled.retained_bytes > before.retained_bytes);
    assert_eq!(compiled.temporary_bytes, before.temporary_bytes);
    assert!(matches!(f.expand(&def, 2), Err(MacroError::NoMatch)));
    assert_eq!(
        f.session.budget().snapshot().temporary_bytes,
        before.temporary_bytes
    );
    let out = f.expand(&def, 1).unwrap();
    let successful = f.session.budget().snapshot();
    assert_eq!(successful.temporary_bytes, before.temporary_bytes);
    assert!(
        successful.counters[Counter::Work as usize] > compiled.counters[Counter::Work as usize]
    );
    assert_eq!(
        successful.counters[Counter::Invocations as usize]
            - compiled.counters[Counter::Invocations as usize],
        2
    );
    let held = successful.retained_bytes;
    drop(def);
    assert!(f.session.budget().snapshot().retained_bytes < held);
    assert_eq!(
        f.spell(&out.tokens(&f.session).unwrap()),
        ["one"],
        "output outlives definition"
    );
}
