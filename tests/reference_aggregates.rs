//! References as generic arguments and inside aggregates (G3), stock-checked.
//! Lifetime parameters on structs/enums/impls, reference fields, Option<&T>
//! returns, tuples/arrays of references, plus dangling/aliasing rejections.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn reference_aggregates_match_stock() {
    oracle::check(
        "reference-aggregates",
        &[
            Case {
                name: "option_of_ref",
                source: r#"fn main() {
    let v = 5;
    let o: Option<&i32> = Some(&v);
    match o { Some(r) => println!("{}", *r), None => println!("none") }
}"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "generic_struct_of_ref",
                source: r#"struct S<T> { x: T }
fn main() { let v = 5; let s = S { x: &v }; println!("{}", *s.x); }"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "lifetime_struct_field",
                source: r#"struct Holder<'a> { r: &'a i32 }
fn get<'a>(h: &Holder<'a>) -> &'a i32 { h.r }
fn main() { let v = 7; let h = Holder { r: &v }; let r = get(&h); println!("{}", *r); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "struct_holding_str_with_impl",
                source: r#"struct Tok<'a> { text: &'a str, n: usize }
impl<'a> Tok<'a> { fn text(&self) -> &'a str { self.text } }
fn mk(s: &str) -> Tok { Tok { text: s, n: s.len() } }
fn main() { let t = mk("hello"); println!("{} {}", t.text(), t.n); }"#,
                expect: Expect::Run("hello 5\n"),
            },
            Case {
                name: "enum_with_ref_payload",
                source: r#"enum E<'a> { A(&'a i32), B }
fn main() {
    let v = 5;
    let e = E::A(&v);
    match e { E::A(r) => println!("{}", *r), E::B => println!("b") }
}"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "returns_option_ref",
                source: r#"fn wrap<T>(a: &T) -> Option<&T> { Some(a) }
fn pick<'a>(a: &'a i32, b: &i32) -> Option<&'a i32> { let _ = b; Some(a) }
fn main() {
    let v = 3;
    let w = 4;
    match wrap(&v) { Some(x) => println!("{}", *x), None => {} }
    match pick(&v, &w) { Some(x) => println!("{}", *x), None => {} }
}"#,
                expect: Expect::Run("3\n3\n"),
            },
            Case {
                name: "tuple_of_refs_return",
                source: r#"fn both<'a>(a: &'a i32, b: &'a i32) -> (&'a i32, &'a i32) { (b, a) }
fn main() { let x = 1; let y = 2; let p = both(&x, &y); println!("{} {}", *p.0, *p.1); }"#,
                expect: Expect::Run("2 1\n"),
            },
            Case {
                name: "mut_ref_in_option",
                source: r#"fn main() {
    let mut v = 5;
    let o: Option<&mut i32> = Some(&mut v);
    match o { Some(r) => { *r = 9; } None => {} }
    println!("{}", v);
}"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "constructor_returns_lifetime_struct",
                source: r#"struct S<'a> { x: &'a i32 }
fn mk<'a>(r: &'a i32) -> S<'a> { S { x: r } }
fn main() { let v = 5; let s = mk(&v); println!("{}", *s.x); }"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "dangling_struct_field",
                source: r#"struct S<'a> { x: &'a i32 }
fn main() { let s: S; { let v = 5; s = S { x: &v }; } println!("{}", *s.x); }"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "dangling_option",
                source: r#"fn main() {
    let o: Option<&i32>;
    { let v = 5; o = Some(&v); }
    match o { Some(r) => println!("{}", *r), None => {} }
}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "returns_local_in_option",
                source: r#"fn bad<'a>(a: &'a i32) -> Option<&'a i32> { let v = 1; let _ = a; Some(&v) }
fn main() { let x = 0; let _ = bad(&x); }"#,
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "assign_while_borrowed_in_aggregate",
                source: r#"struct S<'a> { x: &'a i32 }
fn main() { let mut v = 5; let s = S { x: &v }; v = 6; println!("{} {}", *s.x, v); }"#,
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "caller_dangles_through_call",
                source: r#"fn wrap(a: &i32) -> Option<&i32> { Some(a) }
fn main() {
    let o: Option<&i32>;
    { let v = 3; o = wrap(&v); }
    match o { Some(x) => println!("{}", *x), None => {} }
}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "elided_field_lifetime",
                source: r#"struct S { x: &i32 }
fn main() {}"#,
                expect: Expect::Invalid("E0106"),
            },
        ],
    );
}
