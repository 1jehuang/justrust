//! Heap lane: raw pointers, allocator and owning heap types.
//! Fixtures under tests/fixtures/heap carry stock rustc 1.98 (edition 2024,
//! -C panic=abort) stdout; every reject_* fixture is rejected by stock.
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
            let path = base.join(format!("jr-heap-{}-{epoch}-{id}", std::process::id()));
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
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/heap");

fn fixture(name: &str) -> String {
    fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap()
}

fn runs(name: &str) {
    let source = fixture(&format!("{name}.rs"));
    let expected = fixture(&format!("{name}.stdout"));
    runs_source(name, &source, &expected);
}

fn runs_source(name: &str, source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        let checked = capture(&mut compiler(&input, &output, workers, true), &scratch, 60);
        assert!(checked.status.success(), "{name} check: {checked:?}");
        let built = capture(&mut compiler(&input, &output, workers, false), &scratch, 60);
        assert!(built.status.success(), "{name} build: {built:?}");
        let run = capture(&mut Command::new(&output), &scratch, 20);
        assert_eq!(run.status.code(), Some(0), "{name} run: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{name}");
    }
}

const HEAP_SOURCE: &str = include_str!("../libraries/prelude/native_heap.rs");

// Stock is an external oracle, not a compiler dependency. The exact same Rust
// heap policy runs with this direct ABI adapter instead of our backend opcode.
const STOCK_SYSCALL: &str = r#"
macro_rules! __justrust_linux_syscall6 {
    ($number:expr, $a:expr, $b:expr, $c:expr, $d:expr, $e:expr, $f:expr $(,)?) => {{
        let mut result: usize = $number;
        core::arch::asm!("syscall", inlateout("rax") result,
            in("rdi") $a, in("rsi") $b, in("rdx") $c,
            in("r10") $d, in("r8") $e, in("r9") $f,
            lateout("rcx") _, lateout("r11") _, options(nostack));
        result
    }};
}
"#;

fn heap_case(name: &str, body: &str) {
    let source = format!("mod heap {{\n{HEAP_SOURCE}\n}}\n{body}");
    runs_source(
        name,
        &format!("use std::native_heap as heap;\n{body}"),
        "ok\n",
    );
    let available = Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        assert!(
            std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none(),
            "stock rustc required for local heap differential gate"
        );
        eprintln!("{name}: stock oracle unavailable, native executable checks only");
        return;
    }
    for optimize in ["0", "2"] {
        let scratch = Scratch::new();
        let input = scratch.source(&format!("{STOCK_SYSCALL}\n{source}"));
        let image = scratch.path("stock");
        let compiled = capture(
            Command::new("rustc")
                .env_remove("RUSTC_BOOTSTRAP")
                .args(["--edition=2024", "-Cpanic=abort", "-Awarnings", "-C"])
                .arg(format!("opt-level={optimize}"))
                .arg(&input)
                .arg("-o")
                .arg(&image),
            &scratch,
            60,
        );
        assert!(compiled.status.success(), "{name} stock: {compiled:?}");
        let ran = capture(&mut Command::new(image), &scratch, 20);
        assert!(ran.status.success(), "{name} stock: {ran:?}");
        assert_eq!(ran.stdout, b"ok\n", "{name} stock: {ran:?}");
    }
}

#[test]
fn source_heap_allocate_write_read_free() {
    heap_case(
        "allocate_write_read_free",
        r#"
fn transfer(value: heap::Allocation) -> heap::Allocation { value }
fn main() {
    let allocation = transfer(heap::allocate(8192usize, 4096usize));
    let pointer = heap::pointer(&allocation);
    assert!(!heap::is_null::<u8>(pointer as *const u8));
    assert!(heap::len(&allocation) == 8192usize);
    assert!((pointer as usize & 4095usize) == 0usize);
    let second = heap::byte_add(pointer, 4096usize);
    let last = heap::byte_add(pointer, 8191usize);
    unsafe {
        assert!(*pointer == 0u8);
        assert!(*last == 0u8);
        *pointer = 17u8;
        *second = 29u8;
        *last = 43u8;
        assert!(*pointer == 17u8 && *second == 29u8 && *last == 43u8);
        let word = pointer as *mut u64;
        *word = 123456789u64;
        assert!(*word == 123456789u64);
    }
    assert!(heap::byte_sub(last, 8191usize) == pointer);
    assert!(heap::release(allocation));
    let mut resident = 0u8;
    let unmapped = unsafe {
        __justrust_linux_syscall6!(27usize, pointer as usize, 4096usize,
            &mut resident as *mut u8 as usize, 0usize, 0usize, 0usize)
    };
    assert!(unmapped == 18446744073709551604usize);
    println!("ok");
}
"#,
    );
}

