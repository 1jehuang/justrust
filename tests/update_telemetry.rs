//! Auto-update and telemetry end to end against a local stand-in for
//! build.jcode.sh and GitHub: a copied justrust binary updates itself
//! (checksum verified, version checked, atomic replace), refuses a bad
//! checksum, sends its queued telemetry in the background, and sends
//! nothing once telemetry is off.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("jr-{name}-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[derive(Default)]
struct Server {
    files: Mutex<Vec<(String, Vec<u8>)>>,
    posts: Mutex<Vec<(String, String)>>,
    gets: Mutex<Vec<String>>,
}

/// A tiny HTTP/1.0 server: GET serves `files` by path prefix, POST records.
fn serve() -> (String, Arc<Server>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let s = Arc::new(Server::default());
    let s2 = s.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let s = s2.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let (code, body) = if method == "POST" {
                    let mut b = vec![0; len];
                    r.read_exact(&mut b).unwrap();
                    s.posts
                        .lock()
                        .unwrap()
                        .push((path, String::from_utf8_lossy(&b).into_owned()));
                    (200, br#"{"ok":true}"#.to_vec())
                } else {
                    s.gets.lock().unwrap().push(path.clone());
                    let p = path.split('?').next().unwrap_or("").to_string();
                    match s.files.lock().unwrap().iter().find(|(k, _)| *k == p) {
                        Some((_, b)) => (200, b.clone()),
                        None => (404, b"not found".to_vec()),
                    }
                };
                let _ = write!(
                    c,
                    "HTTP/1.0 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = c.write_all(&body);
            });
        }
    });
    (base, s)
}

