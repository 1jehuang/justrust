//! Public CLI coverage for the default-off critical-chain scheduler experiment.

use crate::prelude::*;
use crate::utils::cargo_exe;
use cargo_test_support::{Project, project};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const SWITCH: &str = "JUSTRUST_CARGO_CRITICAL_PATH";
const HINTS: &str = "JUSTRUST_CARGO_CRITICAL_PATH_COSTS";

fn output(p: &Project, args: &str, enabled: bool, hints: &Path) -> Output {
    p.cargo(args)
        .env(SWITCH, if enabled { "1" } else { "0" })
        .env(HINTS, hints)
        .exec_with_output()
        .unwrap()
}

fn assert_fresh(output: &Output) {
    let messages: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|message: &serde_json::Value| message["reason"] == "compiler-artifact")
        .collect();
    assert!(!messages.is_empty(), "no artifact messages: {output:?}");
    for message in messages {
        assert_eq!(message["fresh"], true, "unexpected rebuild: {message}");
    }
}

fn artifact_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(base: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(base, &path, files);
            } else if path.file_name().unwrap() == "scheduler-app"
                || path.file_name().unwrap() == "scheduler-app.exe"
                || matches!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("rlib" | "rmeta" | "so" | "dll" | "dylib")
                )
                || path.file_name().unwrap() == "generated.rs"
            {
                files.insert(
                    path.strip_prefix(base).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    assert!(!files.is_empty());
    files
}

fn assert_same_artifacts(before: &BTreeMap<PathBuf, Vec<u8>>, after: &BTreeMap<PathBuf, Vec<u8>>) {
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in before {
        assert!(
            bytes == &after[path],
            "artifact changed: {}",
            path.display()
        );
    }
}

#[cargo_test]
fn multi_output_build_check_and_switch_are_artifact_transparent() {
    // The same shape as build::tricky_pipelining: the binary needs every
    // transitive rlib even when intermediate metadata becomes ready first.
    let p = project()
        .file("Cargo.toml", "[package]\nname='scheduler-app'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmid={path='mid'}\n")
        .file("src/main.rs", "fn main() { println!(\"{}\", mid::value()); }")
        .file("mid/Cargo.toml", "[package]\nname='mid'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nleaf={path='../leaf'}\n")
        .file("mid/src/lib.rs", "pub fn value() -> u32 { leaf::value() }")
        .file("leaf/Cargo.toml", "[package]\nname='leaf'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmaker={path='../maker'}\n")
        .file("leaf/src/lib.rs", "include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\")); maker::make_value!();")
        .file("leaf/build.rs", r#"fn main() {
            println!("cargo:rerun-if-changed=build.rs");
            std::fs::write(std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("generated.rs"), "const GENERATED: u32 = 42;").unwrap();
        }"#)
        .file("maker/Cargo.toml", "[package]\nname='maker'\nversion='0.1.0'\nedition='2021'\n[lib]\nproc-macro=true\n")
        .file("maker/src/lib.rs", "extern crate proc_macro; #[proc_macro] pub fn make_value(_: proc_macro::TokenStream) -> proc_macro::TokenStream { \"pub fn value() -> u32 { GENERATED }\".parse().unwrap() }")
        .file("costs.json", r#"{"leaf":1000,"maker":2000}"#)
        .build();
    let hints = p.root().join("costs.json");
    for command in [
        "build --message-format=json -j2",
        "check --message-format=json -j2",
    ] {
        output(&p, command, false, &hints);
        let before = artifact_bytes(&p.build_dir());
        assert_fresh(&output(&p, command, true, &hints));
        assert_same_artifacts(&before, &artifact_bytes(&p.build_dir()));
        assert_fresh(&output(&p, command, false, &hints));
        assert_same_artifacts(&before, &artifact_bytes(&p.build_dir()));
    }
    p.process(p.bin("scheduler-app"))
        .with_stdout_data("42\n")
        .run();
    for jobs in [1, 2] {
        let target = format!("target/critical-j{jobs}");
        output(
            &p,
            &format!("build --target-dir {target} -j{jobs}"),
            true,
            &hints,
        );
        p.process(
            p.root()
                .join(&target)
                .join("debug")
                .join(format!("scheduler-app{}", std::env::consts::EXE_SUFFIX)),
        )
        .with_stdout_data("42\n")
        .run();
        output(
            &p,
            &format!("check --target-dir {target} -j{jobs}"),
            true,
            &hints,
        );
        assert_fresh(&output(
            &p,
            &format!("check --target-dir {target} --message-format=json -j{jobs}"),
            true,
            &hints,
        ));
    }
}

#[cargo_test]
fn cold_cli_order_and_invalid_or_missing_cost_fallback() {
    let mut p = project()
        .file("Cargo.toml", "[package]\nname='ordering'\nversion='0.1.0'\n[dependencies]\ndeep_top={path='deep_top'}\nwide_a={path='wide_a'}\nwide_b={path='wide_b'}\nwide_c={path='wide_c'}\nwide_d={path='wide_d'}\n")
        .file("src/lib.rs", "")
        .file("costs.json", r#"{"wide":2000}"#);
    for (name, dep) in [
        ("wide", None),
        ("wide_a", Some("wide")),
        ("wide_b", Some("wide")),
        ("wide_c", Some("wide")),
        ("wide_d", Some("wide")),
        ("deep_leaf", None),
        ("deep_mid", Some("deep_leaf")),
        ("deep_top", Some("deep_mid")),
    ] {
        let mut manifest = format!("[package]\nname='{name}'\nversion='0.1.0'\n");
        if let Some(dep) = dep {
            manifest.push_str(&format!("[dependencies]\n{dep}={{path='../{dep}'}}\n"));
        }
        p = p
            .file(format!("{name}/Cargo.toml"), &manifest)
            .file(format!("{name}/src/lib.rs"), "");
    }
    let p = p.build();
    let hints = p.root().join("costs.json");
    let cases = [
        (false, hints.clone(), "wide"),
        (true, hints.clone(), "wide"),
        (true, p.root().join("invalid.json"), "deep_leaf"),
        (true, p.root().join("missing.json"), "deep_leaf"),
    ];
    p.change_file("invalid.json", r#"{"wide":0}"#);
    for (i, (enabled, path, first)) in cases.iter().enumerate() {
        let result = output(
            &p,
            &format!("check -j1 --target-dir target/case-{i}"),
            *enabled,
            path,
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        let wide = stderr
            .find("Checking wide v")
            .unwrap_or_else(|| panic!("{stderr}"));
        let deep = stderr
            .find("Checking deep_leaf v")
            .unwrap_or_else(|| panic!("{stderr}"));
        assert_eq!(
            wide < deep,
            *first == "wide",
            "wrong first ready unit: {stderr}"
        );
    }
}

#[cargo_test]
fn keep_going_failed_branch_fix_and_retry() {
    let p = project()
        .file("Cargo.toml", "[workspace]\nmembers=['broken','blocked','independent']\nresolver='2'\n")
        .file("broken/Cargo.toml", "[package]\nname='broken'\nversion='0.1.0'\n")
        .file("broken/src/lib.rs", "compile_error!(\"scheduler branch failure\");")
        .file("blocked/Cargo.toml", "[package]\nname='blocked'\nversion='0.1.0'\n[dependencies]\nbroken={path='../broken'}\n")
        .file("blocked/src/lib.rs", "")
        .file("independent/Cargo.toml", "[package]\nname='independent'\nversion='0.1.0'\n")
        .file("independent/src/lib.rs", "pub fn independent() {}")
        .build();
    let hints = p.root().join("missing.json");
    p.cargo("check --workspace --keep-going -j2")
        .env(SWITCH, "1")
        .env(HINTS, &hints)
        .with_status(101)
        .with_stderr_contains("[ERROR] scheduler branch failure")
        .with_stderr_contains("[CHECKING] independent v0.1.0 [..]")
        .with_stderr_does_not_contain("[CHECKING] blocked v0.1.0 [..]")
        .run();
    p.change_file("broken/src/lib.rs", "pub fn fixed() {}");
    output(
        &p,
        "check --workspace --keep-going --message-format=json -j2",
        true,
        &hints,
    );
    assert_fresh(&output(
        &p,
        "check --workspace --message-format=json -j2",
        false,
        &hints,
    ));
}

// Adapted from jobserver::makes_jobserver_used. These are real build-script
// jobs, held at a TCP barrier while the controller observes the token limit.
fn token_fixture() -> Project {
    let mut p = project()
        .file("Cargo.toml", "[package]\nname='tokens'\nversion='0.1.0'\n[dependencies]\nd1={path='d1'}\nd2={path='d2'}\nd3={path='d3'}\n")
        .file("src/lib.rs", "")
        .file("dbuild.rs", r#"fn main() {
            println!("cargo:rerun-if-changed=../dbuild.rs");
            println!("cargo:rerun-if-env-changed=SCHEDULER_ADDR");
            let mut stream = std::net::TcpStream::connect(std::env::var("SCHEDULER_ADDR").unwrap()).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut bytes).unwrap();
        }"#)
        .file("Makefile", "all:\n\t+$(CARGO) build\n");
    for name in ["d1", "d2", "d3"] {
        p = p
            .file(
                format!("{name}/Cargo.toml"),
                &format!("[package]\nname='{name}'\nversion='0.1.0'\nbuild='../dbuild.rs'\n"),
            )
            .file(format!("{name}/src/lib.rs"), "");
    }
    p.build()
}

fn accept_until(listener: &TcpListener, deadline: Instant) -> TcpStream {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            other => panic!("timed out or failed accepting build-script job: {other:?}"),
        }
    }
}

fn token_controller(listener: TcpListener) {
    listener.set_nonblocking(true).unwrap();
    let first = accept_until(&listener, Instant::now() + Duration::from_secs(60));
    let second = accept_until(&listener, Instant::now() + Duration::from_secs(60));
    let held_until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < held_until {
        assert!(
            matches!(listener.accept(), Err(ref e) if e.kind() == ErrorKind::WouldBlock),
            "third job entered while both tokens were occupied"
        );
        thread::sleep(Duration::from_millis(5));
    }
    drop(first);
    let third = accept_until(&listener, Instant::now() + Duration::from_secs(60));
    drop((second, third));
}

fn prewarm_token_fixture(p: &Project) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let controller = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        for _ in 0..3 {
            drop(accept_until(
                &listener,
                Instant::now() + Duration::from_secs(60),
            ));
        }
    });
    p.cargo("build -j2")
        .env(SWITCH, "0")
        .env(HINTS, p.root().join("missing.json"))
        .env("SCHEDULER_ADDR", address)
        .run();
    controller.join().unwrap();
}

