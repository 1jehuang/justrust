//! Real Unix subprocess contract for hybrid routing, with a controlled fake engine.
#![cfg(unix)]
use std::fs;
use std::os::unix::{ffi::OsStringExt as _, fs::PermissionsExt as _, process::ExitStatusExt as _};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    engine: PathBuf,
    fallback: PathBuf,
    tc: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new(body: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "jr-hybrid-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("private home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let tc = root.join("toolchain");
        fs::create_dir_all(tc.join("bin")).unwrap();
        fs::write(tc.join("bin/rustc"), "unused").unwrap();
        fs::write(tc.join("bin/cargo"), "unused").unwrap();
        let engine = root.join("engine");
        fs::write(
            &engine,
            format!("#!/usr/bin/python3\nimport os,sys,signal\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
        let fallback = root.join("fallback");
        fs::write(&fallback, "#!/usr/bin/python3\nimport os,sys\nprint('FALLBACK')\nprint(repr(sys.argv[1:]))\nprint('RUSTC='+os.environ.get('RUSTC','unset'))\nprint('TARGET='+os.environ.get('CARGO_TARGET_DIR','unset'))\nprint('FD='+os.environ.get('JR_HYBRID_DECISION_FD','unset'))\nsys.exit(int(os.environ.get('FALLBACK_STATUS','0')))\n").unwrap();
        fs::set_permissions(&fallback, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root,
            engine,
            fallback,
            tc,
            home,
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_justrust"));
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.root)
            .env("JUSTRUST_HOME", self.root.join("state"))
            .env("JR_ENGINE_HOME", &self.home)
            .env("JUSTRUST_NATIVE_ENGINE", &self.engine)
            .env("JUSTRUST_HYBRID_TOOLCHAIN", &self.tc)
            .env("JUSTRUST_HYBRID_FALLBACK", &self.fallback)
            .current_dir(&self.root)
            .arg("hybrid");
        cmd
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
const MISS: &str =
    "fd=int(os.environ['JR_HYBRID_DECISION_FD'])\nos.write(fd,b'miss\\n')\nos.close(fd)";
const COMMIT: &str =
    "fd=int(os.environ['JR_HYBRID_DECISION_FD'])\nos.write(fd,b'committed\\n')\nos.close(fd)";

#[test]
fn positive_miss_falls_back_once_with_unchanged_arguments_and_compiler_environment() {
    let fixture = Fixture::new(MISS);
    let output = fixture
        .command()
        .env("RUSTC", "/caller/rustc")
        .env("CARGO_TARGET_DIR", "/caller/target")
        .args(["test", "--lib", "--", "with spaces", ""])
        .output()
        .unwrap();
    let out = text(&output);
    assert!(output.status.success(), "{out}");
    assert_eq!(out.matches("FALLBACK\n").count(), 1);
    assert!(
        out.contains("['test', '--lib', '--', 'with spaces', '']"),
        "{out}"
    );
    assert!(
        out.contains("RUSTC=/caller/rustc")
            && out.contains("TARGET=/caller/target")
            && out.contains("FD=unset"),
        "{out}"
    );
}

#[test]
fn committed_failure_and_reserved_status_never_retry() {
    for code in [0, 1, 101, 125] {
        let fixture = Fixture::new(&format!(
            "{COMMIT}\nprint('native result')\nsys.exit({code})"
        ));
        let output = fixture.command().arg("test").output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{}", text(&output));
        assert!(!text(&output).contains("FALLBACK"));
    }
}

#[test]
fn ambiguous_and_failed_misses_do_not_retry() {
    for script in ["sys.exit(125)".to_owned(), format!("{MISS}\nsys.exit(101)"),
        "fd=int(os.environ['JR_HYBRID_DECISION_FD'])\nos.write(fd,b'miss\\ncommitted\\n')\nos.close(fd)".to_owned()] {
        let fixture = Fixture::new(&script);
        let output = fixture.command().arg("test").output().unwrap();
        assert!(!output.status.success(), "{}", text(&output));
        assert!(!text(&output).contains("FALLBACK"));
        assert!(text(&output).contains("refusing an ambiguous Cargo retry"), "{}", text(&output));
    }
}

#[test]
fn committed_signal_is_preserved_without_fallback() {
    let fixture = Fixture::new(&format!("{COMMIT}\nos.kill(os.getpid(),signal.SIGTERM)"));
    let output = fixture.command().arg("run").output().unwrap();
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGTERM),
        "{}",
        text(&output)
    );
    assert!(!text(&output).contains("FALLBACK"));
}

#[test]
fn native_target_and_toolchain_are_isolated_from_fallback_state() {
    let fixture = Fixture::new(&format!(
        "{COMMIT}\nprint(repr(sys.argv[1:]))\nprint(os.environ['RUSTC'])\nprint(os.environ['CARGO_TARGET_DIR'])\nprint(os.environ.get('RUSTC_WRAPPER','unset'))"
    ));
    let output = fixture
        .command()
        .env("RUSTC", "/caller/rustc")
        .env("RUSTC_WRAPPER", "/caller/wrapper")
        .env("CARGO_TARGET_DIR", "/caller/target")
        .args(["test", "--", "exact filter"])
        .output()
        .unwrap();
    let out = text(&output);
    assert!(output.status.success(), "{out}");
    assert!(
        out.contains("['cargo', 'hybrid-ready', '--', 'test', '--', 'exact filter']"),
        "{out}"
    );
    assert!(
        out.contains(fixture.tc.join("bin/rustc").to_str().unwrap()),
        "{out}"
    );
    assert!(
        out.contains(fixture.home.join("targets").to_str().unwrap()),
        "{out}"
    );
    assert!(
        !out.contains("/caller/target")
            && !out.contains("/caller/rustc")
            && !out.contains("/caller/wrapper"),
        "{out}"
    );
}

#[test]
fn explicit_prewarm_is_not_a_test_invocation_or_fallback() {
    let fixture =
        Fixture::new("print(repr(sys.argv[1:]))\nassert 'JR_HYBRID_DECISION_FD' not in os.environ");
    let output = fixture
        .command()
        .args(["prewarm", "test", "--lib", "--", "filter"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output)
            .contains("['cargo', 'hybrid-prewarm', '--', 'test', '--lib', '--', 'filter']")
    );
    assert!(!text(&output).contains("FALLBACK"));
}

#[test]
fn missing_toolchain_uses_normal_path_without_invoking_engine() {
    let fixture = Fixture::new("raise RuntimeError('must not execute engine')");
    let output = fixture
        .command()
        .env_remove("JUSTRUST_HYBRID_TOOLCHAIN")
        .arg("build")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("FALLBACK"));
}

