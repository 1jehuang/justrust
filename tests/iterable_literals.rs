//! Bidirectional scalar item evidence for written range/array for sources.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn iterable_item_constraints_match_stock() {
    oracle::check(
        "iterable-literals",
        &[
            Case {
                name: "frozen_gcd_fib",
                source: include_str!("fixtures/full_programs/05_gcd_fib.rs"),
                expect: Expect::Run(include_str!("fixtures/full_programs/05_gcd_fib.stdout")),
            },
            Case {
                name: "range_call",
                source: "fn f(x:u64)->u64{x}fn main(){let mut total=0;for n in 1..=4{total+=f(n);}println!(\"{}\",total);}",
                expect: Expect::Run("10\n"),
            },
            Case {
                name: "range_return",
                source: "fn f()->u64{for n in 7..8{return n;}0}fn main(){println!(\"{}\",f());}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "range_assignment",
                source: "fn main(){let mut total=0u64;for n in 1..4{total+=n;}println!(\"{}\",total);}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "range_alias",
                source: "fn f(x:u16)->u16{x}fn main(){for n in 1..3{let alias=n;println!(\"{}\",f(alias));}}",
                expect: Expect::Run("1\n2\n"),
            },
            Case {
                name: "range_old_frontier",
                source: "fn f(x:u64)->u64{x}fn main(){let n=1;for n in n..3{println!(\"{}\",f(n));}}",
                expect: Expect::Run("1\n2\n"),
            },
            Case {
                name: "range_shadow",
                source: "fn f(x:u64)->u64{x}fn main(){let n=7i32;for n in 1..3{println!(\"{}\",f(n));}println!(\"{}\",n);}",
                expect: Expect::Run("1\n2\n7\n"),
            },
            Case {
                name: "range_bounds_once",
                source: "fn start()->u64{println!(\"start\");1}fn end()->u64{println!(\"end\");3}fn main(){for n in start()..end(){println!(\"{}\",n);}}",
                expect: Expect::Run("start\nend\n1\n2\n"),
            },
            Case {
                name: "array_call",
                source: "fn f(x:u32)->u32{x}fn main(){for n in [1,2,3]{println!(\"{}\",f(n));}}",
                expect: Expect::Run("1\n2\n3\n"),
            },
            Case {
                name: "array_assignment",
                source: "fn main(){let mut total=0u64;for n in [1,2,3]{total+=n;}println!(\"{}\",total);}",
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "array_repeat",
                source: "fn f(x:u16)->u16{x}fn main(){for n in [3;2]{println!(\"{}\",f(n));}}",
                expect: Expect::Run("3\n3\n"),
            },
            Case {
                name: "array_float",
                source: "fn f(x:f32)->u32{x as u32}fn main(){for n in [1.5,2.5]{println!(\"{}\",f(n));}}",
                expect: Expect::Run("1\n2\n"),
            },
            Case {
                name: "array_element_order",
                source: "fn f(x:u16)->u16{x}fn main(){for n in [{println!(\"first\");1},{println!(\"second\");2}]{println!(\"{}\",f(n));}}",
                expect: Expect::Run("first\nsecond\n1\n2\n"),
            },
            Case {
                name: "array_repeat_once",
                source: "fn f(x:u16)->u16{x}fn main(){for n in [{println!(\"eval\");3};2]{println!(\"{}\",f(n));}}",
                expect: Expect::Run("eval\n3\n3\n"),
            },
            Case {
                name: "array_empty_constrained",
                source: "fn f(x:u32){}fn main(){for n in []{f(n);}println!(\"done\");}",
                expect: Expect::Run("done\n"),
            },
            Case {
                name: "array_empty_repeat_constrained",
                source: "fn f(x:u32){}fn main(){for n in [7;0]{f(n);}println!(\"done\");}",
                expect: Expect::Run("done\n"),
            },
            Case {
                name: "sibling_binders",
                source: "fn a(x:u8)->u8{x}fn b(x:u64)->u64{x}fn main(){for n in [1]{println!(\"{}\",a(n));}for n in 2..3{println!(\"{}\",b(n));}}",
                expect: Expect::Run("1\n2\n"),
            },
            Case {
                name: "range_suffix_fixed",
                source: "fn f(x:u64){}fn main(){for n in 0i32..3{f(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "range_annotation_fixed",
                source: "fn f(x:u64){}fn main(){let end:i32=3;for n in 0..end{f(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "range_overflow",
                source: "fn f(x:u8){}fn main(){for n in 0..256{f(n);}}",
                expect: Expect::Invalid("overflowing_literals"),
            },
            Case {
                name: "range_negative_unsigned",
                source: "fn f(x:u32){}fn main(){for n in -1..2{f(n);}}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "range_conflict",
                source: "fn a(x:u32){}fn b(x:u64){}fn main(){for n in 0..3{a(n);b(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "array_suffix_fixed",
                source: "fn f(x:u64){}fn main(){for n in [1u32,2]{f(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "array_float_fixed",
                source: "fn f(x:f32){}fn main(){for n in [1.0f64,2.0]{f(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "array_overflow",
                source: "fn f(x:u8){}fn main(){for n in [255,256]{f(n);}}",
                expect: Expect::Invalid("overflowing_literals"),
            },
            Case {
                name: "array_conflict",
                source: "fn a(x:u32){}fn b(x:u64){}fn main(){for n in [0,1]{a(n);b(n);}}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "array_empty_conflict",
                source: "fn a(x:u32){}fn b(x:u64){}fn main(){for n in []{a(n);b(n);}}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn unconstrained_empty_iterable_does_not_guess_an_element_type() {
    use std::{fs, process::Command};
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-empty-iterable-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("main.rs");
    let native = root.join("native");
    fs::write(&source, "fn main(){for _ in []{}}").unwrap();
    // The legacy checker describes missing empty-array context as an explicit
    // unsupported inference boundary. It must remain rejected, never silently
    // assigned i32, even though constrained empty iterables now work.
    for check in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
        command.arg(&source);
        if check {
            command.arg("--check");
        } else {
            command.arg("-o").arg(&native);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success(), "ambiguous iterable accepted");
        assert!(!output.stderr.is_empty());
        assert!(!native.exists(), "ambiguous iterable published an image");
    }
    if Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        let output = Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(&source)
            .arg("-o")
            .arg(root.join("stock"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("E0282"));
    }
    fs::remove_dir_all(root).unwrap();
}
