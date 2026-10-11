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
            .env_remove("JUSTRUST_DEPCACHE_DIR")
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