#[test]
fn source_heap_scalar_memory_widths_preserve_neighbors() {
    heap_case(
        "scalar_memory_widths",
        r#"
fn main() {
    let allocation = heap::allocate(4096usize, 16usize);
    let pointer = heap::pointer(&allocation);
    assert!(!heap::is_null::<u8>(pointer as *const u8));
    unsafe {
        let mut i = 0usize;
        while i < 128usize {
            let slot = heap::byte_add(pointer, i);
            *slot = 91u8;
            i = i + 1usize;
        }
        let p8 = heap::byte_add(pointer, 1usize) as *mut i8;
        let p16 = heap::byte_add(pointer, 2usize) as *mut u16;
        let n16 = heap::byte_add(pointer, 4usize) as *mut i16;
        let p32 = heap::byte_add(pointer, 8usize) as *mut u32;
        let n32 = heap::byte_add(pointer, 12usize) as *mut i32;
        let ch = heap::byte_add(pointer, 32usize) as *mut char;
        let flag = heap::byte_add(pointer, 36usize) as *mut bool;
        let f32ptr = heap::byte_add(pointer, 40usize) as *mut f32;
        let f64ptr = heap::byte_add(pointer, 48usize) as *mut f64;
        let wide = heap::byte_add(pointer, 64usize) as *mut u128;
        *pointer = 17u8;
        *p8 = -3i8;
        *p16 = 65500u16;
        *n16 = -301i16;
        *p32 = 4000000000u32;
        *n32 = -40001i32;
        *ch = '猫';
        *flag = true;
        *f32ptr = 1.25f32;
        *f64ptr = -2.5f64;
        *wide = 123456789123456789123456789u128;
        assert!(*pointer == 17u8);
        assert!(*p8 == -3i8);
        assert!(*p16 == 65500u16);
        assert!(*n16 == -301i16);
        assert!(*p32 == 4000000000u32);
        assert!(*n32 == -40001i32);
        assert!(*ch == '猫');
        assert!(*flag);
        assert!(*f32ptr == 1.25f32);
        assert!(*f64ptr == -2.5f64);
        assert!(*wide == 123456789123456789123456789u128);
        assert!(*heap::byte_add(pointer, 6usize) == 91u8);
        assert!(*heap::byte_add(pointer, 37usize) == 91u8);
        assert!(*heap::byte_add(pointer, 44usize) == 91u8);
        assert!(*heap::byte_add(pointer, 80usize) == 91u8);
        let end8 = heap::byte_add(pointer, 4095usize);
        *end8 = 123u8;
        assert!(*end8 == 123u8);
        let end16 = heap::byte_add(pointer, 4094usize) as *mut i16;
        *end16 = -1321i16;
        assert!(*end16 == -1321i16);
        let end32 = heap::byte_add(pointer, 4092usize) as *mut u32;
        *end32 = 12345678u32;
        assert!(*end32 == 12345678u32);
    }
    assert!(heap::release(allocation));
    println!("ok");
}
"#,
    );
}

#[test]
fn source_heap_layout_and_kernel_failure_are_null() {
    heap_case(
        "layouts_and_errors",
        r#"
fn failed(size: usize, align: usize) {
    let allocation = heap::allocate(size, align);
    assert!(heap::is_null::<u8>(heap::pointer(&allocation) as *const u8));
    assert!(heap::len(&allocation) == 0usize);
    assert!(heap::release(allocation));
}

fn main() {
    failed(0usize, 8usize);
    failed(16usize, 0usize);
    failed(16usize, 3usize);
    failed(16usize, 8192usize);
    failed(18446744073709551615usize, 8usize);
    failed(9223372036854775807usize, 8usize);
    let error = unsafe { __justrust_linux_syscall6!(18446744073709551615usize,
        1usize, 2usize, 3usize, 4usize, 5usize, 6usize) };
    assert!(error == 18446744073709551578usize);
    println!("ok");
}
"#,
    );
}

#[test]
fn source_pointer_null_and_checked_byte_arithmetic() {
    heap_case(
        "pointer_arithmetic",
        r#"
fn main() {
    assert!(heap::is_null::<u64>(heap::null::<u64>() as *const u64));
    let mut values = [7u64, 11u64, 19u64];
    let base = &mut values as *mut [u64; 3] as *mut u8;
    let one = heap::byte_add(base, 8usize) as *mut u64;
    let two = heap::byte_add(base, 16usize) as *mut u64;
    unsafe {
        assert!(*one == 11u64 && *two == 19u64);
        *one = 41u64;
    }
    assert!(values[1] == 41u64);
    assert!(heap::byte_sub(two as *mut u8, 16usize) == base);
    assert!(heap::is_null::<u8>(heap::byte_add(18446744073709551615usize as *mut u8, 1usize) as *const u8));
    assert!(heap::is_null::<u8>(heap::byte_sub(1usize as *mut u8, 2usize) as *const u8));
    println!("ok");
}
"#,
    );
}

fn rejects(name: &str, needle: &str) {
    let source = fixture(&format!("{name}.rs"));
    rejects_source(name, &source, needle);
}

