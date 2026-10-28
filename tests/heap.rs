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
fn box_debug_display_forward_to_unsized_payload() {
    // Stock rustc 1.98 (edition 2024, panic=abort) stdout.
    runs_source(
        "box_debug_display",
        r#"#[derive(Debug)]
enum List { Cons(i32, Box<List>), Nil }
#[derive(Debug)]
struct N { v: i32, b: Option<Box<N>> }
fn main() {
    let b = Box::new(3);
    println!("{} {:?}", b, b);
    println!("{:?}", List::Cons(1, Box::new(List::Cons(2, Box::new(List::Nil)))));
    println!("{:?}", N { v: 1, b: Some(Box::new(N { v: 2, b: None })) });
}
"#,
        "3 3\nCons(1, Cons(2, Nil))\nN { v: 1, b: Some(N { v: 2, b: None }) }\n",
    );
}

#[test]
fn recursive_box_traversal_converges_without_losing_loans() {
    heap_case(
        "shared_loop_reborrow",
        r#"fn main() {
    let values = [2, 4, 6];
    let mut current = &values[0];
    let mut index = 0;
    while index < 3 {
        current = &values[index];
        index += 1;
    }
    assert!(*current == 6);
    println!("ok");
}"#,
    );
    heap_case(
        "recursive_box_traversal",
        r#"
struct Link { value: i32, next: Option<Box<Link>> }
struct Chain { head: Option<Box<Link>> }
impl Chain {
    fn new() -> Self { Chain { head: None } }
    fn push(&mut self, value: i32) {
        let next = self.head.take();
        self.head = Some(Box::new(Link { value, next }));
    }
    fn sum(&self) -> i32 {
        let mut cursor = &self.head;
        let mut total = 0;
        while let Some(node) = cursor {
            total += node.value;
            cursor = &node.next;
        }
        total
    }
}
fn main() {
    let mut list = Chain::new();
    for value in 0..64 { list.push(value); }
    assert!(list.sum() == 2016);
    println!("ok");
}
"#,
    );
}

#[test]
fn shared_reference_switch_selects_variant_across_reassigned_carrier() {
    heap_case(
        "shared_switch_reassigned",
        r#"
fn first_then_other(a: &Option<i32>, b: &Option<i32>) -> i32 {
    let mut c = a;
    let mut t = 0;
    if let Some(x) = c { t += *x; c = b; }
    if let Some(y) = c { t += *y; }
    t
}
fn walk<'a>(a: &'a Option<i32>, b: &'a Option<i32>, k: bool) -> i32 {
    let mut c = if k { a } else { b };
    let mut t = 0;
    while let Some(x) = c { t += *x; c = b; if t > 10 { break; } }
    t
}
fn main() {
    assert!(first_then_other(&Some(2), &Some(3)) == 5);
    assert!(first_then_other(&None, &Some(3)) == 0);
    assert!(walk(&Some(1), &Some(2), true) == 11);
    assert!(walk(&None, &Some(5), false) == 15);
    println!("ok");
}
"#,
    );
}

#[test]
fn generic_functions_drop_concrete_owning_roots() {
    heap_case("generic_owner", &fixture("generic_owner.rs"));
}

#[test]
fn owning_box_projection_moves_drop_remaining_fields_once() {
    heap_case(
        "box_projection_drop",
        r#"
struct Witness { count: *mut u64, digit: u64 }
impl Drop for Witness {
    fn drop(&mut self) {
        unsafe { *self.count = *self.count * 10u64 + self.digit; }
    }
}
struct Pair { first: Witness, second: Witness }
fn partial(count: *mut u64, take: bool) {
    let owner = Box::new(Pair {
        first: Witness { count: count, digit: 1u64 },
        second: Witness { count: count, digit: 2u64 },
    });
    if take { let moved = owner.first; }
}
fn main() {
    let mut count = 0u64;
    let pointer = &mut count as *mut u64;
    partial(pointer, true);
    assert!(count == 12u64);
    count = 0u64;
    partial(pointer, false);
    assert!(count == 12u64);
    count = 0u64;
    {
        let owner = Box::new(Pair {
            first: Witness { count: pointer, digit: 3u64 },
            second: Witness { count: pointer, digit: 4u64 },
        });
        let moved = *owner;
    }
    assert!(count == 34u64);
    count = 0u64;
    {
        let owner = Box::<Box<Pair>>::new(Box::<Pair>::new(Pair {
            first: Witness { count: pointer, digit: 5u64 },
            second: Witness { count: pointer, digit: 6u64 },
        }));
        let moved = owner.first;
    }
    assert!(count == 56u64);
    count = 0u64;
    {
        let mut owner = Box::new(Pair {
            first: Witness { count: pointer, digit: 1u64 },
            second: Witness { count: pointer, digit: 2u64 },
        });
        { let moved = owner.first; }
        assert!(count == 1u64);
        owner.first = Witness { count: pointer, digit: 3u64 };
    }
    assert!(count == 132u64);
    count = 0u64;
    {
        let owner = Box::new(Witness { count: pointer, digit: 7u64 });
        let moved = *owner;
    }
    assert!(count == 7u64);
    println!("ok");
}
"#,
    );
}

