//! Frozen stock-first projected-method witnesses for PROJECTED-METHODS.md.
//! Stock JSON SHA256 49a8acf730f731caafc3956e95a9e3f54fac3810cdd7bc0d0b7298a688a22436.
//! Ordered source SHA256 56e51e20b9d6617fc35443a708d02b256ffe58f7b771fa2a88636258db03d785.
//! All 18 original sources retain their exact bytes/order, including final LF.
//! Cases 19-24 are separately stock-measured positive-check equivalents of 01-06.
//! Equivalent measurement: projected-method-stock/equivalents-20261017T2343Z/results.json
//! Equivalent JSON SHA256 5fa25c25d1b4c4333fff32f657563104cb72d014a18db9c3a0b14ba33ed4455a.
//! Equivalent ordered source SHA256 0370b25e9ed3e381e2de7fdfee844deba9429b1a55b6fdbbcc832743ac7504a5.
//! Native outcomes are acceptance expectations, not predeclared measured results.
//! Originals 01-06 expect Runtime with exact ok+LF now that literal panic! is admitted.
//! Gate 888141ae5r exposes original 07 at the rigid generic projection boundary.
//! Cases 12/13 retain earlier Self where syntax admission boundaries.
//! Unsupported below is a corpus category, not necessarily the diagnostic error kind.
//! Cases 08/09 retain rigid projection boundaries, 10/11 earlier short-projection syntax.
//! Cases 17/18 stop at nominal lifetime syntax, before projected-reference semantics.
//! Cases 08/09/18 are stock-invalid Unsupported boundaries, not ordinary-invalid parity.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler::{self, frontend, ir};
use justrust_engine::{Key, Report};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
const RIGID_BOUNDARY: &str = "unsupported: rigid generic associated projection";
const PARSE_PROJECTION_BOUNDARY: &str =
    "unsupported: associated type projections are unsupported, found \"::\"";
const SELF_WHERE_BOUNDARY: &str = "Self where predicates are unsupported";
const LIFETIME_BOUNDARY: &str =
    "lifetime parameters on nominal and impl declarations are unsupported";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
    InvalidUnsupported,
}

struct Case {
    name: &'static str,
    source: &'static str,
    source_bytes: usize,
    stock_diagnostic: Option<&'static str>,
    outcome: Outcome,
    native_boundary: Option<&'static str>,
}

