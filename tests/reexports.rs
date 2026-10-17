//! Named public reexports, checked against executable stock Rust before native gates.
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
                "jr-reexports-{}-{}",
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
macro_rules! fixture {
    ($name:ident, $valid:literal, $source:literal) => {
        Fixture {
            name: stringify!($name),
            valid: $valid,
            source: $source,
        }
    };
}
const FIXTURES: &[Fixture] = &[
    fixture!(
        historical_modules_reexport,
        true,
        "mod a { pub fn f() {} } pub use a::f; fn main() {}"
    ),
    fixture!(
        historical_grouped_reexport,
        true,
        "mod api{pub fn f()->i32{7}}mod bridge{pub use crate::api::{f};}fn main(){if bridge::f()==7{println!(\"imports ok\");}}"
    ),
    fixture!(
        private_struct_used,
        false,
        "struct S{x:i32}pub use S as Item;fn main(){let x=Item{x:7};}"
    ),
    fixture!(
        private_enum_used,
        false,
        "enum E{A}pub use E as Kind;fn main(){let x=Kind::A;}"
    ),
    fixture!(
        private_trait_used,
        false,
        "trait T{}pub use T as Value;fn read<X:Value>(x:X){}fn main(){}"
    ),
    fixture!(
        private_module_used,
        false,
        "mod a{pub fn f(){}}pub use a as api;fn main(){api::f();}"
    ),
    fixture!(
        private_enum_variant,
        false,
        "enum E{A}pub use E::A;fn main(){}"
    ),
    fixture!(
        private_self_module,
        false,
        "mod a{pub use self as Alias;}fn main(){}"
    ),
    fixture!(
        public_self_module,
        true,
        "pub mod a{pub use self as Alias;pub fn f()->i32{7}}fn main(){if a::Alias::f()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        private_super_module,
        false,
        "mod a{pub mod b{pub use super as Alias;}}fn main(){}"
    ),
    fixture!(
        public_super_module,
        true,
        "pub mod a{pub fn f()->i32{7}pub mod b{pub use super as Alias;}}fn main(){if a::b::Alias::f()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        mixed_private_value_local,
        true,
        "mod a{pub struct Mixed{pub x:i32}fn Mixed()->i32{7}pub use self::Mixed as Alias;pub fn run()->i32{Alias()}}fn main(){if a::run()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        mixed_private_value_outside,
        false,
        "mod a{pub struct Mixed{pub x:i32}fn Mixed()->i32{7}pub use self::Mixed as Alias;}fn main(){a::Alias();}"
    ),
    fixture!(
        mixed_public_type_outside,
        true,
        "mod a{pub struct Mixed{pub x:i32}fn Mixed()->i32{7}pub use self::Mixed as Alias;}fn main(){let x=a::Alias{x:7};if x.x==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        function_alias,
        true,
        "mod a{pub fn f()->i32{7}}pub use a::f as g;fn main(){if g()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        private_module_public_targets,
        true,
        "mod hidden{pub struct S{pub x:i32}pub enum E{A}pub trait T{fn f(&self)->i32;}impl T for S{fn f(&self)->i32{self.x}}pub fn make()->S{S{x:7}}}pub use hidden::{S as Item,E as Kind,T as Value,make};fn read<X:Value>(x:X)->i32{x.f()}fn main(){let x:Item=make();let e=Kind::A;match e{Kind::A=>if read(x)==7{println!(\"reexports ok\");}}}"
    ),
    fixture!(
        module_reexport,
        true,
        "mod hidden{pub mod inner{pub fn f()->i32{7}}}pub use hidden::inner as api;fn main(){if api::f()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        grouped_type_self,
        true,
        "mod hidden{pub enum E{A}}pub use hidden::E::{self as Kind,A};fn main(){let x:Kind=A;match x{Kind::A=>println!(\"reexports ok\")}}"
    ),
    fixture!(
        module_self_value_namespace,
        true,
        "pub mod api{pub fn f()->i32{3}}mod a{pub fn f()->i32{4}}pub use api::{self as Shared};pub use a::f as Shared;fn main(){if Shared::f()+Shared()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        struct_type_self_value_namespace,
        false,
        "mod a{pub struct S{pub x:i32}pub fn f()->i32{7}}pub use a::S::{self as Same};pub use a::f as Same;fn main(){let x=Same{x:Same()};if x.x==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        separate_namespaces,
        true,
        "mod a{pub struct S{pub x:i32}pub fn f()->i32{7}}pub use a::S as Same;pub use a::f as Same;fn main(){let x=Same{x:Same()};if x.x==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        forward_chain,
        true,
        "pub use b::g as top;mod b{pub use crate::a::f as g;}mod a{pub fn f()->i32{7}}fn main(){if top()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        same_scope_private_import_promoted,
        false,
        "mod a{pub fn f()->i32{7}}mod b{use crate::a::f as local;pub use local as g;}fn main(){if b::g()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        ancestor_private_import_promoted,
        false,
        "mod a{pub fn f()->i32{7}}use a::f as local;mod b{pub use crate::local as g;}fn main(){if b::g()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        public_chain,
        true,
        "mod a{pub fn f()->i32{7}}mod b{pub use crate::a::f as g;}mod c{pub use crate::b::g as h;}fn main(){if c::h()==7{println!(\"reexports ok\");}}"
    ),
    fixture!(
        private_function_unused,
        false,
        "fn f(){}pub use f as g;fn main(){}"
    ),
    fixture!(
        private_function_used,
        false,
        "fn f(){}pub use f as g;fn main(){g();}"
    ),
    fixture!(
        private_struct_unused,
        false,
        "struct S{x:i32}pub use S as Item;fn main(){}"
    ),
    fixture!(
        private_enum_unused,
        false,
        "enum E{A}pub use E as Kind;fn main(){}"
    ),
    fixture!(
        private_trait_unused,
        false,
        "trait T{}pub use T as Value;fn main(){}"
    ),
    fixture!(
        private_module_unused,
        false,
        "mod a{}pub use a as api;fn main(){}"
    ),
    fixture!(
        private_nested_original,
        false,
        "mod a{fn f(){}}pub use a::f;fn main(){}"
    ),
    fixture!(
        private_intermediary_unused,
        false,
        "mod a{pub fn f(){}}mod b{use crate::a::f as g;}pub use b::g as h;fn main(){}"
    ),
    fixture!(
        private_intermediary_used,
        false,
        "mod a{pub fn f(){}}mod b{use crate::a::f as g;}pub use b::g as h;fn main(){h();}"
    ),
    fixture!(
        sibling_private_intermediary,
        false,
        "mod a{pub fn f(){}}mod b{use crate::a::f as g;}mod c{pub use crate::b::g as h;}fn main(){}"
    ),
    fixture!(
        private_origin_via_private_binding,
        false,
        "fn f(){}use f as local;pub use local as g;fn main(){}"
    ),
    fixture!(
        duplicate_value,
        false,
        "mod a{pub fn f(){}pub fn g(){}}pub use a::f as Same;pub use a::g as Same;fn main(){}"
    ),
    fixture!(
        duplicate_type,
        false,
        "mod a{pub struct S{x:i32}pub enum E{A}}pub use a::S as Same;pub use a::E as Same;fn main(){}"
    ),
    fixture!(
        duplicate_same_target,
        false,
        "mod a{pub fn f(){}}pub use a::f;pub use a::f;fn main(){}"
    ),
    fixture!(
        unresolved_forward,
        false,
        "pub use b::g;mod b{pub use crate::missing::f as g;}fn main(){}"
    ),
    fixture!(
        cyclic_chain,
        false,
        "mod a{pub use crate::b::f;}mod b{pub use crate::a::f;}fn main(){}"
    ),
    fixture!(self_cycle, false, "pub use f as f;fn main(){}"),
    fixture!(
        value_self_leaf,
        false,
        "mod a{pub fn f(){}}pub use a::f::{self as g};fn main(){}"
    ),
];
const IMPORTED_MAIN: Fixture = fixture!(
    imported_main,
    true,
    "mod a{pub fn main(){println!(\"reexports ok\");}}pub use a::main;"
);
fn expected_stdout(f: &Fixture) -> Vec<u8> {
    match f.name {
        "historical_modules_reexport" => Vec::new(),
        "historical_grouped_reexport" => b"imports ok\n".to_vec(),
        _ => b"reexports ok\n".to_vec(),
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
        if f.valid { 0 } else { 1 },
        "stock {}: {}\n{}",
        f.name,
        String::from_utf8_lossy(&result.2),
        f.source
    );
    if f.valid {
        assert_eq!(&fs::read(&output).unwrap()[..6], b"\x7fELF\x02\x01");
        assert_eq!(
            capture(&mut Command::new(output), &s),
            (0, expected_stdout(f), Vec::new()),
            "stock {}",
            f.name
        );
    } else {
        assert!(!result.2.is_empty(), "stock diagnostic {}", f.name);
        assert!(!output.exists(), "stock invalid output {}", f.name);
    }
}
fn oracle_all() {
    for f in FIXTURES.iter().chain(std::iter::once(&IMPORTED_MAIN)) {
        stock(f);
    }
}
#[test]
fn stock_oracle() {
    oracle_all();
    eprintln!(
        "stock reexports: {} valid, {} invalid (including legal imported main)",
        FIXTURES.iter().filter(|f| f.valid).count() + 1,
        FIXTURES.iter().filter(|f| !f.valid).count()
    );
}
#[test]
fn native_after_complete_stock_oracle() {
    oracle_all();
    for f in FIXTURES.iter().chain(std::iter::once(&IMPORTED_MAIN)) {
        let unsupported = f.name == IMPORTED_MAIN.name;
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
                assert_eq!(
                    result.0,
                    if f.valid && !unsupported { 0 } else { 1 },
                    "{} workers={workers} check={check}: {diagnostic}\n{}",
                    f.name,
                    f.source
                );
                assert!(!diagnostic.contains("panicked at"), "{diagnostic}");
                if unsupported {
                    assert!(
                        diagnostic.contains("unsupported"),
                        "legal imported main misclassified: {diagnostic}"
                    );
                } else if !f.valid {
                    assert!(
                        !diagnostic.is_empty() && !diagnostic.contains("unsupported"),
                        "invalid fixture not validated {}: {diagnostic}",
                        f.name
                    );
                }
                if check || !f.valid || unsupported {
                    for path in [output, s.path("input"), s.path("a.out")] {
                        assert!(
                            !path.exists(),
                            "CHECK/failure published {}: {}",
                            f.name,
                            path.display()
                        );
                    }
                } else {
                    let bytes = fs::read(&output).unwrap();
                    assert!(bytes.len() >= 64);
                    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                    assert_eq!(
                        capture(&mut Command::new(output), &s),
                        (0, expected_stdout(f), Vec::new()),
                        "{} workers={workers}",
                        f.name
                    );
                }
            }
        }
    }
}
