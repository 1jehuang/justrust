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