fn rejects_source(name: &str, source: &str, needle: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 60);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{name}: accepted");
            assert!(stderr.contains(needle), "{name}: {stderr}");
            assert!(!stderr.contains("panicked at"), "{name}: {stderr}");
            assert!(!output.exists(), "{name}: published executable");
        }
    }
}

#[test]
fn native_syscall_requires_unsafe_even_in_unused_function() {
    for source in [
        "fn main() { __justrust_linux_syscall6!(39usize,0usize,0usize,0usize,0usize,0usize,0usize); }",
        "fn unused() { __justrust_linux_syscall6!(39usize,0usize,0usize,0usize,0usize,0usize,0usize); } fn main() {}",
    ] {
        rejects_source("safe_syscall", source, "E0133");
    }
}

#[test]
fn native_syscall_rejects_wrong_arity_and_types() {
    for source in [
        "fn main() { unsafe { __justrust_linux_syscall6!(39usize,0usize); } }",
        "fn main() { unsafe { __justrust_linux_syscall6!(39usize,0usize,0usize,0usize,0usize,0usize,0usize,0usize); } }",
        "fn main() { unsafe { __justrust_linux_syscall6!(39usize,true,0usize,0usize,0usize,0usize,0usize); } }",
        "fn main() { unsafe { __justrust_linux_syscall6!(39usize,1u8,0usize,0usize,0usize,0usize,0usize); } }",
    ] {
        rejects_source("syscall_signature", source, "");
    }
}

#[test]
fn native_syscall_arguments_evaluate_once_in_order() {
    heap_case(
        "syscall_evaluation",
        r#"
fn next(calls: &mut usize, expected: usize, value: usize) -> usize {
    assert!(*calls == expected);
    *calls = *calls + 1usize;
    value
}
fn main() {
    let mut calls = 0usize;
    let result = unsafe {
        __justrust_linux_syscall6!(next(&mut calls, 0usize, 18446744073709551615usize),
            next(&mut calls, 1usize, 11usize), next(&mut calls, 2usize, 22usize),
            next(&mut calls, 3usize, 33usize), next(&mut calls, 4usize, 44usize),
            next(&mut calls, 5usize, 55usize), next(&mut calls, 6usize, 66usize))
    };
    assert!(calls == 7usize);
    assert!(result == 18446744073709551578usize);
    println!("ok");
}
"#,
    );
}

#[test]
fn source_heap_handle_cannot_be_forged_or_released_twice() {
    for body in [
        "fn main() { let allocation = heap::Allocation { address: 1usize as *mut u8, size: 4096usize }; heap::release(allocation); }",
        "fn main() { let allocation = heap::allocate(8usize, 8usize); heap::release(allocation); heap::release(allocation); }",
    ] {
        let source = format!("mod heap {{\n{HEAP_SOURCE}\n}}\n{body}");
        rejects_source("allocation_ownership", &source, "");
    }
}

#[test]
fn source_heap_private_guard_blocks_user_copy_impl() {
    let body = r#"
impl Copy for heap::Allocation {}
impl Clone for heap::Allocation {
    fn clone(&self) -> Self { *self }
}
fn main() {}
"#;
    let source = format!("mod heap {{\n{HEAP_SOURCE}\n}}\n{body}");
    rejects_source("allocation_copy", &source, "trait obligation");
    let available = Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|output| output.status.success());
    if available {
        let scratch = Scratch::new();
        let input = scratch.source(&format!("{STOCK_SYSCALL}\n{source}"));
        let result = capture(
            Command::new("rustc")
                .args(["--edition=2024", "-Cpanic=abort"])
                .arg(input)
                .arg("-o")
                .arg(scratch.path("must-not-exist")),
            &scratch,
            60,
        );
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("E0204"),
            "{result:?}"
        );
    } else {
        assert!(
            std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none(),
            "stock oracle required"
        );
    }
}

#[test]
fn source_scalar_copy_impl_remains_legal() {
    heap_case(
        "legitimate_copy_control",
        r#"
struct Plain { address: *mut u8, size: usize }
impl Copy for Plain {}
impl Clone for Plain { fn clone(&self) -> Self { *self } }
fn size(value: Plain) -> usize { value.size }
fn main() {
    let value = Plain { address: 0usize as *mut u8, size: 17usize };
    assert!(size(value) == 17usize);
    assert!(size(value) == 17usize);
    let clone = value.clone();
    assert!(clone.address == value.address);
    println!("ok");
}
"#,
    );
}

#[test]
fn raw_pointer_reads_writes_casts_and_equality() {
    runs("raw_read_write");
}

#[test]
fn raw_pointer_deref_requires_unsafe() {
    rejects("reject_deref_outside_unsafe", "E0133");
}

#[test]
fn raw_pointer_deref_requires_unsafe_in_unused_function() {
    rejects("reject_unused_deref", "E0133");
}

#[test]
fn raw_pointer_const_is_not_writable() {
    rejects("reject_write_const", "*const");
}

#[test]
fn shared_reference_does_not_cast_to_mut_pointer() {
    rejects("reject_shared_to_mut", "E0606");
}
