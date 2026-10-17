//! Executable stock oracle and source-envelope acceptance, not full Rust support.
//! Oracle first: every accepted source is run and every rejected source has an
//! independently pinned diagnostic. Legal unsupported/policy-boundary programs
//! remain distinct from invalid Rust. No rustc-private attributes or bootstrap.
//! Initial oracle freeze, 2026-10-17: rustc 1.98.1,
//! 48a229ceaefd4985c50990b14116b6d856af0985, x86_64-unknown-linux-gnu.
//! Authored fixtures use edition 2021, opt-level 0, overflow checks, panic abort.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const EXPECTED: &[u8] = b"configuration ok\n";
const COMPILE_TIMEOUT: Duration = Duration::from_secs(20);
const RUN_TIMEOUT: Duration = Duration::from_secs(5);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-source-configuration-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("scratch {}: {error}", path.display()),
            }
        }
        panic!("scratch allocation exhausted")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        new_file(&path).write_all(bytes).unwrap();
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
fn capture(command: &mut Command, scratch: &Scratch, timeout: Duration) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    let mut child = Running(
        command
            .stdin(Stdio::null())
            .stdout(new_file(&out))
            .stderr(new_file(&err))
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}")),
    );
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "output limit: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(start.elapsed() < timeout, "timeout: {command:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    Captured {
        code: status.code().expect("process terminated by signal"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Supported,
    Invalid,
    Unsupported,
    // Stock accepts with a warning, but the bounded metadata policy rejects.
    PolicyBoundary,
}
struct Fixture {
    name: &'static str,
    files: Vec<(&'static str, Vec<u8>)>,
    cfg: Vec<&'static str>,
    outcome: Outcome,
    stock_diagnostic: &'static str,
    // Original lexical path, Unicode scalar column, and CRLF-aware line.
    location: Option<&'static str>,
    // A missing-comma stock diagnostic can anchor the whitespace immediately
    // before the native offending token. Keep both precise anchors explicit.
    stock_location: Option<&'static str>,
}
fn case(name: &'static str, prefix: &str, outcome: Outcome, diagnostic: &'static str) -> Fixture {
    Fixture {
        name,
        files: vec![(
            "src/input.rs",
            format!("{prefix}\nfn main() {{ println!(\"configuration ok\"); }}\n").into_bytes(),
        )],
        cfg: vec![],
        outcome,
        stock_diagnostic: diagnostic,
        location: None,
        stock_location: None,
    }
}
impl Fixture {
    fn child(mut self, path: &'static str, bytes: impl AsRef<[u8]>) -> Self {
        self.files.push((path, bytes.as_ref().to_vec()));
        self
    }
    fn source(mut self, source: &str) -> Self {
        self.files[0].1 = source.as_bytes().to_vec();
        self
    }
    fn at(mut self, location: &'static str) -> Self {
        self.location = Some(location);
        self
    }
    fn stock_at(mut self, location: &'static str) -> Self {
        self.stock_location = Some(location);
        self
    }
    fn configured(mut self, cfg: &[&'static str]) -> Self {
        self.cfg = cfg.to_vec();
        self
    }
    fn materialize(&self) -> Scratch {
        let scratch = Scratch::new();
        for (path, bytes) in &self.files {
            scratch.write(path, bytes);
        }
        scratch
    }
}
fn fixtures() -> Vec<Fixture> {
    use Outcome::*;
    let mut cases = vec![
        case("disabled_missing_child", "#[cfg(any())] mod absent;", Supported, ""),
        case("disabled_ambiguous_child", "#[cfg(any())] mod child;", Supported, "")
            .child("src/child.rs", "fn broken( {")
            .child("src/child/mod.rs", "not valid Rust"),
        case("disabled_invalid_utf8_child", "#[cfg(any())] mod child;", Supported, "")
            .child("src/child.rs", [0xff, 0xfe]),
        case("disabled_invalid_body_child", "#[cfg(any())] mod child;", Supported, "")
            .child("src/child.rs", "fn unused() { missing(); }"),
        case("active_missing_child", "#[cfg(all())] mod absent;", Invalid, "file not found for module"),
        case("active_ambiguous_child", "#[cfg(all())] mod child;", Invalid, "found at both")
            .child("src/child.rs", "pub fn value() -> i32 { 1 }")
            .child("src/child/mod.rs", "pub fn value() -> i32 { 2 }"),
        case("active_invalid_unused_child", "#[cfg(all())] mod child;", Invalid, "cannot find function")
            .child("src/child.rs", "fn unused() { missing(); }"),
        case("child_inner_cfg_blocks_descendant", "mod child;", Supported, "")
            .child("src/child.rs", "#![cfg(any())]\nmod absent;\nfn unused(){ missing(); }"),
        case("inline_inner_cfg_blocks_descendant", "mod child { #![cfg(any())] mod absent; fn unused(){ missing(); } }", Supported, ""),
        case("nested_inactive_path", "#[cfg_attr(all(), cfg_attr(any(), path = \"absent.rs\"))] mod child;", Supported, "")
            .child("src/child.rs", "pub fn unused() {}"),
        case("inactive_path", "#[cfg_attr(any(), path = \"absent.rs\")] mod child;", Supported, "")
            .child("src/child.rs", "pub fn unused() {}"),
        case("active_path_boundary", "#[cfg_attr(all(), path = \"other.rs\")] mod child;", Unsupported, "")
            .child("src/other.rs", "pub fn unused() {}"),
        case("inactive_semantic", "#[cfg_attr(any(), track_caller)] fn unused() {}", Supported, ""),
        case("nested_inactive_semantic", "#[cfg_attr(all(), cfg_attr(any(), track_caller))] fn unused() {}", Supported, ""),
        case("inactive_unknown", "#[cfg_attr(any(), unknown_attr)] fn unused() {}", Supported, ""),
        case("inactive_malformed_inline", "#[cfg_attr(any(), inline(sometimes))] fn unused() {}", Supported, ""),
        case("inactive_malformed_predicate", "#[cfg_attr(any(), cfg(not()))] fn unused() {}", Supported, ""),
        case("inactive_unknown_token_tree", "#[cfg_attr(any(), unknown_attr(foo =))] fn unused() {}", Supported, ""),
        case("inactive_missing_meta_value", "#[cfg_attr(any(), doc =)] fn unused() {}", Invalid, "expected expression"),
        case("unknown_before_false_cfg", "#[unknown_attr] #[cfg(any())] fn unused() {}", Supported, ""),
        case("unknown_after_false_cfg", "#[cfg(any())] #[unknown_attr] fn unused() {}", Supported, ""),
        case("numeric_doc_before_false_cfg", "#[doc=3] #[cfg(any())] fn unused() {}", Supported, ""),
        case("numeric_doc_after_false_cfg", "#[cfg(any())] #[doc=3] fn unused() {}", Supported, ""),
        case("numeric_inline_before_false_cfg", "#[inline(3)] #[cfg(any())] fn unused() {}", Supported, ""),
        case("numeric_inline_after_false_cfg", "#[cfg(any())] #[inline(3)] fn unused() {}", Supported, ""),
        case("malformed_inline_before_false_cfg", "#[inline(sometimes)] #[cfg(any())] fn unused() {}", Supported, ""),
        case("malformed_inline_after_false_cfg", "#[cfg(any())] #[inline(sometimes)] fn unused() {}", Supported, ""),
        case("missing_doc_rhs_before_false_cfg", "#[doc=] #[cfg(any())] fn unused() {}", Invalid, "expected expression"),
        case("missing_doc_rhs_after_false_cfg", "#[cfg(any())] #[doc=] fn unused() {}", Invalid, "expected expression"),
        case("malformed_cfg_before_false", "#[cfg(not())] #[cfg(any())] fn unused() {}", Invalid, "malformed `cfg`"),
        case("malformed_cfg_after_false", "#[cfg(any())] #[cfg(not())] fn unused() {}", Supported, ""),
        case("bad_operator_before_false", "#[cfg(malformed(all()))] #[cfg(any())] fn unused() {}", Invalid, "malformed `cfg`"),
        case("bad_operator_after_false", "#[cfg(any())] #[cfg(malformed(all()))] fn unused() {}", Supported, ""),
        case("disabled_unknown_semantics", "#[cfg(any())] fn unused() { missing(); }", Supported, ""),
        case("disabled_unclosed_body", "#[cfg(any())] fn unused() {", Invalid, "unclosed delimiter"),
        case("disabled_bad_lexical_token", "#[cfg(any())] fn unused() { ` }", Invalid, "unknown start of token"),
        case("disabled_unterminated_string", "#[cfg(any())] fn unused() { \"", Invalid, "unterminated double quote string"),
        case("disabled_syntactically_invalid_body", "#[cfg(any())] fn unused() { let = ; }", Invalid, "expected pattern"),
        case("cfg_combinators", "#[cfg(all(all(), not(any()), any(any(), all())))] fn unused() {}", Supported, ""),
        case("cfg_unknown_name_closed", "#[cfg(unset_name)] mod absent;", Supported, ""),
        case("cfg_unknown_value_closed", "#[cfg(mode = \"unset\")] mod absent;", Supported, ""),
        case("cfg_bad_not_arity", "#[cfg(not(any(), all()))] fn unused() {}", Invalid, "malformed `cfg`"),
        case("cfg_bad_value", "#[cfg(mode = 42)] fn unused() {}", Invalid, "literal"),
        case("doc_and_inline", "/// Function docs\n#[doc = \"metadata\"] #[doc(hidden)] #[inline] fn helper() {}\n#[inline(always)] fn always() {}\n#[inline(never)] fn never() {}", Supported, ""),
        case("crate_and_module_docs", "//! Crate docs\n#![doc = \"crate\"]\nmod child { //! Child docs\n#![doc(hidden)]\n/// Item docs\npub fn helper() {} }", Supported, ""),
        case("malformed_inline", "#[inline(sometimes)] fn unused() {}", Invalid, "malformed `inline`"),
        case("inline_wrong_value", "#[inline = 42] fn unused() {}", Invalid, "valid forms"),
        case("inline_wrong_owner", "#[inline] struct Thing { value: i32 }", Invalid, "cannot be used on structs"),
        case("unknown_attribute", "#[unknown_attr] fn unused() {}", Invalid, "cannot find attribute"),
        case("doc_unknown_policy", "#[doc(unknown)] fn unused() {}", PolicyBoundary, "unknown `doc` attribute"),
        case("doc_numeric_policy", "#[doc = 42] fn unused() {}", PolicyBoundary, "malformed `doc`"),
        case("semantic_track_caller", "#[track_caller] fn unused() {}", Unsupported, ""),
        case("semantic_repr", "#[repr(C)] struct Thing { value: i32 }", Unsupported, ""),
        case("semantic_derive", "#[derive(Clone)] struct Thing { value: i32 }", Unsupported, ""),
        case("semantic_lint_control", "#[allow(dead_code)] fn unused() {}", Unsupported, ""),
        case("cfg_field_position", "struct Thing { #[cfg(all())] value: i32 }", Unsupported, ""),
        case("cfg_variant_position", "enum Thing { #[cfg(all())] Value }", Unsupported, ""),
        case("cfg_parameter_position", "fn unused(#[cfg(all())] value: i32) {}", Unsupported, ""),
        case("cfg_statement_position", "fn unused() { #[cfg(any())] missing(); }", Unsupported, ""),
        case("doc_detached", "", Invalid, "expected item")
            .source("fn main() {}\n#[doc = \"detached\"]"),
        case("inner_inline_wrong_owner", "#![inline]\nfn unused() {}", Invalid, "cannot be used on crates"),
        case("inert_comments_and_literals", "// mod absent; #[path = \"absent.rs\"]\n#[cfg(any())] fn unused() { let text = r###\"mod absent; } #[unknown_attr] #[path=\"absent.rs\"]\"###; }", Supported, ""),
        case("cfg_attr_generated_cfg", "#[cfg_attr(all(), cfg(any()), track_caller)] fn unused() { missing(); }", Supported, ""),
        case("free_item_cfg_runtime_selection", "", Supported, "")
            .source("#[cfg(any())] fn answer() -> i32 { missing() }\n#[cfg(all())] fn answer() -> i32 { 42 }\nfn main() { if answer() == 42 { println!(\"configuration ok\"); } }"),
        case("child_unicode_crlf_parse_span", "mod outer;", Invalid, "expected pattern")
            .child("src/outer.rs", "pub mod child;")
            .child("src/outer/child.rs", "// α🙂\r\npub fn broken() { /* α🙂 */ let = 1; }\r\n")
            .at("src/outer/child.rs:2:32"),
        case("directory_parse_span_after_removed_item", "#[cfg(any())] mod absent;\nmod outer;", Invalid, "expected pattern")
            .child("src/outer/mod.rs", "// metadata\r\n#[cfg(any())] fn removed(){missing();}\r\npub fn broken() { let = 1; }\r\n")
            .at("src/outer/mod.rs:3:23"),
        case("inline_child_parse_span", "mod outer { pub mod child; }", Invalid, "expected pattern")
            .child("src/outer/child.rs", "pub fn broken() { let = 1; }\n")
            .at("src/outer/child.rs:1:23"),
    ];
    // Explicit configuration snapshots exercise names, values and repeated keys.
    cases.push(
        case(
            "configured_name",
            "#[cfg(not(enabled))] mod absent;",
            Supported,
            "",
        )
        .configured(&["enabled"]),
    );
    cases.push(
        case(
            "configured_value",
            "#[cfg(not(mode = \"fast\"))] mod absent;",
            Supported,
            "",
        )
        .configured(&["mode=\"fast\""]),
    );
    cases.push(
        case(
            "configured_multiple_values",
            "#[cfg(not(all(mode = \"fast\", mode = \"safe\")))] mod absent;",
            Supported,
            "",
        )
        .configured(&["mode=\"fast\"", "mode=\"safe\""]),
    );
    cases.push(case("configured_target_builtins", "#[cfg(not(all(target_os = \"linux\", target_arch = \"x86_64\", target_pointer_width = \"64\")))] mod absent;", Supported, ""));
    // S1a validates the disabled syntax using its bounded raw parser. These are
    // legal Rust, but not yet admitted by that parser. Do not call them invalid,
    // or count their rejection as configured-source Rust compatibility.
    cases.push(case(
        "disabled_const_fn_boundary",
        "#[cfg(any())] const fn unused(value: i32) -> i32 { value }",
        Unsupported,
        "",
    ));
    cases.push(case(
        "disabled_char_boundary",
        "#[cfg(any())] fn unused() { let value = 'x'; }",
        Unsupported,
        "",
    ));
    cases.push(case(
        "disabled_async_boundary",
        "#[cfg(any())] async fn unused() {}",
        Unsupported,
        "",
    ));
    cases.push(case(
        "semicolon_doc_rhs_before_false_cfg",
        "#[doc = ;] #[cfg(any())] fn unused() {}",
        Invalid,
        "expected expression",
    ));
    cases.push(case(
        "semicolon_doc_rhs_after_false_cfg",
        "#[cfg(any())] #[doc = ;] fn unused() {}",
        Invalid,
        "expected expression",
    ));
    cases.push(case(
        "plus_unknown_rhs_before_false_cfg",
        "#[unknown = +] #[cfg(any())] fn unused() {}",
        Invalid,
        "expected expression",
    ));
    cases.push(case(
        "plus_unknown_rhs_after_false_cfg",
        "#[cfg(any())] #[unknown = +] fn unused() {}",
        Invalid,
        "expected expression",
    ));
    cases.push(case(
        "cfg_unicode_escape_leading_underscore",
        r##"#[cfg(mode = "\u{_41}")] fn unused() {}"##,
        Invalid,
        "invalid start of unicode escape",
    ));
    cases.push(case(
        "cfg_unicode_escape_before_false_cfg",
        r##"#[cfg(mode = "\u{_41}")] #[cfg(any())] fn unused() {}"##,
        Invalid,
        "invalid start of unicode escape",
    ));
    cases.push(case(
        "cfg_unicode_escape_after_false_cfg",
        r##"#[cfg(any())] #[cfg(mode = "\u{_41}")] fn unused() {}"##,
        Invalid,
        "invalid start of unicode escape",
    ));
    cases.push(case(
        "disabled_module_keyword",
        "#[cfg(any())] mod fn;",
        Invalid,
        "expected identifier, found keyword `fn`",
    ));
    cases.push(case(
        "disabled_visibility_keyword",
        "#[cfg(any())] pub(in fn) mod child;",
        Invalid,
        "expected identifier, found keyword `fn`",
    ));
    cases.push(
        case(
            "child_unicode_cfg_error_span",
            "mod child;",
            Invalid,
            "attribute items not separated",
        )
        .child(
            "src/child.rs",
            "// α🙂\r\n#[cfg(a β)] pub fn unused() {}\r\n",
        )
        .at("src/child.rs:2:9")
        .stock_at("src/child.rs:2:8"),
    );
    cases.push(case(
        "disabled_function_unresolved_visibility",
        "#[cfg(any())] pub(in foo) fn unused() { missing(); }",
        Supported,
        "",
    ));
    cases.push(case(
        "disabled_inline_module_unresolved_visibility",
        "#[cfg(any())] pub(in crate::missing) mod hidden { mod absent; fn unused() { missing(); } }",
        Supported,
        "",
    ));
    let mut names = BTreeSet::new();
    for fixture in &cases {
        assert!(
            names.insert(fixture.name),
            "duplicate fixture {}",
            fixture.name
        );
    }
    cases
}

fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 64 && &bytes[..6] == b"\x7fELF\x02\x01"
}
fn verify_runtime(path: &Path, scratch: &Scratch, name: &str) {
    assert!(is_elf(&fs::read(path).unwrap()), "not ELF64: {name}");
    let run = capture(&mut Command::new(path), scratch, RUN_TIMEOUT);
    assert_eq!(run.code, 0, "runtime {name}: {run:?}");
    assert_eq!(run.stdout, EXPECTED, "runtime stdout {name}");
    assert!(run.stderr.is_empty(), "runtime stderr {name}: {run:?}");
}
fn stock(fixture: &Fixture) {
    let scratch = fixture.materialize();
    let output = scratch.path("stock");
    let mut command = Command::new("rustc");
    command
        .current_dir(&scratch.0)
        .args([
            "--edition=2021",
            "--target=x86_64-unknown-linux-gnu",
            "--crate-name=source_configuration_oracle",
            "--color=never",
        ])
        .args([
            "-C",
            "opt-level=0",
            "-C",
            "overflow-checks=yes",
            "-C",
            "panic=abort",
        ])
        .arg(scratch.path("src/input.rs"))
        .arg("-o")
        .arg(&output);
    for cfg in &fixture.cfg {
        command.arg("--cfg").arg(cfg);
    }
    let result = capture(&mut command, &scratch, COMPILE_TIMEOUT);
    assert!(
        result.stdout.is_empty(),
        "stock compiler stdout {}",
        fixture.name
    );
    let diagnostic = String::from_utf8_lossy(&result.stderr);
    assert_eq!(
        result.code,
        if fixture.outcome == Outcome::Invalid {
            1
        } else {
            0
        },
        "stock {}: {diagnostic}",
        fixture.name
    );
    if !fixture.stock_diagnostic.is_empty() {
        assert!(
            diagnostic.contains(fixture.stock_diagnostic),
            "stock {} missing {:?}: {diagnostic}",
            fixture.name,
            fixture.stock_diagnostic
        );
    }
    if let Some(location) = fixture.stock_location.or(fixture.location) {
        assert!(
            diagnostic.contains(location),
            "stock {} missing location {location}: {diagnostic}",
            fixture.name
        );
    }
    if fixture.outcome == Outcome::Invalid {
        assert!(!output.exists(), "stock invalid output {}", fixture.name);
    } else {
        verify_runtime(&output, &scratch, fixture.name);
    }
    eprintln!("stock source PASS {:?} {}", fixture.outcome, fixture.name);
}
fn oracle_all(cases: &[Fixture]) {
    let scratch = Scratch::new();
    let version = capture(Command::new("rustc").arg("-Vv"), &scratch, COMPILE_TIMEOUT);
    assert_eq!(version.code, 0);
    eprintln!(
        "source stock oracle:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    for fixture in cases {
        stock(fixture);
    }
    let count = |outcome| cases.iter().filter(|case| case.outcome == outcome).count();
    eprintln!(
        "source frozen corpus: {} fixtures = {} supported + {} invalid + {} legal unsupported + {} stock-warning policy boundaries",
        cases.len(),
        count(Outcome::Supported),
        count(Outcome::Invalid),
        count(Outcome::Unsupported),
        count(Outcome::PolicyBoundary),
    );
}
#[test]
fn stock_oracle() {
    oracle_all(&fixtures());
}
#[test]
fn native_after_complete_stock_oracle() {
    let cases = fixtures();
    oracle_all(&cases);
    let mut failures = Vec::new();
    let mut passed = 0;
    for fixture in &cases {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = fixture.materialize();
                let output = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("src/input.rs"))
                    .args(["--workers", &workers.to_string()]);
                for cfg in &fixture.cfg {
                    command.arg("--cfg").arg(cfg);
                }
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&output);
                }
                let result = capture(&mut command, &scratch, COMPILE_TIMEOUT);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                let rejects = fixture.outcome != Outcome::Supported;
                let mut errors = Vec::new();
                if result.code != i32::from(rejects) {
                    errors.push(format!(
                        "exit {}, expected {}",
                        result.code,
                        i32::from(rejects)
                    ));
                }
                // Successful compilation has a normal JSON report on stdout.
                if (rejects || result.code != 0) && !result.stdout.is_empty() {
                    errors.push("failure printed stdout".into());
                }
                if diagnostic.contains("panicked at") {
                    errors.push("compiler panicked".into());
                }
                if rejects && diagnostic.is_empty() {
                    errors.push("missing diagnostic".into());
                }
                if matches!(
                    fixture.outcome,
                    Outcome::Unsupported | Outcome::PolicyBoundary
                ) && !diagnostic.contains("unsupported")
                {
                    errors.push(
                        "stock-accepted boundary lacks explicit unsupported diagnostic".into(),
                    );
                }
                if fixture.outcome == Outcome::Invalid && diagnostic.contains("unsupported") {
                    errors.push("invalid Rust misclassified unsupported".into());
                }
                if let Some(location) = fixture.location {
                    if !diagnostic.contains(location) {
                        errors.push(format!("missing original location {location}"));
                    }
                    if diagnostic.contains("positions refer to expanded") {
                        errors.push("legacy expanded-source attribution".into());
                    }
                }
                if check || rejects || result.code != 0 {
                    for path in [&output, &scratch.path("input"), &scratch.path("a.out")] {
                        if path.exists() {
                            errors.push(format!("CHECK/failure published {}", path.display()));
                        }
                    }
                } else if !output.exists() {
                    errors.push("BUILD missing executable".into());
                } else {
                    verify_runtime(&output, &scratch, fixture.name);
                }
                if errors.is_empty() {
                    passed += 1;
                } else {
                    failures.push(format!(
                        "{} {:?} workers={workers} check={check}: {}\n{diagnostic}",
                        fixture.name,
                        fixture.outcome,
                        errors.join(", ")
                    ));
                }
            }
        }
    }
    eprintln!(
        "native source results: {passed}/{} gates passed, {} failed",
        cases.len() * 4,
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
