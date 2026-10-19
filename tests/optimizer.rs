//! Graph-native optimizer (OPTIMIZATION.md): differential acceptance on real
//! programs. Each case is compiled twice through the public compiler, with
//! the optimizer off (stack emitter only) and on (SSA, LOCAL_OPT, SUMMARY,
//! IPO_OPT, register-allocating backend), and both executables must produce
//! identical exit status, stdout and stderr.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::Key;
use justrust_engine::compiler::{self, source::SourceOptions, ssa::OptMode};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn options(opt: OptMode) -> SourceOptions {
    SourceOptions {
        opt,
        ..SourceOptions::default()
    }
}

fn build(source: &str, opt: OptMode, workers: usize) -> compiler::Compilation {
    compiler::compile_observed_with_options(source.into(), workers, false, options(opt))
        .unwrap_or_else(|error| panic!("compile ({opt:?}) failed: {error}\n{source}"))
}

fn execute(bytes: &[u8]) -> Output {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join(format!(
        "jr-opt-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    drop(file);
    let output = Command::new(&path).output().unwrap();
    let _ = fs::remove_file(&path);
    output
}

/// Compile with the optimizer off and on, run both, compare everything.
/// Returns (off, on) compilations for further assertions.
fn differential(source: &str) -> (compiler::Compilation, compiler::Compilation, Output) {
    let off = build(source, OptMode::Off, 1);
    let on = build(source, OptMode::Adaptive, 1);
    let expected = execute(off.executable.as_ref().unwrap());
    let actual = execute(on.executable.as_ref().unwrap());
    assert_eq!(expected.status.code(), actual.status.code(), "{source}");
    assert_eq!(expected.stdout, actual.stdout, "{source}");
    assert_eq!(expected.stderr, actual.stderr, "{source}");
    // Determinism: a parallel build produces byte-identical output.
    let parallel = build(source, OptMode::Adaptive, 8);
    assert_eq!(
        parallel.executable, on.executable,
        "nondeterministic: {source}"
    );
    (off, on, actual)
}

const SCALAR_PROGRAM: &str = r#"
fn add(a: i32, b: i32) -> i32 { a + b }
fn square(x: i64) -> i64 { x * x }
fn sumsq(a: i64, b: i64) -> i64 { square(a) + square(b) }
fn clamp(x: i32, lo: i32, hi: i32) -> i32 { if x < lo { lo } else if x > hi { hi } else { x } }
fn sum_to(n: u32) -> u32 {
    let mut total = 0;
    let mut i = 0;
    while i < n { i += 1; total += i; }
    total
}
fn fib(n: u64) -> u64 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn collatz(mut n: u64) -> u32 {
    let mut steps = 0;
    while n != 1 { if n % 2 == 0 { n = n / 2; } else { n = 3 * n + 1; } steps += 1; }
    steps
}
fn mix(a: u8, b: i16) -> i64 { (a as i64) * 1000 + (b as i64) - ((a ^ 0x5a) as i64) }
fn shifts(x: u32, s: u32) -> u32 { (x << (s % 32)) | (x >> ((32 - s % 32) % 32)) }
fn main() {
    if add(2, 3) == 5 { println!("add ok") }
    if square(-12) == 144 { println!("square ok") }
    if sumsq(3, 4) == 25 { println!("sumsq ok") }
    if clamp(50, 0, 10) == 10 && clamp(-5, 0, 10) == 0 && clamp(7, 0, 10) == 7 { println!("clamp ok") }
    if sum_to(1000) == 500500 { println!("sum ok") }
    if fib(20) == 6765 { println!("fib ok") }
    if collatz(27) == 111 { println!("collatz ok") }
    if mix(200, -3) == 199997 - 146 { println!("mix ok") }
    if shifts(1, 33) == 2 { println!("shift ok") }
}
"#;

#[test]
fn optimized_scalar_program_matches_stack_emitter() {
    let (off, on, output) = differential(SCALAR_PROGRAM);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "add ok\nsquare ok\nsumsq ok\nclamp ok\nsum ok\nfib ok\ncollatz ok\nmix ok\nshift ok\n"
    );
    assert_eq!(off.optimizer.mode, OptMode::Off);
    assert_eq!(off.optimizer.optimized, 0, "off must not run the optimizer");
    // Every scalar helper takes the SSA path; main (println) falls back.
    // sumsq inlines both calls to the leaf `square`.
    assert!(on.optimizer.optimized >= 9, "{:?}", on.optimizer);
    assert!(
        on.optimizer.inlined >= 1,
        "small leaves inline: {:?}",
        on.optimizer
    );
}

#[test]
fn optimized_code_preserves_overflow_division_and_explicit_panics() {
    for (source, code) in [
        (
            "fn f(x: i32) -> i32 { x + 1 }\nfn main() { if f(2147483647) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: u8) -> u8 { x * 2 }\nfn main() { if f(200) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: i32, y: i32) -> i32 { x / y }\nfn main() { if f(1, 0) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: i64, y: i64) -> i64 { x % y }\nfn main() { if f(-9223372036854775807 - 1, -1) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: i8) -> i8 { -x }\nfn main() { if f(-128) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: u32, s: u32) -> u32 { x << s }\nfn main() { if f(1, 40) == 0 { println!(\"no\") } }",
            101,
        ),
        (
            "fn f(x: i32) -> i32 { if x > 3 { panic!(\"too big\") } x }\nfn main() { if f(9) == 0 { println!(\"no\") } }",
            134,
        ),
        (
            "fn f(x: u16) -> u16 { x - 1 }\nfn main() { if f(1) == 0 { println!(\"zero ok\") } }",
            0,
        ),
    ] {
        let (_, _, output) = differential(source);
        let status = output.status.code().unwrap_or(128 + 6);
        assert_eq!(status, code, "{source}\n{output:?}");
    }
}

#[test]
fn optimized_register_pressure_spills_and_calls_are_correct() {
    // More simultaneously live values than registers, live across calls.
    let source = r#"
fn id(x: i64) -> i64 { if x > 1000000 { x - 1 } else { x } }
fn many(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64,
        i: i64, j: i64, k: i64, l: i64, m: i64, n: i64) -> i64 {
    let p = a * 2 + id(b);
    let q = c * 3 + id(d);
    let r = e * 5 + id(f);
    let s = g * 7 + id(h);
    let t = i * 11 + id(j);
    let u = k * 13 + id(l);
    let v = m * 17 + id(n);
    p + q + r + s + t + u + v + a + b + c + d + e + f + g + h + i + j + k + l + m + n
}
fn main() {
    let expected = 2 + 2 + 9 + 4 + 25 + 6 + 49 + 8 + 99 + 10 + 143 + 12 + 221 + 14 + 105;
    if many(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14) == expected { println!("spill ok") }
}
"#;
    let (_, on, output) = differential(source);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "spill ok\n");
    assert!(on.optimizer.optimized >= 2);
}

