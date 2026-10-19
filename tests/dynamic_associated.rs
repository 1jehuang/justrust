#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

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