#[test]
fn owning_box_projection_cannot_move_fields_of_user_drop_type() {
    let source = r#"
struct Field { value: i32 }
struct Owner { field: Field }
impl Drop for Owner { fn drop(&mut self) {} }
fn main() {
    let owner = Box::new(Owner { field: Field { value: 1 } });
    let moved = owner.field;
}
"#;
    rejects_source("box_user_drop_partial_move", source, "E0509");
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let result = capture(
        Command::new("rustc")
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(input)
            .arg("-o")
            .arg(scratch.path("invalid-box-move")),
        &scratch,
        60,
    );
    assert!(!result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("E0509"));
}

#[test]
fn owning_box_projection_composes_alias_depth() {
    heap_case(
        "box_alias_depth",
        r#"
fn main() {
    let mut outer = Box::<Box<i32>>::new(Box::new(7i32));
    let inner = &mut *outer;
    let number = &mut **inner;
    *number += 1;
    assert!(*number == 8i32);
    println!("ok");
}
"#,
    );
}

#[test]
fn owning_box_projection_rejects_borrowed_shared_and_reused_moves() {
    for (name, source, stock_code) in [
        (
            "shared_loop_reborrow_cannot_outlive_local",
            "fn main() { let initial = 1; let mut selected = &initial; let mut i = 0; while i < 2 { let local = 2; selected = &local; i += 1; } println!(\"{}\", selected); }",
            "E0597",
        ),
        (
            "recursive_traversal_keeps_root_borrowed",
            "struct Link { next: Option<Box<Link>> } fn bad(root: &mut Option<Box<Link>>) { let mut cursor = &*root; while let Some(node) = cursor { *root = None; cursor = &node.next; } } fn main() {}",
            "E0506",
        ),
        (
            "borrowed_box_move",
            "struct Value { n: i32 } fn take(value: &Box<Value>) { let moved = **value; } fn main() {}",
            "E0507",
        ),
        (
            "rc_pointee_move",
            "use std::rc::Rc; struct Value { n: i32 } fn main() { let value = Rc::new(Value { n: 1 }); let moved = *value; }",
            "E0507",
        ),
        (
            "array_element_move",
            "struct Value { n: i32 } fn main() { let values = [Value { n: 1 }]; let moved = values[0]; }",
            "E0508",
        ),
        (
            "box_moved_field_reuse",
            "struct Value { n: i32 } struct Pair { first: Value, second: Value } fn main() { let value = Box::new(Pair { first: Value { n: 1 }, second: Value { n: 2 } }); let first = value.first; let again = value.first; }",
            "E0382",
        ),
        (
            "box_moved_owner_reuse",
            "struct Value { n: i32 } fn main() { let value = Box::new(Value { n: 1 }); let moved = *value; let again = *value; }",
            "E0382",
        ),
    ] {
        rejects_source(name, source, "");
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let result = capture(
            Command::new("rustc")
                .args(["--edition=2024", "-Cpanic=abort"])
                .arg(input)
                .arg("-o")
                .arg(scratch.path("invalid-move")),
            &scratch,
            60,
        );
        assert!(!result.status.success(), "{name}: {result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(stock_code),
            "{name}: {result:?}"
        );
    }
}

#[test]
fn generic_pointer_field_owners_drop_concrete_instances() {
    heap_case("generic_guard", &fixture("generic_guard.rs"));
}