fn cases() -> Vec<Case> {
    vec![
        // Source SHA256 9f954fc4f36be79a3fa21cf55e1ba5faf9a2cddf242dc741b17bf266a9144970
        Case {
            name: "01_associated_only",
            source: r#"trait Source { type Out; }
struct S;
impl Source for S { type Out = i32; }
fn main() { let x: <S as Source>::Out = 42; if x != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 162,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 6b4779a47886f525bfb33112e5689f1109d7a7cc90908b34f1f2b35e9bb23d41
        Case {
            name: "02_closed_required_get",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 42 } }
fn main() { let x: <S as Source>::Out = S.get(); if x != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 227,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 adf27966476de500243062396da07e0ee2346f16394f5d7bba0da700522320c1
        Case {
            name: "03_same_self_trait_args",
            source: r#"trait Source<A> { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get(self) -> Self::Out { 123 } }
impl Source<bool> for S { type Out = bool; fn get(self) -> Self::Out { true } }
fn main() { let a: <S as Source<i32>>::Out = <S as Source<i32>>::get(S); let b: <S as Source<bool>>::Out = <S as Source<bool>>::get(S); if a != 123 || !b { panic!(); } println!("ok"); }
"#,
            source_bytes: 411,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 182b63ec1aac669658995121e8b793c05d6cb3217c6a1e86aee052a861dd0da5
        Case {
            name: "04_noncopy_cell_move",
            source: r#"struct Cell { value: i32 }
trait Source { type Out; fn get(self) -> Self::Out; }
struct S(Cell);
impl Source for S { type Out = Cell; fn get(self) -> Self::Out { self.0 } }
fn consume(x: Cell) -> i32 { x.value }
fn main() { let x: <S as Source>::Out = S(Cell { value: 42 }).get(); let moved = x; if consume(moved) != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 352,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 6b59a59b88970b96366e53282913d1c7d50c9b91f73becc8a879ab7096bcd52b
        Case {
            name: "05_tuple_i128",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = (i128, bool); fn get(self) -> Self::Out { (170141183460469231731687303715884105727, true) } }
fn main() { let x: <S as Source>::Out = S.get(); if x.0 != 170141183460469231731687303715884105727 || !x.1 { panic!(); } println!("ok"); }
"#,
            source_bytes: 328,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 0300cdde291a9a8e5ff8321fda8f6a4bf85a71538d215fa3be7f26ef9f54c0c4
        Case {
            name: "06_generic_impl_wrap",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> Self::Out { self.0 } }
fn main() { let a = Wrap(42i32).get(); let b = Wrap(true).get(); if a != 42 || !b { panic!(); } println!("ok"); }
"#,
            source_bytes: 269,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Source SHA256 ab87023553b8fd6dfbef14d702725c4862f9c9145de75b76ae5d7bc287920a58
        Case {
            name: "07_generic_forward",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> Self::Out { self.0 } }
fn forward<T: Source>(x: T) -> <T as Source>::Out { x.get() }
fn main() { let a = forward(Wrap(42i32)); let b = forward(Wrap(true)); if a != 42 || !b { panic!(); } println!("ok"); }
"#,
            source_bytes: 337,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(RIGID_BOUNDARY),
        },
        // Source SHA256 f4cd7e597fcfc5509d48b741456e55103634bf1b17ed00ce629904409255df2f
        Case {
            name: "08_generic_bad_return42",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
fn bad<T: Source>(_x: T) -> <T as Source>::Out { 42 }
fn main() {}
"#,
            source_bytes: 121,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::InvalidUnsupported,
            native_boundary: Some(RIGID_BOUNDARY),
        },
        // Source SHA256 76d3f2c568337ff4737c34d08650e0080a551261f125b867b89900cd47ea0ab9
        Case {
            name: "09_generic_duplicate_without_copy",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
fn bad<T: Source>(x: T) -> (<T as Source>::Out, <T as Source>::Out) { let y = x.get(); (y, y) }
fn main() {}
"#,
            source_bytes: 163,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::InvalidUnsupported,
            native_boundary: Some(RIGID_BOUNDARY),
        },
        // Source SHA256 9903fa8e2d500885bf5e7b5f7c33d9f705610ddab2266a16799e30cdda2bc309
        Case {
            name: "10_explicit_out_copy",
            source: r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 42 } }
fn duplicate<T: Source>(x: T) -> (T::Out, T::Out) { let y = x.get(); (y, y) }
fn main() { let x = duplicate(S); if x.0 != 42 || x.1 != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 311,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(PARSE_PROJECTION_BOUNDARY),
        },
        // Source SHA256 7028424f9528fb4af4aca48bd6d9f01eef061adf0c5f644d099a6df3b2d56731
        Case {
            name: "11_explicit_out_sized",
            source: r#"trait Source { type Out: Sized; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 42 } }
fn forward<T: Source>(x: T) -> T::Out { x.get() }
fn main() { if forward(S) != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 256,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(PARSE_PROJECTION_BOUNDARY),
        },
        // Source SHA256 d64b7f5e8b9207e57a2bb6569de605f2628d6f18814caf74ac74861838f121d6
        Case {
            name: "12_projected_default_get",
            source: r#"trait Source { type Out; fn make(self) -> Self::Out; fn get(self) -> Self::Out where Self: Sized { self.make() } }
struct S;
impl Source for S { type Out = i32; fn make(self) -> Self::Out { 42 } }
fn main() { let x: <S as Source>::Out = S.get(); if x != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 289,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(SELF_WHERE_BOUNDARY),
        },
        // Source SHA256 69656a8d3296975e1deaab104e6dbc7b79f0c4fb9f0e138ff407de3dcbde47c0
        Case {
            name: "13_projected_default_override",
            source: r#"trait Source { type Out; fn make(self) -> Self::Out; fn get(self) -> Self::Out where Self: Sized { self.make() } }
struct S;
impl Source for S { type Out = i32; fn make(self) -> Self::Out { 7 } fn get(self) -> Self::Out { 42 } }
fn main() { let x: <S as Source>::Out = S.get(); if x != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 321,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(SELF_WHERE_BOUNDARY),
        },
        // Source SHA256 aa4f20274f2330dc143b2f96383a06a59b1c895a5623b7c1afa1e4fb8f9757c2
        Case {
            name: "14_wrong_impl_signature",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> bool { true } }
fn main() {}
"#,
            source_bytes: 145,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
            native_boundary: None,
        },
        // Source SHA256 aabd6ed189ceec5b727a15885065245aac1f4026f2b8b4c321eb3fd6d702dafd
        Case {
            name: "15_missing_required_method",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; }
fn main() {}
"#,
            source_bytes: 115,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
            native_boundary: None,
        },
        // Source SHA256 dba3ca4fed36068099496fe959956ee63ff4e792e2e6fd9295ffb36f06d725f6
        Case {
            name: "16_unused_bad_impl_body",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { true } }
fn main() {}
"#,
            source_bytes: 150,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
            native_boundary: None,
        },
        // Source SHA256 e925cececd18202de84128a1b7e609a31ea80b6f3193e846c8dd14830becc7ff
        Case {
            name: "17_reference_output",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S<'a>(&'a i32);
impl<'a> Source for S<'a> { type Out = &'a i32; fn get(self) -> Self::Out { self.0 } }
fn forward<T: Source>(x: T) -> T::Out { x.get() }
fn main() { let n = 42; let x = forward(S(&n)); if *x != 42 { panic!(); } println!("ok"); }
"#,
            source_bytes: 306,
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
            native_boundary: Some(LIFETIME_BOUNDARY),
        },
        // Source SHA256 592111f492d34a9056accb716d1e27026d7fcd53bec0eb6688e2d85f011a33df
        Case {
            name: "18_reference_escape_invalid",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S<'a>(&'a i32);
impl<'a> Source for S<'a> { type Out = &'a i32; fn get(self) -> Self::Out { self.0 } }
fn bad() -> &'static i32 { let n = 42; S(&n).get() }
fn main() {}
"#,
            source_bytes: 230,
            stock_diagnostic: Some("error[E0515]"),
            outcome: Outcome::InvalidUnsupported,
            native_boundary: Some(LIFETIME_BOUNDARY),
        },
        // Equivalent of original 01, separately stock-measured.
        // Source SHA256 a0cf8b67aa2212a3392fb8dbf636e4f1c69be313574a6698f71856752729b7f9
        Case {
            name: "19_associated_only_positive",
            source: r#"trait Source { type Out; }
struct S;
impl Source for S { type Out = i32; }
fn main() { let x: <S as Source>::Out = 42; if x == 42 { println!("ok"); } }
"#,
            source_bytes: 152,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Equivalent of original 02, separately stock-measured.
        // Source SHA256 313853f004c0a4d598ec29690dc64eb7c85ebafbbb131cb7193da4cce788b458
        Case {
            name: "20_closed_required_get_positive",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 42 } }
fn main() { let x: <S as Source>::Out = S.get(); if x == 42 { println!("ok"); } }
"#,
            source_bytes: 217,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Equivalent of original 03, separately stock-measured.
        // Source SHA256 633493fc87f85b2121568087a8eb0bf9ecf782e9e71083d37c52433f6c6c2366
        Case {
            name: "21_same_self_trait_args_positive",
            source: r#"trait Source<A> { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get(self) -> Self::Out { 123 } }
impl Source<bool> for S { type Out = bool; fn get(self) -> Self::Out { true } }
fn main() { let a: <S as Source<i32>>::Out = <S as Source<i32>>::get(S); let b: <S as Source<bool>>::Out = <S as Source<bool>>::get(S); if a == 123 { if b { println!("ok"); } } }
"#,
            source_bytes: 404,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Equivalent of original 04, separately stock-measured.
        // Source SHA256 030f14dd3a7c67501bd8391078cc8ee8cdca82fdcf43d1596834a5275f725e28
        Case {
            name: "22_noncopy_cell_move_positive",
            source: r#"struct Cell { value: i32 }
trait Source { type Out; fn get(self) -> Self::Out; }
struct S(Cell);
impl Source for S { type Out = Cell; fn get(self) -> Self::Out { self.0 } }
fn consume(x: Cell) -> i32 { x.value }
fn main() { let x: <S as Source>::Out = S(Cell { value: 42 }).get(); let moved = x; if consume(moved) == 42 { println!("ok"); } }
"#,
            source_bytes: 342,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Equivalent of original 05, separately stock-measured.
        // Source SHA256 c6235160f3c32098438e2ad65d8f4d5b7471b81c5f6c64e26435b39ac0d93b6c
        Case {
            name: "23_tuple_i128_positive",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = (i128, bool); fn get(self) -> Self::Out { (170141183460469231731687303715884105727, true) } }
fn main() { let x: <S as Source>::Out = S.get(); if x.0 == 170141183460469231731687303715884105727 { if x.1 { println!("ok"); } } }
"#,
            source_bytes: 321,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
        // Equivalent of original 06, separately stock-measured.
        // Source SHA256 41672b5a3482d2550ed7fae7faf2a52b1854c040aea7cfc74e38ac54377240dc
        Case {
            name: "24_generic_impl_wrap_positive",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> Self::Out { self.0 } }
fn main() { let a = Wrap(42i32).get(); let b = Wrap(true).get(); if a == 42 { if b { println!("ok"); } } }
"#,
            source_bytes: 262,
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
            native_boundary: None,
        },
    ]
}

fn partition_index(outcome: Outcome) -> usize {
    match outcome {
        Outcome::Runtime => 0,
        Outcome::Invalid => 1,
        Outcome::Unsupported => 2,
        Outcome::InvalidUnsupported => 3,
    }
}

#[test]
fn projected_methods_stock_partition_and_frozen_source_metadata() {
    let cases = cases();
    assert_eq!(cases.len(), 24);
    let mut partition = [0; 4];
    let mut stock = [0; 2];
    for (index, case) in cases.iter().enumerate() {
        assert!(case.name.starts_with(&format!("{:02}_", index + 1)));
        assert_eq!(case.source.len(), case.source_bytes, "{}", case.name);
        assert!(case.source.ends_with('\n'), "{}", case.name);
        partition[partition_index(case.outcome)] += 1;
        stock[usize::from(case.stock_diagnostic.is_some())] += 1;
        assert_eq!(
            case.native_boundary.is_some(),
            matches!(
                case.outcome,
                Outcome::Unsupported | Outcome::InvalidUnsupported
            ),
            "{}",
            case.name
        );
        assert_eq!(
            case.stock_diagnostic.is_some(),
            matches!(case.outcome, Outcome::Invalid | Outcome::InvalidUnsupported),
            "{}",
            case.name
        );
    }
    assert_eq!(
        cases[..18]
            .iter()
            .map(|case| case.source.len())
            .sum::<usize>(),
        4493
    );
    assert_eq!(
        cases[18..]
            .iter()
            .map(|case| case.source.len())
            .sum::<usize>(),
        1698
    );
    assert_eq!(stock, [18, 6]);
    assert_eq!(partition, [12, 3, 6, 3]);
    for (index, case) in cases[..18].iter().enumerate() {
        let boundary = match index + 1 {
            1..=6 => None,
            12 | 13 => Some(SELF_WHERE_BOUNDARY),
            7..=9 => Some(RIGID_BOUNDARY),
            10 | 11 => Some(PARSE_PROJECTION_BOUNDARY),
            14..=16 => None,
            17 | 18 => Some(LIFETIME_BOUNDARY),
            _ => unreachable!(),
        };
        assert_eq!(case.native_boundary, boundary, "{}", case.name);
    }
}

#[test]
fn projected_methods_positive_equivalents_preserve_every_original_check() {
    let cases = cases();
    for (original, equivalent) in cases[..6].iter().zip(&cases[18..]) {
        let (prefix, tail) = original.source.split_once("if ").unwrap();
        let (negative_checks, _) = tail.split_once(" { panic!(); }").unwrap();
        let mut expected = prefix.to_owned();
        let mut checks = 0;
        for negative in negative_checks.split(" || ") {
            let positive = if let Some(boolean) = negative.strip_prefix('!') {
                boolean.to_owned()
            } else {
                let (value, expected_value) = negative.split_once(" != ").unwrap();
                format!("{value} == {expected_value}")
            };
            expected.push_str(&format!("if {positive} {{ "));
            checks += 1;
        }
        expected.push_str("println!(\"ok\"); ");
        for _ in 0..checks {
            expected.push_str("} ");
        }
        expected.push_str("}\n");
        assert_eq!(equivalent.source, expected, "{}", equivalent.name);
        assert!(!equivalent.source.contains("panic!"), "{}", equivalent.name);
        assert_eq!(equivalent.outcome, Outcome::Runtime);
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "jr-projected-methods-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let mut file = new_file(&self.path(name));
        file.write_all(bytes).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
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
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            // Retry only Linux pre-spawn ETXTBSY, never an executed child.
            Err(error) if error.raw_os_error() == Some(26) && retries < 20 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("spawn {command:?}: {error}"),
        }
    };
    let mut child = Running(child);
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "output bound: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    Captured {
        code: status.code().expect("signal termination"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn runtime(path: &Path, scratch: &Scratch) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, b"ok\n");
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}
fn assert_native_diagnostic(case: &Case, diagnostic: &str) {
    assert!(!diagnostic.is_empty(), "{}: missing diagnostic", case.name);
    assert!(
        !diagnostic.contains("panicked at"),
        "{}:{diagnostic}",
        case.name
    );
    if let Some(boundary) = case.native_boundary {
        assert!(
            diagnostic.contains(boundary),
            "{}: expected exact boundary {boundary:?}, got {diagnostic}",
            case.name
        );
        if case.name == "10_explicit_out_copy" {
            assert!(
                diagnostic.contains(&format!(":4:36: {PARSE_PROJECTION_BOUNDARY}")),
                "{} expected recorded short-projection parser location:{diagnostic}",
                case.name
            );
        }
    } else {
        assert_eq!(case.outcome, Outcome::Invalid);
        assert!(
            !diagnostic.contains("unsupported:"),
            "{} ordinary invalid must not become Unsupported:{diagnostic}",
            case.name
        );
        if case.name.starts_with("14_") {
            assert!(
                diagnostic
                    .contains("trait implementation signature does not match required declaration"),
                "{}:{diagnostic}",
                case.name
            );
        } else if case.name.starts_with("15_") {
            assert!(
                diagnostic.contains("projected owner is missing a required method"),
                "{}:{diagnostic}",
                case.name
            );
        }
    }
}
fn stock_all() {
    let scratch = Scratch::new();
    // Scratch measurements pin /usr/bin/rustc. Hosted gates use their actual toolchain.
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(
        Command::new(&rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .arg("-vV"),
        &scratch,
        10,
    );
    assert_eq!(version.code, 0, "{version:?}");
    eprintln!(
        "PROJECTED METHODS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = [0; 2];
    for case in cases() {
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=projected_methods_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
        if let Some(expected) = case.stock_diagnostic {
            counts[1] += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(diagnostic.contains(expected), "{}:{diagnostic}", case.name);
            assert!(!image.exists());
        } else {
            counts[0] += 1;
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, &scratch);
        }
    }
    assert_eq!(counts, [18, 6]);
    eprintln!(
        "STOCK PROJECTED METHODS:18 unchanged originals+6 independently measured equivalents,18 accepted with exact ok+LF,6 rejected"
    );
}

#[test]
fn stock_projected_methods_oracle_edition2024_actual_toolchain() {
    stock_all();
}

#[test]
fn native_projected_methods_workers1_8_check_build_no_compiler_fallback() {
    // --profile is telemetry, not optimization. Run this target under both
    // debug and release BOOTSTRAP compiler profiles for the acceptance gate.
    // Stock compilation is an independent oracle, never a native fallback.
    stock_all();
    let mut cells = 0;
    let mut modes = [[0; 2]; 4];
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.write("client.rs", case.source.as_bytes());
                let image = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                no_compiler_tools(&mut command, &scratch);
                command
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                assert_eq!(
                    result.code,
                    i32::from(case.outcome != Outcome::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.outcome != Outcome::Runtime {
                    assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
                    assert_native_diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
                }
                if !check && case.outcome == Outcome::Runtime {
                    runtime(&image, &scratch);
                    runtimes += 1;
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path}:{}",
                            case.name
                        );
                    }
                }
                modes[partition_index(case.outcome)][usize::from(!check)] += 1;
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 96);
    assert_eq!(modes, [[24, 24], [6, 6], [12, 12], [6, 6]]);
    assert_eq!(runtimes, 24);
    eprintln!(
        "NATIVE PROJECTED METHODS:96 CLI cells,modes{modes:?},24 exact-output native images from01-06/19-24;24 stock-valid Unsupported cells (4 rigid projection,8 Self-where,8 associated-bounds,4 nominal-lifetime) and12 stock-invalid Unsupported cells remain separate from12 ordinary-invalid cells;12/13 do not reach projected semantics"
    );
}

fn evaluated_once(report: &Report, kind: u32, index: usize, case: &Case) {
    let key = Key::new(kind, vec![index as u64]);
    let entries: Vec<_> = report
        .node_evaluations
        .iter()
        .filter(|(actual, _)| *actual == key)
        .collect();
    assert_eq!(entries.len(), 1, "{}: missing/duplicate {key:?}", case.name);
    assert_eq!(entries[0].1, 1, "{}: repeated {key:?}", case.name);
}
fn assert_successful_all_body_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = frontend::parse(case.source).expect("frozen Runtime source parses");
    frontend::check(&program).expect("all original bodies check independently of reachability");
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
        .expect("real source declarations and impl binders");
    let expected_source_bodies = match case.name {
        "01_associated_only" | "19_associated_only_positive" => 1,
        "02_closed_required_get"
        | "05_tuple_i128"
        | "06_generic_impl_wrap"
        | "20_closed_required_get_positive"
        | "23_tuple_i128_positive"
        | "24_generic_impl_wrap_positive" => 2,
        "03_same_self_trait_args"
        | "04_noncopy_cell_move"
        | "21_same_self_trait_args_positive"
        | "22_noncopy_cell_move_positive" => 3,
        _ => panic!("unexpected Runtime case {}", case.name),
    };
    assert_eq!(compiled.functions, expected_source_bodies, "{}", case.name);
    assert_eq!(compiled.source_bytes, case.source.len());
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.executable.is_none(), check);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert!(compiled.report.capture_complete);
    let mut source_bodies = 0;
    let mut generic_source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        evaluated_once(&compiled.report, 110, index, case);
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += 1;
                if plan.callable_env(declaration).unwrap().is_generic() {
                    generic_source_bodies += 1;
                    true
                } else {
                    false
                }
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => {
                panic!(
                    "{}: unavailable body entered executable inventory",
                    case.name
                )
            }
        };
        if universal {
            evaluated_once(&compiled.report, 111, index, case);
            evaluated_once(&compiled.report, 112, index, case);
        }
    }
    assert_eq!(source_bodies, expected_source_bodies, "{}", case.name);
    assert_eq!(
        generic_source_bodies,
        match case.name {
            "06_generic_impl_wrap" | "24_generic_impl_wrap_positive" => 1,
            _ => 0,
        },
        "{}: original impl binder must remain universally checked",
        case.name
    );
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        plan.callable_count(),
        "{}: every real source and recipe body checked",
        case.name
    );
    assert_eq!(
        compiled.control_flow.len(),
        expected_source_bodies
            + match case.name {
                "06_generic_impl_wrap" | "24_generic_impl_wrap_positive" => 1,
                _ => 0,
            },
        "{}: native inventory includes both distinct Wrap instantiations",
        case.name
    );
    assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    for kind in [102, 108, 109] {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(nodes.len(), compiled.control_flow.len(), "{}", case.name);
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 103)
            .count(),
        if check {
            0
        } else {
            compiled.control_flow.len()
        },
        "{}: no object generation in CHECK",
        case.name
    );
}
fn all_body_graph() {
    let mut cells = 0;
    let mut modes = [[0; 2]; 4];
    let mut unused_body_failures = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                match compiler::compile_observed(case.source.to_owned(), workers, check) {
                    Ok(compiled) => {
                        assert_eq!(case.outcome, Outcome::Runtime, "{}", case.name);
                        assert_successful_all_body_graph(&case, &compiled, check);
                    }
                    Err(failure) => {
                        assert_ne!(case.outcome, Outcome::Runtime, "{}:{failure}", case.name);
                        assert_native_diagnostic(&case, &failure.to_string());
                        let report = failure.report.expect("failure retains real graph report");
                        assert!(report.capture_complete, "{}", case.name);
                        assert_eq!(report.events_dropped, 0);
                        assert!(report.failures > 0, "{}", case.name);
                        assert!(
                            !report
                                .node_evaluations
                                .iter()
                                .any(|(key, _)| key.kind == 103),
                            "{}: failure must precede object generation",
                            case.name
                        );
                        if case.name == "16_unused_bad_impl_body" {
                            let program = frontend::parse(case.source).unwrap();
                            let plan = frontend::PlanCtx::from_program(
                                &program,
                                &ir::PlanBudget::default(),
                            )
                            .unwrap();
                            let mut source_bodies = 0;
                            let mut saw_get = false;
                            for index in 0..plan.callable_count() {
                                let declaration = plan.callable_decl(index).unwrap();
                                if let frontend::CallableProvider::Source { .. } =
                                    plan.callable_provider(declaration).unwrap()
                                {
                                    source_bodies += 1;
                                    evaluated_once(&report, 110, index, &case);
                                    let body = plan.callable_body(&program, declaration).unwrap();
                                    saw_get |= body.name.segments().last().is_some_and(|name| {
                                        program.names.spelling(*name).unwrap() == "get"
                                    });
                                }
                            }
                            assert_eq!(source_bodies, 2, "unchanged main and unused impl body");
                            assert!(saw_get, "unused required-method implementation was checked");
                            unused_body_failures += 1;
                        }
                    }
                }
                modes[partition_index(case.outcome)][usize::from(!check)] += 1;
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 96);
    assert_eq!(modes, [[24, 24], [6, 6], [12, 12], [6, 6]]);
    assert_eq!(unused_body_failures, 4);
    eprintln!(
        "NATIVE PROJECTED METHODS ALL-BODY GRAPH:96 cells,modes{modes:?},4 unused-impl rejection cells;01-06/19-24 cover successful bodies and generic Wrap<T>::get retains its original universal binder;07 retains the rigid generic projection boundary"
    );
}

#[test]
fn native_projected_methods_all_body_graph_workers1_8_check_build() {
    const CHILD: &str = "JR_PROJECTED_METHODS_ALL_BODY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        all_body_graph();
        return;
    }
    // Bound deadlock risk and poison external compiler discovery for the whole
    // graph child without changing the parent test process's environment.
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args([
            "--exact",
            "native_projected_methods_all_body_graph_workers1_8_check_build",
            "--nocapture",
        ])
        .env(CHILD, "1");
    let result = capture(&mut command, &scratch, 180);
    assert_eq!(result.code, 0, "all-body graph child:{result:?}");
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("NATIVE PROJECTED METHODS ALL-BODY GRAPH:96 cells"),
        "missing complete graph receipt:{result:?}"
    );
}
