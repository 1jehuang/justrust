//! Retained source parameter patterns use ordinary checked entry destructuring.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nested_shared_reference_parameter_copies_the_referent() {
    oracle::check(
        "closure_parameter_reference_pattern",
        &[Case {
            name: "double_reference_scalar",
            source: "fn main(){let n=7i32;let r=&n;let f=|&&x|x+1;assert_eq!(f(&r),8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn parameter_patterns_preserve_irrefutable_bindings() {
    oracle::check(
        "closure_parameter_bindings",
        &[
            Case {
                name: "tuple_shadow_and_wildcard",
                source: "fn main(){let x=90i32;let mut order=0i32;let f=|(x,_) :(i32,i32)|x+1;assert_eq!(f(({order=order*10+1;7},{order=order*10+2;8})),8);assert_eq!(order,12);assert_eq!(x,90);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_pattern_binding",
                source: "fn main(){let f=|mut x:i32|{x+=1;x};assert_eq!(f(7),8);}",
                expect: Expect::Run(""),
            },
        ],
    );
}

// Keep each invalid source independently named so an interrupted hosted run
// retains completed semantic verdicts rather than one buffered seven-case test.
fn invalid(name: &'static str, source: &'static str, fragment: &'static str) {
    oracle::check(
        "closure_parameter_invalid",
        &[Case {
            name,
            source,
            expect: Expect::Invalid(fragment),
        }],
    );
}

#[test]
fn unused_invalid_noncopy_shared_extraction() {
    invalid(
        "noncopy_shared_extraction",
        "fn unused(){let f=|&x:&String|x;}fn main(){}",
        "E0507",
    );
}
#[test]
fn unused_invalid_immutable_destructured_binding() {
    invalid(
        "immutable_destructured_binding",
        "fn unused(){let f=|&x:&i32|{x=2;};}fn main(){}",
        "E0384",
    );
}
#[test]
fn unused_invalid_wrong_reference_depth() {
    invalid(
        "wrong_reference_depth",
        "fn unused(){let f=|&&x:&i32|x;}fn main(){}",
        "E0308",
    );
}
#[test]
fn unused_invalid_mutable_pattern_on_shared_reference() {
    invalid(
        "mutable_pattern_on_shared_reference",
        "fn unused(){let f=|&mut x:&i32|x;}fn main(){}",
        "E0308",
    );
}
#[test]
fn unused_invalid_duplicate_parameter_binding() {
    invalid(
        "duplicate_parameter_binding",
        "fn unused(){let f=|(x,x):(i32,i32)|x;}fn main(){}",
        "E0415",
    );
}
#[test]
fn unused_invalid_refutable_parameter() {
    invalid(
        "refutable_parameter",
        "fn unused(){let f=|0:i32|1;}fn main(){}",
        "E0005",
    );
}
#[test]
fn unused_invalid_wrong_call_arity() {
    invalid(
        "wrong_call_arity",
        "fn unused(){let f=|&&x:&&i32|x;f();}fn main(){}",
        "E0057",
    );
}