#[cargo_test]
fn internal_jobserver_obeys_two_real_tokens() {
    let p = token_fixture();
    prewarm_token_fixture(&p);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let controller = thread::spawn(move || token_controller(listener));
    p.cargo("build -j2")
        .env(SWITCH, "1")
        .env(HINTS, p.root().join("missing.json"))
        .env("SCHEDULER_ADDR", address)
        .run();
    controller.join().unwrap();
}

#[cargo_test]
#[cfg(unix)]
fn inherited_make_jobserver_obeys_two_real_tokens() {
    let version = match Command::new("make").arg("--version").output() {
        Ok(version) => version,
        Err(error) => {
            eprintln!("inherited jobserver test skipped: make unavailable ({error})");
            return;
        }
    };
    assert!(version.status.success());
    eprintln!(
        "inherited jobserver tested with {}",
        String::from_utf8_lossy(&version.stdout)
    );
    let p = token_fixture();
    prewarm_token_fixture(&p);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let controller = thread::spawn(move || token_controller(listener));
    p.process("make")
        .arg("-j2")
        .env("CARGO", cargo_exe())
        .env(SWITCH, "1")
        .env(HINTS, p.root().join("missing.json"))
        .env("SCHEDULER_ADDR", address)
        .run();
    controller.join().unwrap();
}
