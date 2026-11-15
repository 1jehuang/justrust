//! External dispatcher background-prewarm contract using only fake subprocesses.
//! No compiler build or native engine execution is performed by these fixtures.
#![cfg(unix)]

use std::fs;
use std::os::unix::{ffi::OsStringExt as _, fs::PermissionsExt as _, process::ExitStatusExt as _};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DEADLINE: Duration = Duration::from_secs(20);

const ENGINE: &str = r#"#!/usr/bin/python3
import os, pathlib, sys, time
root = pathlib.Path(os.environ['PROBE_ROOT'])
def event(value):
    fd = os.open(root / 'events', os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o600)
    try: os.write(fd, (value + '\n').encode())
    finally: os.close(fd)
if sys.argv[1:3] == ['cargo', 'hybrid-ready']:
    event('ready-probe')
    fd = int(os.environ['JR_HYBRID_DECISION_FD'])
    committed = 'COMMITTED_STATUS' in os.environ
    os.write(fd, b'committed\n' if committed else b'miss\n')
    os.close(fd)
    sys.exit(int(os.environ.get('COMMITTED_STATUS', '0')))
assert sys.argv[1:4] == ['cargo', 'hybrid-prewarm', '--'], repr(sys.argv)
assert 'JR_HYBRID_DECISION_FD' not in os.environ
assert os.environ['RUSTC'] == str(root / 'toolchain/bin/rustc')
assert not os.environ.get('RUSTC_WRAPPER')
assert 'fallback-done' in (root / 'events').read_text().splitlines()
event('prewarm-start')
event('prewarm-priority=' + str(os.getpriority(os.PRIO_PROCESS, 0)))
assert os.getpriority(os.PRIO_PROCESS, 0) == int(os.environ['PROBE_EXPECTED_PRIORITY'])
event('prewarm-arguments=' + repr(sys.argv[4:]))
print('PRIVATE_PREWARM_STDOUT', flush=True)
print('PRIVATE_PREWARM_STDERR', file=sys.stderr, flush=True)
if os.environ.get('PREWARM_BLOCK') == '1':
    deadline = time.monotonic() + 15
    while not (root / 'release-prewarm').exists():
        if time.monotonic() >= deadline:
            event('prewarm-timeout')
            sys.exit(86)
        time.sleep(.01)
event('prewarm-done')
sys.exit(int(os.environ.get('PREWARM_STATUS', '0')))
"#;

const FALLBACK: &str = r#"#!/usr/bin/python3
import os, pathlib, sys, time
root = pathlib.Path(os.environ['PROBE_ROOT'])
def event(value):
    fd = os.open(root / 'events', os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o600)
    try: os.write(fd, (value + '\n').encode())
    finally: os.close(fd)
event('fallback-start')
assert 'JR_HYBRID_DECISION_FD' not in os.environ
assert os.environ['RUSTC'] == '/caller/stock-rustc'
assert os.environ['CARGO_TARGET_DIR'] == '/caller/stock-target'
assert os.environ['RUSTC_WRAPPER'] == '/caller/stock-wrapper'
if os.environ.get('FALLBACK_BLOCK') == '1':
    deadline = time.monotonic() + 15
    while not (root / 'release-fallback').exists():
        if time.monotonic() >= deadline:
            sys.exit(87)
        time.sleep(.01)
event('fallback-done')
print('FOREGROUND_RESULT', flush=True)
print('FALLBACK_ARGUMENTS=' + repr(sys.argv[1:]), flush=True)
if 'FALLBACK_SIGNAL' in os.environ:
    os.kill(os.getpid(), int(os.environ['FALLBACK_SIGNAL']))