#[test]
fn unsupported_functions_fall_back_without_changing_behavior() {
    // Structs, references and output are outside the current SSA subset.
    let source = r#"
struct P { x: i32, y: i32 }
fn len2(p: &P) -> i32 { p.x * p.x + p.y * p.y }
fn twice(x: i32) -> i32 { x * 2 }
fn main() { let p = P { x: 3, y: 4 }; if len2(&p) == twice(12) + 1 { println!("mixed ok") } }
"#;
    let (_, on, output) = differential(source);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "mixed ok\n");
    assert!(on.optimizer.optimized >= 1);
    assert!(!on.optimizer.fallback.is_empty());
}

#[test]
fn optimizer_nodes_are_per_function_graph_facts_and_off_demands_none() {
    let source = "fn leaf(x: i32) -> i32 { x + 1 }\nfn mid(x: i32) -> i32 { leaf(x) * 2 }\nfn main() { if mid(3) == 8 { println!(\"ok\") } }";
    let kinds = |opt: OptMode, check_only: bool| -> Vec<Key> {
        let compiled =
            compiler::compile_observed_with_options(source.into(), 1, check_only, options(opt))
                .unwrap();
        let mut keys: Vec<Key> = compiled
            .report
            .events
            .iter()
            .filter(|event| event.kind == "discovered")
            .filter_map(|event| {
                [130u32, 131, 132, 133].into_iter().find_map(|kind| {
                    (0..8u64)
                        .map(|index| Key::new(kind, vec![index]))
                        .find(|key| format!("{key:?}") == event.detail)
                })
            })
            .collect();
        keys.sort_by_key(|key| (key.kind, key.params.clone()));
        keys
    };
    assert!(
        kinds(OptMode::Off, false).is_empty(),
        "off demands no optimizer node"
    );
    assert!(
        kinds(OptMode::Adaptive, true).is_empty(),
        "check never demands codegen"
    );
    let on = kinds(OptMode::Adaptive, false);
    // Every optimizer key is parameterized by one instance index: there is no
    // whole-program optimizer node.
    assert!(on.iter().all(|key| key.params.len() == 1), "{on:?}");
    for kind in [130, 131, 133] {
        assert!(
            on.iter().any(|key| key.kind == kind),
            "missing kind {kind}: {on:?}"
        );
    }
    // The leaf's summary is requested because `mid` considered inlining it.
    assert!(on.iter().any(|key| key.kind == 132), "{on:?}");
}

