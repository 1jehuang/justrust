//! Positional pattern inference must resolve the nominal declaration before
//! treating the path as an enum variant owner.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[derive(Clone, Copy)]
enum Expect {
    Run(&'static str),
    Invalid(&'static str),
}
struct Case {
    name: &'static str,
    source: &'static str,
    expect: Expect,
}
fn check(tag: &str, cases: &[Case]) {
    let directory = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for case in cases {
        let source = directory.join("main.rs");
        let image = directory.join("program");
        std::fs::write(&source, case.source).unwrap();
        let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
        let stock = Command::new(rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "-Awarnings", "-Cpanic=abort"])
            .arg(&source)
            .arg("-o")
            .arg(&image)
            .output()
            .unwrap();
        match case.expect {
            Expect::Run(expected) => {
                assert!(stock.status.success(), "{}: {stock:?}", case.name);
                let actual = Command::new(&image).output().unwrap();
                assert!(actual.status.success(), "{}: {actual:?}", case.name);
                assert_eq!(actual.stdout, expected.as_bytes(), "{}", case.name);
            }
            Expect::Invalid(code) => {
                assert!(!stock.status.success(), "{}", case.name);
                assert!(
                    String::from_utf8_lossy(&stock.stderr).contains(code),
                    "{}: {stock:?}",
                    case.name
                );
            }
        }
        for workers in [1, 8] {
            for checking in [true, false] {
                let compiled = compiler::compile(case.source.to_owned(), workers, checking);
                match case.expect {
                    Expect::Run(expected) => {
                        let compiled = compiled.unwrap_or_else(|error| {
                            panic!("{} workers={workers} check={checking}: {error}", case.name)
                        });
                        assert_eq!(compiled.executable.is_none(), checking, "{}", case.name);
                        if let Some(bytes) = compiled.executable {
                            std::fs::write(&image, bytes).unwrap();
                            std::fs::set_permissions(
                                &image,
                                std::fs::Permissions::from_mode(0o700),
                            )
                            .unwrap();
                            let actual = Command::new(&image).output().unwrap();
                            assert!(actual.status.success(), "{}: {actual:?}", case.name);
                            assert_eq!(
                                actual.stdout,
                                expected.as_bytes(),
                                "{} workers={workers}",
                                case.name
                            );
                        }
                    }
                    Expect::Invalid(_) => {
                        let message = match compiled {
                            Ok(_) => panic!("{} accepted invalid input", case.name),
                            Err(error) => error.to_string(),
                        };
                        assert!(!message.contains("unsupported"), "{}: {message}", case.name);
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn positional_patterns_preserve_nominal_identity() {
    check(
        "positional-pattern-inference",
        &[
            Case {
                name: "tuple_struct_nested_transport",
                source: r#"
struct Marker;
struct Pair(i32, i32);
struct Packet(Pair, Marker, i32);
fn left()->i32 { println!("first"); 2 }
fn right()->i32 { println!("second"); 40 }
fn swap(p:Pair)->Pair { let Pair(a,b)=p; Pair(b,a) }
fn pass(p:Packet)->Packet { p }
fn main() {
    let packet=pass(Packet(swap(Pair(left(),right())),Marker,7));
    let Packet(pair,marker,n)=packet;
    let Pair(a,b)=pair;
    let Marker=marker;
    let n=(n as u8) as u16;
    if a==40 && b==2 && n==7 { println!("{} {} {}",a,b,n); }
}
"#,
                expect: Expect::Run("first\nsecond\n40 2 7\n"),
            },
            Case {
                name: "qualified_tuple_struct_and_enum_variant",
                source: r#"
mod a { pub struct Pair(pub u8,pub u8); }
mod b { pub enum Pair { Both(u16,u16) } }
fn main() {
    let a::Pair(x,y)=a::Pair(3,4);
    let b::Pair::Both(p,q)=b::Pair::Both(5,6);
    println!("{} {} {} {}",x,y,p,q);
}
"#,
                expect: Expect::Run("3 4 5 6\n"),
            },
            Case {
                name: "unused_mismatched_nominal_pattern",
                source: "struct A(u8);struct B(u8);fn invalid(value:A){let B(x)=value;}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
