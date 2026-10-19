//! Stock-matching owned and consumed closure environment acceptance.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn alternate_callable_receiver_modes() {
    oracle::check(
        "closure_receiver_modes",
        &[
            Case {
                name: "fn_as_fnonce",
                source: "fn take<F:FnOnce(i32)->i32>(f:F)->i32{f(2)}fn main(){let n=7i32;let f=move |x:i32|n+x;assert_eq!(take(f),9);assert_eq!(f(3),10);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "fn_as_fnmut",
                source: "fn twice<F:FnMut(i32)->i32>(mut f:F)->i32{f(2)+f(3)}fn main(){let n=7i32;let f=move |x:i32|n+x;assert_eq!(twice(f),19);}",
                expect: Expect::Run(""),
            },
        ],
    );
}

#[test]
fn unused_capture_bodies_are_still_checked() {
    oracle::check(
        "unused_capture_bodies",
        &[
            Case {
                name: "unused_capture_type_error",
                source: "fn unused(){let n=7i32;let f=move ||->bool{n};}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_capture_move_reuse",
                source: "struct Item{i:i32}fn unused(){let x=Item{i:7};let f=move ||{let a=x;let b=x;b};}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}

#[test]
fn copy_move_captures_match_stock() {
    oracle::check(
        "closure_captures",
        &[
            Case {
                name: "scalar_copy",
                source: "fn main(){let n=7i32;let f=move |x:i32|n+x;assert_eq!(f(2),9);assert_eq!(f(3),10);assert_eq!(n,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "generic_fn",
                source: "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn main(){let n=7i32;let f=move |x:i32|n+x;assert_eq!(apply(f,2),9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "consume_once",
                source: "struct Item{i:i32}fn take<F:FnOnce()->Item>(f:F)->Item{f()}fn main(){let x=Item{i:7};let f=move ||x;assert_eq!(take(f).i,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "consume_without_move_keyword",
                source: "struct Item{i:i32}fn main(){let x=Item{i:7};let f=||x;assert_eq!(f().i,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "moved_capture_reused",
                source: "struct Item{i:i32}fn main(){let x=Item{i:7};let f=move ||x;let y=x;}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "closure_consumed_twice",
                source: "struct Item{i:i32}fn main(){let x=Item{i:7};let f=move ||x;let a=f();let b=f();}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "consuming_closure_is_not_fn",
                source: "struct Item{i:i32}fn apply<F:Fn()->Item>(f:F)->Item{f()}fn main(){let x=Item{i:7};let f=move ||x;let y=apply(f);}",
                expect: Expect::Invalid("E0525"),
            },
        ],
    );
}
