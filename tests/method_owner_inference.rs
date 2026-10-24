//! Partial method inference substitutes enclosing impl parameters by identity.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn nominal_callable_outputs_preserve_enclosing_owner_bindings() {
    oracle::check(
        "nominal-callable-owner",
        &[
            Case {
                name: "inherent_owner_inside_option",
                source: "struct Holder<T>{value:T}impl<T> Holder<T>{fn replacement<F:FnOnce()->Option<T>>(self,f:F)->Option<T>{f()}}fn main(){let n=Holder{value:1i32}.replacement(||Some(7));println!(\"{}\",n.unwrap());}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "trait_owner_inside_option",
                source: "struct Seed;trait Choose<T>:Sized{fn choose<F:FnOnce()->Option<T>>(self,f:F)->Option<T>{f()}}impl Choose<i32> for Seed{}fn main(){let n=Seed.choose(||Some(8));println!(\"{}\",n.unwrap());}",
                expect: Expect::Run("8\n"),
            },
            Case {
                name: "wrong_owner_stays_rigid",
                source: "struct Holder<T>{value:T}impl<T> Holder<T>{fn replacement<F:FnOnce()->Option<T>>(self,f:F)->Option<T>{f()}}fn unused(){let _=Holder{value:1i32}.replacement(||Some(true));}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn method_holes_preserve_generic_owner_constraints() {
    oracle::check(
        "method-owner-inference",
        &[
            Case {
                name: "partial_and_omitted_method_actuals",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> {
    fn pair<U>(self, extra: U) -> (T, U) { (self.value, extra) }
    fn combine<U>(&self, pair: (T,U)) -> U { pair.1 }
}
fn main() {
    let a: (u64, u8) = Packet { value: 42u64 }.pair::<_>(7);
    let b: (i64, u16) = Packet { value: 9i64 }.pair(8);
    let c = Packet { value: 10u32 }.combine::<_>((11u32,12u8));
    println!("{} {} {} {} {}", a.0, a.1, b.0, b.1, c);
}
"#,
                expect: Expect::Run("42 7 9 8 12\n"),
            },
            Case {
                name: "disjoint_inherent_candidates",
                source: r#"
struct Packet<T> { value: T }
impl Packet<u8> { fn choose<U>(self, x: U) -> U { x } }
impl Packet<u64> { fn choose<U>(self, x: U) -> U { x } }
fn main() {
    let x = Packet { value: 2u8 }.choose::<_>(7u16);
    let y = Packet { value: 3u64 }.choose::<_>(9i32);
    println!("{} {}", x,y);
}
"#,
                expect: Expect::Run("7 9\n"),
            },
            Case {
                name: "owner_concrete_argument_remains_rigid",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> { fn combine<U>(&self, x: (T,U)) -> U { x.1 } }
fn main() { let x = Packet { value: 1u32 }.combine::<_>((2u8,3u16)); }
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
