//! Method closure values retain declaration identity rather than method spelling.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;

#[test]
fn implementation_closure_values_match_stock() {
    oracle::check(
        "closure_method_parents",
        &[
            oracle::Case {
                name: "associated",
                source: "struct A;impl A{fn run(n:i32)->i32{let f=|x:i32|x+1;f(n)}}fn main(){println!(\"{}\",A::run(4));}",
                expect: oracle::Expect::Run("5\n"),
            },
            oracle::Case {
                name: "borrow_self",
                source: "struct A{n:i32}impl A{fn run(&self)->i32{let f=||self.n+1;f()}}fn main(){let a=A{n:7};println!(\"{}\",a.run());}",
                expect: oracle::Expect::Run("8\n"),
            },
            oracle::Case {
                name: "same_spelling",
                source: "struct A;struct B;impl A{fn run()->i32{let f=||1i32;f()}}impl B{fn run()->i32{let f=||2i32;f()}}fn main(){println!(\"{} {}\",A::run(),B::run());}",
                expect: oracle::Expect::Run("1 2\n"),
            },
            oracle::Case {
                name: "unused_bad_body",
                source: "struct A;impl A{fn unused(){let f=||true+1i32;}}fn main(){}",
                expect: oracle::Expect::Invalid("E0369"),
            },
        ],
    );
}
