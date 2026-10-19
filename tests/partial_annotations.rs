//! Placeholder annotations and source never types, compared with pinned stock rustc.
//! Never-position admission follows the 1.101.0-nightly oracle (upstream
//! 1d81eb4ad, 2026-10-07), not the older host 1.98.1 compiler. The shared harness
//! logs and pins the actual executable and full version hash for each run.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn partial_annotations_match_stock() {
    oracle::check(
        "partial-annotations",
        &[
            Case {
                name: "never_return_rejects_inhabited_value",
                source: "fn bad() -> ! { 7 } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "never_generic_argument",
                source: "fn f<T>() {} fn main() { f::<!>(); println!(\"never generic\"); }",
                expect: Expect::Run("never generic\n"),
            },
            Case {
                name: "never_aggregate_field",
                source: "struct P { field: ! } fn main() { println!(\"never field\"); }",
                expect: Expect::Run("never field\n"),
            },
            Case {
                name: "never_local_annotation",
                source: "fn diverge() { let x: ! = loop {}; } fn main() { println!(\"never local\"); }",
                expect: Expect::Run("never local\n"),
            },
            Case {
                name: "never_fn_trait_output",
                source: "fn f(x: impl Fn() -> !) {} fn main() { println!(\"never bound\"); }",
                expect: Expect::Run("never bound\n"),
            },
            Case {
                name: "nominal_tuple_array_reference",
                source: r#"
struct Pair<A, B> { a: A, b: B }
fn make() -> Pair<i32, u8> { Pair { a: 40, b: 2 } }
fn main() {
    let p: Pair<_, u8> = make();
    let t: (_, i64) = (3u16, 4i64);
    let a: [_; 2] = [5u8, 6];
    let b: &[_; 2] = &a;
    println!("{} {} {} {} {}", p.a, p.b, t.0, t.1, (*b)[1]);
}
"#,
                expect: Expect::Run("40 2 3 4 6\n"),
            },
            Case {
                name: "associated_owner_concrete_constraints",
                source: r#"struct P<T, U> { a: T, b: U }
impl<T,U> P<T,U> { fn make(a:T,b:U)->Self { Self { a,b } } }
fn main() { let p = P::<u8,_>::make(7,9u16); println!("{} {}",p.a,p.b); }"#,
                expect: Expect::Run("7 9\n"),
            },
            Case {
                name: "associated_owner_rejects_changed_concrete_type",
                source: r#"struct P<T,U> { a:T,b:U }
impl<T,U> P<T,U> { fn make(a:T,b:U)->Self { Self {a,b} } }
fn main() { let p = P::<u8,_>::make(7i32,9u16); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "mismatched_partial_leaf",
                source: r#"struct P<T, U> { a: T, b: U }
fn make() -> P<i32, u8> { P { a: 1, b: 2 } }
fn main() { let p: P<_, bool> = make(); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "nominal_identity_not_spelling",
                source: r#"mod a { pub struct P<T> { pub value: T } }
mod b { pub struct P<T> { pub value: T } }
fn main() { let p: a::P<_> = b::P { value: 1i32 }; }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "explicit_generic_placeholders",
                source: r#"fn id<T>(x: T) -> T { x }
fn pair<T, U>(x: T, y: U) -> (T, U) { (x, y) }
struct P<T> { value: T }
fn main() {
    let a = id::<_>(7u8);
    let b = pair::<i64, _>(8, 9u16);
    let c = id::<P<_>>(P { value: 10i32 });
    println!("{} {} {} {}", a, b.0, b.1, c.value);
}"#,
                expect: Expect::Run("7 8 9 10\n"),
            },
            Case {
                name: "method_generic_placeholders",
                source: r#"struct P { value: i32 }
impl P {
    fn get<T>(&self, x: T) -> T { x }
    fn pair<T, U>(&self, x: T, y: U) -> (T, U) { (x, y) }
    fn make<T>(x: T) -> T { x }
}
fn main() {
    let p = P { value: 1 };
    let a = p.get::<_>(7u8);
    let b = p.pair::<i64, _>(8, 9u16);
    let c = P::make::<_>(10i32);
    println!("{} {} {} {}", a, b.0, b.1, c);
}"#,
                expect: Expect::Run("7 8 9 10\n"),
            },
            Case {
                name: "argument_impl_trait_named_prefix",
                source: r#"trait Value { fn value(&self) -> i32; }
struct N { n: i32 }
impl Value for N { fn value(&self) -> i32 { self.n } }
fn read<T>(a: T, b: impl Value) -> i32 { b.value() }
fn main() {
    println!("{} {}", read::<u8>(1, N { n: 7 }), read::<_>(2u16, N { n: 8 }));
}"#,
                expect: Expect::Run("7 8\n"),
            },
            Case {
                name: "argument_impl_trait_is_not_nameable",
                source: r#"trait Value { fn value(&self) -> i32; }
struct N { n: i32 }
impl Value for N { fn value(&self) -> i32 { self.n } }
fn read<T>(a: T, b: impl Value) -> i32 { b.value() }
fn main() { let _ = read::<u8, N>(1, N { n: 7 }); }"#,
                expect: Expect::Invalid("E0107"),
            },
            Case {
                name: "signature_hole_rejected",
                source: "fn bad(x: _) {} fn main() {}",
                expect: Expect::Invalid("E0121"),
            },
            Case {
                name: "never_return_annotation",
                source: r#"fn diverge() -> ! { loop {} }
fn main() { if false { diverge(); } println!("done"); }"#,
                expect: Expect::Run("done\n"),
            },
        ],
    );
}