#[test]
fn generic_owner_semantic_controls_reject() {
    let prefix = "struct Guard<T>{pointer:*mut T}impl<T> Drop for Guard<T>{fn drop(&mut self){}}";
    let stock = Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(stock || std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none());
    for (body, needle, stock_code) in [
        (
            "impl<T> Copy for Guard<T>{}impl<T> Clone for Guard<T>{fn clone(&self)->Self{*self}}fn main(){}",
            "Copy and Drop",
            "E0184",
        ),
        (
            "fn main(){let mut owner=Guard::<u8>{pointer:0usize as *mut u8};owner.drop();}",
            "explicit canonical destructor",
            "E0040",
        ),
        (
            "fn consume<T>(owner:Guard<T>){}fn main(){let owner=Guard::<u8>{pointer:0usize as *mut u8};consume::<u8>(owner);consume::<u8>(owner);}",
            "",
            "E0382",
        ),
        (
            "fn observe<T>(owner:&Guard<T>){}fn main(){let mut owner=Guard::<u8>{pointer:0usize as *mut u8};let loan=&owner;owner=Guard::<u8>{pointer:0usize as *mut u8};observe::<u8>(loan);}",
            "",
            "E0506",
        ),
    ] {
        let source = format!("{prefix}{body}");
        rejects_source("drop_borrow_escape", &source, needle);
        if stock {
            let scratch = Scratch::new();
            let input = scratch.source(&source);
            let result = capture(
                Command::new("rustc")
                    .args(["--edition=2024", "-Cpanic=abort"])
                    .arg(input)
                    .arg("-o")
                    .arg(scratch.path("invalid-generic-owner")),
                &scratch,
                60,
            );
            assert!(!result.status.success(), "{source}: {result:?}");
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(stock_code),
                "{source}: {result:?}"
            );
        }
    }
}

#[test]
fn generic_drop_specialization_and_extra_constraints_reject() {
    let stock = Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(stock || std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none());
    for (source, needle, stock_code) in [
        (
            "struct Guard<T>{pointer:*mut T}impl Drop for Guard<i32>{fn drop(&mut self){}}fn main(){}",
            "cannot specialize",
            "E0366",
        ),
        (
            "struct Guard<A,B>{a:*mut A,b:*mut B}impl<T> Drop for Guard<T,T>{fn drop(&mut self){}}fn main(){}",
            "cannot specialize",
            "E0366",
        ),
        (
            "struct Guard<T>{pointer:*mut T}impl<T:Copy> Drop for Guard<T>{fn drop(&mut self){}}fn main(){}",
            "unsupported: generic Drop with additional owner predicates",
            "E0367",
        ),
        (
            "struct Guard<T:Copy>{pointer:*mut T}impl<T:Copy> Drop for Guard<T>{fn drop(&mut self){}}fn main(){}",
            "generic Drop with additional owner predicates",
            "",
        ),
    ] {
        rejects_source("generic_drop_admission", source, needle);
        if stock {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let result = capture(
                Command::new("rustc")
                    .args(["--edition=2024", "-Cpanic=abort"])
                    .arg(input)
                    .arg("-o")
                    .arg(scratch.path("stock-generic-boundary")),
                &scratch,
                60,
            );
            assert_eq!(
                result.status.success(),
                stock_code.is_empty(),
                "{source}: {result:?}"
            );
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(stock_code),
                "{source}: {result:?}"
            );
        }
    }
}

