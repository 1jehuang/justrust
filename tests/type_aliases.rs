//! Type aliases, checked against executable stock Rust before native gates.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-type-alias-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("scratch: {e}"),
            }
        }
        panic!("scratch allocation exhausted")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn capture(command: &mut Command, scratch: &Scratch) -> (i32, Vec<u8>, Vec<u8>) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    let mut child = Running(
        command
            .stdin(Stdio::null())
            .stdout(file(&out))
            .stderr(file(&err))
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "capture exceeded 64KiB: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(
            fs::metadata(path).unwrap().len() <= 65536,
            "capture exceeded 64KiB"
        );
    }
    (
        status.code().expect("process terminated by signal"),
        fs::read(out).unwrap(),
        fs::read(err).unwrap(),
    )
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Valid,
    Invalid,
    Unsupported,
}
struct Fixture {
    name: &'static str,
    source: &'static str,
    outcome: Outcome,
    code: &'static str,
}
// Unsupported means legal stock Rust, never invalid conformance.
const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "scalar",
        outcome: Outcome::Valid,
        source: "type Number=i32;fn main(){let x:Number=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "tuple",
        outcome: Outcome::Valid,
        source: "type Pair=(i32,bool);fn main(){let x:Pair=(7,true);if x.0==7 && x.1{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "array",
        outcome: Outcome::Valid,
        source: "type Items=[i32;2];fn main(){let x:Items=[3,4];if x[0]+x[1]==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "forward_chain",
        outcome: Outcome::Valid,
        source: "type A=B;type B=C;type C=i32;fn main(){let x:A=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "import_reexport",
        outcome: Outcome::Valid,
        source: "mod a{pub type A=i32;}mod b{pub use crate::a::A as B;}use b::B as C;fn main(){let x:C=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "restricted_alias",
        outcome: Outcome::Valid,
        source: "mod a{mod b{pub(in crate::a) type A=i32;}pub fn run()->i32{let x:b::A=7;x}}fn main(){if a::run()==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "restricted_alias_outside",
        outcome: Outcome::Invalid,
        source: "mod a{pub mod b{pub(in crate::a) type A=i32;}}fn main(){let x:a::b::A=7;}",
        code: "",
    },
    Fixture {
        name: "private_alias_outside",
        outcome: Outcome::Invalid,
        source: "mod a{type A=i32;}fn main(){let x:a::A=7;}",
        code: "",
    },
    Fixture {
        name: "private_definition_target",
        outcome: Outcome::Valid,
        source: "mod a{type Hidden=i32;pub type A=Hidden;}fn main(){let x:a::A=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "definition_context",
        outcome: Outcome::Valid,
        source: "type Inner=bool;mod a{type Inner=i32;pub type A=Inner;}fn main(){let x:a::A=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_scalar",
        outcome: Outcome::Valid,
        source: "type Id<T>=T;fn main(){let x:Id<i32>=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_tuple",
        outcome: Outcome::Valid,
        source: "type Pair<T,U>=(T,U);fn main(){let x:Pair<i32,bool>=(7,true);if x.0==7 && x.1{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_array",
        outcome: Outcome::Valid,
        source: "type Items<T>=[T;2];fn main(){let x:Items<i32>=[3,4];if x[0]+x[1]==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_nested",
        outcome: Outcome::Valid,
        source: "type Pair<T>=(T,T);type Nested<U>=Pair<U>;fn main(){let x:Nested<i32>=(3,4);if x.0+x.1==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "same_name_hygiene",
        outcome: Outcome::Valid,
        source: "type Swap<T,U>=(U,T);type Again<U,T>=Swap<T,U>;fn main(){let x:Again<i32,bool>=(7,true);if x.0==7 && x.1{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_arity_missing",
        outcome: Outcome::Invalid,
        source: "type Pair<T,U>=(T,U);fn main(){let x:Pair<i32>=(1,2);}",
        code: "",
    },
    Fixture {
        name: "generic_arity_extra",
        outcome: Outcome::Invalid,
        source: "type Id<T>=T;fn main(){let x:Id<i32,bool>=1;}",
        code: "",
    },
    Fixture {
        name: "mono_arity_extra",
        outcome: Outcome::Invalid,
        source: "type Id=i32;fn main(){let x:Id<i32>=1;}",
        code: "",
    },
    Fixture {
        name: "unused_parameter",
        outcome: Outcome::Invalid,
        source: "type Ignore<T>=i32;fn main(){}",
        code: "E0091",
    },
    Fixture {
        name: "unused_indirect_parameter",
        outcome: Outcome::Invalid,
        source: "type Id<T>=T;type Ignore<T>=Id<i32>;fn main(){}",
        code: "E0091",
    },
    Fixture {
        name: "nominal_fields",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;fn main(){let x:A=S{x:7};if x.x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "alias_struct_literal",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;fn main(){let x=A{x:7};if x.x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "nominal_method",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}impl S{fn read(&self)->i32{self.x}}type A=S;fn main(){let x:A=S{x:7};if x.read()==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "alias_impl_identity",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;impl A{fn read(&self)->i32{self.x}}fn main(){let x=S{x:7};if x.read()==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "alias_trait_identity",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;trait Read{fn read(&self)->i32;}impl Read for A{fn read(&self)->i32{self.x}}fn main(){let x=S{x:7};if x.read()==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_nominal",
        outcome: Outcome::Valid,
        source: "struct S<T>{x:T}type A<U>=S<U>;fn main(){let x:A<i32>=S{x:7};if x.x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "enum_qualified_unit",
        outcome: Outcome::Valid,
        source: "enum E{A,B}type Alias=E;fn main(){let x=Alias::A;match x{E::A=>println!(\"alias ok\"),E::B=>{}}}",
        code: "",
    },
    Fixture {
        name: "enum_qualified_tuple",
        outcome: Outcome::Valid,
        source: "enum E{A(i32),B}type Alias=E;fn main(){let x=Alias::A(7);match x{E::A(v)=>{if v==7{println!(\"alias ok\");}},E::B=>{}}}",
        code: "",
    },
    Fixture {
        name: "enum_qualified_struct",
        outcome: Outcome::Valid,
        source: "enum E{A{x:i32},B}type Alias=E;fn main(){let x=Alias::A{x:7};match x{E::A{x:v}=>{if v==7{println!(\"alias ok\");}},E::B=>{}}}",
        code: "",
    },
    Fixture {
        name: "tuple_struct_constructor",
        outcome: Outcome::Invalid,
        source: "struct S(i32);type A=S;fn main(){let x=A(7);}",
        code: "",
    },
    Fixture {
        name: "unit_struct_constructor",
        outcome: Outcome::Invalid,
        source: "struct S;type A=S;fn main(){let x=A;}",
        code: "",
    },
    Fixture {
        name: "private_target_nominal",
        outcome: Outcome::Invalid,
        source: "mod a{struct S{pub x:i32}pub type A=S;}fn main(){let x=a::A{x:7};}",
        code: "",
    },
    Fixture {
        name: "cycle_unused",
        outcome: Outcome::Invalid,
        source: "type A=A;fn main(){}",
        code: "E0391",
    },
    Fixture {
        name: "mutual_cycle_unused",
        outcome: Outcome::Invalid,
        source: "type A=B;type B=A;fn main(){}",
        code: "E0391",
    },
    Fixture {
        name: "growing_cycle_unused",
        outcome: Outcome::Invalid,
        source: "type A<T>=A<(T,T)>;fn main(){}",
        code: "E0391",
    },
    Fixture {
        name: "growing_mutual_cycle",
        outcome: Outcome::Invalid,
        source: "type A<T>=B<(T,T)>;type B<U>=A<U>;fn main(){}",
        code: "E0391",
    },
    Fixture {
        name: "unknown_target_unused",
        outcome: Outcome::Invalid,
        source: "type A=Missing;fn main(){}",
        code: "",
    },
    Fixture {
        name: "reference",
        outcome: Outcome::Unsupported,
        source: "type A=&'static i32;fn main(){}",
        code: "",
    },
    Fixture {
        name: "lifetime",
        outcome: Outcome::Unsupported,
        source: "type A<'a>=&'a i32;fn main(){}",
        code: "",
    },
    Fixture {
        name: "associated",
        outcome: Outcome::Unsupported,
        source: "trait Tr{type Item;}struct S;impl Tr for S{type Item=i32;}type A=<S as Tr>::Item;fn main(){}",
        code: "",
    },
    Fixture {
        name: "where_clause",
        outcome: Outcome::Unsupported,
        source: "type A<T> where T: Copy = (T,T);fn main(){}",
        code: "",
    },
    Fixture {
        name: "const_generic",
        outcome: Outcome::Unsupported,
        source: "type A<const N:usize>=[i32;N];fn main(){}",
        code: "",
    },
    Fixture {
        name: "target_bounds_deferred",
        outcome: Outcome::Unsupported,
        source: "trait Tr{}struct S<T:Tr>{x:T}type A<T>=S<T>;fn main(){println!(\"alias ok\");}",
        code: "",
    },
    Fixture {
        name: "alias_decl_bound_ignored",
        outcome: Outcome::Unsupported,
        source: "trait Tr{}type A<T:Tr>=T;fn main(){let x:A<i32>=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "module_target_unused",
        outcome: Outcome::Invalid,
        source: "mod m{}type A=m;fn main(){}",
        code: "",
    },
    Fixture {
        name: "hole_unused",
        outcome: Outcome::Invalid,
        source: "type A=_;fn main(){}",
        code: "E0121",
    },
    Fixture {
        name: "nested_hole_unused",
        outcome: Outcome::Invalid,
        source: "type A=(i32,_);fn main(){}",
        code: "E0121",
    },
    Fixture {
        name: "generic_default",
        outcome: Outcome::Unsupported,
        source: "type A<T=i32>=T;fn main(){let x:A=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "historical_restricted_type_alias",
        outcome: Outcome::Valid,
        source: "mod a{pub(crate) struct S{pub x:i32}pub(crate) type Alias=S;}fn main(){let s=a::Alias{x:7};if s.x==7{println!(\"visibility ok\");}}",
        code: "",
    },
    Fixture {
        name: "struct_alias_pattern",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;fn main(){let a=S{x:7};let A{x}=a;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "enum_alias_pattern",
        outcome: Outcome::Valid,
        source: "enum E{V(i32)}type A=E;fn main(){let e=E::V(7);match e{A::V(v)=>{if v==7{println!(\"alias ok\");}}}}",
        code: "",
    },
    Fixture {
        name: "function_value_namespace",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;fn A()->i32{7}fn main(){let x=A();if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "local_value_namespace",
        outcome: Outcome::Valid,
        source: "struct S{x:i32}type A=S;fn main(){let A=7;let x=A;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "fixed_generic_alias_pattern_mismatch",
        outcome: Outcome::Invalid,
        source: "struct S<T>{x:T}type A=S<i32>;fn main(){let s=S{x:true};let A{x}=s;}",
        code: "E0308",
    },
    Fixture {
        name: "fixed_generic_alias_pattern",
        outcome: Outcome::Valid,
        source: "struct S<T>{x:T}type A=S<i32>;fn main(){let s=S{x:7};let A{x}=s;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "generic_alias_pattern_inference",
        outcome: Outcome::Unsupported,
        source: "struct S<T>{x:T}type A<T>=S<(T,T)>;fn main(){let s=S{x:(3,4)};let A{x}=s;if x.0+x.1==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "alias_grouped_self_import",
        outcome: Outcome::Invalid,
        source: "struct S{x:i32}type A=S;use A::{self as B};fn main(){let x=B{x:7};}",
        code: "E0432",
    },
    Fixture {
        name: "mixed_const_generic",
        outcome: Outcome::Unsupported,
        source: "type A<T,const N:usize>=(T,[i32;N]);fn main(){}",
        code: "",
    },
    Fixture {
        name: "module_prefix_generic_arguments",
        outcome: Outcome::Invalid,
        source: "mod m{pub struct S{pub x:i32}pub type A=S;}fn main(){let x=m::<i32>::A{x:7};}",
        code: "E0109",
    },
    Fixture {
        name: "public_nominal_private_module",
        outcome: Outcome::Valid,
        source: "mod a{mod hidden{pub struct S{pub x:i32}}pub type A=hidden::S;}fn main(){let x=a::A{x:7};if x.x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "private_type_public_function_namespace",
        outcome: Outcome::Valid,
        source: "mod a{type F=i32;pub fn F()->i32{7}}fn main(){let x=a::F();if x==7{println!(\"alias ok\");}}",
        code: "",
    },
    Fixture {
        name: "formal_parameter_shadows_outer_alias",
        outcome: Outcome::Valid,
        source: "type T=A<i32>;type A<T>=T;fn main(){let x:T=7;if x==7{println!(\"alias ok\");}}",
        code: "",
    },
];

fn expected_stdout(f: &Fixture) -> Vec<u8> {
    if f.name == "historical_restricted_type_alias" {
        b"visibility ok\n".to_vec()
    } else if f.source.contains("println!") {
        b"alias ok\n".to_vec()
    } else {
        Vec::new()
    }
}
fn stock(f: &Fixture) {
    let s = Scratch::new();
    let input = s.path("input.rs");
    let output = s.path("stock");
    file(&input).write_all(f.source.as_bytes()).unwrap();
    let result = capture(
        Command::new("rustc")
            .arg("--edition=2021")
            .arg(&input)
            .arg("-o")
            .arg(&output),
        &s,
    );
    assert_eq!(
        result.0,
        if f.outcome == Outcome::Invalid { 1 } else { 0 },
        "stock {}: {}\n{}",
        f.name,
        String::from_utf8_lossy(&result.2),
        f.source
    );
    if f.outcome != Outcome::Invalid {
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64);
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        assert_eq!(
            capture(&mut Command::new(output), &s),
            (0, expected_stdout(f), Vec::new()),
            "stock {}",
            f.name
        );
    } else {
        let diagnostic = String::from_utf8_lossy(&result.2);
        assert!(!diagnostic.is_empty(), "stock diagnostic {}", f.name);
        assert!(
            diagnostic.contains(f.code),
            "stock {} missing {}: {diagnostic}",
            f.name,
            f.code
        );
        assert!(!output.exists(), "stock invalid output {}", f.name);
    }
    eprintln!("stock alias PASS {}", f.name);
}
fn oracle_all() {
    for f in FIXTURES {
        stock(f);
    }
}
fn counts() -> (usize, usize, usize) {
    (
        FIXTURES
            .iter()
            .filter(|f| f.outcome == Outcome::Valid)
            .count(),
        FIXTURES
            .iter()
            .filter(|f| f.outcome == Outcome::Invalid)
            .count(),
        FIXTURES
            .iter()
            .filter(|f| f.outcome == Outcome::Unsupported)
            .count(),
    )
}
#[test]
fn stock_oracle() {
    oracle_all();
    eprintln!(
        "stock alias baseline (valid, invalid, legal unsupported): {:?}",
        counts()
    );
}
#[test]
fn native_after_complete_stock_oracle() {
    // Complete every stock outcome before running even the first native command.
    oracle_all();
    eprintln!("complete stock alias baseline: {:?}", counts());
    let mut failures = Vec::new();
    let mut passed = 0;
    for f in FIXTURES {
        let s = Scratch::new();
        let input = s.path("input.rs");
        file(&input).write_all(f.source.as_bytes()).unwrap();
        for workers in [1, 8] {
            for check in [true, false] {
                let output = s.path(&format!("native-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&s.0)
                    .arg(&input)
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&output);
                }
                let result = capture(&mut command, &s);
                let diagnostic = String::from_utf8_lossy(&result.2);
                let label = format!("{} workers={workers} check={check}", f.name);
                // Collect independent failures, never retry a failed invocation.
                let expected = if f.outcome == Outcome::Valid { 0 } else { 1 };
                let mut errors = Vec::new();
                if result.0 != expected {
                    errors.push(format!("exit {} expected {expected}", result.0));
                }
                if diagnostic.contains("panicked at") {
                    errors.push("compiler panic".into());
                }
                if f.outcome != Outcome::Valid {
                    if !result.1.is_empty() {
                        errors.push("failure printed stdout".into());
                    }
                    if diagnostic.is_empty() {
                        errors.push("missing failure diagnostic".into());
                    }
                    if f.outcome == Outcome::Unsupported && !diagnostic.contains("unsupported") {
                        errors
                            .push("legal unsupported lacks explicit unsupported diagnostic".into());
                    }
                    if f.name == "generic_alias_pattern_inference"
                        && !diagnostic.contains(
                            "inferred generic type alias constructors and patterns are unsupported",
                        )
                    {
                        errors
                            .push("missing precise alias inference unsupported diagnostic".into());
                    }
                    if f.outcome == Outcome::Invalid && diagnostic.contains("unsupported") {
                        errors.push("invalid Rust misclassified unsupported".into());
                    }
                }
                if check || f.outcome != Outcome::Valid || result.0 != 0 {
                    for path in [&output, &s.path("input"), &s.path("a.out")] {
                        if path.exists() {
                            errors.push(format!("CHECK/failure published {}", path.display()));
                        }
                    }
                } else if output.exists() {
                    let bytes = fs::read(&output).unwrap();
                    if bytes.len() < 64 || &bytes[..6] != b"\x7fELF\x02\x01" {
                        errors.push("not ELF64 little-endian".into());
                    } else {
                        let run = capture(&mut Command::new(&output), &s);
                        if run != (0, expected_stdout(f), Vec::new()) {
                            errors.push(format!("runtime {run:?}"));
                        }
                    }
                } else {
                    errors.push("BUILD missing output".into());
                }
                if errors.is_empty() {
                    passed += 1;
                } else {
                    failures.push(format!(
                        "{label}: {}\n{diagnostic}\n{}",
                        errors.join(", "),
                        f.source
                    ));
                }
            }
        }
    }
    eprintln!(
        "native alias frozen results: {passed}/{} command gates passed, {} failed",
        FIXTURES.len() * 4,
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
