//! Stock-first original module macro products and retained unsupported boundaries.
//! Supported generic functions and associated products exercise real calls,
//! paired with unused invalid-body controls rather than declaration admission.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact pre-native JSON SHA256, baseline11 then addendum13:
// 338914a5bf3fc526ce880f7efdb00ac4305398f0e0d64dd0ec1d3d227c77e165
// 1c68a1357468ef29d4f001b360a8f4eb9d5d2ef99102178ae4292add0492ba20
// Three appended original helper/alias/nominal resolution controls:
// 1ce75e4c9bb20edefdb98edf36c121af7d07b49f2c5948a2bc362d2e23325445
// Empty/unit structs are original syntax boundaries, not generated-impl guard proof.
// Fielded-struct twin independently reaches the output guard:
// 4e2806261da5413ca4f44f4f8eb585071b0f6732298d5174db6cafa5ba8cd726
// 80dc7178f5382769ec82f5706958a6e0fff81dbc60b43473d95f4090b805f4be
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Syntax,
    Semantic,
    Unsupported,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    outcome: Outcome,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "captured_function_name_literal_return",
            source: r#"macro_rules! make{($name:ident,$value:literal)=>{fn $name()->i32{$value}};}make!(answer,42);fn main(){if answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "duplicate_generated_function",
            source: r#"macro_rules! make{($name:ident,$value:literal)=>{fn $name()->i32{$value}};}make!(answer,42);make!(answer,7);fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("the name `answer` is defined multiple times"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "unused_generated_invalid_body",
            source: r#"macro_rules! make{($name:ident,$value:literal)=>{fn $name()->i32{$value}};}make!(unused,true);fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "item_invocation_before_definition",
            source: r#"make!(answer,42);macro_rules! make{($name:ident,$value:literal)=>{fn $name()->i32{$value}};}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `make` in this scope"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "parent_macro_before_inline_module_public_function",
            source: r#"macro_rules! make{($name:ident)=>{pub fn $name()->i32{42}};}mod api{make!(answer);}fn main(){if api::answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "parent_macro_after_inline_module_frontier",
            source: r#"mod api{make!(answer);}macro_rules! make{($name:ident)=>{pub fn $name()->i32{42}};}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `make` in this scope"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "inline_module_macro_shadow_restores_parent",
            source: r#"macro_rules! make{($name:ident)=>{pub fn $name()->i32{40}};}mod api{macro_rules! make{($name:ident)=>{pub fn $name()->i32{5}};}make!(answer);}make!(answer);fn main(){if answer()==40&&api::answer()==5{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_private_inline_function",
            source: r#"macro_rules! make{($name:ident)=>{fn $name()->i32{42}};}mod api{make!(answer);}fn main(){let result=api::answer();}"#,
            stdout: None,
            stock_diagnostic: Some("function `answer` is private"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "inline_module_local_macro_before_definition",
            source: r#"mod api{make!(answer);macro_rules! make{($name:ident)=>{pub fn $name()->i32{42}};}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `make` in this scope"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "generated_false_cfg_missing_module",
            source: r#"macro_rules! make{()=>{#[cfg(any())]mod missing;};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generated_macro_definition_future_boundary",
            source: r#"macro_rules! define{()=>{macro_rules! answer{()=>{42};}};}define!();fn main(){if answer!()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generated_body_local",
            source: r#"macro_rules! make{($n:ident)=>{fn $n()->i32{let x:i32=40;x+2}};}make!(answer);fn main(){if answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "captured_parameter_vs_marked_local",
            source: r#"macro_rules! make{($n:ident,$p:ident)=>{fn $n($p:i32)->i32{let x:i32=2;$p+x}};}make!(add,x);fn main(){if add(40)==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "template_free_name_cannot_capture_parameter",
            source: r#"macro_rules! make{($n:ident,$p:ident)=>{fn $n($p:i32)->i32{x}};}make!(read,x);fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find value `x` in this scope"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "literal_function_name",
            source: r#"macro_rules! make{()=>{fn answer()->i32{42}};}make!();fn main(){if answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "inactive_undefined_item_invocation",
            source: r#"#[cfg(any())]undefined!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_function_attribute_boundary",
            source: r#"macro_rules! make{()=>{#[inline]fn answer()->i32{42}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generated_module_boundary",
            source: r#"macro_rules! make{()=>{mod generated{}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generated_use_boundary",
            source: r#"macro_rules! make{()=>{use core::convert::identity;};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generated_trait_boundary",
            source: r#"macro_rules! make{()=>{trait Generated{}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_impl_boundary",
            source: r#"struct Marker;macro_rules! make{()=>{impl Marker{fn answer()->i32{42}}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "empty_item_output",
            source: r#"macro_rules! make{()=>{};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "multiple_function_output",
            source: r#"macro_rules! make{()=>{fn first()->i32{40}fn second()->i32{2}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_function_output_runtime",
            source: r#"macro_rules! make{()=>{fn identity<T>(x:T)->T{x}};}make!();fn main(){if identity(7i32)==7{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unused_generated_generic_invalid_body",
            source: r#"macro_rules! make{()=>{fn unused<T>(x:T)->i32{x}};}make!();fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "generated_body_original_helper",
            source: r#"fn helper(x:i32)->i32{x}macro_rules! make{()=>{fn answer()->i32{helper(42)}};}make!();fn main(){if answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_body_original_import_alias",
            source: r#"mod m{pub fn helper(x:i32)->i32{x}}use m::helper as h;macro_rules! make{()=>{fn answer()->i32{h(42)}};}make!();fn main(){if answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_signature_original_nominal_type",
            source: r#"struct Data{x:i32}macro_rules! make{()=>{fn answer()->Data{Data{x:42}}};}make!();fn main(){let value=answer();if value.x==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generated_impl_braces_struct_runtime",
            source: r#"struct Marker{}macro_rules! make{()=>{impl Marker{fn answer()->i32{42}}};}make!();fn main(){if Marker::answer()==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unused_generated_impl_braces_struct_invalid_body",
            source: r#"struct Marker{}macro_rules! make{()=>{impl Marker{fn unused()->u8{true}}};}make!();fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "generated_impl_fielded_struct_boundary",
            source: r#"struct Marker{x:i32}macro_rules! make{()=>{impl Marker{fn answer()->i32{42}}};}make!();fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "repeat_names_values",
            source: concat!(
                r#"macro_rules! make{($($n:ident=$v:literal),*)=>{$(fn $n()->i32{$v})*};}make!(first=40,second=2);fn main(){if first()+second()==42{println!("ok");}}"#,
                "\n"
            ),
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "empty_repeat",
            source: concat!(
                r#"macro_rules! make{($($n:ident),*)=>{$(fn $n()->i32{42})*};}make!();fn main(){println!("ok");}"#,
                "\n"
            ),
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "sibling_forward_call",
            source: concat!(
                r#"macro_rules! make{()=>{fn first()->i32{second()+40}fn second()->i32{2}};}make!();fn main(){if first()==42{println!("ok");}}"#,
                "\n"
            ),
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unused_bad_second",
            source: concat!(
                r#"macro_rules! make{()=>{fn good()->i32{42}fn unused()->i32{true}};}make!();fn main(){if good()==42{println!("ok");}}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "unused_bad_first",
            source: concat!(
                r#"macro_rules! make{()=>{fn unused()->i32{true}fn good()->i32{42}};}make!();fn main(){if good()==42{println!("ok");}}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "unused_bad_repeat",
            source: concat!(
                r#"macro_rules! make{($($n:ident=$v:literal),*)=>{$(fn $n()->i32{$v})*};}make!(good=42,unused=true);fn main(){if good()==42{println!("ok");}}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "duplicate_siblings",
            source: concat!(
                r#"macro_rules! make{()=>{fn answer()->i32{40}fn answer()->i32{2}};}make!();fn main(){}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("the name `answer` is defined multiple times"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "malformed_second",
            source: concat!(
                r#"macro_rules! make{()=>{fn good()->i32{42}fn broken()->i32{let =1;0}};}make!();fn main(){}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("expected pattern"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "trailing_garbage",
            source: concat!(
                r#"macro_rules! make{()=>{fn good()->i32{42}@};}make!();fn main(){}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("macro expansion ignores `@`"),
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "second_module_boundary",
            source: concat!(
                r#"macro_rules! make{()=>{fn good()->i32{42}mod nested{}};}make!();fn main(){println!("ok");}"#,
                "\n"
            ),
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "sibling_capture_hygiene",
            source: concat!(
                r#"macro_rules! make{($a:ident,$b:ident,$p:ident)=>{fn $a($p:i32)->i32{let x:i32=2;$p+x}fn $b($p:i32)->i32{let x:i32=3;$p+x}};}make!(first,second,x);fn main(){if first(38)+second(0)==43{println!("ok");}}"#,
                "\n"
            ),
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "second_private_module",
            source: concat!(
                r#"macro_rules! make{()=>{pub fn first()->i32{40}fn second()->i32{2}};}mod api{make!();}fn main(){let n=api::second();}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("function `second` is private"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "trailing_supported_punct",
            source: concat!(
                r#"macro_rules! make{()=>{fn good()->i32{42}+};}make!();fn main(){}"#,
                "\n"
            ),
            stdout: None,
            stock_diagnostic: Some("macro expansion ignores `+`"),
            outcome: Outcome::Syntax,
        },
    ]
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
            "jr-item-source-macros-{}-{nonce}-{}",
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
        drop(file);
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
fn runtime(path: &Path, expected: &str, scratch: &Scratch) {
    assert!(!expected.is_empty(), "runtime oracle cannot be empty");
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = stock_rustc::stock_rustc();
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch, 10);
    assert_eq!(version.code, 0);
    eprintln!(
        "ITEM SOURCE BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Syntax => partition.1 += 1,
            Outcome::Semantic => partition.2 += 1,
            Outcome::Unsupported => partition.3 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        assert!(!scratch.path("missing.rs").exists());
        assert!(!scratch.path("missing/mod.rs").exists());
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=item_source_macros_oracle",
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
        if let Some(expected) = case.stdout {
            counts.0 += 1;
            assert!(case.stock_diagnostic.is_none());
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            counts.1 += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains(case.stock_diagnostic.expect("measured invalid category")),
                "{}:{diagnostic}",
                case.name
            );
            assert!(!image.exists());
        }
    }
    assert_eq!(counts, (27, 17));
    assert_eq!(partition, (21, 5, 11, 7));
}
#[test]
fn stock_original_module_macro_item_oracle_edition2024() {
    stock_all();
}
#[test]
fn item_source_parser_categories_preserve_unsupported_and_macro_frontier_syntax() {
    let mut counts = (0, 0, 0);
    for case in cases() {
        if case.outcome == Outcome::Runtime || case.outcome == Outcome::Semantic {
            continue;
        }
        let prepared =
            source::prepare_text("items.rs", case.source, &source::SourceOptions::default())
                .unwrap_or_else(|e| panic!("{} preparation:{e}", case.name));
        assert_eq!(prepared.files, 1);
        assert_eq!(prepared.sources.files.len(), 1);
        assert_eq!(prepared.source_bytes, case.source.len());
        let error = match frontend::parse_prepared(&prepared) {
            Err(e) => e,
            Ok(_) => panic!("{} must reject before publication", case.name),
        };
        let expected = match case.outcome {
            Outcome::Syntax => {
                counts.0 += 1;
                source::SourceErrorKind::Syntax
            }
            Outcome::Unsupported => {
                counts.1 += 1;
                source::SourceErrorKind::Unsupported
            }
            _ => unreachable!(),
        };
        assert_eq!(error.kind, expected, "{}:{error}", case.name);
        if case.name == "generated_false_cfg_missing_module"
            || case.name == "generated_module_boundary"
        {
            // Public text ingress has no filesystem capability. This pins one retained
            // original snapshot and rejection without claiming private reader I/O counts.
            assert_eq!(prepared.sources.files[0].contents.as_ref(), case.source);
            counts.2 += 1;
        }
    }
    assert_eq!(counts, (5, 7, 2));
    let invalid = cases()
        .into_iter()
        .find(|c| c.name == "unused_generated_invalid_body")
        .unwrap();
    let prepared = source::prepare_text(
        "unused.rs",
        invalid.source,
        &source::SourceOptions::default(),
    )
    .unwrap();
    frontend::parse_prepared(&prepared).unwrap_or_else(|e| {
        panic!("unused invalid type body must parse before universal checking:{e}")
    });
}
#[test]
fn inactive_item_macro_needs_no_lookup_and_keeps_one_original_source() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "inactive_undefined_item_invocation")
        .unwrap();
    for workers in [1, 8] {
        let checked = compiler::compile_observed(case.source.to_owned(), workers, true)
            .unwrap_or_else(|e| panic!("inactive undefined macro:{e}"));
        assert_eq!(checked.source_files, 1);
        assert_eq!(checked.source_bytes, case.source.len());
        assert!(checked.executable.is_none());
    }
}
#[test]
fn native_item_source_macros_workers1_8_check_build_no_compiler_fallback() {
    stock_all();
    let mut gates = 0;
    let mut modes = (0, 0, 0, 0, 0, 0);
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.write("client.rs", case.source.as_bytes());
                let image = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !diagnostic.contains("panicked at"),
                    "{}:{diagnostic}",
                    case.name
                );
                assert_eq!(
                    result.code,
                    i32::from(case.outcome != Outcome::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.outcome != Outcome::Runtime {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    if case.name.starts_with("unused_bad_") {
                        assert!(
                            (diagnostic.contains("expected Int(I32), got Bool")
                                || diagnostic.contains("E0308: mismatched types: expected Fixed(Bool), found Fixed(Int(I32))"))
                                && ["ResourceLimit", "unsupported:", "contract:", "infrastructure:"]
                                    .iter()
                                    .all(|&boundary| !diagnostic.contains(boundary)),
                            "{} must reject the unused generated function semantically, not by a resource/contract boundary:{diagnostic}",
                            case.name
                        );
                    }
                    if matches!(case.name, "trailing_supported_punct" | "malformed_second") {
                        let location = if case.name == "trailing_supported_punct" {
                            ":1:42:"
                        } else {
                            ":1:63:"
                        };
                        assert!(diagnostic.contains(location), "{}:{diagnostic}", case.name);
                        assert!(
                            diagnostic.contains("expanded at file"),
                            "{}:{diagnostic}",
                            case.name
                        );
                        assert!(
                            diagnostic.contains("defined at file"),
                            "{}:{diagnostic}",
                            case.name
                        );
                    }
                    if case.outcome == Outcome::Unsupported {
                        assert!(
                            diagnostic.contains("unsupported:"),
                            "{} explicit boundary:{diagnostic}",
                            case.name
                        );
                    } else {
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                    }
                }
                if !check && case.outcome == Outcome::Runtime {
                    runtime(
                        &image,
                        case.stdout.expect("runtime source oracle"),
                        &scratch,
                    );
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path} {}",
                            case.name
                        );
                    }
                }
                match (case.outcome, check) {
                    (Outcome::Runtime, true) => modes.0 += 1,
                    (Outcome::Runtime, false) => modes.1 += 1,
                    (Outcome::Syntax | Outcome::Semantic, true) => modes.2 += 1,
                    (Outcome::Syntax | Outcome::Semantic, false) => modes.3 += 1,
                    (Outcome::Unsupported, true) => modes.4 += 1,
                    (Outcome::Unsupported, false) => modes.5 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 176);
    assert_eq!(modes, (42, 42, 32, 32, 14, 14));
    eprintln!(
        "NATIVE ITEM SOURCE:176 cells, modes{modes:?};42 executed native images;28 Unsupported cells are phase boundaries, not Rust-invalid parity"
    );
}