sys.exit(int(os.environ.get('FALLBACK_STATUS', '0')))
"#;

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    engine: PathBuf,
    fallback: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "jr-hybrid-background-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("private home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir_all(root.join("toolchain/bin")).unwrap();
        for name in ["rustc", "cargo"] {
            fs::write(root.join("toolchain/bin").join(name), "unused").unwrap();
        }
        let engine = root.join("fake-engine");
        let fallback = root.join("fake-fallback");
        for (path, content) in [(&engine, ENGINE), (&fallback, FALLBACK)] {
            fs::write(path, content).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            root,
            home,
            engine,
            fallback,
        }
    }

    fn command(&self) -> Command {
        // A resident daemon outlives preparation. Lowering this supervisor's nice
        // value would permanently slow every future warm request via inheritance.
        // SAFETY: getpriority reads scheduling state and has no memory preconditions.
        let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.root)
            .env("PROBE_ROOT", &self.root)
            .env("PROBE_EXPECTED_PRIORITY", priority.to_string())
            .env("JUSTRUST_HOME", self.root.join("state"))
            .env("JR_ENGINE_HOME", &self.home)
            .env("JUSTRUST_NATIVE_ENGINE", &self.engine)
            .env("JUSTRUST_HYBRID_TOOLCHAIN", self.root.join("toolchain"))
            .env("JUSTRUST_HYBRID_FALLBACK", &self.fallback)
            .env("JUSTRUST_HYBRID_PREWARM", "after-success")
            .env("RUSTC", "/caller/stock-rustc")
            .env("RUSTC_WRAPPER", "/caller/stock-wrapper")
            .env("CARGO_TARGET_DIR", "/caller/stock-target")
            .current_dir(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("hybrid");
        command
    }

    fn events(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("events"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn wait_event(&self, event: &str, count: usize) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let events = self.events();
            if events.iter().filter(|line| line.as_str() == event).count() >= count {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "waiting for {event} x{count}, observed {events:?}, log {:?}",
                fs::read_to_string(self.home.join("prewarm.log"))
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn release(&self) {
        fs::write(self.root.join("release-prewarm"), "release").unwrap();
        self.wait_event("prewarm-done", 1);
        self.wait_log("justrust hybrid: background prewarm finished status=0");
    }

    fn wait_log(&self, receipt: &str) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let log = fs::read_to_string(self.home.join("prewarm.log")).unwrap_or_default();
            if log.contains(receipt) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "waiting for log receipt {receipt:?}: {log}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("release-prewarm"), "release");
        let _ = fs::write(self.root.join("release-fallback"), "release");
        // Avoid removing the event directory while an already-started fake engine
        // is finishing, including during an assertion unwind.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.events().iter().any(|line| line == "prewarm-start")
            && !self.events().iter().any(|line| line == "prewarm-done")
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn output(mut child: Child) -> Output {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("foreground did not finish independently: {}", text(&output));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn successful_foreground_finishes_before_compile_only_prewarm_and_logs_privately() {
    let fixture = Fixture::new();
    let result = output(
        fixture
            .command()
            .env("PREWARM_BLOCK", "1")
            .args(["test", "--lib", "--", "filter with spaces", ""])
            .spawn()
            .unwrap(),
    );
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(text(&result).matches("FOREGROUND_RESULT").count(), 1);
    assert!(!text(&result).contains("PRIVATE_PREWARM"));
    fixture.wait_event("prewarm-start", 1);
    let events = fixture.events();
    assert!(
        events
            .iter()
            .position(|line| line == "fallback-done")
            .unwrap()
            < events
                .iter()
                .position(|line| line == "prewarm-start")
                .unwrap(),
        "{events:?}"
    );
    assert!(!events.iter().any(|line| line == "prewarm-done"));
    // SAFETY: this read-only syscall observes the parent's priority.
    let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    fixture.wait_event(&format!("prewarm-priority={priority}"), 1);
    fixture.wait_event(
        "prewarm-arguments=['test', '--lib', '--', 'filter with spaces', '']",
        1,
    );
    fixture.release();
    let log = fs::read_to_string(fixture.home.join("prewarm.log")).unwrap();
    assert!(log.contains("PRIVATE_PREWARM_STDOUT"), "{log}");
    assert!(log.contains("PRIVATE_PREWARM_STDERR"), "{log}");
    assert_eq!(
        fs::metadata(fixture.home.join("prewarm.log"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
}

#[test]
fn failed_foreground_never_schedules_prewarm() {
    let fixture = Fixture::new();
    let result = output(
        fixture
            .command()
            .env("FALLBACK_STATUS", "101")
            .arg("test")
            .spawn()
            .unwrap(),
    );
    assert_eq!(result.status.code(), Some(101), "{}", text(&result));
    assert_eq!(
        fixture.events(),
        ["ready-probe", "fallback-start", "fallback-done"]
    );
    assert!(!fixture.home.join("prewarm.log").exists());
}

#[test]
fn duplicate_cold_requests_share_one_prewarm_without_blocking_fallback() {
    let fixture = Fixture::new();
    let first = output(
        fixture
            .command()
            .env("PREWARM_BLOCK", "1")
            .arg("test")
            .spawn()
            .unwrap(),
    );
    assert!(first.status.success(), "{}", text(&first));
    fixture.wait_event("prewarm-start", 1);
    let second = output(
        fixture
            .command()
            .env("PREWARM_BLOCK", "1")
            .arg("test")
            .spawn()
            .unwrap(),
    );
    assert!(second.status.success(), "{}", text(&second));
    fixture.wait_event("fallback-done", 2);
    fixture.wait_log("preparation already active in this namespace");
    assert_eq!(
        fixture
            .events()
            .iter()
            .filter(|line| *line == "prewarm-start")
            .count(),
        1
    );
    assert!(!fixture.events().iter().any(|line| line == "prewarm-done"));
    fixture.release();
}

#[test]
fn background_compile_failure_does_not_replace_foreground_status() {
    let fixture = Fixture::new();
    let result = output(
        fixture
            .command()
            .env("PREWARM_STATUS", "101")
            .arg("build")
            .spawn()
            .unwrap(),
    );
    assert!(result.status.success(), "{}", text(&result));
    fixture.wait_event("prewarm-done", 1);
    fixture.wait_log("background native preparation ended with");
    assert!(
        !fs::read_to_string(fixture.home.join("prewarm.log"))
            .unwrap()
            .contains("background prewarm finished status=0")
    );
    assert_eq!(text(&result).matches("FOREGROUND_RESULT").count(), 1);
}

#[test]
fn simultaneous_foregrounds_do_not_wait_for_each_other_or_background_compile() {
    let fixture = Fixture::new();
    let first = fixture
        .command()
        .env("FALLBACK_BLOCK", "1")
        .env("PREWARM_BLOCK", "1")
        .arg("test")
        .spawn()
        .unwrap();
    fixture.wait_event("fallback-start", 1);
    let second = output(
        fixture
            .command()
            .env("PREWARM_BLOCK", "1")
            .arg("test")
            .spawn()
            .unwrap(),
    );
    assert!(second.status.success(), "{}", text(&second));
    fixture.wait_event("prewarm-start", 1);
    assert_eq!(
        fixture
            .events()
            .iter()
            .filter(|line| *line == "fallback-done")
            .count(),
        1
    );
    fs::write(fixture.root.join("release-fallback"), "release").unwrap();
    let first = output(first);
    assert!(first.status.success(), "{}", text(&first));
    fixture.wait_event("fallback-done", 2);
    fixture.wait_log("preparation already active in this namespace");
    fixture.release();
}

#[test]
fn committed_native_success_or_error_never_schedules_background_work() {
    for status in ["0", "101"] {
        let fixture = Fixture::new();
        let result = output(
            fixture
                .command()
                .env("COMMITTED_STATUS", status)
                .arg("test")
                .spawn()
                .unwrap(),
        );
        assert_eq!(
            result.status.code(),
            Some(status.parse().unwrap()),
            "{}",
            text(&result)
        );
        assert_eq!(fixture.events(), ["ready-probe"]);
        assert!(
            text(&result).contains("justrust hybrid: resident committed"),
            "{}",
            text(&result)
        );
        assert!(!fixture.home.join("prewarm.log").exists());
    }
}

#[test]
fn foreground_signal_is_preserved_without_scheduling_background_work() {
    let fixture = Fixture::new();
    let result = output(
        fixture
            .command()
            .env("FALLBACK_SIGNAL", "15")
            .arg("test")
            .spawn()
            .unwrap(),
    );
    assert_eq!(result.status.signal(), Some(15), "{}", text(&result));
    assert_eq!(
        fixture.events(),
        ["ready-probe", "fallback-start", "fallback-done"]
    );
    assert!(!fixture.home.join("prewarm.log").exists());
}

#[test]
fn non_utf8_workload_argument_is_preserved_without_background_preparation() {
    let fixture = Fixture::new();
    let result = output(
        fixture
            .command()
            .arg("test")
            .arg(std::ffi::OsString::from_vec(vec![255]))
            .spawn()
            .unwrap(),
    );
    assert!(result.status.success(), "{}", text(&result));
    assert!(
        text(&result).contains("FALLBACK_ARGUMENTS=['test', '\\udcff']"),
        "{}",
        text(&result)
    );
    assert_eq!(fixture.events(), ["fallback-start", "fallback-done"]);
    assert!(!fixture.home.join("prewarm.log").exists());
}

#[test]
fn log_creation_failure_does_not_replace_foreground_status() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.home.join("prewarm.log")).unwrap();
    let result = output(fixture.command().arg("check").spawn().unwrap());
    assert!(result.status.success(), "{}", text(&result));
    assert_eq!(text(&result).matches("FOREGROUND_RESULT").count(), 1);
}

#[test]
fn absent_or_unrecognized_prewarm_flag_never_loads_background_engine() {
    for flag in [None, Some("off"), Some("1"), Some("true")] {
        let fixture = Fixture::new();
        let mut command = fixture.command();
        match flag {
            Some(value) => command.env("JUSTRUST_HYBRID_PREWARM", value),
            None => command.env_remove("JUSTRUST_HYBRID_PREWARM"),
        };
        let result = output(command.arg("test").spawn().unwrap());
        assert!(result.status.success(), "{}", text(&result));
        assert_eq!(
            fixture.events(),
            ["ready-probe", "fallback-start", "fallback-done"]
        );
        assert!(!fixture.home.join("prewarm.log").exists());
    }
}
