//! Source-locked original `core::convert::identity` function probe only.
//! This does not compile the whole convert module or core. Installed-core stock
//! behavior is a separate oracle from the exact attributed source, whose ordinary
//! private-attribute rejection and approved bootstrap duplicate-item experiment
//! are recorded independently. Native never delegates to either stock path.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/stock_rustc.rs"]
mod stock_rustc;

const PROVIDER: &[u8] = include_bytes!("../libraries/probes/identity/convert.rs");
const LOCK: &str = include_str!("../libraries/source-lock.json");
const START: usize = 2258;
const END: usize = 3893;
const SOURCE_HASH: &str = "0fd346eff31bfa6cc7c79fc4cba8367e98c579ccd7072dd141ce0bea76b602ac";
const ITEM_HASH: &str = "c4187b524514fd5daf107703898a3489da0cdf7806eea13988f4f7a365c4d6a8";
const ITEM_NAME: &str = "core::convert::identity";
const EXPECTED: &[u8] = b"identity ok\n";
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
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-core-identity-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch {}: {error}", path.display()),
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
        let mut file = new_file(&path);
        file.write_all(bytes).unwrap();
        drop(file);
    }
    fn probe_files(&self, client: &str, lock: &str, provider: &[u8]) {
        self.write("client.rs", client.as_bytes());
        self.write("libraries/source-lock.json", lock.as_bytes());
        self.write("libraries/probes/identity/convert.rs", provider);
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
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    // Retry only a failed pre-spawn ETXTBSY syscall, never an executed child.
    let mut attempts = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error) if error.raw_os_error() == Some(26) && attempts < 20 => {
                attempts += 1;
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
        code: status.code().expect("child terminated by signal"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 64 && &bytes[..6] == b"\x7fELF\x02\x01"
}
fn runtime(path: &Path, scratch: &Scratch, name: &str) {
    assert!(is_elf(&fs::read(path).unwrap()), "{name}: not ELF64");
    let result = capture(&mut Command::new(path), scratch, Duration::from_secs(5));
    assert_eq!(result.code, 0, "{name}: runtime {result:?}");
    assert_eq!(result.stdout, EXPECTED, "{name}: runtime stdout");
    assert!(
        result.stderr.is_empty(),
        "{name}: runtime stderr {result:?}"
    );
}
fn stock_command(scratch: &Scratch, source: &Path, output: &Path) -> Command {
    let mut command = Command::new(stock_rustc::stock_rustc());
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .args([
            "--edition=2021",
            "--target=x86_64-unknown-linux-gnu",
            "--crate-name=identity_probe_oracle",
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
        .arg(source)
        .arg("-o")
        .arg(output);
    command
}
fn original_item() -> &'static str {
    std::str::from_utf8(&PROVIDER[START..END]).unwrap()
}
fn verify_locked_fixture() {
    assert_eq!(PROVIDER.len(), 33122);
    assert!(LOCK.contains(SOURCE_HASH) && LOCK.contains(ITEM_HASH));
    assert!(LOCK.contains("d907fba494adcabfdd642a18a7918b0c9847d35b"));
    let original = original_item();
    assert!(original.starts_with("/// The identity function.\n"));
    assert!(original.contains("#[stable(feature = \"convert_id\", since = \"1.33.0\")]"));
    assert!(
        original
            .contains("#[rustc_const_stable(feature = \"const_identity\", since = \"1.33.0\")]")
    );
    assert!(
        original.contains("#[inline(always)]\n#[rustc_diagnostic_item = \"convert_identity\"]")
    );
    assert!(original.ends_with("pub const fn identity<T>(x: T) -> T {\n    x\n}\n"));
}
// These are policy experiments, not same-source executable acceptance or a
// substitute for native execution of the locked original item.
fn stock_original_policy_rejections() {
    verify_locked_fixture();
    for bootstrap_experiment in [false, true] {
        let scratch = Scratch::new();
        let wrapper = if bootstrap_experiment {
            "#![feature(staged_api, rustc_attrs)]\n#![stable(feature=\"probe\", since=\"1.0.0\")]\n"
        } else {
            ""
        };
        let source = format!(
            "{wrapper}mod provider {{\n{}}}\nfn main() {{ if provider::identity(42i32)==42i32 {{ println!(\"identity ok\"); }} }}\n",
            original_item()
        );
        scratch.write("original.rs", source.as_bytes());
        let output = scratch.path("stock");
        let mut command = stock_command(&scratch, &scratch.path("original.rs"), &output);
        if bootstrap_experiment {
            command.env("RUSTC_BOOTSTRAP", "1");
        }
        let result = capture(&mut command, &scratch, Duration::from_secs(20));
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert_eq!(
            result.code, 1,
            "original stock policy experiment={bootstrap_experiment}: {diagnostic}"
        );
        assert!(result.stdout.is_empty() && !output.exists());
        if bootstrap_experiment {
            assert!(
                diagnostic.contains("duplicate diagnostic item")
                    && diagnostic.contains("convert_identity")
                    && diagnostic.contains("crate `core`"),
                "bootstrap stock-policy experiment: {diagnostic}"
            );
        } else {
            assert!(
                diagnostic.contains(
                    "stability attributes may not be used outside of the standard library"
                ) && diagnostic.contains("use of an internal attribute"),
                "ordinary stock policy: {diagnostic}"
            );
        }
        eprintln!(
            "original identity stock POLICY REJECTION bootstrap_experiment={bootstrap_experiment}, not exact-source execution"
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    ConstInvalid,
}
struct Fixture {
    name: &'static str,
    client: String,
    outcome: Outcome,
    diagnostic: &'static str,
}
fn valid(name: &'static str, definitions: &str, checks: &str) -> Fixture {
    Fixture {
        name,
        client: format!(
            "{definitions}\nfn run()->bool{{{checks} true}}\nfn main(){{if run(){{println!(\"identity ok\");}}}}\n"
        ),
        outcome: Outcome::Runtime,
        diagnostic: "",
    }
}
fn invalid(name: &'static str, client: &str, diagnostic: &'static str) -> Fixture {
    Fixture {
        name,
        client: client.into(),
        outcome: Outcome::Invalid,
        diagnostic,
    }
}
fn fixtures() -> Vec<Fixture> {
    let cases = vec![
        valid("primitive_i32", "", "if identity(-42i32)!=-42i32{return false;}"),
        valid("primitive_u64", "", "if identity(18446744073709551615u64)!=18446744073709551615u64{return false;}"),
        valid("primitive_bool", "", "if !identity(true)||identity(false){return false;}"),
        valid("generic_forward", "fn forward<T>(value:T)->T{identity(value)}", "if forward(42i32)!=42i32||!forward(true){return false;}"),
        valid("noncopy_value", "struct Token{value:i32}", "let token=Token{value:42};let moved=identity(token);if moved.value!=42{return false;}"),
        valid("noncopy_forward", "struct Token{value:i32} fn forward<T>(value:T)->T{identity(value)}", "let token=forward(Token{value:42});if token.value!=42{return false;}"),
        valid("shared_reference", "", "let value=42i32;let reference=identity(&value);if *reference!=42{return false;}"),
        valid("mutable_reference", "", "let mut value=1i32;let reference=identity(&mut value);*reference=42;if value!=42{return false;}"),
        valid("tuple_forward", "fn forward<T>(value:T)->T{identity(value)}", "let pair=forward((7i32,35i32));if pair.0+pair.1!=42{return false;}"),
        valid("array_forward", "", "let values=identity([7i32,35i32]);if values[0usize]+values[1usize]!=42{return false;}"),
        invalid("unused_invalid_body", "fn unused(){missing();} fn main(){}", "E0425"),
        invalid("noncopy_use_after_move", "struct Token{value:i32} fn main(){let token=Token{value:42};let moved=identity(token);let invalid=token.value;}", "E0382"),
        invalid("noncopy_double_move", "struct Token{value:i32} fn main(){let token=Token{value:42};let first=identity(token);let second=identity(token);}", "E0382"),
        invalid("generic_use_after_move", "fn invalid<T>(value:T)->T{let moved=identity(value);value} fn main(){}", "E0382"),
        invalid("conflicting_mutable_loans", "fn main(){let mut value=1i32;let first=&mut value;let second=identity(&mut value);*first=*second;}", "E0499"),
        invalid("shared_loan_mutation", "fn main(){let mut value=1i32;let reference=identity(&value);value=2;let observed=*reference;}", "E0506"),
        Fixture { name: "unused_const_calls_runtime", client: "fn runtime()->i32{7} const fn invalid()->i32{runtime()} fn main(){}".into(), outcome: Outcome::ConstInvalid, diagnostic: "E0015" },
        Fixture { name: "const_required_boundary", client: "const ANSWER:i32=identity(42i32); fn main(){if ANSWER==42i32{println!(\"identity ok\");}}".into(), outcome: Outcome::Runtime, diagnostic: "" },
    ];
    let mut names = BTreeSet::new();
    for fixture in &cases {
        assert!(names.insert(fixture.name));
    }
    cases
}
fn stock(fixture: &Fixture) {
    let scratch = Scratch::new();
    // This import deliberately uses the INSTALLED stock core implementation.
    // The native client omits it and obtains the unchanged source-locked item.
    scratch.write(
        "stock.rs",
        format!("use core::convert::identity;\n{}", fixture.client).as_bytes(),
    );
    let output = scratch.path("stock");
    let result = capture(
        &mut stock_command(&scratch, &scratch.path("stock.rs"), &output),
        &scratch,
        Duration::from_secs(20),
    );
    let diagnostic = String::from_utf8_lossy(&result.stderr);
    let stock_invalid = matches!(fixture.outcome, Outcome::Invalid | Outcome::ConstInvalid);
    assert_eq!(
        result.code,
        if stock_invalid { 1 } else { 0 },
        "stock installed-core behavior {}: {diagnostic}",
        fixture.name
    );
    assert!(result.stdout.is_empty());
    if stock_invalid {
        assert!(
            diagnostic.contains(fixture.diagnostic),
            "stock {} missing {}: {diagnostic}",
            fixture.name,
            fixture.diagnostic
        );
        assert!(!output.exists());
    } else {
        runtime(&output, &scratch, fixture.name);
    }
    eprintln!(
        "installed-core identity stock BEHAVIOR PASS {:?} {}",
        fixture.outcome, fixture.name
    );
}
fn oracle_all(cases: &[Fixture]) {
    let scratch = Scratch::new();
    let version = capture(
        Command::new(stock_rustc::stock_rustc()).arg("-Vv"),
        &scratch,
        Duration::from_secs(20),
    );
    assert_eq!(version.code, 0);
    eprintln!(
        "identity STOCK TOOLCHAIN (not vendored 1.101.0 source):\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    stock_original_policy_rejections();
    for fixture in cases {
        stock(fixture);
    }
    eprintln!(
        "identity behavioral corpus: 18 = 11 runtime (including const identity execution) + 6 ownership/unused invalid + 1 stock-invalid const-body rejection; exact-source stock policy rejections are separate"
    );
}
fn native_command(scratch: &Scratch, workers: usize, check: bool) -> Command {
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
        command.arg("-o").arg(scratch.path("native"));
    }
    command
}
fn add_probe(command: &mut Command, scratch: &Scratch) {
    command
        .arg("--source-lock")
        .arg(scratch.path("libraries/source-lock.json"))
        .args(["--source-item", ITEM_NAME]);
}
fn no_artifact(scratch: &Scratch) {
    for name in ["native", "client", "a.out"] {
        assert!(
            !scratch.path(name).exists(),
            "CHECK/failure published {name}"
        );
    }
}
#[test]
fn stock_oracle() {
    oracle_all(&fixtures());
}

#[test]
fn const_required_identity_executes_with_exact_locked_provider() {
    let fixture = fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "const_required_boundary")
        .unwrap();
    stock(&fixture);
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = Scratch::new();
            scratch.probe_files(&fixture.client, LOCK, PROVIDER);
            let mut command = native_command(&scratch, workers, check);
            add_probe(&mut command, &scratch);
            let result = capture(&mut command, &scratch, Duration::from_secs(20));
            assert_eq!(
                result.code,
                0,
                "workers={workers} check={check}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                no_artifact(&scratch);
            } else {
                runtime(&scratch.path("native"), &scratch, fixture.name);
            }
        }
    }
}
#[test]
fn native_exact_locked_source_after_complete_stock_oracle() {
    let cases = fixtures();
    oracle_all(&cases);
    let mut failures = Vec::new();
    let mut passed = 0;
    for fixture in &cases {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.probe_files(&fixture.client, LOCK, PROVIDER);
                let mut command = native_command(&scratch, workers, check);
                add_probe(&mut command, &scratch);
                let result = capture(&mut command, &scratch, Duration::from_secs(20));
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                let rejects = fixture.outcome != Outcome::Runtime;
                let mut errors = Vec::new();
                if result.code != i32::from(rejects) {
                    errors.push(format!(
                        "exit{}, expected{}",
                        result.code,
                        i32::from(rejects)
                    ));
                }
                if (rejects || result.code != 0) && !result.stdout.is_empty() {
                    errors.push("failure printed stdout".into());
                }
                if diagnostic.contains("panicked at") {
                    errors.push("compiler panic".into());
                }
                if rejects && diagnostic.is_empty() {
                    errors.push("missing diagnostic".into());
                }
                if fixture.outcome == Outcome::ConstInvalid && !diagnostic.contains("E0015") {
                    errors.push("const-invalid body lacks E0015 diagnostic".into());
                }
                if matches!(fixture.outcome, Outcome::Invalid | Outcome::ConstInvalid)
                    && diagnostic.contains("unsupported")
                {
                    errors.push("stock-invalid client misclassified unsupported".into());
                }
                if check || rejects || result.code != 0 {
                    if ["native", "client", "a.out"]
                        .iter()
                        .any(|name| scratch.path(name).exists())
                    {
                        eprintln!(
                            "unexpected artifact: fixture={} outcome={:?} workers={workers} check={check} exit={} scratch={}\nstdout={}\nstderr={diagnostic}",
                            fixture.name,
                            fixture.outcome,
                            result.code,
                            scratch.0.display(),
                            String::from_utf8_lossy(&result.stdout),
                        );
                    }
                    no_artifact(&scratch);
                } else if scratch.path("native").exists() {
                    runtime(&scratch.path("native"), &scratch, fixture.name);
                } else {
                    errors.push("BUILD missing executable".into());
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
        "identity native gates:{passed}/{}; runtime acceptance, invalid rejections and unsupported boundary remain separately classified",
        cases.len() * 4
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn historical_disabled_const_exact_source_positive_before_promotion() {
    // Exactly the authored source bytes and stdout from the historical fixture.
    let source = "#[cfg(any())] const fn unused(value: i32) -> i32 { value }\nfn main() { println!(\"configuration ok\"); }\n";
    let scratch = Scratch::new();
    scratch.write("client.rs", source.as_bytes());
    let stock_output = scratch.path("stock");
    let stock = capture(
        &mut stock_command(&scratch, &scratch.path("client.rs"), &stock_output),
        &scratch,
        Duration::from_secs(20),
    );
    assert_eq!(stock.code, 0, "historical stock: {stock:?}");
    assert!(is_elf(&fs::read(&stock_output).unwrap()));
    let run = capture(
        &mut Command::new(stock_output),
        &scratch,
        Duration::from_secs(5),
    );
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, b"configuration ok\n");
    assert!(run.stderr.is_empty());
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = Scratch::new();
            scratch.write("client.rs", source.as_bytes());
            let result = capture(
                &mut native_command(&scratch, workers, check),
                &scratch,
                Duration::from_secs(20),
            );
            assert_eq!(
                result.code, 0,
                "historical native workers={workers} check={check}: {result:?}"
            );
            if check {
                no_artifact(&scratch);
            } else {
                assert!(is_elf(&fs::read(scratch.path("native")).unwrap()));
                let run = capture(
                    &mut Command::new(scratch.path("native")),
                    &scratch,
                    Duration::from_secs(5),
                );
                assert_eq!(run.code, 0);
                assert_eq!(run.stdout, b"configuration ok\n");
                assert!(run.stderr.is_empty());
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ProtocolCase {
    UnpairedLock,
    UnpairedItem,
    MissingLockValue,
    MissingItemValue,
    DuplicateLock,
    DuplicateItem,
    MissingItem,
    MalformedLock,
    UnknownLockField,
    WrongSourceHash,
    WrongItemHash,
    WrongRange,
    MutatedProviderOutsideItem,
    MutatedSelectedProvider,
    DefaultClientSpoof,
    ProbeClientSpoof,
}
#[test]
fn native_probe_cli_and_lock_fail_closed() {
    use ProtocolCase::*;
    let cases = [
        UnpairedLock,
        UnpairedItem,
        MissingLockValue,
        MissingItemValue,
        DuplicateLock,
        DuplicateItem,
        MissingItem,
        MalformedLock,
        UnknownLockField,
        WrongSourceHash,
        WrongItemHash,
        WrongRange,
        MutatedProviderOutsideItem,
        MutatedSelectedProvider,
        DefaultClientSpoof,
        ProbeClientSpoof,
    ];
    for case in cases {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                let mut lock = LOCK.to_string();
                let mut provider = PROVIDER.to_vec();
                let client = if matches!(case, DefaultClientSpoof | ProbeClientSpoof) {
                    "#[rustc_diagnostic_item = \"convert_identity\"] fn spoof(value:i32)->i32{value} fn main(){}"
                } else {
                    "fn main(){if identity(42i32)==42i32{println!(\"identity ok\");}}"
                };
                match case {
                    MalformedLock => lock = "{".into(),
                    UnknownLockField => {
                        lock = LOCK.replacen(
                            "\"version\": 1",
                            "\"unrecognized\": true, \"version\": 1",
                            1,
                        )
                    }
                    WrongSourceHash => lock = LOCK.replace(SOURCE_HASH, &"0".repeat(64)),
                    WrongItemHash => lock = LOCK.replace(ITEM_HASH, &"0".repeat(64)),
                    WrongRange => lock = LOCK.replace("\"start\": 2258", "\"start\": 2259"),
                    MutatedProviderOutsideItem => provider.push(b'\n'),
                    MutatedSelectedProvider => provider[END - 3] = b'y',
                    _ => {}
                }
                scratch.probe_files(client, &lock, &provider);
                let mut command = native_command(&scratch, workers, check);
                match case {
                    UnpairedLock => {
                        command
                            .arg("--source-lock")
                            .arg(scratch.path("libraries/source-lock.json"));
                    }
                    UnpairedItem => {
                        command.args(["--source-item", ITEM_NAME]);
                    }
                    MissingLockValue => {
                        command.arg("--source-lock");
                    }
                    MissingItemValue => {
                        command.arg("--source-item");
                    }
                    DuplicateLock => {
                        add_probe(&mut command, &scratch);
                        command
                            .arg("--source-lock")
                            .arg(scratch.path("libraries/source-lock.json"));
                    }
                    DuplicateItem => {
                        add_probe(&mut command, &scratch);
                        command.args(["--source-item", ITEM_NAME]);
                    }
                    MissingItem => {
                        command
                            .arg("--source-lock")
                            .arg(scratch.path("libraries/source-lock.json"))
                            .args(["--source-item", "core::convert::not_identity"]);
                    }
                    DefaultClientSpoof => {}
                    _ => add_probe(&mut command, &scratch),
                }
                let result = capture(&mut command, &scratch, Duration::from_secs(20));
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert_eq!(
                    result.code, 1,
                    "protocol {case:?} workers={workers} check={check}: {diagnostic}"
                );
                assert!(
                    result.stdout.is_empty()
                        && !diagnostic.is_empty()
                        && !diagnostic.contains("panicked at"),
                    "protocol {case:?}: {result:?}"
                );
                no_artifact(&scratch);
            }
        }
    }
    eprintln!(
        "identity probe protocol:{} rejection gates, separate from Rust behavioral conformance",
        cases.len() * 4
    );
}

const RUNTIME_CLIENT: &str = "fn main(){if identity(42i32)==42i32{println!(\"identity ok\");}}";
const NAMESPACE_DIAGNOSTIC: &str = "immutable native source namespace";
const UNPROVEN_WARNING: &str = "native source input capture is unproven";

/// The reviewed probe CLI path must run under the same immutable, bounded,
/// mutation-checked namespace as ordinary files: it prints the namespace
/// diagnostic, never the unproven-capture warning, and both client and lock
/// roots are authorized even when they are disjoint directories.
#[test]
fn native_probe_runs_under_immutable_namespace_without_unproven_warning() {
    for workers in [1, 8] {
        for check in [true, false] {
            // Disjoint layout: client lives under app/, the reviewed lock and
            // its provider live under a separate pinned/ tree. Neither root is
            // an ancestor of the other, so both must be authorized explicitly.
            let scratch = Scratch::new();
            scratch.write("app/client.rs", RUNTIME_CLIENT.as_bytes());
            scratch.write("pinned/libraries/source-lock.json", LOCK.as_bytes());
            scratch.write("pinned/libraries/probes/identity/convert.rs", PROVIDER);
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .current_dir(&scratch.0)
                .env_remove("RUSTC_BOOTSTRAP")
                .env("RUSTC", scratch.path("no-rustc"))
                .env("PATH", scratch.path("no-tools"))
                .arg(scratch.path("app/client.rs"))
                .args(["--workers", &workers.to_string()])
                .arg("--source-lock")
                .arg(scratch.path("pinned/libraries/source-lock.json"))
                .args(["--source-item", ITEM_NAME]);
            let native = scratch.path("app/native");
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(&native);
            }
            let result = capture(&mut command, &scratch, Duration::from_secs(20));
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert_eq!(
                result.code, 0,
                "disjoint probe workers={workers} check={check}: {diagnostic}"
            );
            assert!(
                diagnostic.contains(NAMESPACE_DIAGNOSTIC),
                "missing immutable namespace diagnostic workers={workers} check={check}: {diagnostic}"
            );
            assert!(
                !diagnostic.contains(UNPROVEN_WARNING),
                "reviewed probe still prints unproven-capture warning: {diagnostic}"
            );
            assert!(!diagnostic.contains("panicked at"), "{diagnostic}");
            if check {
                assert!(!native.exists(), "CHECK published executable");
            } else {
                assert!(is_elf(&fs::read(&native).unwrap()), "probe build not ELF64");
                let run = capture(&mut Command::new(&native), &scratch, Duration::from_secs(5));
                assert_eq!(run.code, 0, "probe runtime workers={workers}: {run:?}");
                assert_eq!(run.stdout, EXPECTED);
                assert!(run.stderr.is_empty());
            }
        }
    }
    eprintln!("identity probe immutable-namespace acceptance: disjoint client/lock roots, 1/8 workers, CHECK+BUILD+run");
}

#[derive(Clone, Copy, Debug)]
enum CaptureCase {
    MissingProvider,
    MissingLock,
    SymlinkProvider,
    SymlinkLock,
    ProviderOutsideAuthorizedRoots,
}

/// Capture-time rejections for reviewed probes. Each must fail closed with a
/// diagnostic, no stdout, no panic, and no published artifact. These exercise
/// the immutable namespace boundary itself (absent entries stay absent, no host
/// fallback, symlinks unsupported, provider confined to an authorized root).
#[test]
fn native_probe_namespace_boundary_fails_closed() {
    use CaptureCase::*;
    for case in [
        MissingProvider,
        MissingLock,
        SymlinkProvider,
        SymlinkLock,
        ProviderOutsideAuthorizedRoots,
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.probe_files(RUNTIME_CLIENT, LOCK, PROVIDER);
                let lock = scratch.path("libraries/source-lock.json");
                let provider = scratch.path("libraries/probes/identity/convert.rs");
                match case {
                    MissingProvider => fs::remove_file(&provider).unwrap(),
                    MissingLock => fs::remove_file(&lock).unwrap(),
                    SymlinkProvider => {
                        // Replace the provider with a symlink to identical bytes
                        // outside the captured tree. Capture must not follow it.
                        let target = scratch.path("outside-convert.rs");
                        fs::write(&target, PROVIDER).unwrap();
                        fs::remove_file(&provider).unwrap();
                        std::os::unix::fs::symlink(&target, &provider).unwrap();
                    }
                    SymlinkLock => {
                        let target = scratch.path("outside-lock.json");
                        fs::write(&target, LOCK).unwrap();
                        fs::remove_file(&lock).unwrap();
                        std::os::unix::fs::symlink(&target, &lock).unwrap();
                    }
                    ProviderOutsideAuthorizedRoots => {
                        // A lock that points its provider outside both the
                        // client root and the lock-directory root via parent
                        // traversal must be rejected rather than reaching the
                        // host. The provider is placed where only an escaping
                        // path could find it.
                        fs::remove_file(&provider).unwrap();
                        let escaped = scratch.path("escaped-convert.rs");
                        fs::write(&escaped, PROVIDER).unwrap();
                        let rewritten = LOCK.replace(
                            "\"path\": \"probes/identity/convert.rs\"",
                            "\"path\": \"../../escaped-convert.rs\"",
                        );
                        assert_ne!(rewritten, LOCK, "lock path substitution failed");
                        fs::write(&lock, rewritten).unwrap();
                    }
                }
                let mut command = native_command(&scratch, workers, check);
                add_probe(&mut command, &scratch);
                let result = capture(&mut command, &scratch, Duration::from_secs(20));
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert_eq!(
                    result.code, 1,
                    "capture {case:?} workers={workers} check={check}: {diagnostic}"
                );
                assert!(
                    result.stdout.is_empty()
                        && !diagnostic.is_empty()
                        && !diagnostic.contains("panicked at"),
                    "capture {case:?}: {result:?}"
                );
                no_artifact(&scratch);
            }
        }
    }
    eprintln!("identity probe namespace boundary: 5 capture-time rejections x 1/8 workers x CHECK+BUILD, fail closed");
}