#[test]
fn generated_function_original_helper_alias_and_nominal_sources_parse_before_checking() {
    let mut count = 0;
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.name,
            "generated_body_original_helper"
                | "generated_body_original_import_alias"
                | "generated_signature_original_nominal_type"
        )
    }) {
        let prepared = source::prepare_text(
            "resolution.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{} prepare:{error}", case.name));
        frontend::parse_prepared(&prepared).unwrap_or_else(|error| {
            panic!(
                "{} existing helper/alias/nominal syntax must parse:{error}",
                case.name
            )
        });
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn zero_and_sibling_products_share_provenance_not_function_identity_or_source_copies() {
    for case in cases().into_iter().filter(|case| {
        matches!(
            case.name,
            "empty_repeat"
                | "repeat_names_values"
                | "sibling_forward_call"
                | "sibling_capture_hygiene"
        )
    }) {
        let prepared = source::prepare_text(
            "products.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        assert_eq!(prepared.files, 1);
        assert_eq!(prepared.sources.files.len(), 1);
        assert_eq!(prepared.sources.files[0].contents.as_ref(), case.source);
        let parsed = frontend::parse_prepared(&prepared)
            .unwrap_or_else(|error| panic!("{}:{error}", case.name));
        let program = &parsed.program;
        let generated: Vec<_> = program
            .functions
            .iter()
            .filter(|function| {
                let name = *function.name.segments().last().unwrap();
                let span = program.names.span(name).unwrap();
                program
                    .names
                    .session()
                    .span(span)
                    .unwrap()
                    .expansion
                    .is_some()
            })
            .collect();
        if case.name == "empty_repeat" {
            assert!(generated.is_empty());
        } else {
            assert_eq!(generated.len(), 2);
            let first = *generated[0].name.segments().last().unwrap();
            let second = *generated[1].name.segments().last().unwrap();
            assert_ne!(
                program.names.span(first).unwrap(),
                program.names.span(second).unwrap()
            );
            assert_ne!(generated[0].lexical_scope, generated[1].lexical_scope);
            let first_span = program
                .names
                .session()
                .span(program.names.span(first).unwrap())
                .unwrap();
            let second_span = program
                .names
                .session()
                .span(program.names.span(second).unwrap())
                .unwrap();
            assert_eq!(first_span.expansion, second_span.expansion);
        }
        assert_eq!(prepared.sources.files.len(), 1);
        assert_eq!(prepared.sources.files[0].contents.as_ref(), case.source);
    }
}

fn exported_original_cases() -> Vec<Case> {
    vec![
        Case {
            name: "private_module_public_root_mixed_site",
            source: r#"mod hidden { #[macro_export] macro_rules! exported { ($n:ident) => {{ let temp:i32=5; temp+$n }}; } } fn main(){let temp:i32=100;let input:i32=2;let value:i32=crate::exported!(input);if value==7&&temp==100{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "defining_crate_helper_not_caller_module",
            source: r#"pub fn helper()->i32{7} mod definitions { #[macro_export] macro_rules! exported {()=>{$crate::helper()};} } mod caller {fn helper()->i32{99} pub fn value()->i32{crate::exported!()}} fn main(){if caller::value()==7{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "textual_shadow_is_not_root_export_shadow",
            source: r#"mod definition {#[macro_export] macro_rules! exported {()=>{7};}} macro_rules! exported {()=>{99};} fn main(){if crate::exported!()==7&&exported!()==99{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unused_export_preserves_ordinary_import",
            source: r#"#[macro_export] macro_rules! exported {()=>{9};} mod values{pub fn helper()->i32{7}} use values::helper; fn main(){if helper()==7{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unused_export_preserves_ordinary_item_macro",
            source: r#"#[macro_export] macro_rules! exported {()=>{9};} macro_rules! make{()=>{fn helper()->i32{7}};} make!(); fn main(){if helper()==7{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "inactive_calls_and_imports_do_not_open_export_namespace",
            source: r#"#[macro_export] macro_rules! exported {()=>{7};} #[cfg(any())] missing!(); #[cfg(any())] use crate::exported as alias; fn main(){if crate::exported!()==7{println!("export7");}}"#,
            stdout: Some("export7\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "active_export_duplicate_unused",
            source: r#"mod a {#[macro_export] macro_rules! same {()=>{1};}} mod b {#[macro_export] macro_rules! same {()=>{2};}} fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("defined multiple times"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "export_cannot_capture_caller_local",
            source: r#"mod definition {#[macro_export] macro_rules! exported {()=>{hidden};}} fn main(){let hidden:i32=7;let value=crate::exported!();}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find value `hidden`"),
            outcome: Outcome::Semantic,
        },
        Case {
            name: "unused_invalid_item_not_hidden_by_export",
            source: r#"#[macro_export] macro_rules! exported {()=>{7};} fn unused()->i32{true} fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("mismatched types"),
            outcome: Outcome::Semantic,
        },
    ]
}

#[test]
fn exported_original_declarations_stock_native_check_build_workers1_8() {
    let rustc = stock_rustc::stock_rustc();
    for case in exported_original_cases() {
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock-export");
        let stock = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=export_original_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        if let Some(expected) = case.stdout {
            assert_eq!(stock.code, 0, "{}:{stock:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            assert_ne!(stock.code, 0, "{}:{stock:?}", case.name);
            assert!(
                String::from_utf8_lossy(&stock.stderr).contains(case.stock_diagnostic.unwrap()),
                "{}:{stock:?}",
                case.name
            );
        }
        for workers in [1, 8] {
            for check in [true, false] {
                let image = scratch.path(&format!("native-export-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
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
                    i32::from(case.stdout.is_none()),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if !check && let Some(expected) = case.stdout {
                    runtime(&image, expected, &scratch);
                } else {
                    assert!(!image.exists());
                }
            }
        }
    }
}

#[test]
fn export_namespace_check_uses_source_declarations_without_codegen() {
    let case = exported_original_cases().remove(0);
    for workers in [1, 8] {
        let checked = compiler::compile_observed(case.source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert_eq!(checked.report.events_dropped, 0);
        let source_nodes: std::collections::HashSet<_> = checked
            .report
            .node_evaluations
            .iter()
            .zip(&checked.report.node_ids)
            .filter_map(|((key, _), id)| (key.kind == 107).then_some(*id))
            .collect();
        let task_nodes: std::collections::HashMap<_, _> = checked
            .report
            .events
            .iter()
            .filter_map(|event| event.task.zip(event.node))
            .collect();
        for &(task, producer, _) in &checked.report.dependency_edges {
            if source_nodes.contains(&producer) {
                assert_ne!(
                    task_nodes.get(&task),
                    Some(&producer),
                    "SOURCE must not read its own published source output"
                );
            }
        }
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, count)| *count == 1 && !matches!(key.kind, 103 | 105 | 130..=133))
        );
    }
}

#[test]
fn exported_namespace_open_authority_is_unsupported_not_absent_workers1_8() {
    let rustc = stock_rustc::stock_rustc();
    for (name, text) in [
        (
            "active-item-may-add-duplicate",
            r#"#[macro_export] macro_rules! exported{()=>{7};} macro_rules! make{()=>{fn helper()->i32{7}};}make!();fn main(){let n:i32=crate::exported!();}"#,
        ),
        (
            "configured-root-reexport",
            r#"mod hidden{#[macro_export]macro_rules! exported{()=>{7};}}pub use crate::exported as alias;fn main(){let n:i32=crate::alias!();}"#,
        ),
        (
            "configured-import-does-not-prove-unique-original",
            r#"#[macro_export]macro_rules! exported{()=>{7};}mod values{pub fn helper()->i32{7}}use values::helper;fn main(){let n:i32=crate::exported!();}"#,
        ),
    ] {
        let scratch = Scratch::new();
        scratch.write("client.rs", text.as_bytes());
        let stock = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=export_open_oracle",
                    "--color=never",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(scratch.path("stock")),
            &scratch,
            20,
        );
        assert_eq!(stock.code, 0, "valid Rust frontier {name}: {stock:?}");
        for workers in [1, 8] {
            for check in [true, false] {
                let image = scratch.path(&format!("native-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                assert_eq!(
                    result.code, 1,
                    "{name} workers{workers} check{check}: {result:?}"
                );
                let message = String::from_utf8_lossy(&result.stderr);
                assert!(message.contains("unsupported:"), "{name}: {message}");
                assert!(message.contains("completeness"), "{name}: {message}");
                assert!(!message.contains("no exported macro"), "{name}: {message}");
                assert!(!message.contains("panicked at"), "{name}: {message}");
                assert!(
                    !image.exists(),
                    "failed completeness cannot publish an executable"
                );
            }
        }
    }
}
