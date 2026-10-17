//! Irrefutable let-pattern native acceptance. Each fixture was independently classified with
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
            let path = base.join(format!("jr-patterns-{}-{epoch}-{id}", std::process::id()));
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
fn accepts_tuple_bindings_and_per_binding_mut() {
    runs(
        r#"fn run() -> i32 { let (a, mut b) = (3, 4); b = b + 3; a * 10 + b }
fn main() { if run() == 37 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_nested_tuple_patterns() {
    runs(
        r#"fn run() -> i32 { let ((a, _), (mut b, (c,))) = ((2, true), (3, (4,))); b = b + 3; a + b + c }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_renamed_bindings() {
    runs(
        r#"struct Pair { left: i32, right: i32 } fn run() -> i32 { let Pair { right: b, left: a } = Pair { left: 3, right: 7 }; a * 10 + b }
fn main() { if run() == 37 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_shorthand_per_binding_mut() {
    runs(
        r#"struct Pair { left: i32, right: i32 } fn run() -> i32 { let Pair { left, mut right } = Pair { left: 3, right: 4 }; right = right + 3; left * 10 + right }
fn main() { if run() == 37 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_nested_struct_tuple_patterns() {
    runs(
        r#"struct Leaf { n: i32 } struct Root { pair: (Leaf, i32), tail: Leaf } fn run() -> i32 { let root = Root { pair: (Leaf { n: 4 }, 5), tail: Leaf { n: 12 } }; let Root { pair: (Leaf { n: a }, mut b), tail: Leaf { n: c } } = root; b = b + 1; a + b + c }
fn main() { if run() == 22 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_tuple_noncopy_binding_moves_only_selected_field() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let (a, _) = pair; consume(a) + consume(pair.1) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_noncopy_binding_moves_only_selected_field() {
    runs(
        r#"struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let Pair { a, b: _ } = pair; consume(a) + consume(pair.b) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_binding_does_not_move_whole_value() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let _ = x; consume(x) }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_binding_does_not_read_moved_value() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = x; n }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_tuple_wildcard_skips_moved_field() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let a = pair.0; let (_, b) = pair; consume(a) + consume(b) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_wildcard_skips_moved_field() {
    runs(
        r#"struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let a = pair.a; let Pair { a: _, b } = pair; consume(a) + consume(b) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_all_wildcards_ignore_wholly_moved_tuple() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let n = consume(pair); let (_, _) = pair; n }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_all_wildcards_ignore_wholly_moved_struct() {
    runs(
        r#"struct Pair { a: i32, b: i32 } fn consume(x: Pair) -> i32 { x.a + x.b } fn run() -> i32 { let pair = Pair { a: 5, b: 7 }; let n = consume(pair); let Pair { a: _, b: _ } = pair; n }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_scalar_pattern_bindings_copy_without_moving_parent() {
    runs(
        r#"struct Pair { a: i32, b: i32 } fn consume(x: Pair) -> i32 { x.a + x.b } fn run() -> i32 { let pair = Pair { a: 5, b: 7 }; let Pair { a, b } = pair; a + b + consume(pair) }
fn main() { if run() == 24 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_pattern_initializer_evaluated_once() {
    runs(
        r#"fn run() -> i32 { let mut order = 0; let (a, b) = { order = order + 1; (order, { order = order * 10 + 2; order }) }; order * 100 + a * 10 + b }
fn main() { if run() == 1222 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_initializer_source_order_not_pattern_order() {
    runs(
        r#"struct Pair { left: i32, right: i32 } fn run() -> i32 { let mut order = 0; let Pair { left: a, right: b } = Pair { right: { order = order * 10 + 1; order }, left: { order = order * 10 + 2; order } }; order * 100 + a * 10 + b }
fn main() { if run() == 1321 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_initializer_still_runs_side_effects() {
    runs(
        r#"fn run() -> i32 { let mut n = 0; let _ = { n = n + 1; (3, 4) }; n }
fn main() { if run() == 1 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_shadowing_initializer_uses_old_bindings() {
    runs(
        r#"fn run() -> i32 { let (x, y) = (1, 2); let (x, y) = (y + 3, x + 4); x * 10 + y }
fn main() { if run() == 55 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_nested_scope_pattern_shadowing() {
    runs(
        r#"fn run() -> i32 { let (x, y) = (5, 7); { let (x, y) = (1, 2); let _ = x + y; } x * 10 + y }
fn main() { if run() == 57 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_mutable_binding_reinitializes_moved_field() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let (mut x, _) = (Item { n: 5 }, 0); let n = consume(x); x = Item { n: 7 }; n + consume(x) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_annotated_tuple_pattern() {
    runs(
        r#"fn run() -> i32 { let (a, b): (i32, i32) = (3, 4); a + b }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_qualified_struct_pattern_use_alias() {
    runs(
        r#"mod inner { pub struct Item { pub n: i32 } } use inner::Item as Alias; fn run() -> i32 { let Alias { n } = inner::Item { n: 7 }; n }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_rest_omits_private_field() {
    runs(
        r#"mod inner { pub struct Item { pub n: i32, secret: i32 } pub fn make() -> Item { Item { n: 7, secret: 9 } } } fn run() -> i32 { let inner::Item { n, .. } = inner::make(); n }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_rest_does_not_move_omitted_subtree() {
    runs(
        r#"struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let Pair { a, .. } = pair; consume(a) + consume(pair.b) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_struct_rest_skips_moved_subtree() {
    runs(
        r#"struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let a = pair.a; let Pair { b, .. } = pair; consume(a) + consume(b) }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_nested_wildcards_skip_moved_ancestors() {
    runs(
        r#"struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Pair) -> i32 { x.a.n + x.b.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let n = consume(pair); let Pair { a: Item { n: _ }, b: _ } = pair; n }
fn main() { if run() == 12 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_annotated_struct_pattern() {
    runs(
        r#"struct Item { n: i32 } fn run() -> i32 { let Item { n }: Item = Item { n: 7 }; n }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_pattern_partial_move_then_source_field_reinit() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let mut pair = (Item { n: 1 }, Item { n: 7 }); let (old, _) = pair; pair.0 = Item { n: 6 }; old.n + consume(pair) }
fn main() { if run() == 14 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_pattern_bindings_fresh_each_loop_lifetime() {
    runs(
        r#"fn run() -> i32 { let mut i = 0; let mut sum = 0; while i < 3 { let (x, mut y) = (1, 1); y = y + 1; sum = sum + x + y; i = i + 1; } sum }
fn main() { if run() == 9 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn rejects_invalid_tuple_duplicate_binding() {
    // Stock Rust rejects this fixture with E0416.
    rejects(
        "tuple_duplicate_binding",
        r#"fn run() -> i32 { let (x, x) = (1, 2); x } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_nested_duplicate_binding() {
    // Stock Rust rejects this fixture with E0416.
    rejects(
        "nested_duplicate_binding",
        r#"fn run() -> i32 { let ((x, _), x) = ((1, 2), 3); x } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_duplicate_binding() {
    // Stock Rust rejects this fixture with E0416.
    rejects(
        "struct_duplicate_binding",
        r#"struct Pair { a: i32, b: i32 } fn run() -> i32 { let Pair { a: x, b: x } = Pair { a: 1, b: 2 }; x } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_duplicate_field() {
    // Stock Rust rejects this fixture with E0025.
    rejects(
        "struct_duplicate_field",
        r#"struct Item { n: i32 } fn run() -> i32 { let Item { n: a, n: b } = Item { n: 1 }; a + b } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_tuple_pattern_shape_mismatch() {
    // Stock Rust rejects this fixture with E0308.
    rejects(
        "tuple_pattern_shape_mismatch",
        r#"fn run() -> i32 { let (a, b) = (1, 2, 3); a + b } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_tuple_pattern_on_scalar() {
    // Stock Rust rejects this fixture with E0308.
    rejects(
        "tuple_pattern_on_scalar",
        r#"fn run() -> i32 { let (_, _) = 3; 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_pattern_nominal_type_mismatch() {
    // Stock Rust rejects this fixture with E0308.
    rejects(
        "struct_pattern_nominal_type_mismatch",
        r#"struct A { n: i32 } struct B { n: i32 } fn run() -> i32 { let A { n } = B { n: 1 }; n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_pattern_missing_field() {
    // Stock Rust rejects this fixture with E0027.
    rejects(
        "struct_pattern_missing_field",
        r#"struct Pair { a: i32, b: i32 } fn run() -> i32 { let Pair { a } = Pair { a: 1, b: 2 }; a } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_pattern_unknown_field() {
    // Stock Rust rejects this fixture with E0026.
    rejects(
        "struct_pattern_unknown_field",
        r#"struct Item { n: i32 } fn run() -> i32 { let Item { n, nope: _ } = Item { n: 1 }; n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_pattern_private_field_even_wildcard() {
    // Stock Rust rejects this fixture with E0451.
    rejects(
        "struct_pattern_private_field_even_wildcard",
        r#"mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 1 } } } fn run() -> i32 { let inner::Item { n: _ } = inner::make(); 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_immutable_pattern_binding_reassignment() {
    // Stock Rust rejects this fixture with E0384.
    rejects(
        "immutable_pattern_binding_reassignment",
        r#"fn run() -> i32 { let (a, mut b) = (1, 2); a = 3; b = 4; a + b } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_immutable_noncopy_binding_reassignment_after_move() {
    // Stock Rust rejects this fixture with E0384.
    rejects(
        "immutable_noncopy_binding_reassignment_after_move",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let (x, _) = (Item { n: 1 }, 0); let n = consume(x); x = Item { n: 2 }; n + consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_partial_pattern_move_invalidates_parent() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "partial_pattern_move_invalidates_parent",
        r#"struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let pair = (Item { n: 1 }, Item { n: 2 }); let (a, _) = pair; a.n + consume(pair) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_pattern_binding_reads_moved_field() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "pattern_binding_reads_moved_field",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 1 }, Item { n: 2 }); let first = pair.0; let (again, _) = pair; consume(first) + consume(again) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_underscore_named_binding_is_not_wildcard() {
    // Stock Rust rejects this fixture with E0382.
    rejects(
        "underscore_named_binding_is_not_wildcard",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let _ignored = x; consume(x) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_pattern_private_field_explicit_wildcard_with_rest() {
    // Stock Rust rejects this fixture with E0451.
    rejects(
        "struct_pattern_private_field_explicit_wildcard_with_rest",
        r#"mod inner { pub struct Item { pub n: i32, secret: i32 } pub fn make() -> Item { Item { n: 7, secret: 9 } } } fn run() -> i32 { let inner::Item { secret: _, .. } = inner::make(); 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_pattern_initializer_cannot_see_new_binding() {
    // Stock Rust rejects this fixture with E0425.
    rejects(
        "pattern_initializer_cannot_see_new_binding",
        r#"fn run() -> i32 { let (x, y) = (1, x); x + y } fn main() { run(); }"#,
    );
}

#[test]
fn accepts_positional_tuple_rest_pattern() {
    runs(
        r#"fn run() -> i32 { let (a, ..) = (1, 2, 3); a } fn main() { run(); }"#,
        "",
    );
}

#[test]
fn rejects_legal_unsupported_at_binding_pattern() {
    // Legal stock Rust, deliberately outside this let-pattern milestone.
    rejects(
        "at_binding_pattern",
        r#"fn run() -> i32 { let whole @ (a, b) = (1, 2); whole.0 + whole.1 + a + b } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_legal_unsupported_reference_binding_pattern() {
    // Legal stock Rust, deliberately outside this let-pattern milestone.
    rejects(
        "reference_binding_pattern",
        r#"fn run() -> i32 { let (ref a, _) = (1, 2); *a } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_legal_unsupported_function_parameter_pattern() {
    // Legal stock Rust, deliberately outside this let-pattern milestone.
    rejects(
        "function_parameter_pattern",
        r#"fn add((a, b): (i32, i32)) -> i32 { a + b } fn run() -> i32 { add((1, 2)) } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_legal_unsupported_destructuring_assignment() {
    // Legal stock Rust, deliberately outside this let-pattern milestone.
    rejects(
        "destructuring_assignment",
        r#"fn run() -> i32 { let (mut a, mut b) = (1, 2); (a, b) = (3, 4); a + b } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_legal_unsupported_let_else_pattern() {
    // Legal stock Rust, deliberately outside this let-pattern milestone.
    rejects(
        "let_else_pattern",
        r#"fn run() -> i32 { let (a, b) = (1, 2) else { return 0; }; a + b } fn main() { run(); }"#,
    );
}

#[test]
fn accepts_parenthesized_wildcard_source_does_not_read_moved_value() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = (x); n }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn accepts_self_shadowing_noncopy_pattern_uses_old_rhs() {
    runs(
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let s = (Item { n: 7 }, 0); let (s, _) = s; consume(s) }
fn main() { if run() == 7 { println!("pattern oracle ok"); } }"#,
        "pattern oracle ok\n",
    );
}

#[test]
fn rejects_invalid_block_snapshot_wildcard_still_reads_moved_source() {
    // Stock Rust rejects this value-producing snapshot with E0382.
    rejects(
        "block_snapshot_wildcard_still_reads_moved_source",
        r#"struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = { x }; n } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_rest_only_trailing_comma() {
    // Stock parser rejects a comma after the final struct-pattern rest token.
    rejects(
        "struct_rest_only_trailing_comma",
        r#"struct S { x: i32 } fn run() -> i32 { let s = S { x: 1 }; let S { .., } = s; 0 } fn main() { run(); }"#,
    );
}

#[test]
fn rejects_invalid_struct_rest_after_field_trailing_comma() {
    // Stock parser rejects a comma after the final struct-pattern rest token.
    rejects(
        "struct_rest_after_field_trailing_comma",
        r#"struct S { x: i32 } fn run() -> i32 { let s = S { x: 1 }; let S { x, .., } = s; 0 } fn main() { run(); }"#,
    );
}