/// Generated code quality versus the stack emitter on a compute-bound
/// program. Prints runtime and code size; asserts only the robust property
/// (optimized code is not larger). Runtime is reported, not asserted, because
/// shared CI machines are noisy.
#[test]
fn optimized_code_is_smaller_and_report_runtime() {
    let source = r#"
fn step(x: u64) -> u64 { if x % 2 == 0 { x / 2 } else { 3 * x + 1 } }
fn steps(start: u64) -> u64 {
    let mut n = start;
    let mut count = 0;
    while n != 1 { n = step(n); count += 1; }
    count
}
fn total(limit: u64) -> u64 {
    let mut i = 1;
    let mut sum = 0;
    while i < limit { sum += steps(i); i += 1; }
    sum
}
fn main() { if total(300000) == 35669673 { println!("ok") } }
"#;
    let time = |bytes: &[u8]| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let output = execute(bytes);
            best = best.min(start.elapsed());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), "ok\n");
        }
        best
    };
    let off = build(source, OptMode::Off, 1);
    let on = build(source, OptMode::Final, 1);
    let (off_bytes, on_bytes) = (
        off.executable.as_ref().unwrap(),
        on.executable.as_ref().unwrap(),
    );
    let (off_time, on_time) = (time(off_bytes), time(on_bytes));
    eprintln!(
        "optimizer: off {} bytes {:?}, on {} bytes {:?}, speedup {:.2}x, {:?}",
        off_bytes.len(),
        off_time,
        on_bytes.len(),
        on_time,
        off_time.as_secs_f64() / on_time.as_secs_f64(),
        on.optimizer,
    );
    assert!(on_bytes.len() <= off_bytes.len());
    assert_eq!(on.optimizer.inlined, 1, "steps inlines the leaf step");
}

/// Every `pass` program in the full-program corpus, whose stdout is pinned to
/// the stock rustc oracle, must still match that oracle with the optimizer on.
#[test]
fn full_program_corpus_matches_stock_oracle_with_optimizer() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/full_programs");
    let manifest = fs::read_to_string(dir.join("manifest.tsv")).unwrap();
    let mut checked = 0;
    let mut optimized = 0;
    for line in manifest.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if line.starts_with('#') || fields.len() < 2 || fields[1] != "pass" {
            continue;
        }
        let name = fields[0];
        let source = fs::read_to_string(dir.join(format!("{name}.rs"))).unwrap();
        let expected = fs::read(dir.join(format!("{name}.stdout"))).unwrap();
        let compiled = build(&source, OptMode::Final, 4);
        let output = execute(compiled.executable.as_ref().unwrap());
        assert!(output.status.success(), "{name}: {output:?}");
        assert_eq!(output.stdout, expected, "{name}");
        checked += 1;
        optimized += compiled.optimizer.optimized;
    }
    // FULL-PROGRAMS.md ratchet: 3 programs pass today (hello, collatz,
    // gcd_fib). This grows as the manifest flips entries to `pass`.
    assert!(checked >= 3, "corpus shrank: {checked}");
    eprintln!("corpus: {checked} programs, {optimized} functions optimized");
}
