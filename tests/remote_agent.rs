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
use std::time::{Duration, Instant};

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

// A concurrent test fork can briefly inherit the just-closed copy writer.
// Linux rejects exec with ETXTBSY until that writer is closed in the child.
// Retry only this setup condition, never a protocol/build failure.
fn spawn_after_copy(command: &mut Command) -> std::io::Result<Child> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

#[test]
fn copied_executable_waits_for_writer_to_close() {
    let dir = scratch("copy-writer");
    let path = dir.join("justrust");
    std::fs::copy(env!("CARGO_BIN_EXE_justrust"), &path).unwrap();
    let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let mut command = Command::new(&path);
    command.arg("--version").stdout(Stdio::null());
    assert_eq!(
        command.spawn().unwrap_err().kind(),
        std::io::ErrorKind::ExecutableFileBusy
    );
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        drop(writer);
    });
    let result = spawn_after_copy(&mut command);
    closer.join().unwrap();
    assert!(result.unwrap().wait().unwrap().success());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn copied_executable_propagates_non_busy_spawn_error() {
    let dir = scratch("copy-missing");
    let mut command = Command::new(dir.join("missing"));
    assert_eq!(
        spawn_after_copy(&mut command).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    std::fs::remove_dir_all(dir).unwrap();
}

impl Agent {
    fn start(name: &str) -> Agent {
        Self::start_copy(name, false)
    }

    fn start_copy(name: &str, copy: bool) -> Agent {
        let dir = scratch(name);
        let executable = if copy {
            let path = dir.join("justrust");
            std::fs::copy(env!("CARGO_BIN_EXE_justrust"), &path).unwrap();
            path
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_justrust"))
        };
        let mut command = Command::new(executable);
        command
            .arg("__agent")
            .arg(dir.join("root"))
            .env("JUSTRUST_HOME", dir.join("home"))
            .env("JUSTRUST_NO_TELEMETRY", "1")
            .env("JUSTRUST_NO_UPDATE", "1")
            .env("JUSTRUST_PREBUILT", "off")
            .env_remove("JUSTRUST_DEPCACHE_DIR")
            .env_remove("JUSTRUST_RUN_DIR")
            .env_remove("JUSTRUST_DISABLE")
            .env_remove("RUSTC")
            .env_remove("RUSTDOC")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("JUSTRUST_REAL_RUSTC")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = if copy {
            spawn_after_copy(&mut command)
        } else {
            command.spawn()
        }
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
            (
                Msg::BarrierAck {
                    id: got, failed, ..
                },
                _,
            ) => {
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
fn authenticated_enrollment_survives_agent_exit_and_native_gc_defaults_dry() {
    let mut agent = Agent::start("durable-gc-restart");
    agent.send(
        Msg::Write {
            path: "/project/src/lib.rs".into(),
            mode: 0o644,
        },
        b"// source survives GC\n",
    );
    assert!(agent.barrier(1).is_empty());
    let target = agent.at("/project/target/debug");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("artifact"), b"recomputable").unwrap();
    agent.send(
        Msg::Synced {
            path: "/project".into(),
        },
        &[],
    );
    assert!(agent.barrier(2).is_empty());
    agent.send(
        Msg::Run {
            id: 3,
            cwd: "/project".into(),
            args: vec!["--version".into()],
            env: vec![],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    );
    let (version, code) = agent.wait_run(3);
    assert_eq!(code, 0);
    assert!(version.starts_with("justrust "), "{version}");
    drop(agent.w.take());
    assert!(agent.child.wait().unwrap().success());
    let output = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .args(["remote", "gc", "--prefix"])
        .arg(agent.dir.join("root"))
        .env("JUSTRUST_HOME", agent.dir.join("home"))
        .env("JUSTRUST_REMOTE_EVICT", "on")
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .output()
        .unwrap();
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{diagnostics}");
    assert!(
        diagnostics.contains("native dry plan registered=1"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("Recent"), "{diagnostics}");
    assert_eq!(
        read(&agent.at("/project/src/lib.rs")),
        "// source survives GC\n"
    );
    assert!(target.join("artifact").exists());
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

#[test]
fn run_children_preserve_distinct_umasks_without_changing_agent_default() {
    use std::os::unix::fs::MetadataExt;
    let output = Command::new("sh").args(["-c", "umask"]).output().unwrap();
    let original =
        u32::from_str_radix(std::str::from_utf8(&output.stdout).unwrap().trim(), 8).unwrap();
    let mut a = Agent::start("run-masks");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    for (id, mask) in [(80, Some("0077")), (81, Some("0022")), (82, None)] {
        let mut env = vec![("JUSTRUST_REAL_CARGO".into(), "sh".into())];
        if let Some(mask) = mask {
            env.push((remote_proto::CLIENT_UMASK.into(), mask.into()));
        }
        a.send(Msg::Run {
            id, cwd: "/proj".into(),
            args: vec!["cargo".into(), "-c".into(), format!("umask; printf 'metadata=%s\n' \"${{JUSTRUST_CLIENT_UMASK-unset}}\"; : > file-{id}; mkdir dir-{id}")],
            env, roots: vec![], start: false, run_id: String::new(),
        }, &[]);
    }
    let mut pending = std::collections::BTreeSet::from([80, 81, 82]);
    let mut outputs = std::collections::BTreeMap::<u64, String>::new();
    while !pending.is_empty() {
        match a.recv() {
            (Msg::Out { id, .. }, data) => outputs
                .entry(id)
                .or_default()
                .push_str(&String::from_utf8_lossy(&data)),
            (Msg::Exit { id, code }, _) => {
                assert_eq!(code, 0);
                assert!(pending.remove(&id));
            }
            other => panic!("unexpected mask frame {other:?}"),
        }
    }
    for (id, mask) in [(80, 0o077), (81, 0o022), (82, original)] {
        let output = &outputs[&id];
        assert_eq!(
            u32::from_str_radix(output.lines().next().unwrap().trim(), 8).unwrap(),
            mask
        );
        assert!(
            output.contains("metadata=unset"),
            "transport metadata must not become Cargo env: {output}"
        );
        assert_eq!(
            std::fs::metadata(a.at(&format!("/proj/file-{id}")))
                .unwrap()
                .mode()
                & 0o777,
            0o666 & !mask
        );
        assert_eq!(
            std::fs::metadata(a.at(&format!("/proj/dir-{id}")))
                .unwrap()
                .mode()
                & 0o777,
            0o777 & !mask
        );
    }
}

#[test]
fn invalid_caller_umask_rejects_run_before_execution() {
    let mut a = Agent::start("invalid-mask");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    assert!(a.barrier(91).is_empty());
    a.send(
        Msg::Run {
            id: 92,
            cwd: "/proj".into(),
            args: vec!["--version".into()],
            env: vec![(remote_proto::CLIENT_UMASK.into(), "1777".into())],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    );
    match a.recv() {
        (Msg::Error { id: 92, msg }, _) => {
            assert!(msg.contains("invalid caller file creation mask"), "{msg}")
        }
        other => panic!("invalid mask must not execute: {other:?}"),
    }
}

#[test]
fn sync_run_rejects_failed_writes_then_checks_latest_source_and_reuses_artifacts() {
    let mut a = Agent::start("pipeline-check");
    for (path, data) in [
        (
            "/proj/Cargo.toml",
            &br#"[package]
name="pipeline-check"
version="0.0.0"
edition="2024"
"#[..],
        ),
        ("/proj/src/lib.rs", &b"pub fn answer() -> u32 { 42 }\n"[..]),
        ("/block", &b"not a directory"[..]),
        ("/block/nested.rs", &b"this write must fail"[..]),
    ] {
        a.send(
            Msg::Write {
                path: path.into(),
                mode: 0o644,
            },
            data,
        );
    }
    let request = |a: &Agent, id| Msg::RunSync {
        id,
        cwd: "/proj".into(),
        args: vec!["check".into(), "--offline".into()],
        env: vec![
            ("JUSTRUST_TOOLCHAIN".into(), "system".into()),
            ("JUSTRUST_DISABLE".into(), "0".into()),
            (
                "CARGO_TARGET_DIR".into(),
                a.at("/proj/target").to_string_lossy().into_owned(),
            ),
        ],
        roots: vec![],
        start: false,
        run_id: format!("pipeline-check-{}-{id}", std::process::id()),
    };
    for id in [50, 51] {
        let msg = request(&a, id);
        a.send(msg, &[]);
        assert_eq!(
            a.recv().0,
            Msg::BarrierAck {
                id,
                failed: vec!["/block/nested.rs".into()],
                disk_full: false,
            }
        );
        assert!(matches!(a.recv().0, Msg::Error { id: got, .. } if got == id));
        assert!(
            !a.at("/proj/target").exists(),
            "rejected request must never invoke cargo"
        );
    }
    a.send(
        Msg::Delete {
            path: "/block".into(),
        },
        &[],
    );
    a.send(
        Msg::Write {
            path: "/block/nested.rs".into(),
            mode: 0o644,
        },
        b"repaired",
    );
    for (id, source, success, compiled) in [
        (52, "pub fn answer() -> u32 { 42 }\n", true, 1),
        (53, "pub fn answer() -> u32 { false }\n", false, 1),
        (54, "pub fn answer() -> u32 { 43 }\n", true, 1),
        (55, "pub fn answer() -> u32 { 43 }\n", true, 0),
    ] {
        // No standalone barrier, source write and run are ordered on one stream.
        if id != 55 {
            a.send(
                Msg::Write {
                    path: "/proj/src/lib.rs".into(),
                    mode: 0o644,
                },
                source.as_bytes(),
            );
        }
        let msg = request(&a, id);
        a.send(msg, &[]);
        assert_eq!(
            a.recv().0,
            Msg::BarrierAck {
                id,
                failed: vec![],
                disk_full: false
            }
        );
        let mut output = String::new();
        loop {
            match a.recv() {
                (Msg::Out { id: got, .. }, data) if got == id => {
                    output.push_str(&String::from_utf8_lossy(&data))
                }
                (Msg::RunFiles { id: got }, _) if got == id => {}
                (Msg::Exit { id: got, code }, data) if got == id => {
                    assert_eq!(
                        code == 0,
                        success,
                        "latest source must determine verdict: {output}"
                    );
                    let summary: serde_json::Value = serde_json::from_slice(&data).unwrap();
                    assert_eq!(summary["units"]["compiled"], compiled, "{output}");
                    if !success {
                        assert!(output.contains("mismatched types"), "{output}");
                    }
                    break;
                }
                other => panic!("unexpected pipeline frame {other:?}: {output}"),
            }
        }
    }
}

#[test]
fn replaced_agent_executable_still_checks_with_stable_recording_shims() {
    use std::os::unix::fs::PermissionsExt;
    let mut a = Agent::start_copy("replace-check", true);
    let replacement = a.dir.join("replacement");
    std::fs::write(&replacement, b"#!/bin/sh\nexit 97\n").unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&replacement, a.dir.join("justrust")).unwrap();
    // The old agent has now lost its installed pathname. Running the new
    // pathname exits 97, while spawning current_exe on Linux gives ENOENT.
    assert!(
        !std::fs::read_link(format!("/proc/{}/exe", a.child.id()))
            .unwrap()
            .exists()
    );
    for (path, data) in [
        (
            "/proj/Cargo.toml",
            b"[package]\nname='snapshot-check'\nversion='0.0.0'\nedition='2021'\n[workspace]\n"
                .as_slice(),
        ),
        (
            "/proj/src/lib.rs",
            b"pub fn answer() -> u32 { 42 }\n".as_slice(),
        ),
    ] {
        a.send(
            Msg::Write {
                path: path.into(),
                mode: 0o644,
            },
            data,
        );
    }
    assert!(a.barrier(100).is_empty());
    let mut snapshots = std::fs::read_dir(a.dir.join("home/bin/agent")).unwrap();
    let pinned = snapshots.next().unwrap().unwrap().path().join("justrust");
    assert!(
        snapshots.next().is_none(),
        "one reusable snapshot per running binary"
    );
    for id in [101, 102, 103] {
        let recorded = id != 102;
        a.send(
            Msg::Run {
                id,
                cwd: "/proj".into(),
                args: if recorded {
                    vec!["check".into(), "--offline".into()]
                } else {
                    vec!["cargo".into(), "check".into(), "--offline".into()]
                },
                env: vec![
                    ("JUSTRUST_TOOLCHAIN".into(), "system".into()),
                    (
                        "JUSTRUST_DISABLE".into(),
                        if recorded { "0" } else { "1" }.into(),
                    ),
                    (
                        "CARGO_TARGET_DIR".into(),
                        a.at("/proj/target").to_string_lossy().into_owned(),
                    ),
                ],
                roots: vec![],
                start: false,
                run_id: if recorded {
                    format!("snapshot-check-{}-{id}", std::process::id())
                } else {
                    String::new()
                },
            },
            &[],
        );
        let mut output = String::new();
        loop {
            match a.recv() {
                (Msg::Out { .. }, data) => output.push_str(&String::from_utf8_lossy(&data)),
                (Msg::RunFiles { .. }, _) => {}
                (Msg::Exit { id: got, code }, data) => {
                    assert_eq!(got, id);
                    assert_eq!(code, 0, "real check after replacement: {output}");
                    if recorded {
                        let summary: serde_json::Value = serde_json::from_slice(&data).unwrap();
                        assert_eq!(
                            summary["units"]["compiled"].as_u64().unwrap(),
                            if id == 101 { 1 } else { 0 },
                            "recorded and unrecorded checks must share fresh artifacts: {output}"
                        );
                    }
                    break;
                }
                (Msg::Error { msg, .. }, _) => {
                    panic!("run after atomic replacement failed: {msg}\n{output}")
                }
                other => panic!("unexpected frame {other:?}"),
            }
        }
    }
    let shim = a.dir.join("home/bin/shim/rustc");
    assert_eq!(
        std::fs::canonicalize(shim).unwrap(),
        std::fs::canonicalize(pinned).unwrap()
    );
}

#[test]
fn remote_shadow_preserves_leaf_verdicts_and_normal_build_fallback() {
    let mut a = Agent::start("shadow-routing");
    for (path, data) in [
        (
            "/proj/Cargo.toml",
            "[workspace]\nmembers=['big']\nresolver='2'\n[profile.dev]\ndebug=0\n",
        ),
        (
            "/proj/big/Cargo.toml",
            "[package]\nname='big'\nversion='0.0.0'\nedition='2021'\n",
        ),
        (
            "/proj/big/src/lib.rs",
            "mod leaf;\nmod other;\npub fn api() -> u32 { leaf::double(other::three()) }\n",
        ),
        (
            "/proj/big/src/other.rs",
            "pub(crate) fn three() -> u32 { 3 }\n",
        ),
        (
            "/proj/big/src/leaf.rs",
            "pub(crate) fn double(x: u32) -> u32 { x * 2 }\n#[cfg(test)]\nmod tests {\nuse super::*;\n#[test]\nfn doubles() { assert_eq!(double(2), 4); }\n}\n",
        ),
    ] {
        a.send(
            Msg::Write {
                path: path.into(),
                mode: 0o644,
            },
            data.as_bytes(),
        );
    }
    assert!(a.barrier(200).is_empty());
    let leaf = read(&a.at("/proj/big/src/leaf.rs"));
    // First use records the normal build. Leaf-only edits use the existing
    // opt-in route, including failed tests. Other edits and explicit opt-out
    // must still run the original package. The final repeat reuses artifacts.
    for (id, changed, source, enabled, shadow, code) in [
        (201, "", "".into(), true, false, 0),
        (
            202,
            "/proj/big/src/leaf.rs",
            leaf.replace("x * 2", "x + x"),
            true,
            true,
            0,
        ),
        (
            203,
            "/proj/big/src/leaf.rs",
            leaf.replace("x * 2", "x * 3"),
            true,
            true,
            101,
        ),
        (
            204,
            "/proj/big/src/other.rs",
            "pub(crate) fn three() -> u32 { 1 + 2 }\n".into(),
            true,
            false,
            101,
        ),
        (205, "/proj/big/src/leaf.rs", leaf.clone(), true, false, 0),
        (206, "", "".into(), false, false, 0),
        (207, "", "".into(), true, true, 0),
        (208, "", "".into(), true, true, 0),
    ] {
        if !changed.is_empty() {
            a.send(
                Msg::Write {
                    path: changed.into(),
                    mode: 0o644,
                },
                source.as_bytes(),
            );
            assert!(a.barrier(id + 100).is_empty());
        }
        a.send(
            Msg::Run {
                id,
                cwd: "/proj".into(),
                args: vec![
                    "test".into(),
                    "-p".into(),
                    "big".into(),
                    "--lib".into(),
                    "--offline".into(),
                    "--".into(),
                    "leaf".into(),
                ],
                env: vec![
                    ("JUSTRUST_TOOLCHAIN".into(), "system".into()),
                    (
                        "JUSTRUST_SHADOW".into(),
                        if enabled { "1" } else { "0" }.into(),
                    ),
                    ("JUSTRUST_BUILDD".into(), "0".into()),
                    (
                        "CARGO_TARGET_DIR".into(),
                        a.at("/proj/target").to_string_lossy().into_owned(),
                    ),
                ],
                roots: vec![],
                start: false,
                run_id: format!("shadow-routing-{}-{id}", std::process::id()),
            },
            &[],
        );
        let mut output = String::new();
        loop {
            match a.recv() {
                (Msg::Out { id: got, .. }, data) if got == id => {
                    output.push_str(&String::from_utf8_lossy(&data))
                }
                (Msg::RunFiles { id: got }, _) if got == id => {}
                (
                    Msg::Exit {
                        id: got,
                        code: actual,
                    },
                    data,
                ) if got == id => {
                    assert_eq!(
                        actual, code,
                        "latest source must determine verdict: {output}"
                    );
                    assert_eq!(
                        output.contains("leaf crate jr-shadow-leaf"),
                        shadow,
                        "{output}"
                    );
                    let summary: serde_json::Value = serde_json::from_slice(&data).unwrap();
                    assert_eq!(
                        summary["tests"][if code == 0 { "passed" } else { "failed" }],
                        1,
                        "{output}"
                    );
                    assert_eq!(
                        summary["args"][2],
                        if shadow { "jr-shadow-leaf" } else { "big" },
                        "{output}"
                    );
                    if id == 208 {
                        assert_eq!(summary["units"]["compiled"], 0, "{output}");
                    }
                    break;
                }
                other => panic!("unexpected shadow frame {other:?}: {output}"),
            }
        }
    }
    assert!(
        !a.at("/proj/jr-shadow-leaf").exists(),
        "user workspace must not be rewritten"
    );
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
fn immediate_pipeline_cancel_is_not_lost_before_pid_registration() {
    let mut a = Agent::start("immediate-cancel");
    a.send(
        Msg::Mkdir {
            path: "/proj".into(),
        },
        &[],
    );
    for id in 100..112 {
        let start = std::time::Instant::now();
        a.send(
            Msg::RunSync {
                id,
                cwd: "/proj".into(),
                args: vec!["cargo".into(), "30.404".into()],
                env: vec![("JUSTRUST_REAL_CARGO".into(), "sleep".into())],
                roots: vec![],
                start: false,
                run_id: String::new(),
            },
            &[],
        );
        // Do not wait for Pong, output, or PID publication before cancelling.
        a.send(Msg::Cancel { id }, &[]);
        assert_eq!(
            a.recv().0,
            Msg::BarrierAck {
                id,
                failed: vec![],
                disk_full: false
            }
        );
        loop {
            match a.recv().0 {
                Msg::Out { id: got, .. } if got == id => {}
                Msg::Exit { id: got, code } if got == id => {
                    assert_ne!(code, 0);
                    break;
                }
                Msg::Error { id: got, msg } if got == id => {
                    assert!(msg.contains("cancelled before starting"), "{msg}");
                    break;
                }
                other => panic!("unexpected cancel frame {other:?}"),
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "Cancel must not wait for sleep to finish"
        );
    }
    assert!(!running("sleep 30.404"));
    a.send(Msg::Ping, &[]);
    assert!(matches!(a.recv().0, Msg::Pong { active: 0, .. }));
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
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_PREBUILT", "off")
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
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_PREBUILT", "off")
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

// ------------------------------------------------------------- peer cache

/// Put a depcache entry (one text output, so no binary hashes to compute)
/// into the cache under `home`. Returns (base, out, line).
fn seed_entry(home: &Path, tag: char) -> (String, String, String) {
    let base = tag.to_string().repeat(32);
    let out = "0123456789abcdef".repeat(2);
    let out = format!("{}{}", &out[..31], tag);
    let root = home.join("cache/depcache");
    let dir = root.join("o").join(&out);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("x-abc.d"),
        "@@JUSTRUST_PROFILE_DIR@@/deps/x: a.rs\n",
    )
    .unwrap();
    let meta = r#"{"files":[{"name":"x-abc.d","hash":"h","text":true}],"stderr":[],"wall":4.5}"#;
    std::fs::write(dir.join("meta.json"), meta).unwrap();
    let line = format!(r#"{{"inputs":[],"env":[],"out":"{out}"}}"#);
    std::fs::create_dir_all(root.join("m")).unwrap();
    std::fs::write(
        root.join("m").join(format!("{base}.jsonl")),
        format!("{line}\n"),
    )
    .unwrap();
    (base, out, line)
}

/// Next frame that is not the agent's key-set prefetch (answered with
/// `keys`, as the daemon would).
fn recv_answering_keys(a: &mut Agent, keys: &str) -> (Msg, Vec<u8>) {
    loop {
        match a.recv() {
            (Msg::CacheKeys { id }, _) => a.send(Msg::CacheKeySet { id }, keys.as_bytes()),
            f => return f,
        }
    }
}

fn fetch_bundle(a: &mut Agent, id: u64, out: &str) -> Vec<u8> {
    a.send(
        Msg::CacheFetch {
            id,
            out: out.into(),
        },
        &[],
    );
    let mut data = Vec::new();
    loop {
        match recv_answering_keys(a, "") {
            (Msg::CacheData { id: i, last, ok }, p) if i == id => {
                assert!(ok, "fetch failed");
                data.extend_from_slice(&p);
                if last {
                    return data;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// The agent answers cache requests from its own cache, and an entry moved
/// from one machine to another with `CachePut` arrives intact.
#[test]
fn peer_cache_serves_and_installs_entries() {
    let mut a = Agent::start("peer-a");
    let mut b = Agent::start("peer-b");
    let (base, out, line) = seed_entry(&a.dir.join("home"), 'a');

    a.send(
        Msg::CacheQuery {
            id: 1,
            base: base.clone(),
        },
        &[],
    );
    match recv_answering_keys(&mut a, "") {
        (
            Msg::CacheLines {
                id: 1, lines, outs, ..
            },
            _,
        ) => {
            assert_eq!(lines, vec![line.clone()]);
            assert_eq!(outs.len(), 1);
            assert_eq!(outs[0].out, out);
            assert_eq!(outs[0].wall, 4.5);
            assert!(outs[0].bytes > 0);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Unknown and hostile keys: nothing.
    a.send(
        Msg::CacheQuery {
            id: 2,
            base: "../../etc".into(),
        },
        &[],
    );
    assert!(matches!(
        recv_answering_keys(&mut a, ""),
        (Msg::CacheLines { id: 2, ref lines, .. }, _) if lines.is_empty()
    ));
    a.send(
        Msg::CacheFetch {
            id: 3,
            out: "f".repeat(32),
        },
        &[],
    );
    assert!(matches!(
        recv_answering_keys(&mut a, ""),
        (
            Msg::CacheData {
                id: 3,
                ok: false,
                last: true
            },
            _
        )
    ));
    // Key set.
    a.send(Msg::CacheKeys { id: 4 }, &[]);
    match recv_answering_keys(&mut a, "") {
        (Msg::CacheKeySet { id: 4 }, p) => assert_eq!(String::from_utf8(p).unwrap(), base),
        other => panic!("unexpected {other:?}"),
    }

    let bundle = fetch_bundle(&mut a, 5, &out);
    assert!(!bundle.is_empty());

    // b takes it only for a compiler it uses.
    let item = |tc: &str| remote_proto::OfferItem {
        base: base.clone(),
        line: line.clone(),
        out: out.clone(),
        bytes: 10,
        wall: 4.5,
        tc: tc.into(),
    };
    let broot = b.dir.join("home/cache/depcache");
    std::fs::create_dir_all(&broot).unwrap();
    std::fs::write(broot.join("peer-toolchains"), "1.99 abc\n").unwrap();
    b.send(
        Msg::CacheOffer {
            id: 6,
            items: vec![item("1.99 abc"), item("1.0 other")],
        },
        &[],
    );
    match recv_answering_keys(&mut b, "") {
        (Msg::CacheWant { id: 6, outs }, _) => {
            assert_eq!(outs, vec![out.clone(), out.clone()][..1])
        }
        other => panic!("unexpected {other:?}"),
    }
    // Sent in two chunks.
    let mid = bundle.len() / 2;
    for (chunk, last) in [(&bundle[..mid], false), (&bundle[mid..], true)] {
        b.send(
            Msg::CachePut {
                id: 7,
                base: base.clone(),
                line: line.clone(),
                out: out.clone(),
                last,
            },
            chunk,
        );
    }
    // Installed (asynchronously): b now serves it, byte for byte.
    let mut ok = false;
    for i in 0..100 {
        b.send(
            Msg::CacheQuery {
                id: 100 + i,
                base: base.clone(),
            },
            &[],
        );
        if let (Msg::CacheLines { lines, .. }, _) = recv_answering_keys(&mut b, "")
            && lines == vec![line.clone()]
        {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ok, "entry not installed");
    assert_eq!(
        read(&broot.join("o").join(&out).join("x-abc.d")),
        read(
            &a.dir
                .join("home/cache/depcache/o")
                .join(&out)
                .join("x-abc.d")
        )
    );
    // A corrupt bundle installs nothing.
    let (base2, out2, line2) = (
        "c".repeat(32),
        "d".repeat(32),
        format!(r#"{{"inputs":[],"env":[],"out":"{}"}}"#, "d".repeat(32)),
    );
    b.send(
        Msg::CachePut {
            id: 8,
            base: base2.clone(),
            line: line2,
            out: out2.clone(),
            last: true,
        },
        b"not a bundle",
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(!broot.join("o").join(&out2).exists());
    assert!(!broot.join("m").join(format!("{base2}.jsonl")).exists());
}

/// A rustc shim on the agent's machine reaches the daemon's cache through
/// the agent's `peer.sock`: queries and fetches are relayed over the link.
#[test]
fn peer_cache_relays_shim_requests_to_the_daemon() {
    use std::os::unix::net::UnixStream;
    let mut a = Agent::start("peer-relay");
    let sock = a.dir.join("home/peer.sock");
    for _ in 0..200 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let base = "e".repeat(32);
    let out = "f".repeat(32);
    // Before the key set arrives, queries go to the daemon. Answer the
    // prefetch with our one key.
    let s = UnixStream::connect(&sock).expect("peer.sock");
    let mut sw = BufWriter::new(s.try_clone().unwrap());
    let mut sr = BufReader::new(s);
    write_frame(
        &mut sw,
        &Msg::CacheQuery {
            id: 9,
            base: base.clone(),
        },
        &[],
    )
    .unwrap();
    let fwd_id = match recv_answering_keys(&mut a, &base) {
        (Msg::CacheQuery { id, base: b }, _) => {
            assert_eq!(b, base);
            id
        }
        other => panic!("unexpected {other:?}"),
    };
    let info = remote_proto::OutInfo {
        out: out.clone(),
        bytes: 123,
        wall: 2.0,
    };
    a.send(
        Msg::CacheLines {
            id: fwd_id,
            lines: vec!["L".into()],
            outs: vec![info.clone()],
            busy: 0,
            bw: 0.0,
        },
        &[],
    );
    match read_frame(&mut sr).unwrap().unwrap() {
        (
            Msg::CacheLines {
                id: 9, lines, outs, ..
            },
            _,
        ) => {
            assert_eq!(lines, vec!["L".to_string()]);
            assert_eq!(outs, vec![info]);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Fetch: relayed chunk by chunk.
    write_frame(
        &mut sw,
        &Msg::CacheFetch {
            id: 10,
            out: out.clone(),
        },
        &[],
    )
    .unwrap();
    let fwd = match recv_answering_keys(&mut a, &base) {
        (Msg::CacheFetch { id, out: o }, _) => {
            assert_eq!(o, out);
            id
        }
        other => panic!("unexpected {other:?}"),
    };
    a.send(
        Msg::CacheData {
            id: fwd,
            last: false,
            ok: true,
        },
        b"hello ",
    );
    a.send(
        Msg::CacheData {
            id: fwd,
            last: true,
            ok: true,
        },
        b"world",
    );
    let mut got = Vec::new();
    loop {
        match read_frame(&mut sr).unwrap().unwrap() {
            (
                Msg::CacheData {
                    id: 10,
                    last,
                    ok: true,
                },
                p,
            ) => {
                got.extend_from_slice(&p);
                if last {
                    break;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(got, b"hello world");
    // Once the key set is known, a base the daemon lacks is answered
    // without a round trip.
    write_frame(
        &mut sw,
        &Msg::CacheQuery {
            id: 11,
            base: "a".repeat(32),
        },
        &[],
    )
    .unwrap();
    match read_frame(&mut sr).unwrap().unwrap() {
        (Msg::CacheLines { id: 11, lines, .. }, _) => assert!(lines.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
}
