//! Canonical qualifier order must not misclassify a const unsafe fn as a const item.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn const_unsafe_function_qualifiers_match_stock() {
    oracle::check(
        "function-qualifiers",
        &[
            Case {
                name: "free_and_associated",
                source: r#"
pub const unsafe fn width(byte: u8) -> usize { if byte < 128 { 1 } else { 2 } }
struct S { value: u8 }
impl S { pub const unsafe fn read(&self) -> u8 { self.value } }
const MARK: u8 = 7;
fn main() { let s = S { value: MARK }; println!("{} {}", unsafe { width(65) }, unsafe { s.read() }); }
"#,
                expect: Expect::Run("1 7\n"),
            },
            Case {
                name: "unsafe_call_required",
                source: "pub const unsafe fn f()->u8{1}fn main(){let _=f();}",
                expect: Expect::Invalid("E0133"),
            },
            Case {
                name: "unused_body_checked",
                source: "pub const unsafe fn f()->u8{true}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "reversed_qualifiers",
                source: "unsafe const fn f(){}fn main(){}",
                expect: Expect::Invalid("expected"),
            },
            Case {
                name: "duplicate_const",
                source: "const const fn f(){}fn main(){}",
                expect: Expect::Invalid("expected"),
            },
            Case {
                name: "duplicate_unsafe",
                source: "const unsafe unsafe fn f(){}fn main(){}",
                expect: Expect::Invalid("expected"),
            },
            Case {
                name: "const_async",
                source: "const async fn f(){}fn main(){}",
                expect: Expect::Invalid("both"),
            },
        ],
    );
}

#[test]
fn unsupported_qualifiers_keep_explicit_semantic_boundaries() {
    use justrust_engine::compiler::frontend;
    for source in [
        "async fn f(){}fn main(){}",
        "async unsafe fn f(){}fn main(){}",
        "extern \"system\" fn f(){}fn main(){}",
        "unsafe extern \"C-unwind\" fn f(){}fn main(){}",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(error.contains("unsupported"), "{source}: {error}");
    }
    let program = frontend::parse("pub const unsafe fn f()->u8{1}fn main(){}").unwrap();
    assert_eq!(program.functions[0].constness, frontend::Constness::Const);
    assert!(program.functions[0].unsafety);
}