#[test]
fn non_utf8_workload_arguments_are_not_lost() {
    let fixture = Fixture::new("raise RuntimeError('must not execute engine')");
    let output = fixture
        .command()
        .arg("test")
        .arg(std::ffi::OsString::from_vec(vec![255]))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("FALLBACK"));
}

#[test]
fn unsafe_native_home_is_never_repurposed() {
    let fixture = Fixture::new(COMMIT);
    fs::set_permissions(&fixture.home, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture.command().arg("check").output().unwrap();
    assert!(!output.status.success());
    assert!(text(&output).contains("owner-only"), "{}", text(&output));
    assert!(!text(&output).contains("FALLBACK"));
    assert_eq!(
        fs::metadata(&fixture.home).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn fallback_exit_code_is_preserved() {
    let fixture = Fixture::new(MISS);
    let output = fixture
        .command()
        .env("FALLBACK_STATUS", "101")
        .arg("test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101), "{}", text(&output));
}

#[test]
fn opt_in_environment_routes_normal_commands_without_recursive_fallback() {
    let fixture = Fixture::new(MISS);
    let template = fixture.command();
    let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
    command
        .env_clear()
        .envs(
            template
                .get_envs()
                .filter_map(|(key, value)| value.map(|v| (key, v))),
        )
        .env("JUSTRUST_HYBRID", "ready")
        .current_dir(&fixture.root)
        .args(["test", "--lib"]);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(text(&output).matches("FALLBACK\n").count(), 1);
    assert!(text(&output).contains("['test', '--lib']"));
}

#[test]
fn help_does_not_require_engine_account_or_private_home() {
    let fixture = Fixture::new("sys.exit(99)");
    let output = fixture
        .command()
        .env_remove("JUSTRUST_NATIVE_ENGINE")
        .env_remove("JUSTRUST_HYBRID_TOOLCHAIN")
        .env_remove("JR_ENGINE_HOME")
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("experimental") || text(&output).contains("Experimental"));
    assert!(!fixture.root.join("state").exists());
}
