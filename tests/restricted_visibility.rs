//! Restricted visibility, checked against executable stock Rust before native gates.
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
                "jr-restricted-{}-{}",
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
// All declarations, including unused invalid ones, are checked by stock rustc first.
// No legal unsupported fixture is treated as a negative conformance case.
const FIXTURES: &[Fixture] = &[
    fixture!(
        historical_crate_module,
        true,
        "pub(crate) mod a {} fn main() {}"
    ),
    fixture!(
        historical_super_function,
        true,
        "mod a { pub(super) fn f() {} } fn main() {}"
    ),
    fixture!(
        self_module,
        true,
        "mod a{pub(self) mod b{pub fn f()->i32{7}}pub fn run()->i32{b::f()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_module,
        true,
        "mod a{mod b{pub(in crate::a) mod c{pub fn f()->i32{7}}}pub fn run()->i32{b::c::f()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_module_outside,
        false,
        "mod a{pub mod b{pub(in crate::a) mod c{pub fn f(){}}}}fn main(){a::b::c::f();}"
    ),
    fixture!(
        self_type_outside,
        false,
        "mod a{pub(self) struct S{pub x:i32}}fn main(){let s=a::S{x:7};}"
    ),
    fixture!(
        ancestor_type_outside,
        false,
        "mod a{pub mod b{pub(in crate::a) struct S{pub x:i32}}}fn main(){let s=a::b::S{x:7};}"
    ),
    fixture!(
        self_field_read_outside,
        false,
        "mod a{pub struct S{pub(self) x:i32}pub fn make()->S{S{x:7}}}fn main(){let s=a::make();let x=s.x;}"
    ),
    fixture!(
        super_field_write_outside,
        false,
        "mod a{pub mod b{pub struct S{pub(super) x:i32}pub fn make()->S{S{x:7}}}}fn main(){let mut s=a::b::make();s.x=8;}"
    ),
    fixture!(
        super_super_scope,
        true,
        "mod a{mod b{mod c{pub(in super::super) fn f()->i32{7}}pub(super) fn g()->i32{c::f()}}pub fn run()->i32{b::g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        self_named_scope,
        true,
        "mod a{pub(in self) fn f()->i32{7}mod b{pub fn g()->i32{super::f()}}pub fn run()->i32{b::g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_reexport_equal,
        true,
        "mod a{mod b{pub(in crate::a) fn f()->i32{7}}pub(in crate::a) use b::f as g;pub fn run()->i32{g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        narrow_reexport_sibling,
        true,
        "mod a{pub fn f()->i32{7}}mod b{pub(crate) use crate::a::f as g;}mod c{pub fn run()->i32{crate::b::g()}}fn main(){if c::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        alias_widen_self_module_unused,
        false,
        "mod a{pub(self) mod b{}pub(super) use self::b as Alias;}fn main(){}"
    ),
    fixture!(
        alias_lexical_shadow_unused,
        false,
        "mod a{mod b{use crate::a as parent;pub(in self::parent) fn f(){}}}fn main(){}"
    ),
    fixture!(
        invalid_alias_scope_unused,
        false,
        "mod a{}mod b{pub(in crate::a) type T=i32;}fn main(){}"
    ),
    fixture!(
        invalid_use_scope_unused,
        false,
        "mod a{pub fn f(){}}mod b{pub(in crate::a) use crate::a::f;}fn main(){}"
    ),
    fixture!(
        root_super_field_unused,
        false,
        "struct S{pub(super) x:i32}fn main(){}"
    ),
    fixture!(
        root_super_method_unused,
        false,
        "struct S{pub x:i32}impl S{pub(super) fn f(){}}fn main(){}"
    ),
    fixture!(
        crate_function,
        true,
        "mod a{pub(crate) fn f()->i32{7}}fn main(){if a::f()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        super_function,
        true,
        "mod a{mod b{pub(super) fn f()->i32{7}}pub fn run()->i32{b::f()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        self_descendant,
        true,
        "mod a{pub(self) fn f()->i32{7}mod b{pub(super) fn g()->i32{super::f()}}pub fn run()->i32{b::g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_function,
        true,
        "mod a{mod b{mod c{pub(in crate::a) fn f()->i32{7}}pub(super) fn g()->i32{c::f()}}pub fn run()->i32{b::g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        crate_module,
        true,
        "pub(crate) mod a{pub fn f()->i32{7}}fn main(){if a::f()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        super_module,
        true,
        "mod a{mod b{pub(super) mod c{pub fn f()->i32{7}}}pub fn run()->i32{b::c::f()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        crate_type_field_method,
        true,
        "mod a{pub(crate) struct S{pub(crate) x:i32}impl S{pub(crate) fn read(&self)->i32{self.x}}}fn main(){let s=a::S{x:7};if s.read()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        super_type_field_method,
        true,
        "mod a{mod b{pub(super) struct S{pub(super) x:i32}impl S{pub(super) fn read(&self)->i32{self.x}}}pub fn run()->i32{let s=b::S{x:7};s.read()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_type_field_method,
        true,
        "mod a{mod b{pub(in crate::a) struct S{pub(in crate::a) x:i32}impl S{pub(in crate::a) fn read(&self)->i32{self.x}}}pub fn run()->i32{let s=b::S{x:7};s.read()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        self_type_field_method,
        true,
        "mod a{pub(self) struct S{pub(self) x:i32}impl S{pub(self) fn read(&self)->i32{self.x}}pub fn run()->i32{let s=S{x:7};s.read()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        crate_alias_reexport,
        true,
        "mod a{pub(crate) fn f()->i32{7}}pub(crate) use a::f as g;fn main(){if g()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        narrow_public_reexport,
        true,
        "mod a{pub fn f()->i32{7}}mod b{pub(super) use crate::a::f as g;}fn main(){if b::g()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        ancestor_reexport,
        true,
        "mod a{mod b{pub(in crate::a) fn f()->i32{7}}pub(self) use b::f as g;pub fn run()->i32{g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        restricted_type_use_alias,
        true,
        "mod a{pub(crate) struct S{pub x:i32}pub(crate) use self::S as Alias;}fn main(){let s=a::Alias{x:7};if s.x==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        in_crate,
        true,
        "mod a{pub(in crate) fn f()->i32{7}}fn main(){if a::f()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        in_self,
        true,
        "mod a{pub(in self) fn f()->i32{7}pub fn run()->i32{f()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(
        in_super,
        true,
        "mod a{pub(in super) fn f()->i32{7}}fn main(){if a::f()==7{println!(\"visibility ok\");}}"
    ),
    fixture!(root_super_invalid, false, "pub(super) fn f(){}fn main(){}"),
    fixture!(
        self_function_outside,
        false,
        "mod a{pub(self) fn f(){}}fn main(){a::f();}"
    ),
    fixture!(
        super_function_outside,
        false,
        "mod a{pub mod b{pub(super) fn f(){}}}fn main(){a::b::f();}"
    ),
    fixture!(
        ancestor_function_outside,
        false,
        "mod a{pub mod b{pub(in crate::a) fn f(){}}}fn main(){a::b::f();}"
    ),
    fixture!(
        self_module_outside,
        false,
        "mod a{pub(self) mod b{pub fn f(){}}}fn main(){a::b::f();}"
    ),
    fixture!(
        super_type_outside,
        false,
        "mod a{pub mod b{pub(super) struct S{pub x:i32}}}fn main(){let s=a::b::S{x:7};}"
    ),
    fixture!(
        self_field_outside,
        false,
        "mod a{pub struct S{pub(self) x:i32}}fn main(){let s=a::S{x:7};}"
    ),
    fixture!(
        super_field_outside,
        false,
        "mod a{pub mod b{pub struct S{pub(super) x:i32}}}fn main(){let s=a::b::S{x:7};}"
    ),
    fixture!(
        ancestor_field_outside,
        false,
        "mod a{pub mod b{pub struct S{pub(in crate::a) x:i32}}}fn main(){let s=a::b::S{x:7};}"
    ),
    fixture!(
        self_method_outside,
        false,
        "mod a{pub struct S{pub x:i32}impl S{pub(self) fn read(&self)->i32{self.x}}}fn main(){let s=a::S{x:7};s.read();}"
    ),
    fixture!(
        super_method_outside,
        false,
        "mod a{pub mod b{pub struct S{pub x:i32}impl S{pub(super) fn read(&self)->i32{self.x}}}}fn main(){let s=a::b::S{x:7};s.read();}"
    ),
    fixture!(
        ancestor_method_outside,
        false,
        "mod a{pub mod b{pub struct S{pub x:i32}impl S{pub(in crate::a) fn read(&self)->i32{self.x}}}}fn main(){let s=a::b::S{x:7};s.read();}"
    ),
    fixture!(
        widen_crate_function_unused,
        false,
        "mod a{pub(crate) fn f(){}}pub use a::f as g;fn main(){}"
    ),
    fixture!(
        widen_super_function_unused,
        false,
        "mod a{mod b{pub(super) fn f(){}}pub(crate) use b::f as g;}fn main(){}"
    ),
    fixture!(
        widen_self_function_unused,
        false,
        "mod a{pub(self) fn f(){}pub(super) use self::f as g;}fn main(){}"
    ),
    fixture!(
        widen_ancestor_function_unused,
        false,
        "mod a{mod b{pub(in crate::a) fn f(){}}pub use b::f as g;}fn main(){}"
    ),
    fixture!(
        widen_module_unused,
        false,
        "pub(crate) mod a{}pub use a as Alias;fn main(){}"
    ),
    fixture!(
        widen_struct_unused,
        false,
        "pub(crate) struct S{pub x:i32}pub use S as Alias;fn main(){}"
    ),
    fixture!(
        widen_type_use_alias_unused,
        false,
        "pub struct S{pub x:i32}pub(crate) use S as A;pub use A as Alias;fn main(){}"
    ),
    fixture!(
        widen_import_chain_unused,
        false,
        "mod a{pub fn f(){}}mod b{pub(crate) use crate::a::f as g;}pub use b::g as h;fn main(){}"
    ),
    fixture!(
        narrow_alias_outside,
        false,
        "mod a{pub fn f(){}}mod b{pub(self) use crate::a::f as g;}fn main(){b::g();}"
    ),
    fixture!(
        nonancestor_unused,
        false,
        "mod a{}mod b{pub(in crate::a) fn f(){}}fn main(){}"
    ),
    fixture!(
        descendant_unused,
        false,
        "mod a{pub(in crate::a::b) fn f(){}mod b{}}fn main(){}"
    ),
    fixture!(
        missing_scope_unused,
        false,
        "mod a{pub(in crate::missing) fn f(){}}fn main(){}"
    ),
    fixture!(
        alias_scope_unused,
        false,
        "mod a{use crate::a as alias;mod b{pub(in crate::a::alias) fn f(){}}}fn main(){}"
    ),
    fixture!(
        root_alias_scope_unused,
        false,
        "mod a{mod b{pub(in crate::alias) fn f(){}}}use a as alias;fn main(){}"
    ),
    fixture!(
        type_scope_unused,
        false,
        "struct S{pub x:i32}mod a{pub(in crate::S) fn f(){}}fn main(){}"
    ),
    fixture!(
        relative_scope_2021_unused,
        false,
        "mod a{mod b{pub(in a) fn f(){}}}fn main(){}"
    ),
    fixture!(
        invalid_field_scope_unused,
        false,
        "mod a{}mod b{pub struct S{pub(in crate::a) x:i32}}fn main(){}"
    ),
    fixture!(
        invalid_method_scope_unused,
        false,
        "mod a{}mod b{pub struct S{pub x:i32}impl S{pub(in crate::a) fn f(){}}}fn main(){}"
    ),
    fixture!(
        invalid_module_scope_unused,
        false,
        "mod a{}mod b{pub(in crate::a) mod c{}}fn main(){}"
    ),
    fixture!(
        invalid_type_scope_unused,
        false,
        "mod a{}mod b{pub(in crate::a) struct S{pub x:i32}}fn main(){}"
    ),
];
// Promoted unchanged by the subsequent native type-alias milestone.
const PROMOTED_ALIAS: Fixture = fixture!(
    restricted_type_alias,
    true,
    "mod a{pub(crate) struct S{pub x:i32}pub(crate) type Alias=S;}fn main(){let s=a::Alias{x:7};if s.x==7{println!(\"visibility ok\");}}"
);
fn expected_stdout(f: &Fixture) -> Vec<u8> {
    if matches!(
        f.name,
        "historical_crate_module" | "historical_super_function"
    ) {
        Vec::new()
    } else {
        b"visibility ok\n".to_vec()
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
    for f in FIXTURES.iter().chain(std::iter::once(&PROMOTED_ALIAS)) {
        stock(f);
    }
}
#[test]
fn stock_oracle() {
    oracle_all();
    eprintln!(
        "stock restricted visibility: {} supported valid, {} invalid",
        FIXTURES.iter().filter(|f| f.valid).count() + 1,
        FIXTURES.iter().filter(|f| !f.valid).count()
    );
}
#[test]
fn native_after_complete_stock_oracle() {
    oracle_all();
    for f in FIXTURES.iter().chain(std::iter::once(&PROMOTED_ALIAS)) {
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
                if !f.valid {
                    assert!(
                        result.1.is_empty(),
                        "invalid compiler stdout {}: {:?}",
                        f.name,
                        result.1
                    );
                }
                assert_eq!(
                    result.0,
                    if f.valid { 0 } else { 1 },
                    "{} workers={workers} check={check}: {diagnostic}\n{}",
                    f.name,
                    f.source
                );
                assert!(!diagnostic.contains("panicked at"), "{diagnostic}");
                if !f.valid {
                    assert!(
                        !diagnostic.is_empty() && !diagnostic.contains("unsupported"),
                        "invalid fixture not validated {}: {diagnostic}",
                        f.name
                    );
                }
                if check || !f.valid {
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
