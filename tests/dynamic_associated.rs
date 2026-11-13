#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/closure_oracle.rs"]
mod output_oracle;
use oracle::{Case, Expect};

#[test]
fn nominal_dynamic_string_output_recipes() {
    use output_oracle::{Case, Expect};
    output_oracle::check(
        "nominal-dynamic-string-output",
        &[Case {
            name: "nominal_string_output",
            source: r#"
trait Get { type Out; fn get(&self) -> Self::Out; }
struct Source;
impl Get for Source { type Out = String; fn get(&self) -> String { String::from("recipe") } }
struct Holder { inner: Box<dyn Get<Out = String>> }
fn main() { let h = Holder { inner: Box::new(Source) }; println!("{}", h.inner.get()); }
"#,
            expect: Expect::Run("recipe\n"),
        }],
    );
}

#[test]
fn nominal_dynamic_non_string_output_recipes() {
    use output_oracle::{Case, Expect};
    output_oracle::check(
        "nominal-dynamic-non-string-output",
        &[Case {
            name: "non_string_nominal_output",
            source: r#"
struct Payload { value: i32 }
trait Get { type Out; fn get(&self) -> Self::Out; }
struct Source;
impl Get for Source { type Out = Payload; fn get(&self) -> Payload { Payload { value: 42 } } }
struct Holder { inner: Box<dyn Get<Out = Payload>> }
fn main() { let h = Holder { inner: Box::new(Source) }; println!("{}", h.inner.get().value); }
"#,
            expect: Expect::Run("42\n"),
        }],
    );
}

#[test]
fn nominal_dynamic_unused_invalid_recipes() {
    use output_oracle::{Case, Expect};
    output_oracle::check(
        "nominal-dynamic-unused-invalid",
        &[
            Case {
                name: "unused_unknown_associated_output",
                source: "trait Get { type Out; fn get(&self) -> Self::Out; } struct Bad { inner: Box<dyn Get<Wrong = String>> } fn main() {}",
                expect: Expect::Invalid("E0220"),
            },
            Case {
                name: "unused_missing_associated_output",
                source: "trait Get { type Out; fn get(&self) -> Self::Out; } struct Bad { inner: Box<dyn Get> } fn main() {}",
                expect: Expect::Invalid("E0191"),
            },
        ],
    );
}

