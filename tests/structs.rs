//! Named-struct native acceptance. Each fixture was independently classified with
//! local stock rustc 1.98.1 (48a229cea, edition 2024) as an external oracle only.
//! No stock compiler is invoked by these tests.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-structs-{}-{epoch}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        new_file(&path).write_all(source.as_bytes()).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn compiler(input: &Path, output: &Path, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(input).args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(output);
    }
    command
}
fn runs(source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                result.status.success(),
                "workers={workers}, check={check}: {result:?}\n{}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert!(
            result.status.success(),
            "runtime failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        assert!(
            result.stderr.is_empty(),
            "unexpected runtime stderr: {result:?}"
        );
    }
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn accepts_reordered_fields() {
    runs(
        r#"struct Pair { left: i32, right: i32 } fn run() -> i32 { let p = Pair { right: 2, left: 3 }; p.left * 10 + p.right }
fn main() { if run() == 32 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_nested_tuple_struct() {
    runs(
        r#"struct Leaf { n: i32 } struct Root { pair: (Leaf, i32), tail: Leaf } fn run() -> i32 { let r = Root { pair: (Leaf { n: 4 }, 5), tail: Leaf { n: 12 } }; r.pair.0.n + r.pair.1 + r.tail.n }
fn main() { if run() == 21 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_scalar_field_copy_preserves_parent() {
    runs(
        r#"struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let x = Leaf { n: 6 }; let n = x.n; n + consume(x) }
fn main() { if run() == 12 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_subtree_move_preserves_sibling() {
    runs(
        r#"struct Leaf { n: i32 } struct Root { a: Leaf, b: Leaf } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let r = Root { a: Leaf { n: 5 }, b: Leaf { n: 7 } }; let a = r.a; consume(a) + consume(r.b) }
fn main() { if run() == 12 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_partial_field_reinit_restores_whole() {
    runs(
        r#"struct Leaf { n: i32 } struct Root { a: Leaf, b: Leaf } fn consume(x: Root) -> i32 { x.a.n + x.b.n } fn run() -> i32 { let mut r = Root { a: Leaf { n: 1 }, b: Leaf { n: 8 } }; let a = r.a; r.a = Leaf { n: 6 }; a.n + consume(r) }
fn main() { if run() == 15 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_whole_reinit_after_move() {
    runs(
        r#"struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let mut x = Leaf { n: 4 }; let old = consume(x); x = Leaf { n: 5 }; old + consume(x) }
fn main() { if run() == 9 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_nested_tuple_partial_reinit() {
    runs(
        r#"struct Leaf { n: i32 } fn consume(x: (Leaf, Leaf)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let mut p = (Leaf { n: 1 }, Leaf { n: 3 }); let old = p.0; p.0 = Leaf { n: 6 }; old.n + consume(p) }
fn main() { if run() == 10 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_branch_moves_reinit() {
    runs(
        r#"struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn choose(flag: bool) -> i32 { let mut x = Leaf { n: 1 }; if flag { let old = consume(x); x = Leaf { n: old + 2 }; } else { x.n = 6; } consume(x) } fn run() -> i32 { choose(true) + choose(false) }
fn main() { if run() == 9 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_loop_move_reinit() {
    runs(
        r#"struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let mut x = Leaf { n: 1 }; let mut i = 0; while i < 3 { let n = consume(x); x = Leaf { n: n + 1 }; i = i + 1; } consume(x) }
fn main() { if run() == 4 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_alias_constructor() {
    runs(
        r#"struct Item { n: i32 } use Item as Alias; fn run() -> i32 { let x = Alias { n: 7 }; x.n }
fn main() { if run() == 7 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_public_module_fields() {
    runs(
        r#"mod inner { pub struct Item { pub n: i32 } } fn run() -> i32 { let x = inner::Item { n: 7 }; x.n }
fn main() { if run() == 7 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_private_field_via_factory() {
    runs(
        r#"mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 8 } } pub fn read(x: Item) -> i32 { x.n } } fn run() -> i32 { inner::read(inner::make()) }
fn main() { if run() == 8 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_type_value_namespace() {
    runs(
        r#"struct Item { n: i32 } fn run() -> i32 { let Item = 5; let x = Item { n: 7 }; Item + x.n }
fn main() { if run() == 12 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_distinct_qualified_struct_names() {
    runs(
        r#"mod a { pub struct Item { pub n: i32 } } mod b { pub struct Item { pub n: i32 } } fn run() -> i32 { let x = a::Item { n: 5 }; let y = b::Item { n: 7 }; x.n + y.n }
fn main() { if run() == 12 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_source_order_initializers() {
    runs(
        r#"struct Pair { left: i32, right: i32 } fn run() -> i32 { let mut n = 0; let p = Pair { right: { n = n * 10 + 1; n }, left: { n = n * 10 + 2; n } }; n * 100 + p.left * 10 + p.right }
fn main() { if run() == 1321 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_immutable_new_loop_lifetime() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let mut i = 0; let mut sum = 0; while i < 3 { let x = Item { n: 2 }; sum = sum + consume(x); i = i + 1; } sum }
fn main() { if run() == 6 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn rejects_invalid_whole_move_second_use() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "whole_move_second_use",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let a = consume(x); a + consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_tuple_containing_struct_not_copy() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "tuple_containing_struct_not_copy",
        r#"struct Item { n: i32 } fn consume(x: (Item, i32)) -> i32 { x.0.n + x.1 } fn run() -> i32 { let x = (Item { n: 1 }, 2); let y = x; consume(y) + consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_partial_move_whole_read() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "partial_move_whole_read",
        r#"struct Item { n: i32 } struct Root { a: Item, b: Item } fn consume(x: Root) -> i32 { x.b.n } fn run() -> i32 { let x = Root { a: Item { n: 1 }, b: Item { n: 2 } }; let a = x.a; a.n + consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_partial_move_same_subtree_twice() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "partial_move_same_subtree_twice",
        r#"struct Item { n: i32 } struct Root { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Root { a: Item { n: 1 }, b: Item { n: 2 } }; let a = x.a; consume(a) + consume(x.a) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_immutable_reassignment_after_move() {
    // Stock Rust rejects this fixture with E0384.
    rejects(
        "immutable_reassignment_after_move",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let n = consume(x); x = Item { n: 2 }; n + consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_immutable_field_write() {
    // Stock Rust rejects this fixture with E0594.
    rejects(
        "immutable_field_write",
        r#"struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1 }; x.n = 2; x.n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_move_on_branch_join() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "move_on_branch_join",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn choose(flag: bool) -> i32 { let x = Item { n: 1 }; if flag { consume(x); } else { } consume(x) } fn run() -> i32 { choose(false) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_loop_consumes_without_reinit() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "loop_consumes_without_reinit",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let mut i = 0; while i < 3 { consume(x); i = i + 1; } 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_nominal_identical_shape_mismatch() {
    // Stock Rust rejects this fixture with E0308.
    rejects(
        "nominal_identical_shape_mismatch",
        r#"struct A { n: i32 } struct B { n: i32 } fn consume(x: A) -> i32 { x.n } fn run() -> i32 { consume(B { n: 1 }) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_missing_field() {
    // Stock Rust rejects this fixture with E0063.
    rejects(
        "missing_field",
        r#"struct Item { a: i32, b: i32 } fn run() -> i32 { let x = Item { a: 1 }; x.a } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_duplicate_field() {
    // Stock Rust rejects this fixture with E0062.
    rejects(
        "duplicate_field",
        r#"struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1, n: 2 }; x.n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_unknown_field_constructor() {
    // Stock Rust rejects this fixture with E0560.
    rejects(
        "unknown_field_constructor",
        r#"struct Item { n: i32 } fn run() -> i32 { let x = Item { nope: 1 }; x.n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_unknown_field_read() {
    // Stock Rust rejects this fixture with E0609.
    rejects(
        "unknown_field_read",
        r#"struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1 }; x.nope } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_field_type_mismatch() {
    // Stock Rust rejects this fixture with E0308.
    rejects(
        "field_type_mismatch",
        r#"struct Item { n: i32 } fn run() -> i32 { let x = Item { n: true }; x.n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_private_field_read() {
    // Stock Rust rejects this fixture with E0616.
    rejects(
        "private_field_read",
        r#"mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 1 } } } fn run() -> i32 { inner::make().n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_private_field_construction() {
    // Stock Rust rejects this fixture with E0451.
    rejects(
        "private_field_construction",
        r#"mod inner { pub struct Item { n: i32 } } fn run() -> i32 { let x = inner::Item { n: 1 }; 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_direct_recursive_by_value() {
    // Stock Rust rejects this fixture with E0072.
    rejects(
        "direct_recursive_by_value",
        r#"struct Item { next: Item } fn run() -> i32 { 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_mutual_recursive_by_value() {
    // Stock Rust rejects this fixture with E0072.
    rejects(
        "mutual_recursive_by_value",
        r#"struct A { b: B } struct B { a: A } fn run() -> i32 { 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_tuple_recursive_by_value() {
    // Stock Rust rejects this fixture with E0072.
    rejects(
        "tuple_recursive_by_value",
        r#"struct Item { next: (i32, Item) } fn run() -> i32 { 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_legal_unsupported_empty_braced_struct() {
    // Legal stock Rust, outside this nonempty named-struct milestone.
    rejects(
        "empty_braced_struct",
        r#"struct Empty {} fn run() -> i32 { let x = Empty {}; 1 } fn main() { run(); }"#,
    );
}

#[test]
fn accepts_wide130_fields_shallow_depth() {
    // Width is not nesting depth. All 130 children have the same shallow type.
    runs(
        r#"struct Leaf { n: i32 } struct Wide { f0: Leaf, f1: Leaf, f2: Leaf, f3: Leaf, f4: Leaf, f5: Leaf, f6: Leaf, f7: Leaf, f8: Leaf, f9: Leaf, f10: Leaf, f11: Leaf, f12: Leaf, f13: Leaf, f14: Leaf, f15: Leaf, f16: Leaf, f17: Leaf, f18: Leaf, f19: Leaf, f20: Leaf, f21: Leaf, f22: Leaf, f23: Leaf, f24: Leaf, f25: Leaf, f26: Leaf, f27: Leaf, f28: Leaf, f29: Leaf, f30: Leaf, f31: Leaf, f32: Leaf, f33: Leaf, f34: Leaf, f35: Leaf, f36: Leaf, f37: Leaf, f38: Leaf, f39: Leaf, f40: Leaf, f41: Leaf, f42: Leaf, f43: Leaf, f44: Leaf, f45: Leaf, f46: Leaf, f47: Leaf, f48: Leaf, f49: Leaf, f50: Leaf, f51: Leaf, f52: Leaf, f53: Leaf, f54: Leaf, f55: Leaf, f56: Leaf, f57: Leaf, f58: Leaf, f59: Leaf, f60: Leaf, f61: Leaf, f62: Leaf, f63: Leaf, f64: Leaf, f65: Leaf, f66: Leaf, f67: Leaf, f68: Leaf, f69: Leaf, f70: Leaf, f71: Leaf, f72: Leaf, f73: Leaf, f74: Leaf, f75: Leaf, f76: Leaf, f77: Leaf, f78: Leaf, f79: Leaf, f80: Leaf, f81: Leaf, f82: Leaf, f83: Leaf, f84: Leaf, f85: Leaf, f86: Leaf, f87: Leaf, f88: Leaf, f89: Leaf, f90: Leaf, f91: Leaf, f92: Leaf, f93: Leaf, f94: Leaf, f95: Leaf, f96: Leaf, f97: Leaf, f98: Leaf, f99: Leaf, f100: Leaf, f101: Leaf, f102: Leaf, f103: Leaf, f104: Leaf, f105: Leaf, f106: Leaf, f107: Leaf, f108: Leaf, f109: Leaf, f110: Leaf, f111: Leaf, f112: Leaf, f113: Leaf, f114: Leaf, f115: Leaf, f116: Leaf, f117: Leaf, f118: Leaf, f119: Leaf, f120: Leaf, f121: Leaf, f122: Leaf, f123: Leaf, f124: Leaf, f125: Leaf, f126: Leaf, f127: Leaf, f128: Leaf, f129: Leaf } fn run() -> i32 { let x = Wide { f0: Leaf { n: 0 }, f1: Leaf { n: 1 }, f2: Leaf { n: 2 }, f3: Leaf { n: 3 }, f4: Leaf { n: 4 }, f5: Leaf { n: 5 }, f6: Leaf { n: 6 }, f7: Leaf { n: 7 }, f8: Leaf { n: 8 }, f9: Leaf { n: 9 }, f10: Leaf { n: 10 }, f11: Leaf { n: 11 }, f12: Leaf { n: 12 }, f13: Leaf { n: 13 }, f14: Leaf { n: 14 }, f15: Leaf { n: 15 }, f16: Leaf { n: 16 }, f17: Leaf { n: 17 }, f18: Leaf { n: 18 }, f19: Leaf { n: 19 }, f20: Leaf { n: 20 }, f21: Leaf { n: 21 }, f22: Leaf { n: 22 }, f23: Leaf { n: 23 }, f24: Leaf { n: 24 }, f25: Leaf { n: 25 }, f26: Leaf { n: 26 }, f27: Leaf { n: 27 }, f28: Leaf { n: 28 }, f29: Leaf { n: 29 }, f30: Leaf { n: 30 }, f31: Leaf { n: 31 }, f32: Leaf { n: 32 }, f33: Leaf { n: 33 }, f34: Leaf { n: 34 }, f35: Leaf { n: 35 }, f36: Leaf { n: 36 }, f37: Leaf { n: 37 }, f38: Leaf { n: 38 }, f39: Leaf { n: 39 }, f40: Leaf { n: 40 }, f41: Leaf { n: 41 }, f42: Leaf { n: 42 }, f43: Leaf { n: 43 }, f44: Leaf { n: 44 }, f45: Leaf { n: 45 }, f46: Leaf { n: 46 }, f47: Leaf { n: 47 }, f48: Leaf { n: 48 }, f49: Leaf { n: 49 }, f50: Leaf { n: 50 }, f51: Leaf { n: 51 }, f52: Leaf { n: 52 }, f53: Leaf { n: 53 }, f54: Leaf { n: 54 }, f55: Leaf { n: 55 }, f56: Leaf { n: 56 }, f57: Leaf { n: 57 }, f58: Leaf { n: 58 }, f59: Leaf { n: 59 }, f60: Leaf { n: 60 }, f61: Leaf { n: 61 }, f62: Leaf { n: 62 }, f63: Leaf { n: 63 }, f64: Leaf { n: 64 }, f65: Leaf { n: 65 }, f66: Leaf { n: 66 }, f67: Leaf { n: 67 }, f68: Leaf { n: 68 }, f69: Leaf { n: 69 }, f70: Leaf { n: 70 }, f71: Leaf { n: 71 }, f72: Leaf { n: 72 }, f73: Leaf { n: 73 }, f74: Leaf { n: 74 }, f75: Leaf { n: 75 }, f76: Leaf { n: 76 }, f77: Leaf { n: 77 }, f78: Leaf { n: 78 }, f79: Leaf { n: 79 }, f80: Leaf { n: 80 }, f81: Leaf { n: 81 }, f82: Leaf { n: 82 }, f83: Leaf { n: 83 }, f84: Leaf { n: 84 }, f85: Leaf { n: 85 }, f86: Leaf { n: 86 }, f87: Leaf { n: 87 }, f88: Leaf { n: 88 }, f89: Leaf { n: 89 }, f90: Leaf { n: 90 }, f91: Leaf { n: 91 }, f92: Leaf { n: 92 }, f93: Leaf { n: 93 }, f94: Leaf { n: 94 }, f95: Leaf { n: 95 }, f96: Leaf { n: 96 }, f97: Leaf { n: 97 }, f98: Leaf { n: 98 }, f99: Leaf { n: 99 }, f100: Leaf { n: 100 }, f101: Leaf { n: 101 }, f102: Leaf { n: 102 }, f103: Leaf { n: 103 }, f104: Leaf { n: 104 }, f105: Leaf { n: 105 }, f106: Leaf { n: 106 }, f107: Leaf { n: 107 }, f108: Leaf { n: 108 }, f109: Leaf { n: 109 }, f110: Leaf { n: 110 }, f111: Leaf { n: 111 }, f112: Leaf { n: 112 }, f113: Leaf { n: 113 }, f114: Leaf { n: 114 }, f115: Leaf { n: 115 }, f116: Leaf { n: 116 }, f117: Leaf { n: 117 }, f118: Leaf { n: 118 }, f119: Leaf { n: 119 }, f120: Leaf { n: 120 }, f121: Leaf { n: 121 }, f122: Leaf { n: 122 }, f123: Leaf { n: 123 }, f124: Leaf { n: 124 }, f125: Leaf { n: 125 }, f126: Leaf { n: 126 }, f127: Leaf { n: 127 }, f128: Leaf { n: 128 }, f129: Leaf { n: 129 } }; x.f0.n + x.f64.n + x.f129.n }
fn main() { if run() == 193 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_if_result_transfers_noncopy_value() {
    runs(
        r#"struct Item { n: i32 } fn choose(flag: bool, x: Item) -> Item { if flag { x } else { Item { n: 7 } } } fn run() -> i32 { let first = choose(true, Item { n: 4 }); let second = choose(false, Item { n: 5 }); first.n + second.n }
fn main() { if run() == 11 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn accepts_loop_break_transfers_noncopy_value() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 9 }; let y = loop { break x; }; consume(y) }
fn main() { if run() == 9 { println!("struct oracle ok"); } }"#,
        "struct oracle ok\n",
    );
}

#[test]
fn rejects_invalid_block_snapshot_projection_moves_whole_parent() {
    // Stock Rust rejects this snapshot of the whole parent with E0382.
    rejects(
        "block_snapshot_projection_moves_whole_parent",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 3 }; let n = { x }.n; n + consume(x) } fn main() { run(); }"#,
    );
}
