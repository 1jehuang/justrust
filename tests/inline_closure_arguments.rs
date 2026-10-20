//! Inline closure literals passed as the single argument of a generic call.
//! The closure lowers to a generated item path, which is not a local binding,
//! so the source inventory must not treat it as a local lookup.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn inline_closure_arguments_to_generic_functions() {
    oracle::check(
        "inline_closure_arguments",
        &[
            Case {
                name: "annotated",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){println!(\"{}\",ap(|x:u32|x*2));}",
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "inferred",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){println!(\"{}\",ap(|x|x*2));}",
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "capturing",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn main(){let k=3u32;println!(\"{}\",ap(|x|x*k));}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "wrong_result",
                source: "fn ap<F:Fn(u32)->u32>(f:F)->u32{f(2)}fn unused(){ap(|x|true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