fn sha256(p: &Path) -> String {
    let o = Command::new("sha256sum").arg(p).output().unwrap();
    String::from_utf8_lossy(&o.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn cmd(bin: &Path, home: &Path, base: &str) -> Command {
    let mut c = Command::new(bin);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().into_owned();
        if k.starts_with("JUSTRUST_")
            || k.starts_with("CARGO_")
            || k.starts_with("RUSTC")
            || k == "DO_NOT_TRACK"
            || k == "CI"
        {
            c.env_remove(k);
        }
    }
    c.env("JUSTRUST_HOME", home)
        .env("JUSTRUST_UPDATE_URL", format!("{base}/v1/release/latest"))
        .env("JUSTRUST_TELEMETRY_URL", format!("{base}/v1/telemetry"))
        .env("JUSTRUST_BUILD_BASE", format!("{base}/v1"))
        // Keep the account checks offline.
        .env("JCODE_API_BASE", format!("{base}/api"))
        .env_remove("JCODE_API_KEY");
    c
}

/// A "new release": a script that answers `--version` like justrust 9.9.9,
/// packaged like the release workflow does.
fn publish(dir: &Path, s: &Server, base: &str, sha_override: Option<&str>) {
    let target = "x86_64-unknown-linux-musl";
    let pkg = dir.join(format!("justrust-{target}"));
    std::fs::create_dir_all(&pkg).unwrap();
    let script = pkg.join("justrust");
    std::fs::write(&script, "#!/bin/sh\necho 'justrust 9.9.9'\n").unwrap();
    let tgz = dir.join(format!("justrust-{target}.tar.gz"));
    assert!(
        Command::new("tar")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(dir)
            .arg(format!("justrust-{target}"))
            .status()
            .unwrap()
            .success()
    );
    let sum = sha_override
        .map(str::to_string)
        .unwrap_or_else(|| sha256(&tgz));
    let asset = format!("/dl/justrust-{target}.tar.gz");
    let info = format!(
        r#"{{"version":"9.9.9","tag":"v9.9.9","update":true,"enabled":true,"url":"{base}{asset}","sha256_url":"{base}{asset}.sha256"}}"#
    );
    let mut f = s.files.lock().unwrap();
    f.clear();
    f.push(("/v1/release/latest".into(), info.into_bytes()));
    f.push((asset.clone(), std::fs::read(&tgz).unwrap()));
    f.push((
        format!("{asset}.sha256"),
        format!("{sum}  justrust-{target}.tar.gz\n").into_bytes(),
    ));
}

#[test]
fn update_replaces_the_binary_after_verifying_it() {
    if !cfg!(target_arch = "x86_64") {
        return;
    }
    let d = scratch("update");
    let (base, s) = serve();
    let bin_dir = d.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("justrust");
    std::fs::copy(env!("CARGO_BIN_EXE_justrust"), &bin).unwrap();
    let home = d.join("home");

    // A corrupted download is refused and the binary is untouched.
    publish(&d.join("bad"), &s, &base, Some(&"0".repeat(64)));
    let before = sha256(&bin);
    let o = cmd(&bin, &home, &base)
        .args(["update", "--force"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "bad checksum must fail");
    assert!(err.contains("checksum mismatch"), "{err}");
    assert_eq!(sha256(&bin), before);
    assert!(
        std::fs::read_dir(&bin_dir).unwrap().count() == 1,
        "temp files left behind"
    );

    // A good one replaces it.
    publish(&d.join("good"), &s, &base, None);
    let o = cmd(&bin, &home, &base)
        .args(["update", "--force"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains("installed 9.9.9"), "{out}");
    let v = Command::new(&bin).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), "justrust 9.9.9");
    let q = s.gets.lock().unwrap().clone();
    assert!(
        q.iter()
            .any(|p| p.contains("version=") && p.contains("target=")),
        "{q:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

fn wait_for(f: impl Fn() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(20) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn telemetry_is_sent_in_the_background_and_stops_when_off() {
    let d = scratch("telemetry");
    let (base, s) = serve();
    let home = d.join("home");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_justrust"));
    let run = |args: &[&str]| {
        let o = cmd(&bin, &home, &base)
            .env("JUSTRUST_TELEMETRY_FLUSH_SECS", "0")
            .env("JUSTRUST_NO_UPDATE", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        o
    };
    let o = run(&["runs", "-n", "1"]);
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("justrust telemetry off"),
        "first use explains telemetry and how to turn it off"
    );
    assert!(
        wait_for(|| !s.posts.lock().unwrap().is_empty()),
        "no telemetry batch arrived"
    );
    let (path, body) = s.posts.lock().unwrap()[0].clone();
    assert_eq!(path, "/v1/telemetry");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = v["install_id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(
        std::fs::read_to_string(home.join("install_id"))
            .unwrap()
            .trim(),
        id
    );
    assert!(v["client"].as_str().unwrap().starts_with("justrust "));
    assert_eq!(v["events"][0]["kind"], "command");
    assert_eq!(v["events"][0]["data"]["name"], "runs");
    assert!(!body.contains(&*home.to_string_lossy()), "no paths: {body}");
    // The queue was sent, not kept.
    assert!(wait_for(|| !home.join("telemetry/sending.jsonl").exists()));

    // Off: nothing queued, nothing sent, and status says why.
    run(&["telemetry", "off"]);
    let sent = s.posts.lock().unwrap().len();
    run(&["runs", "-n", "1"]);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(s.posts.lock().unwrap().len(), sent);
    assert!(!home.join("telemetry/queue.jsonl").exists());
    let st = run(&["telemetry", "status"]);
    assert!(String::from_utf8_lossy(&st.stdout).contains("telemetry: off"));

    // Environment opt-outs win over the setting.
    run(&["telemetry", "on"]);
    let st = cmd(&bin, &home, &base)
        .env("DO_NOT_TRACK", "1")
        .args(["telemetry", "status"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&st.stdout).contains("off (DO_NOT_TRACK is set)"),
        "{}",
        String::from_utf8_lossy(&st.stdout)
    );
    let _ = std::fs::remove_dir_all(&d);
}
