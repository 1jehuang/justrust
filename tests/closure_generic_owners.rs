//! Canonical generic owner domains and unused-body rejection, compared with
//! stock Rust through explicit CHECK/BUILD/run at workers 1 and 8.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn opaque_callable_with_realized_nominal_local() {
    oracle::check(
        "opaque_realized_nominal",
        &[Case {
            name: "nominal_and_opaque_locals",
            source: "struct Holder<T>{value:T}fn wrap<T>(value:T)->Holder<T>{Holder{value}}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let h=wrap(7i32);let f=add(5);println!(\"{} {}\",h.value,f(10));}",
            expect: Expect::Run("7 15\n"),
        }],
    );
}

#[test]
fn opaque_callable_preserves_existing_cleanup() {
    oracle::check(
        "opaque_existing_cleanup",
        &[
            Case {
                name: "root_and_generic_owner_drop_order",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!(\"drop {}\",self.n);}}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn apply<F:Fn(i32)->i32>(f:F)->i32{let guard=Guard{n:1};f(10)}fn main(){let guard=Guard{n:2};let f=add(5);println!(\"{}\",apply(f));}",
                expect: Expect::Run("drop 1\n15\ndrop 2\n"),
            },
            Case {
                name: "vec_with_opaque_callable",
                source: "fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let values=vec![1i32,2i32];let f=add(5);println!(\"{} {}\",values.len(),f(10));}",
                expect: Expect::Run("2 15\n"),
            },
        ],
    );
}

fn callable_reference_forwarding(name: &str) {
    let cases = [
        Case {
            name: "shared_opaque_callable",
            source: "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn add(n:i32)->impl Fn(i32)->i32{move|x|x+n}fn main(){let a=add(5);println!(\"{}\",apply(&a,10));}",
            expect: Expect::Run("15\n"),
        },
        Case {
            name: "nested_shared_callable",
            source: "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn main(){let a=|x:i32|x+5;let first=&a;let second=&first;println!(\"{}\",apply(second,10));}",
            expect: Expect::Run("15\n"),
        },
        Case {
            name: "shared_callable_mut_and_once",
            source: "fn mutable<F:FnMut(i32)->i32>(mut f:F)->i32{f(10)}fn once<F:FnOnce(i32)->i32>(f:F)->i32{f(20)}fn main(){let a=|x:i32|x+5;let r=&a;println!(\"{} {}\",mutable(r),once(r));}",
            expect: Expect::Run("15 25\n"),
        },
        Case {
            name: "mutable_callable_reference",
            source: "fn mutable<F:FnMut(i32)->i32>(mut f:F)->i32{f(10)}fn once<F:FnOnce(i32)->i32>(f:F)->i32{f(20)}fn main(){let mut n=5;let mut a=|x:i32|{n+=x;n};let first=&mut a;println!(\"{}\",mutable(first));let second=&mut a;println!(\"{}\",once(second));}",
            expect: Expect::Run("15\n35\n"),
        },
        Case {
            name: "mutable_reference_does_not_implement_fn",
            source: "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn main(){let mut a=|x:i32|x+5;println!(\"{}\",apply(&mut a,10));}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "shared_reference_cannot_mutate_callable",
            source: "fn apply<F:FnMut()>(mut f:F){f()}fn main(){let mut n=0;let a=||{n+=1;};apply(&a);}",
            expect: Expect::Invalid("E0525"),
        },
        Case {
            name: "shared_reference_cannot_consume_callable",
            source: "fn apply<F:FnOnce()>(f:F){f()}fn main(){let s=String::from(\"owned\");let a=move||{let moved=s;};apply(&a);}",
            expect: Expect::Invalid("E0525"),
        },
    ];
    let case = cases.into_iter().find(|case| case.name == name).unwrap();
    oracle::check("closure_reference_forwarding", &[case]);
}

#[test]
fn shared_opaque_callable() {
    callable_reference_forwarding("shared_opaque_callable");
}
#[test]
fn nested_shared_callable() {
    callable_reference_forwarding("nested_shared_callable");
}
#[test]
fn shared_callable_mut_and_once() {
    callable_reference_forwarding("shared_callable_mut_and_once");
}
#[test]
fn mutable_callable_reference() {
    callable_reference_forwarding("mutable_callable_reference");
}
#[test]
fn mutable_reference_does_not_implement_fn() {
    callable_reference_forwarding("mutable_reference_does_not_implement_fn");
}
#[test]
fn shared_reference_cannot_mutate_callable() {
    callable_reference_forwarding("shared_reference_cannot_mutate_callable");
}
#[test]
fn shared_reference_cannot_consume_callable() {
    callable_reference_forwarding("shared_reference_cannot_consume_callable");
}

#[test]
fn current_parent_generic_owner_consumer() {
    oracle::check(
        "closure_generic_owner_current_parent",
        &[Case {
            name: "ordered_move_owner_instantiations",
            source: "fn outer<A,B>(a:A,b:B)->i32{let f=move||{let first=a;let second=b;9};f()}fn main(){assert_eq!(outer(String::from(\"owned\"),8u8),9);assert_eq!(outer(7u8,String::from(\"second\")),9);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn generic_parent_closures_preserve_owner_domains() {
    oracle::check(
        "closure_generic_owners",
        &[
            Case {
                name: "noncapturing_generic_parent",
                source: "fn outer<T>(value:T)->i32{let f=|x:i32|x+1;f(2)}fn main(){assert_eq!(outer(7u8),3);assert_eq!(outer(8i64),3);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "generic_move_capture",
                source: "fn outer<T>(value:T)->i32{let f=move||{let taken=value;7};f()}fn main(){assert_eq!(outer(String::from(\"owned\")),7);assert_eq!(outer(8u8),7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "ordered_distinct_owner_arguments",
                source: "fn outer<A,B>(a:A,b:B)->i32{let f=move||{let first=a;let second=b;9};f()}fn main(){assert_eq!(outer(7u8,8i64),9);assert_eq!(outer(8i64,7u8),9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_generic_move_reused",
                source: "fn outer<T>(value:T)->i32{let f=move||{let taken=value;7};let first=f();f()}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "unused_generic_bound_not_assumed",
                source: "fn requires_copy<T:Copy>(value:T){}fn outer<T>(value:T)->i32{let f=move||{requires_copy(value);7};f()}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
