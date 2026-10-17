//! Grouped import conformance. Every fixture is checked by external rustc before the native gate.
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
                "jr-grouped-{}-{}",
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

struct Fixture {
    name: &'static str,
    source: &'static str,
    valid: bool,
}
const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "empty_trait_group",
        valid: true,
        source: "trait T{}use T::{};fn main(){println!(\"imports ok\");}",
    },
    Fixture {
        name: "trait_self_alias_bound",
        valid: true,
        source: "trait T{fn f(&self)->i32;}use T::{self as U};struct S{x:i32}impl T for S{fn f(&self)->i32{self.x}}fn read<V:U>(x:V)->i32{x.f()}fn main(){if read(S{x:7})==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "enum_variant_self_prefix_invalid",
        valid: false,
        source: "enum E{A}use E::A::{self as Alias};fn main(){let x=Alias;}",
    },
    Fixture {
        name: "terminal_self_after_group_path",
        valid: true,
        source: "mod a{pub mod b{pub fn f()->i32{7}}}use a::{b::self as x};fn main(){if x::f()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "module_self_type_namespace_value_coexists",
        valid: true,
        source: "mod api{pub fn f()->i32{3}}mod values{pub fn f()->i32{4}}use crate::api::{self as Shared};use crate::values::{f as Shared};fn main(){if Shared::f()+Shared()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "enum_self_and_variant_leaf",
        valid: true,
        source: "mod api{pub enum E{A,B}}use crate::api::E::{self as Kind,A};fn main(){let x:Kind=A;match x{Kind::A=>println!(\"imports ok\"),Kind::B=>()}}",
    },
    Fixture {
        name: "root_self_without_prefix",
        valid: true,
        source: "fn f()->i32{7}use {self as root};fn main(){if root::f()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "crate_self_alias",
        valid: true,
        source: "fn f()->i32{7}use crate::{self as root};fn main(){if root::f()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "enum_self_alias",
        valid: true,
        source: "enum E{A}use E::{self as F};fn main(){let e=F::A;match e{E::A=>println!(\"imports ok\")}}",
    },
    Fixture {
        name: "nested_empty_prefix_group_self",
        valid: true,
        source: "mod a{pub fn f()->i32{7}}use a::{{self as x}};fn main(){if x::f()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "empty_group_valid",
        valid: true,
        source: "mod a{}use a::{};fn main(){println!(\"imports ok\");}",
    },
    Fixture {
        name: "empty_group_unresolved",
        valid: false,
        source: "use missing::{};fn main(){}",
    },
    Fixture {
        name: "empty_group_private_prefix",
        valid: false,
        source: "mod a{mod hidden{}}use a::hidden::{};fn main(){}",
    },
    Fixture {
        name: "nested_groups_trailing_commas",
        valid: true,
        source: r#"
mod api {pub mod inner {pub fn a()->i32{2} pub fn b()->i32{3}} pub fn c()->i32{4}}
use crate::api::{inner::{a,b as bee,},c,};
fn main(){if a()+bee()+c()==9{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "root_group",
        valid: true,
        source: r#"
mod a {pub fn f()->i32{2}} mod b {pub fn g()->i32{3}}
use {crate::a::f,crate::b::g,};
fn main(){if f()+g()==5{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "self_module_aliases",
        valid: true,
        source: r#"
mod api {pub mod nested {pub fn f()->i32{7}}}
use crate::api::{self as first,self as second,nested::{self as inner,f}};
fn main(){if first::nested::f()+second::nested::f()+inner::f()+f()==28{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "self_leaf_module",
        valid: true,
        source: r#"
mod api {pub mod inner {pub fn f()->i32{7}}}
use crate::api::{inner::{self,f}};
fn main(){if inner::f()+f()==14{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "relative_anchors",
        valid: true,
        source: r#"
fn root()->i32{1}
mod outer {pub fn f()->i32{2} pub mod inner {
use super::{f as parent}; use crate::{root as top}; use self::{local as here};
fn local()->i32{3} pub fn run()->i32{parent()+top()+here()}}}
fn main(){if outer::inner::run()==6{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "struct_enum_trait_and_function",
        valid: true,
        source: r#"
mod api {pub struct S{pub x:i32} pub enum E{A,B} pub trait V{fn value(&self)->i32;}
impl V for S{fn value(&self)->i32{self.x}} pub fn make()->S{S{x:7}}}
use crate::api::{S as Item,E as Kind,V as Value,make as create};
fn read<T:Value>(x:T)->i32{x.value()}
fn main(){let s:Item=create();let e=Kind::A;if read(s)==7{match e{Kind::A=>println!("imports ok"),Kind::B=>()}}}
"#,
    },
    Fixture {
        name: "separate_type_value_namespaces",
        valid: true,
        source: r#"
mod types {pub struct Same{pub x:i32}} mod values {pub fn Same()->i32{7}}
use crate::{types::{Same},values::{Same}};
fn main(){let s=Same{x:Same()};if s.x==7{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "local_binding_shadows_import",
        valid: true,
        source: r#"
mod api {pub fn f()->i32{7}} use crate::api::{f};
fn main(){let f=3;if f==3{println!("imports ok");}}
"#,
    },
    Fixture {
        name: "private_function",
        valid: false,
        source: "mod api{fn secret()->i32{7}}use crate::api::{secret};fn main(){secret();}",
    },
    Fixture {
        name: "private_struct",
        valid: false,
        source: "mod api{struct S{x:i32}}use crate::api::{S};fn main(){}",
    },
    Fixture {
        name: "private_module_path",
        valid: false,
        source: "mod api{mod hidden{pub fn f(){}}}use crate::api::{hidden::{f}};fn main(){f();}",
    },
    Fixture {
        name: "private_alias_leakage",
        valid: false,
        source: "mod api{pub fn f(){}}mod bridge{use crate::api::{f as hidden};}use crate::bridge::{hidden};fn main(){hidden();}",
    },
    Fixture {
        name: "duplicate_leaf",
        valid: false,
        source: "mod api{pub fn f(){}}use crate::api::{f,f};fn main(){}",
    },
    Fixture {
        name: "duplicate_alias",
        valid: false,
        source: "mod api{pub fn f(){}pub fn g(){}}use crate::api::{f as same,g as same};fn main(){}",
    },
    Fixture {
        name: "item_import_conflict",
        valid: false,
        source: "mod api{pub fn f(){}}use crate::api::{f};fn f(){}fn main(){}",
    },
    Fixture {
        name: "type_alias_conflict",
        valid: false,
        source: "mod api{pub struct A{x:i32}pub struct B{x:i32}}use crate::api::{A as Same,B as Same};fn main(){}",
    },
    Fixture {
        name: "ambiguous_method",
        valid: false,
        source: "mod api{pub trait A{fn f(&self)->i32;}pub trait B{fn f(&self)->i32;}}struct S{x:i32}impl api::A for S{fn f(&self)->i32{1}}impl api::B for S{fn f(&self)->i32{2}}use crate::api::{A,B};fn main(){let x=S{x:0}.f();}",
    },
    Fixture {
        name: "import_alias_cycle",
        valid: false,
        source: "use crate::{b as a,a as b};fn main(){}",
    },
    Fixture {
        name: "grouped_missing_leaf",
        valid: false,
        source: "mod api{pub fn f(){}}use crate::api::{f,missing};fn main(){f();}",
    },
    Fixture {
        name: "super_above_root",
        valid: false,
        source: "use super::{missing};fn main(){}",
    },
    Fixture {
        name: "self_leaf_struct_prefix_invalid",
        valid: false,
        source: "mod types{pub struct S{pub x:i32}}mod values{pub fn S()->i32{7}}use crate::types::S::{self as Shared};use crate::values::{S as Shared};fn main(){let s=Shared{x:Shared()};if s.x==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "concrete_identity_through_alias_chain",
        valid: true,
        source: "mod api{pub struct S{pub x:i32}}use crate::api::{S as First};use crate::{First as Second};fn take(s:api::S)->i32{s.x}fn main(){let s=Second{x:7};if take(s)==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "self_prefix_dual_namespace",
        valid: true,
        source: "mod api{pub fn f()->i32{7}}use self::api::{f};fn main(){if f()==7{println!(\"imports ok\");}}",
    },
    Fixture {
        name: "nested_self_path_invalid",
        valid: false,
        source: "mod api{pub fn f(){}}use crate::api::{self::f};fn main(){}",
    },
    Fixture {
        name: "nested_super_path_invalid",
        valid: false,
        source: "mod api{pub fn f(){}}use crate::api::{super::api::f};fn main(){}",
    },
    Fixture {
        name: "self_leaf_type_namespace_only",
        valid: false,
        source: "mod api{pub fn f()->i32{7}}use crate::api::f::{self as value};fn main(){value();}",
    },
];

fn stock(fixture: &Fixture, scratch: &Scratch, input: &Path) {
    let output = scratch.path("stock");
    let result = capture(
        Command::new("rustc")
            .arg("--edition=2021")
            .arg(input)
            .arg("-o")
            .arg(&output),
        scratch,
    );
    assert_eq!(
        result.0,
        if fixture.valid { 0 } else { 1 },
        "stock {}: {}\n{}",
        fixture.name,
        String::from_utf8_lossy(&result.2),
        fixture.source
    );
    if fixture.valid {
        assert_eq!(
            capture(&mut Command::new(output), scratch),
            (0, b"imports ok\n".to_vec(), Vec::new()),
            "stock {}",
            fixture.name
        );
    } else {
        assert!(
            !result.2.is_empty(),
            "stock missing diagnostic: {}",
            fixture.name
        );
        assert!(
            !output.exists(),
            "stock published invalid executable: {}",
            fixture.name
        );
    }
}

fn oracle_all(fixtures: &[Fixture]) {
    for fixture in fixtures {
        let scratch = Scratch::new();
        let input = scratch.path("input.rs");
        file(&input).write_all(fixture.source.as_bytes()).unwrap();
        stock(fixture, &scratch, &input);
    }
}

#[test]
fn stock_oracle() {
    oracle_all(FIXTURES);
}

#[test]
fn unchanged_historical_grouped_sources_pass_stock_execution() {
    for (source, expected) in [
        (
            "mod a { pub fn f() {} pub fn g() {} } use a::{f, g}; fn main() {}",
            "",
        ),
        (
            "enum E { A, B, C } use E::{A,B,C}; fn run()->i32{match B{A=>1,B=>2,C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
            "match oracle ok\n",
        ),
    ] {
        let scratch = Scratch::new();
        let input = scratch.path("input.rs");
        file(&input).write_all(source.as_bytes()).unwrap();
        let binary = scratch.path("stock");
        let compiled = capture(
            Command::new("rustc")
                .arg("--edition=2021")
                .arg(&input)
                .arg("-o")
                .arg(&binary),
            &scratch,
        );
        assert_eq!(compiled.0, 0, "{}", String::from_utf8_lossy(&compiled.2));
        assert_eq!(
            capture(&mut Command::new(&binary), &scratch),
            (0, expected.as_bytes().to_vec(), Vec::new())
        );
    }
}

#[test]
fn absolute_local_path_is_invalid_stock_only() {
    // Absolute extern paths are outside the native subset, so native fencing
    // cannot establish that local modules do not resolve as extern crates.
    oracle_all(&[Fixture {
        name: "absolute local path is invalid",
        valid: false,
        source: "mod api{pub fn f(){}}use ::api::{f};fn main(){f();}",
    }]);
}

#[test]
fn grouped_native_after_complete_stock_oracle() {
    // Deliberately complete every external positive and negative oracle before
    // invoking the native compiler even once. Unsupported rejection is not a pass.
    oracle_all(FIXTURES);
    for fixture in FIXTURES {
        let scratch = Scratch::new();
        let input = scratch.path("input.rs");
        file(&input).write_all(fixture.source.as_bytes()).unwrap();
        for workers in [1, 8] {
            for check in [true, false] {
                let output = scratch.path(&format!("native-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .arg(&input)
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&output);
                }
                let result = capture(&mut command, &scratch);
                let diagnostic = String::from_utf8_lossy(&result.2);
                assert_eq!(
                    result.0,
                    if fixture.valid { 0 } else { 1 },
                    "{} workers={workers} check={check}: {diagnostic}\n{}",
                    fixture.name,
                    fixture.source
                );
                assert!(
                    !diagnostic.contains("panicked at"),
                    "compiler panic: {diagnostic}"
                );
                if !fixture.valid {
                    assert!(
                        !diagnostic.is_empty(),
                        "missing diagnostic: {}",
                        fixture.name
                    );
                    assert!(
                        !diagnostic.contains("unsupported"),
                        "invalid fixture fenced off rather than validated: {}: {diagnostic}",
                        fixture.name
                    );
                }
                if check || !fixture.valid {
                    assert!(
                        !scratch.path("input").exists(),
                        "CHECK/failure published default executable: {}",
                        fixture.name
                    );
                    assert!(
                        !scratch.path("a.out").exists(),
                        "CHECK/failure published a.out: {}",
                        fixture.name
                    );
                    assert!(
                        !output.exists(),
                        "CHECK/failure published output: {}",
                        fixture.name
                    );
                } else {
                    let bytes = fs::read(&output).unwrap();
                    assert!(bytes.len() >= 64, "truncated ELF: {}", fixture.name);
                    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                    assert_eq!(
                        capture(&mut Command::new(&output), &scratch),
                        (0, b"imports ok\n".to_vec(), Vec::new()),
                        "{} workers={workers}",
                        fixture.name
                    );
                }
            }
        }
    }
}

#[test]
fn valid_but_unsupported_glob_reexport_and_absolute_extern_stock_only() {
    // These are valid Rust, not negative native fixtures. Native rejection of
    // these sources must never be counted as language-conformance success.
    let sources = [
        "mod api{pub fn f()->i32{7}}use crate::api::{*};fn main(){if f()==7{println!(\"imports ok\");}}",
        "mod api{pub fn f()->i32{7}}mod bridge{pub use crate::api::{f};}fn main(){if bridge::f()==7{println!(\"imports ok\");}}",
        "use ::std::{mem::{size_of}};fn main(){if size_of::<i32>()==4{println!(\"imports ok\");}}",
    ];
    for source in sources {
        oracle_all(&[Fixture {
            name: "valid unsupported, stock only",
            source,
            valid: true,
        }]);
    }
}