#[test]
fn generic_functions_drop_real_allocations() {
    heap_case("generic_allocation", &fixture("generic_allocation.rs"));
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
            if name == "drop_borrow_escape" {
                assert!(!stderr.contains("unsupported"), "{name}: {stderr}");
            }
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
fn source_heap_canonical_drop_blocks_user_copy_impl() {
    let body = r#"
impl Copy for heap::Allocation {}
impl Clone for heap::Allocation {
    fn clone(&self) -> Self { *self }
}
fn main() {}
"#;
    let source = format!("mod heap {{\n{HEAP_SOURCE}\n}}\n{body}");
    rejects_source("allocation_copy", &source, "Copy and Drop");
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
            String::from_utf8_lossy(&result.stderr).contains("E0184"),
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
fn source_heap_automatic_drop_moves_and_early_returns() {
    heap_case(
        "automatic_drop_moves_and_early_returns",
        r#"
fn mapped(address: usize) -> bool {
    let mut status = 0u8;
    let output = &mut status as *mut u8 as usize;
    let result = unsafe { __justrust_linux_syscall6!(27usize, address, 4096usize, output, 0usize, 0usize, 0usize) };
    if result == 0usize { return true; }
    assert!(result == 18446744073709551604usize);
    false
}
fn identity(value: heap::Allocation) -> heap::Allocation { value }
fn early(address: &mut usize, take: bool) {
    let owner = heap::allocate(4096usize, 4096usize);
    *address = heap::pointer(&owner) as usize;
    assert!(mapped(*address));
    if take { return; }
    assert!(heap::len(&owner) == 4096usize);
}
fn main() {
    let address: usize;
    {
        let original = heap::allocate(4096usize, 4096usize);
        address = heap::pointer(&original) as usize;
        assert!(mapped(address));
        let moved = identity(original);
        assert!(mapped(address));
        let final_owner = moved;
        assert!(heap::len(&final_owner) == 4096usize);
    }
    assert!(!mapped(address));
    let mut early_address = 0usize;
    early(&mut early_address, true);
    assert!(!mapped(early_address));
    early(&mut early_address, false);
    assert!(!mapped(early_address));
    let explicit = heap::allocate(4096usize, 4096usize);
    let explicit_address = heap::pointer(&explicit) as usize;
    assert!(heap::release(explicit));
    assert!(!mapped(explicit_address));
    println!("ok");
}
"#,
    );
}

#[test]
fn source_drop_ordinary_borrows_and_copy_field_reads() {
    heap_case(
        "drop_owner_ordinary_borrows",
        r#"
struct Owner { value: i32 }
impl Drop for Owner { fn drop(&mut self) { assert!(self.value == 8); } }
fn inspect(owner: &Owner) -> i32 { owner.value }
fn mutate(owner: &mut Owner) { owner.value = owner.value + 1; }
fn main() {
    {
        let mut owner = Owner { value: 7 };
        assert!(inspect(&owner) == 7);
        mutate(&mut owner);
        let copy = owner.value;
        assert!(copy == 8);
        assert!(inspect(&owner) == 8);
    }
    println!("ok");
}
"#,
    );
}

#[test]
fn source_pointer_field_drop_flags_are_exactly_once() {
    heap_case(
        "pointer_field_drop_flags",
        r#"
struct Owner { count: *mut u64 }
impl Drop for Owner {
    fn drop(&mut self) {
        let pointer = self.count;
        unsafe { *pointer = *pointer + 1u64; }
    }
}
fn transfer(owner: Owner) -> Owner { owner }
fn conditional(pointer: *mut u64, take: bool) {
    let owner = Owner { count: pointer };
    if take { let moved = transfer(owner); }
}
fn main() {
    let mut count = 0u64;
    let pointer = &mut count as *mut u64;
    {
        let original = Owner { count: pointer };
        let moved = transfer(original);
        unsafe { assert!(*pointer == 0u64); }
    }
    assert!(count == 1u64);
    conditional(pointer, true);
    assert!(count == 2u64);
    conditional(pointer, false);
    assert!(count == 3u64);
    let mut step = 0i32;
    while step < 3 {
        let local = Owner { count: pointer };
        step = step + 1;
        if step == 2 { continue; }
    }
    assert!(count == 6u64);
    {
        let mut owner = Owner { count: pointer };
        owner = Owner { count: pointer };
        assert!(count == 7u64);
        owner = owner;
    }
    assert!(count == 8u64);
    println!("ok");
}
"#,
    );
}

#[test]
fn source_heap_abort_does_not_run_unwind_cleanup() {
    let body = r#"
struct Witness { value: i32 }
impl Drop for Witness { fn drop(&mut self) { println!("unexpected unwind cleanup"); } }
fn main() {
    let allocation = heap::allocate(4096usize, 4096usize);
    assert!(!heap::is_null::<u8>(heap::pointer(&allocation) as *const u8));
    let witness = Witness { value: 1 };
    panic!("heap abort control");
}
"#;
    for stock in [false, true] {
        if stock
            && !Command::new("rustc")
                .arg("-vV")
                .output()
                .is_ok_and(|r| r.status.success())
        {
            assert!(std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none());
            continue;
        }
        let scratch = Scratch::new();
        let source = if stock {
            format!("{STOCK_SYSCALL}\nmod heap {{\n{HEAP_SOURCE}\n}}\n{body}")
        } else {
            format!("use std::native_heap as heap;\n{body}")
        };
        let input = scratch.source(&source);
        let image = scratch.path("abort-control");
        let mut command = if stock {
            Command::new("rustc")
        } else {
            Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        };
        if stock {
            command.args(["--edition=2024", "-Cpanic=abort", "-Awarnings"]);
        }
        let compiled = capture(command.arg(input).arg("-o").arg(&image), &scratch, 60);
        assert!(compiled.status.success(), "stock={stock}: {compiled:?}");
        let result = capture(&mut Command::new(image), &scratch, 20);
        assert!(!result.status.success(), "stock={stock}: {result:?}");
        assert!(result.stdout.is_empty(), "stock={stock}: {result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("heap abort control"),
            "stock={stock}: {result:?}"
        );
    }
}

#[test]
fn source_drop_escaping_borrows_and_unsupported_aggregates_reject() {
    let stock = Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|r| r.status.success());
    assert!(stock || std::env::var_os("JUSTRUST_HEAP_STOCK_REQUIRED").is_none());
    for body in [
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn main(){ let escaped; { let owner=D{value:1}; escaped=&owner; } println!(\"{}\",escaped.value); }",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn main(){ let mut owner=D{value:1}; let loan=&owner; owner=D{value:2}; println!(\"{}\",loan.value); }",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn escape() -> &D { let owner=D{value:1}; &owner } fn main() {}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn consume(owner:D){} fn main(){let owner=D{value:1};let field=&owner.value;consume(owner);println!(\"{}\",*field);}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn mutate(owner:&mut D){owner.value=2;} fn main(){let mut owner=D{value:1};let field=&owner.value;mutate(&mut owner);println!(\"{}\",*field);}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn field(owner:&D)->&i32{&owner.value} fn main(){let escaped;{let owner=D{value:1};escaped=field(&owner);}println!(\"{}\",*escaped);}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn main(){let escaped;loop{let owner=D{value:1};escaped=&owner.value;break;}println!(\"{}\",*escaped);}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) {} } fn main(){let seed=0i32;let mut escaped=&seed;let mut step=0i32;while step<1{let owner=D{value:1};escaped=&owner.value;step=step+1;continue;}println!(\"{}\",*escaped);}",
        "fn consume<T>(value:T){}fn bad<T>(value:T){consume::<T>(value);consume::<T>(value);}fn main(){}",
        "fn observe<T>(value:&T){}fn bad<T>(left:T,right:T){let mut owner=left;let loan=&owner;owner=right;observe::<T>(loan);}fn main(){}",
        "struct D { value:i32 } impl Drop for D { fn drop(&mut self) { let escaped; { let value=3i32;escaped=&value; } self.value=*escaped; } } fn main(){}",
    ] {
        if body.contains("continue;") {
            // The SCC loan boundary is additive here. Require the actual
            // cleanup conflict too, not merely an unsupported diagnostic.
            rejects_source("drop_borrow_backedge", body, "BorrowConflict");
        } else {
            rejects_source("drop_borrow_escape", body, "");
        }
        if stock {
            let scratch = Scratch::new();
            let input = scratch.source(body);
            let result = capture(
                Command::new("rustc")
                    .args(["--edition=2024", "-Cpanic=abort"])
                    .arg(input)
                    .arg("-o")
                    .arg(scratch.path("invalid")),
                &scratch,
                60,
            );
            assert!(!result.status.success(), "{body}: {result:?}");
        }
    }
    rejects_source(
        "explicit_allocation_drop",
        "fn main(){let mut owner=std::native_heap::allocate(8usize,8usize);owner.drop();}",
        "explicit canonical destructor",
    );
    if stock {
        let scratch = Scratch::new();
        let input = scratch.source(&format!("{STOCK_SYSCALL}\nmod heap {{\n{HEAP_SOURCE}\n}}\nfn main(){{let mut owner=heap::allocate(8usize,8usize);owner.drop();}}"));
        let result = capture(
            Command::new("rustc")
                .args(["--edition=2024", "-Cpanic=abort"])
                .arg(input)
                .arg("-o")
                .arg(scratch.path("invalid-drop")),
            &scratch,
            60,
        );
        assert!(!result.status.success(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("E0040"),
            "{result:?}"
        );
    }
    let partial_move = "struct N { value:i32 } struct D { value:N } impl Drop for D { fn drop(&mut self){} } fn main(){let owner=D{value:N{value:1}};let moved=owner.value;}";
    rejects_source("drop_partial_move", partial_move, "E0509");
    if stock {
        let scratch = Scratch::new();
        let input = scratch.source(partial_move);
        let result = capture(
            Command::new("rustc")
                .args(["--edition=2024", "-Cpanic=abort"])
                .arg(input)
                .arg("-o")
                .arg(scratch.path("invalid-move")),
            &scratch,
            60,
        );
        assert!(!result.status.success(), "{result:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0509"));
    }
}

#[test]
fn source_drop_aggregate_positions_match_stock() {
    for (name, body) in [
        (
            "allocation_tuple_cleanup",
            "fn main(){let pair=(heap::allocate(8usize,8usize),1i32);println!(\"ok\");}",
        ),
        (
            "generic_allocation_tuple_cleanup",
            "fn wrap<T>(value:T){let pair=(value,1i32);}fn main(){wrap::<heap::Allocation>(heap::allocate(8usize,8usize));println!(\"ok\");}",
        ),
        (
            "generic_allocation_array_cleanup",
            "fn wrap<T>(value:T){let array=[value];}fn main(){wrap::<heap::Allocation>(heap::allocate(8usize,8usize));println!(\"ok\");}",
        ),
    ] {
        heap_case(name, body);
    }
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
