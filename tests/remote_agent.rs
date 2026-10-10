//! Protocol tests for `justrust __agent`, run locally as a subprocess (no
//! network): writes, deletes, barriers, write failures, runs and cancel.
//!
//! The agent gets a prefix, so every absolute path it is sent lands under a
//! scratch directory.

#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "../src/remote_proto.rs"]
mod remote_proto;

use remote_proto::{Msg, VERSION, read_frame, write_frame};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Agent {
    child: Child,
    w: Option<BufWriter<ChildStdin>>,
    rx: Receiver<(Msg, Vec<u8>)>,
    dir: PathBuf,
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("jr-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

impl Agent {
    fn start(name: &str) -> Agent {
        let dir = scratch(name);
        let mut child = Command::new(env!("CARGO_BIN_EXE_justrust"))
            .arg("__agent")
            .arg(dir.join("root"))
            .env("JUSTRUST_HOME", dir.join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut w = BufWriter::new(child.stdin.take().unwrap());
        let mut r = BufReader::new(child.stdout.take().unwrap());
        write_frame(&mut w, &Msg::Hello { version: VERSION }, &[]).unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || read_all(&mut r, tx));
        let a = Agent {
            child,
            w: Some(w),
            rx,
            dir,
        };
        assert_eq!(a.recv().0, Msg::Hello { version: VERSION });
        a
    }

    fn send(&mut self, m: Msg, payload: &[u8]) {
        write_frame(self.w.as_mut().unwrap(), &m, payload).unwrap();
    }

    fn recv(&self) -> (Msg, Vec<u8>) {
        self.rx
            .recv_timeout(Duration::from_secs(20))
            .expect("agent answered")
    }

    /// Paths the agent reports as failed.
    fn barrier(&mut self, id: u64) -> Vec<String> {
        self.send(Msg::Barrier { id }, &[]);
        match self.recv() {
            (Msg::BarrierAck { id: got, failed }, _) => {
                assert_eq!(got, id);
                failed
            }
            other => panic!("expected BarrierAck, got {other:?}"),
        }
    }

    fn at(&self, p: &str) -> PathBuf {
        self.dir.join("root").join(p.trim_start_matches('/'))
    }

    /// Collect a run's output until `Exit`: (stdout, code).
    fn wait_run(&self, id: u64) -> (String, i32) {
        let mut out = String::new();
        loop {
            match self.recv() {
                (Msg::Out { id: i, stream: 1 }, d) if i == id => {
                    out.push_str(&String::from_utf8_lossy(&d))
                }
                (Msg::Out { .. }, _) => {}
                (Msg::Exit { id: i, code }, _) if i == id => return (out, code),
                (Msg::Error { msg, .. }, _) => panic!("run error: {msg}"),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn read_all(r: &mut BufReader<ChildStdout>, tx: std::sync::mpsc::Sender<(Msg, Vec<u8>)>) {
    while let Ok(Some(f)) = read_frame(r) {
        if tx.send(f).is_err() {
            return;
        }
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn writes_deletes_and_barriers() {
    let mut a = Agent::start("writes");
    a.send(
        Msg::Write {
            path: "/proj/src/lib.rs".into(),
            mode: 0o644,
        },
        b"pub fn a() {}",
    );
    a.send(
        Msg::Write {
            path: "/proj/src/new/mod.rs".into(),
            mode: 0o755,
        },
        b"// new",
    );
    a.send(
        Msg::Write {
            path: "/proj/src/old.rs".into(),
            mode: 0o644,
        },
        b"old",
    );
    assert!(a.barrier(1).is_empty());
    assert_eq!(read(&a.at("/proj/src/lib.rs")), "pub fn a() {}");
    assert_eq!(read(&a.at("/proj/src/new/mod.rs")), "// new");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(a.at("/proj/src/new/mod.rs"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);

    // Overwrite, delete a file, delete a whole directory, delete a path that
    // is already gone (not an error).
    a.send(
        Msg::Write {
            path: "/proj/src/lib.rs".into(),
            mode: 0o644,
        },
        b"pub fn b() {}",
    );
    a.send(
        Msg::Delete {
            path: "/proj/src/old.rs".into(),
        },
        &[],
    );
    a.send(
        Msg::Delete {
            path: "/proj/src/new".into(),
        },
        &[],
    );
    a.send(
        Msg::Delete {
            path: "/proj/never-existed.rs".into(),
        },
        &[],
    );
    assert!(a.barrier(2).is_empty());
    assert_eq!(read(&a.at("/proj/src/lib.rs")), "pub fn b() {}");
    assert!(!a.at("/proj/src/old.rs").exists());
    assert!(!a.at("/proj/src/new").exists());
    // No temp files left behind.
    let names: Vec<_> = std::fs::read_dir(a.at("/proj/src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["lib.rs".to_string()]);
}

#[test]
fn failed_writes_fail_barriers_until_fixed() {
    let mut a = Agent::start("fail");
    a.send(
        Msg::Write {
            path: "/proj/a".into(),
            mode: 0o644,
        },
        b"file",
    );
    // `a` is a file, so nothing can be written under it.
    a.send(
        Msg::Write {
            path: "/proj/a/b.rs".into(),
            mode: 0o644,
        },
        b"x",
    );
    a.send(
        Msg::Write {
            path: "/proj/ok.rs".into(),
            mode: 0o644,
        },
        b"ok",
    );
    assert_eq!(a.barrier(1), vec!["/proj/a/b.rs".to_string()]);
    // Still reported: the tree is still wrong.
    assert_eq!(a.barrier(2), vec!["/proj/a/b.rs".to_string()]);
    // Fixed by a later successful write of the same path.
    a.send(
        Msg::Delete {
            path: "/proj/a".into(),
        },
        &[],
    );
    a.send(
        Msg::Write {
            path: "/proj/a/b.rs".into(),
            mode: 0o644,
        },
        b"x",
    );
    assert!(a.barrier(3).is_empty());
    assert_eq!(read(&a.at("/proj/a/b.rs")), "x");

    // A failure cleared by an out-of-band mirror (rsync) of its root.
    a.send(
        Msg::Write {
            path: "/proj/ok.rs/c.rs".into(),
            mode: 0o644,
        },
        b"x",
    );
    assert_eq!(a.barrier(4), vec!["/proj/ok.rs/c.rs".to_string()]);
    a.send(
        Msg::Synced {
            path: "/other".into(),
        },
        &[],
    );
    assert_eq!(a.barrier(5).len(), 1);
    a.send(
        Msg::Synced {
            path: "/proj".into(),
        },
        &[],
    );
    assert!(a.barrier(6).is_empty());
}

#[test]
fn runs_stream_output_and_exit_codes() {
    let mut a = Agent::start("run");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    let cwd = "/proj".to_string();
    a.send(
        Msg::Run {
            id: 7,
            cwd: cwd.clone(),
            args: vec!["--version".into()],
            env: vec![],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    );
    let (out, code) = a.wait_run(7);
    assert_eq!(code, 0);
    assert!(out.starts_with("justrust "), "{out}");

    // A failing command reports its exit code, not an infrastructure error.
    a.send(
        Msg::Run {
            id: 8,
            cwd,
            args: vec!["no-such-subcommand".into()],
            env: vec![],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    );
    let (_, code) = a.wait_run(8);
    assert_ne!(code, 0);

    // A missing cwd is an infrastructure error.
    a.send(
        Msg::Run {
            id: 9,
            cwd: "/nope".into(),
            args: vec!["--version".into()],
            env: vec![],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    );
    match a.recv() {
        (Msg::Error { id: 9, .. }, _) => {}
        other => panic!("expected Error, got {other:?}"),
    }
}

/// `justrust cargo <secs>` with the real cargo pointed at `sleep` is a
/// long-running run that needs no toolchain.
fn sleep_run(id: u64, secs: &str) -> Msg {
    Msg::Run {
        id,
        cwd: "/proj".into(),
        args: vec!["cargo".into(), secs.into()],
        env: vec![("JUSTRUST_REAL_CARGO".into(), "sleep".into())],
        roots: vec![],
        start: false,
        run_id: String::new(),
    }
}

fn running(pattern: &str) -> bool {
    Command::new("pgrep")
        .args(["-f", pattern])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn cancel_interrupts_a_run_and_others_continue() {
    let mut a = Agent::start("cancel");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    a.send(sleep_run(1, "30.101"), &[]);
    a.send(sleep_run(2, "0.3"), &[]);
    // Writes and barriers are not blocked by runs in flight.
    a.send(
        Msg::Write {
            path: "/proj/x.rs".into(),
            mode: 0o644,
        },
        b"x",
    );
    assert!(a.barrier(3).is_empty());
    assert_eq!(a.wait_run(2).1, 0);
    a.send(Msg::Cancel { id: 1 }, &[]);
    let (_, code) = a.wait_run(1);
    assert_eq!(code, 128 + libc_sigint());
    assert!(!running("sleep 30.101"));
}

#[test]
fn closing_the_connection_stops_runs() {
    let mut a = Agent::start("close");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    a.send(sleep_run(1, "30.202"), &[]);
    let t = std::time::Instant::now();
    while !running("sleep 30.202") && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(running("sleep 30.202"));
    a.w.take(); // EOF on the agent's stdin
    let st = a.child.wait().unwrap();
    assert!(st.success(), "{st:?}");
    let t = std::time::Instant::now();
    while running("sleep 30.202") && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running("sleep 30.202"));
}

#[test]
fn killing_the_agent_stops_runs() {
    let mut a = Agent::start("kill");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    a.send(sleep_run(1, "30.303"), &[]);
    let t = std::time::Instant::now();
    while !running("sleep 30.303") && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(running("sleep 30.303"));
    // SAFETY: signalling our own child.
    unsafe {
        libc::kill(a.child.id() as i32, libc::SIGTERM);
    }
    let _ = a.child.wait();
    let t = std::time::Instant::now();
    while running("sleep 30.303") && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running("sleep 30.303"));
}

#[test]
fn refuses_unknown_protocol_versions() {
    let dir = scratch("version");
    let mut child = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .arg("__agent")
        .arg(&dir)
        .env("JUSTRUST_HOME", dir.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = child.stdin.take().unwrap();
    write_frame(&mut w, &Msg::Hello { version: 99 }, &[]).unwrap();
    let mut r = BufReader::new(child.stdout.take().unwrap());
    match read_frame(&mut r).unwrap() {
        Some((Msg::Error { msg, .. }, _)) => assert!(msg.contains("protocol"), "{msg}"),
        other => panic!("expected Error, got {other:?}"),
    }
    assert!(!child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serves_v1_daemons() {
    let dir = scratch("v1");
    let mut child = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .arg("__agent")
        .arg(&dir)
        .env("JUSTRUST_HOME", dir.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = child.stdin.take().unwrap();
    write_frame(&mut w, &Msg::Hello { version: 1 }, &[]).unwrap();
    write_frame(&mut w, &Msg::Barrier { id: 1 }, &[]).unwrap();
    drop(w);
    let mut r = BufReader::new(child.stdout.take().unwrap());
    assert_eq!(
        read_frame(&mut r).unwrap().unwrap().0,
        Msg::Hello { version: 1 }
    );
    assert!(matches!(
        read_frame(&mut r).unwrap().unwrap().0,
        Msg::BarrierAck { id: 1, .. }
    ));
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&dir);
}

fn libc_sigint() -> i32 {
    2
}