#[test]
fn dynamic_associated_types_and_generic_principals() {
    oracle::check(
        "dyn-associated",
        &[
            Case {
                name: "associated_output",
                source: r#"
trait Get {type Out;fn get(&self)->Self::Out;}
struct N {v:i32}
impl Get for N {type Out=i32;fn get(&self)->i32{self.v}}
fn read(x:&dyn Get<Out=i32>)->i32{x.get()}
fn main(){let n=N{v:37};println!("{}",read(&n));}
"#,
                expect: Expect::Run("37\n"),
            },
            Case {
                name: "generic_caller_and_principal",
                source: r#"
trait Get<A>{type Out;fn get(&self,a:A)->Self::Out;}
struct N{v:i32}
impl Get<i32> for N{type Out=i32;fn get(&self,a:i32)->i32{self.v+a}}
fn read<T>(x:&dyn Get<T,Out=i32>,a:T)->i32{x.get(a)}
fn main(){let n=N{v:37};println!("{}",read::<i32>(&n,5));}
"#,
                expect: Expect::Run("42\n"),
            },
            Case {
                name: "ordered_bindings_and_distinct_output",
                source: r#"
trait Get{type A;type B;fn a(&self)->Self::A;fn b(&self)->Self::B;}
struct N;
impl Get for N{type A=i32;type B=bool;fn a(&self)->i32{9}fn b(&self)->bool{true}}
fn read(x:&dyn Get<B=bool,A=i32>)->i32{if x.b(){x.a()}else{0}}
fn same(x:&dyn Get<A=i32,B=bool>)->i32{read(x)}
fn main(){let n=N;println!("{}",same(&n));}
"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "inherited_binding",
                source: r#"
trait Base<A>{type Out;fn get(&self,a:A)->Self::Out;}
trait Sub:Base<i32>{fn extra(&self)->i32;}
struct N;
impl Base<i32> for N{type Out=i32;fn get(&self,a:i32)->i32{a+1}}
impl Sub for N{fn extra(&self)->i32{2}}
fn read(x:&dyn Sub<Out=i32>)->i32{x.get(5)+x.extra()}
fn main(){let n=N;println!("{}",read(&n));}
"#,
                expect: Expect::Run("8\n"),
            },
            Case {
                name: "missing_binding",
                source: "trait Get{type Out;fn get(&self)->Self::Out;}fn bad(_: &dyn Get){}fn main(){}",
                expect: Expect::Invalid("E0191"),
            },
            Case {
                name: "wrong_binding",
                source: "trait Get{type Out;fn get(&self)->Self::Out;}struct N;impl Get for N{type Out=i32;fn get(&self)->i32{1}}fn use_it(_: &dyn Get<Out=bool>){}fn main(){let n=N;use_it(&n);}",
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "wrong_principal_actual",
                source: "trait Get<A>{fn get(&self,a:A);}struct N;impl Get<i32> for N{fn get(&self,a:i32){}}fn use_it(_: &dyn Get<bool>){}fn main(){let n=N;use_it(&n);}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn boxed_object_whole_reborrow() {
    oracle::check(
        "boxed-object-whole-reborrow",
        &[Case {
            name: "boxed_object_method_through_deref",
            source: r#"
trait T { fn a(&self) -> i32; }
impl T for i32 { fn a(&self) -> i32 { *self } }
impl<X: T + ?Sized> T for Box<X> { fn a(&self) -> i32 { (**self).a() + 100 } }
struct W<X: ?Sized>(Box<X>);
impl<X: T + ?Sized> W<X> { fn g(&self) -> i32 { (*self.0).a() } }
fn g<X: T + ?Sized>(b: Box<X>) -> i32 { (*b).a() }
fn main() {
    let b: Box<dyn T> = Box::new(4);
    println!("{}", b.a());
    println!("{}", W(b).g());
    let c: Box<dyn T> = Box::new(5);
    println!("{}", g(c));
}
"#,
            expect: Expect::Run("104\n4\n5\n"),
        }],
    );
}

#[test]
fn dyn_iterator_objects() {
    oracle::check(
        "dyn-iterator-objects",
        &[
            Case {
                name: "mut_dyn_iterator",
                source: r#"
fn total(it: &mut dyn Iterator<Item = i32>) -> i32 {
    let mut s = 0;
    while let Some(x) = it.next() { s += x; }
    s
}
fn main() {
    let mut r = 1..5;
    println!("{}", total(&mut r));
    let mut v = vec![4, 5, 6].into_iter();
    let d: &mut dyn Iterator<Item = i32> = &mut v;
    println!("{:?} {:?}", d.next(), d.nth(0));
}
"#,
                expect: Expect::Run("10\nSome(4) Some(5)\n"),
            },
            Case {
                name: "boxed_dyn_iterator",
                source: r#"
struct C(i32);
impl Iterator for C {
    type Item = i32;
    fn next(&mut self) -> Option<i32> { self.0 += 1; if self.0 < 4 { Some(self.0) } else { None } }
}
fn mk() -> Box<dyn Iterator<Item = i32>> { Box::new(C(0)) }
fn main() {
    let mut b = mk();
    println!("{:?}", b.next());
    let r: &mut dyn Iterator<Item = i32> = &mut *b;
    println!("{:?} {:?}", r.next(), r.size_hint());
}
"#,
                expect: Expect::Run("Some(1)\nSome(2) (0, None)\n"),
            },
            Case {
                name: "sized_method_skipped_by_vtable",
                source: r#"
trait T { fn a(&self) -> i32; fn g<B>(&self, b: B) -> B where Self: Sized { b } fn by(self) -> i32 where Self: Sized { 7 } }
impl T for i32 { fn a(&self) -> i32 { *self } }
fn main() { let x: &dyn T = &5; println!("{} {} {}", x.a(), 3.g(9), 4.by()); }
"#,
                expect: Expect::Run("5 9 7\n"),
            },
            Case {
                name: "sized_method_on_object",
                source: "trait T { fn a(&self) -> i32; fn b(self) -> i32 where Self: Sized { 7 } } impl T for i32 { fn a(&self) -> i32 { *self } } fn main() { let y: Box<dyn T> = Box::new(3); y.b(); }",
                expect: Expect::Invalid("cannot be invoked on a trait object"),
            },
            Case {
                name: "generic_method_object",
                source: "trait T { fn g<A>(&self, a: A); } fn f(_: &dyn T) {} fn main() {}",
                expect: Expect::Invalid("E0038"),
            },
        ],
    );
}

#[test]
fn implicit_sized_parameter_disjoint_from_unsized_impls() {
    oracle::check(
        "implicit-sized-impl-disjoint",
        &[
            Case {
                name: "blanket_and_str_impls",
                source: r#"
trait Name { fn name(&self) -> String; }
impl<T: Clone + std::fmt::Debug> Name for T { fn name(&self) -> String { format!("{:?}", self) } }
impl Name for str { fn name(&self) -> String { format!("str:{}", self) } }
impl Name for [u8] { fn name(&self) -> String { format!("bytes:{}", self.len()) } }
fn main() {
    println!("{} {} {}", 5.name(), "hi".name(), b"abc"[..].name());
}
"#,
                expect: Expect::Run("5 str:hi bytes:3\n"),
            },
            Case {
                name: "relaxed_blanket_overlaps_str",
                source: "trait Name { fn n(&self) {} } impl<T: ?Sized> Name for T {} impl Name for str {} fn main() {}",
                expect: Expect::Invalid("E0119"),
            },
        ],
    );
}
