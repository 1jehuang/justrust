//! Stock-matching borrowed capture environment controls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};
#[test]
fn borrowed_capture_environments() {
    oracle::check(
        "closure_borrows",
        &[
            Case {
                name: "shared_read",
                source: "fn main(){let n=7i32;let f=|x:i32|n+x;assert_eq!(f(2),9);assert_eq!(f(3),10);assert_eq!(n,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_write",
                source: "fn main(){let mut n=7i32;let mut f=||{n+=1;n};assert_eq!(f(),8);assert_eq!(f(),9);assert_eq!(n,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "generic_fnmut",
                source: "fn twice<F:FnMut()->i32>(mut f:F)->i32{f()+f()}fn main(){let mut n=7i32;let f=||{n+=1;n};assert_eq!(twice(f),17);assert_eq!(n,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "move_owned_mutation",
                source: "fn main(){let n=7i32;let mut f=move ||{let x=n+1;x};assert_eq!(f(),8);assert_eq!(n,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "fnmut_not_fn",
                source: "fn apply<F:Fn()->i32>(f:F)->i32{f()}fn main(){let mut n=7i32;let f=||{n+=1;n};let x=apply(f);}",
                expect: Expect::Invalid("E0525"),
            },
            Case {
                name: "immutable_callable",
                source: "fn main(){let mut n=7i32;let f=||{n+=1;};f();}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "live_shared_capture",
                source: "fn main(){let mut n=7i32;let f=||n;n=9;let x=f();}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "live_mut_capture",
                source: "fn main(){let mut n=7i32;let mut f=||{n+=1;n};let x=n;let y=f();}",
                expect: Expect::Invalid("E0503"),
            },
            Case {
                name: "unused_borrow_conflict",
                source: "fn unused(){let mut n=7i32;let f=||n;n=9;let x=f();}fn main(){}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "unused_mutable_call",
                source: "fn unused(){let mut n=7i32;let f=||{n+=1;};f();}fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "escaping_borrow",
                source: "fn main(){let f={let n=7i32;||n};let x=f();}",
                expect: Expect::Invalid("E0373"),
            },
        ],
    );
}
