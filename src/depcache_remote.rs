//! Remote tier of the dependency cache: economies of scale across machines
//! and users. Server: the private justrust-cloud worker (`build.jcode.sh`,
//! `src/cache.js` there). This file defines the client half of the wire
//! format (MIT): manifest lines are depcache `Entry` JSON, objects are
//! bundles (below).
//!
//! ```text
//! GET  /v1/build/cache/m/<base>              {private: [line], shared: [line]}
//! GET  /v1/build/cache/o/<private|shared>/<out>   bundle
//! PUT  /v1/build/cache/o/<out>[?tier=shared]      bundle
//! POST /v1/build/cache/m/<base>[?tier=shared]     one line
//! ```
//!
//! Two tiers:
//! - shared: built only by justrust's prebuild machines from crates.io
//!   sources and readable by every signed-in account. Users can never write
//!   it (the server requires `X-Cache-Publish-Token`), so one user cannot
//!   poison another's build.
//! - private: the account's own uploads, read back on its other machines,
//!   checkouts, CI, and hosted build hosts.
//!
//! Lookup happens only after a local miss, for the same eligible units as the
//! local cache (registry and git crates). An entry is used only if every
//! input file it lists has the same content hash here, exactly like a local
//! entry. Downloaded bundles are unpacked into the local cache and then
//! restored by the normal local path, so remote hits stay local afterwards.
//!
//! Uploads never slow a build: a successful local store appends to
//! `upload.jsonl` and a detached `justrust cache --sync` uploads after the run.
//!
//! Fail open, always: any error means "compile normally". A failing server
//! (down, signed out, cache disabled) trips a breaker so the next 10 minutes
//! of builds skip remote lookups entirely.
//!
//! Env: `JUSTRUST_REMOTE_CACHE=0` off, `=read` lookups only (no uploads).
//! Prebuild fleet: `JUSTRUST_CACHE_PUBLISH_TOKEN` uploads to the shared tier.
//!
//! Bundle: gzip of `JRDC1\n` then per file `u32 name_len, name, u32 mode,
//! u64 len, bytes` (little endian), `meta.json` included.

use crate::depcache::{self, Entry, Outputs, Plan};
use crate::paths;
use crate::remote_hosted::{self, Creds};
use crate::sys::Detach;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAGIC: &[u8] = b"JRDC1\n";
const BREAKER_SECS: f64 = 600.0;
// Authenticated Worker and peer traffic retain the Workers request cap.
const MAX_BUNDLE: u64 = 95 * 1024 * 1024;
pub(crate) const PUBLIC_MAX_BUNDLE: u64 = 512 * 1024 * 1024;
pub(crate) const PUBLIC_MAX_EXPANDED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 64;
const MAX_META: u64 = 256 * 1024;

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
}

/// Publishing uses the same finite budgets as public reads, not Worker limits.
pub(crate) fn pack_public(dir: &Path, dst: &Path) -> Option<()> {
    let meta_path = dir.join("meta.json");
    if std::fs::metadata(&meta_path).ok()?.len() > MAX_META {
        return None;
    }
    let meta: Outputs = serde_json::from_slice(&std::fs::read(meta_path).ok()?).ok()?;
    if meta.files.len() > MAX_FILES {
        return None;
    }
    let mut expanded = MAGIC.len() as u64;
    let mut seen = std::collections::HashSet::new();
    for name in meta
        .files
        .iter()
        .map(|f| f.name.as_str())
        .chain(["meta.json"])
    {
        if !safe_name(name) || !seen.insert(name) {
            return None;
        }
        let m = std::fs::symlink_metadata(dir.join(name)).ok()?;
        if !m.is_file() {
            return None;
        }
        expanded = expanded
            .checked_add(16 + name.len() as u64)?
            .checked_add(m.len())?;
        if expanded > PUBLIC_MAX_EXPANDED {
            return None;
        }
    }
    pack_limited(dir, dst, PUBLIC_MAX_BUNDLE)?;
    (std::fs::metadata(dst).ok()?.len() <= PUBLIC_MAX_BUNDLE).then_some(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Off,
    Read,
    ReadWrite,
}

/// Off by default while the backend moves to AWS. Opt in with
/// `JUSTRUST_REMOTE_CACHE=1` (or `read`).
pub fn mode() -> Mode {
    if !depcache::enabled() {
        return Mode::Off;
    }
    match std::env::var("JUSTRUST_REMOTE_CACHE").as_deref() {
        Ok("1") | Ok("on") | Ok("true") | Ok("yes") => Mode::ReadWrite,
        Ok("read") | Ok("ro") => Mode::Read,
        _ => Mode::Off,
    }
}

fn publish_token() -> Option<String> {
    std::env::var("JUSTRUST_CACHE_PUBLISH_TOKEN")
        .ok()
        .filter(|t| {
            !t.is_empty()
                && t.chars()
                    .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
        })
}

fn breaker(root: &Path) -> PathBuf {
    root.join("remote.down")
}

fn tripped(root: &Path) -> bool {
    std::fs::metadata(breaker(root))
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|e| e.as_secs_f64() < BREAKER_SECS)
}

fn trip(root: &Path, why: &str) {
    let _ = std::fs::write(breaker(root), why);
}

// ---------------------------------------------------------------- transport

struct Client {
    creds: Creds,
    publish: Option<String>,
}

impl Client {
    fn new() -> Option<Client> {
        let publish = publish_token();
        let creds = match remote_hosted::creds() {
            Ok(c) => c,
            // The prebuild fleet publishes without a user account.
            Err(_) if publish.is_some() => {
                let mut c = Creds::anonymous(remote_hosted::api_base()).ok()?;
                c.set_build_base(remote_hosted::build_base()).ok()?;
                c
            }
            Err(_) => return None,
        };
        Some(Client { creds, publish })
    }

    /// One curl call. `body` is a file to upload. Returns (status, body file).
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Path>,
        out: &Path,
        max_time: u32,
    ) -> Option<u16> {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "--proto",
            "=https,http",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "3",
        ])
        .args(["--max-time", &max_time.to_string()])
        .args(["--max-filesize", &MAX_BUNDLE.to_string()])
        .args(["-X", method, "-o"])
        .arg(out)
        .args(["-w", "%{http_code}", "--config", "-"]);
        if let Some(b) = body {
            cmd.args([
                "-H",
                "Content-Type: application/octet-stream",
                "--data-binary",
            ])
            .arg(format!("@{}", b.display()));
        }
        cmd.arg(self.creds.url(path));
        let mut config = self.creds.curl_auth_config();
        if let Some(t) = &self.publish
            && method != "GET"
        {
            config.push_str(&format!("header = \"X-Cache-Publish-Token: {t}\"\n"));
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(config.as_bytes()).ok()?;
        let o = child.wait_with_output().ok()?;
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse()
            .ok()
            .filter(|c| *c != 0)
    }
}

fn scratch(root: &Path, tag: &str) -> PathBuf {
    let dir = root.join("tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{tag}.{}.{}", std::process::id(), rand_suffix()))
}

fn rand_suffix() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    t ^ N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ------------------------------------------------------------------- bundle

/// Pack `dir` (one `o/<out>` entry) into a gzip bundle at `dst`.
pub(crate) fn pack(dir: &Path, dst: &Path) -> Option<()> {
    pack_limited(dir, dst, u64::MAX)
}

struct BundleWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W: Write> Write for BundleWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() as u64 > self.remaining {
            return Err(std::io::Error::other("compressed bundle budget exceeded"));
        }
        let n = self.inner.write(buf)?;
        self.remaining -= n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn pack_limited(dir: &Path, dst: &Path, compressed: u64) -> Option<()> {
    let meta: Outputs = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    let mut names: Vec<String> = meta.files.iter().map(|f| f.name.clone()).collect();
    names.push("meta.json".into());
    let f = std::fs::File::create(dst).ok()?;
    let mut gz = flate2::write::GzEncoder::new(
        std::io::BufWriter::new(BundleWriter {
            inner: f,
            remaining: compressed,
        }),
        flate2::Compression::fast(),
    );
    gz.write_all(MAGIC).ok()?;
    for name in names {
        let p = dir.join(&name);
        let m = std::fs::metadata(&p).ok()?;
        gz.write_all(&(name.len() as u32).to_le_bytes()).ok()?;
        gz.write_all(name.as_bytes()).ok()?;
        gz.write_all(&(crate::sys::mode(&m, &p) & 0o777).to_le_bytes())
            .ok()?;
        gz.write_all(&m.len().to_le_bytes()).ok()?;
        std::io::copy(&mut std::fs::File::open(&p).ok()?, &mut gz).ok()?;
    }
    gz.finish().ok()?.flush().ok()
}

/// Unpack a bundle into a fresh `dst` dir, verifying names, the file list
/// against `meta.json`, and every binary output's content hash.
pub(crate) fn unpack(src: &Path, dst: &Path) -> Option<()> {
    unpack_limited(src, dst, MAX_BUNDLE, 4 * MAX_BUNDLE)
}

/// Static public artifacts bypass the Worker, but still have finite disk budgets.
pub(crate) fn unpack_public(src: &Path, dst: &Path) -> Option<()> {
    unpack_limited(src, dst, PUBLIC_MAX_BUNDLE, PUBLIC_MAX_EXPANDED)
}

fn unpack_limited(src: &Path, dst: &Path, compressed: u64, expanded: u64) -> Option<()> {
    let f = std::fs::File::open(src).ok()?;
    if f.metadata().ok()?.len() > compressed {
        return None;
    }
    // One extra byte distinguishes genuine EOF from exhaustion of the budget.
    let mut r = flate2::read::GzDecoder::new(std::io::BufReader::new(f)).take(expanded + 1);
    let mut magic = [0u8; 6];
    r.read_exact(&mut magic).ok()?;
    if magic != MAGIC {
        return None;
    }
    std::fs::create_dir_all(dst).ok()?;
    let mut seen = Vec::new();
    loop {
        let mut n = [0u8; 4];
        match r.read(&mut n[..1]) {
            Ok(0) => break,
            Ok(_) => r.read_exact(&mut n[1..]).ok()?,
            Err(_) => return None,
        }
        let n = u32::from_le_bytes(n) as usize;
        if n == 0 || n > 255 || seen.len() > MAX_FILES {
            return None;
        }
        let mut name = vec![0u8; n];
        r.read_exact(&mut name).ok()?;
        let name = String::from_utf8(name).ok()?;
        if !safe_name(&name) || seen.contains(&name) {
            return None;
        }
        let mut mode = [0u8; 4];
        r.read_exact(&mut mode).ok()?;
        let mut len = [0u8; 8];
        r.read_exact(&mut len).ok()?;
        let len = u64::from_le_bytes(len);
        if len > r.limit().saturating_sub(1) || (name == "meta.json" && len > MAX_META) {
            return None;
        }
        let p = dst.join(&name);
        let mut out = std::fs::File::create(&p).ok()?;
        if std::io::copy(&mut (&mut r).take(len), &mut out).ok()? != len {
            return None;
        }
        let mode = u32::from_le_bytes(mode) & 0o755;
        crate::sys::set_mode(&p, mode | 0o600).ok()?;
        seen.push(name);
    }
    if r.limit() == 0 {
        return None;
    }
    let meta: Outputs = serde_json::from_slice(&std::fs::read(dst.join("meta.json")).ok()?).ok()?;
    if seen.len() != meta.files.len() + 1 {
        return None;
    }
    let mut listed = std::collections::HashSet::new();
    for f in &meta.files {
        if f.name == "meta.json" || !listed.insert(&f.name) || !seen.contains(&f.name) {
            return None;
        }
        if !f.text && depcache::hash_file_content(&dst.join(&f.name))? != f.hash {
            return None;
        }
    }
    Some(())
}

// ------------------------------------------------------------------- lookup

/// After a local miss: find a matching remote entry, download and unpack it
/// into the local cache, then restore it locally. `None` on any failure.
pub fn fetch(plan: &Plan, err: &mut dyn Write) -> Option<depcache::Hit> {
    if mode() == Mode::Off || tripped(&plan.root) {
        return None;
    }
    let client = Client::new()?;
    // A publisher without a user account has nothing to read.
    if client.creds.curl_auth_config().is_empty() {
        return None;
    }
    let tmp = scratch(&plan.root, "m");
    let code = client.call(
        "GET",
        &format!("build/cache/m/{}", plan.base),
        None,
        &tmp,
        8,
    );
    let body = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    if code != Some(200) {
        trip(&plan.root, &format!("manifest lookup: {code:?}"));
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let mut memo = std::collections::HashMap::new();
    // Own uploads first, then the shared tier; newest first within each.
    let found = ["private", "shared"].into_iter().find_map(|tier| {
        v[tier].as_array()?.iter().rev().find_map(|l| {
            let line = l.as_str()?;
            let e: Entry = serde_json::from_str(line).ok()?;
            (valid_key(&e.out) && plan.matches(&e, &mut memo))
                .then(|| (tier, e.out, line.to_owned()))
        })
    });
    let (tier, out, line) = found?;
    let final_dir = plan.root.join("o").join(&out);
    if !final_dir.join("meta.json").is_file() {
        let bundle = scratch(&plan.root, "b");
        let code = client.call(
            "GET",
            &format!("build/cache/o/{tier}/{out}"),
            None,
            &bundle,
            120,
        );
        let stage = plan
            .root
            .join("o")
            .join(format!(".{out}.remote{}", std::process::id()));
        let ok = code == Some(200) && unpack(&bundle, &stage).is_some();
        let _ = std::fs::remove_file(&bundle);
        if !ok || std::fs::rename(&stage, &final_dir).is_err() {
            let _ = std::fs::remove_dir_all(&stage);
            if !final_dir.join("meta.json").is_file() {
                return None;
            }
        }
    }
    let known = std::fs::read_to_string(plan.manifest()).unwrap_or_default();
    if !known.lines().any(|l| l == line) {
        std::fs::create_dir_all(plan.root.join("m")).ok()?;
        paths::append_line(&plan.manifest(), &line).ok()?;
    }
    plan.restore(err)
}

fn valid_key(k: &str) -> bool {
    k.len() == 32
        && k.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ------------------------------------------------------------------- upload

fn queue(root: &Path) -> PathBuf {
    root.join("upload.jsonl")
}

/// After a successful local store: remember to upload it after the run.
pub fn enqueue(plan: &Plan, line: &str) {
    if mode() != Mode::ReadWrite || (!remote_hosted::signed_in() && publish_token().is_none()) {
        return;
    }
    let item = serde_json::json!({ "base": plan.base, "line": line });
    let _ = paths::append_line(&queue(&plan.root), &item.to_string());
}

/// Called after every recorded run: spawn a detached `justrust cache --sync`
/// when there is something to upload. Costs one `stat` otherwise.
pub fn maybe_spawn_sync() {
    let Some(root) = depcache::cache_dir() else {
        return;
    };
    if mode() != Mode::ReadWrite || tripped(&root) {
        return;
    }
    if !std::fs::metadata(queue(&root)).is_ok_and(|m| m.len() > 0) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args(["cache", "--sync"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .detached()
        .spawn();
}

#[derive(Debug, Default, PartialEq)]
pub struct Synced {
    pub uploaded: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Upload the queue. Single instance (lock). Items that fail with a server
/// or network error stay queued and the breaker trips.
pub fn sync(root: &Path) -> Option<Synced> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("upload.lock"))
        .ok()?;
    if lock.try_lock().is_err() {
        return None;
    }
    let client = Client::new()?;
    let tier = if client.publish.is_some() {
        "?tier=shared"
    } else {
        ""
    };
    // Take the queue atomically; new items keep appending to a fresh file.
    let taken = root.join(format!("upload.{}.jsonl", std::process::id()));
    std::fs::rename(queue(root), &taken).ok()?;
    let text = std::fs::read_to_string(&taken).unwrap_or_default();
    let mut s = Synced::default();
    let mut keep = Vec::new();
    let mut down = false;
    let mut done = std::collections::HashSet::new();
    for item in text.lines() {
        if down {
            keep.push(item.to_owned());
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(item) else {
            continue;
        };
        let (Some(base), Some(line)) = (v["base"].as_str(), v["line"].as_str()) else {
            continue;
        };
        let Ok(e) = serde_json::from_str::<Entry>(line) else {
            continue;
        };
        if !valid_key(base) || !valid_key(&e.out) || !done.insert(line.to_owned()) {
            continue;
        }
        let dir = root.join("o").join(&e.out);
        let bundle = scratch(root, "u");
        let sink = scratch(root, "r");
        let put = pack(&dir, &bundle).and_then(|_| {
            client.call(
                "PUT",
                &format!("build/cache/o/{}{tier}", e.out),
                Some(&bundle),
                &sink,
                300,
            )
        });
        let _ = std::fs::remove_file(&bundle);
        let res = match put {
            Some(200) => {
                let lf = scratch(root, "l");
                let ok = std::fs::write(&lf, line).ok().and_then(|_| {
                    client.call(
                        "POST",
                        &format!("build/cache/m/{base}{tier}"),
                        Some(&lf),
                        &sink,
                        30,
                    )
                });
                let _ = std::fs::remove_file(&lf);
                ok
            }
            other => other,
        };
        let _ = std::fs::remove_file(&sink);
        match res {
            Some(200) => s.uploaded += 1,
            // Evicted locally, too large, or over quota: drop it.
            None if !dir.is_dir() => s.skipped += 1,
            Some(400 | 409 | 413 | 507) => s.skipped += 1,
            other => {
                s.failed += 1;
                down = true;
                trip(root, &format!("upload: {other:?}"));
                keep.push(item.to_owned());
            }
        }
    }
    for k in keep {
        let _ = paths::append_line(&queue(root), &k);
    }
    let _ = std::fs::remove_file(&taken);
    let _ = std::fs::remove_dir_all(root.join("tmp"));
    Some(s)
}

/// One status line for `justrust cache`.
pub fn status_line(root: &Path) -> String {
    let queued = std::fs::read_to_string(queue(root))
        .map(|t| t.lines().count())
        .unwrap_or(0);
    let m = mode();
    let who = if publish_token().is_some() {
        "publishing to the shared tier"
    } else if remote_hosted::signed_in() {
        "signed in"
    } else {
        "signed out (run `justrust login --no-wait` for the shared cache)"
    };
    let state = match m {
        Mode::Off => "off (JUSTRUST_REMOTE_CACHE=0)".to_string(),
        _ if tripped(root) => format!(
            "paused after a server error ({}), retrying within 10 min",
            std::fs::read_to_string(breaker(root))
                .unwrap_or_default()
                .trim()
        ),
        Mode::Read => "lookups only (JUSTRUST_REMOTE_CACHE=read)".into(),
        Mode::ReadWrite => "lookups and uploads".into(),
    };
    format!("remote    {state}, {who}, {queued} queued for upload")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jr-dcr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry_dir(root: &Path) -> PathBuf {
        let dir = root.join("o/x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libx-abc.rlib"), b"rlib bytes").unwrap();
        std::fs::write(dir.join("build_script_build-abc"), b"\x7fELF").unwrap();
        crate::sys::set_mode(&dir.join("build_script_build-abc"), 0o755).unwrap();
        std::fs::write(
            dir.join("x-abc.d"),
            "@@JUSTRUST_PROFILE_DIR@@/deps/x: a.rs\n",
        )
        .unwrap();
        let h = |n: &str| depcache::hash_file_content(&dir.join(n)).unwrap();
        let meta = serde_json::json!({
            "files": [
                {"name": "build_script_build-abc", "hash": h("build_script_build-abc"), "text": false},
                {"name": "libx-abc.rlib", "hash": h("libx-abc.rlib"), "text": false},
                {"name": "x-abc.d", "hash": "whatever", "text": true},
            ],
            "stderr": [], "wall": 1.5,
        });
        std::fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
        dir
    }

    #[test]
    fn public_unpack_rejects_hostile_record_structure() {
        let root = tmpdir("records");
        let write = |records: Vec<(&str, u64, &[u8])>| {
            let p = root.join("records.gz");
            let mut gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&p).unwrap(),
                flate2::Compression::fast(),
            );
            gz.write_all(MAGIC).unwrap();
            for (name, len, bytes) in records {
                gz.write_all(&(name.len() as u32).to_le_bytes()).unwrap();
                gz.write_all(name.as_bytes()).unwrap();
                gz.write_all(&0o644u32.to_le_bytes()).unwrap();
                gz.write_all(&len.to_le_bytes()).unwrap();
                gz.write_all(bytes).unwrap();
            }
            gz.finish().unwrap();
            p
        };
        let out = root.join("out");
        for records in [
            vec![("x", u64::MAX, b"".as_slice())],
            vec![("meta.json", MAX_META + 1, b"".as_slice())],
            vec![("x", 1, b"x".as_slice()), ("x", 1, b"x".as_slice())],
        ] {
            assert!(unpack_public(&write(records), &out).is_none());
            let _ = std::fs::remove_dir_all(&out);
        }
        let names: Vec<_> = (0..MAX_FILES + 2).map(|i| format!("file{i}")).collect();
        assert!(
            unpack_public(
                &write(
                    names
                        .iter()
                        .map(|n| (n.as_str(), 0, b"".as_slice()))
                        .collect()
                ),
                &out
            )
            .is_none()
        );
        std::fs::remove_dir_all(&out).unwrap();
        let meta = serde_json::json!({"files": [
            {"name": "x", "hash": "unused", "text": true},
            {"name": "x", "hash": "unused", "text": true}
        ], "stderr": [], "wall": 1.0})
        .to_string();
        let records = vec![
            ("x", 0, b"".as_slice()),
            ("y", 0, b"".as_slice()),
            ("meta.json", meta.len() as u64, meta.as_bytes()),
        ];
        assert!(unpack_public(&write(records), &out).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_budgets_reject_oversize_and_exhaustion() {
        let root = tmpdir("budgets");
        let dir = entry_dir(&root);
        let b = root.join("b.gz");
        pack_public(&dir, &b).unwrap();
        let compressed = std::fs::metadata(&b).unwrap().len();
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(&b).unwrap())
            .read_to_end(&mut decoded)
            .unwrap();
        let expanded = decoded.len() as u64;
        assert!(unpack_limited(&b, &root.join("exact"), compressed, expanded).is_some());
        assert!(unpack_limited(&b, &root.join("compressed"), compressed - 1, expanded).is_none());
        assert!(!root.join("compressed").exists());
        let limited = root.join("pack-limited.gz");
        assert!(pack_limited(&dir, &limited, compressed - 1).is_none());
        assert!(std::fs::metadata(&limited).unwrap().len() < compressed);
        assert!(unpack_limited(&b, &root.join("expanded"), compressed, expanded - 1).is_none());
        // A budget ending exactly at a record boundary must not masquerade as EOF.
        let boundary = MAGIC.len() as u64 + 16 + "build_script_build-abc".len() as u64 + 4;
        assert!(unpack_limited(&b, &root.join("boundary"), compressed, boundary).is_none());
        let mut bytes = std::fs::read(&b).unwrap();
        let crc = bytes.len() - 8;
        bytes[crc] ^= 1;
        let corrupt = root.join("corrupt.gz");
        std::fs::write(&corrupt, &bytes).unwrap();
        assert!(unpack_public(&corrupt, &root.join("crc")).is_none());
        bytes.truncate(crc);
        std::fs::write(&corrupt, &bytes).unwrap();
        assert!(unpack_public(&corrupt, &root.join("truncated")).is_none());
        let oversized = root.join("oversized.gz");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(PUBLIC_MAX_BUNDLE + 1)
            .unwrap();
        assert!(unpack_public(&oversized, &root.join("oversized")).is_none());
        assert!(!root.join("oversized").exists());
        std::fs::File::create(dir.join("libx-abc.rlib"))
            .unwrap()
            .set_len(PUBLIC_MAX_EXPANDED)
            .unwrap();
        assert!(pack_public(&dir, &root.join("too-big.gz")).is_none());
        assert!(!root.join("too-big.gz").exists());
        std::fs::File::create(dir.join("meta.json"))
            .unwrap()
            .set_len(MAX_META + 1)
            .unwrap();
        assert!(pack_public(&dir, &root.join("meta-too-big.gz")).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_publish_rejects_unsafe_names_and_symlinks() {
        let root = tmpdir("publish-bad");
        let dir = entry_dir(&root);
        for name in [
            "../escape",
            "..\\escape",
            "C:escape",
            ".hidden",
            "meta.json",
            "x\0y",
        ] {
            let meta = serde_json::json!({"files": [{"name": name, "hash": "x", "text": false}], "stderr": [], "wall": 1.0});
            std::fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
            assert!(pack_public(&dir, &root.join("bad.gz")).is_none(), "{name}");
        }
        #[cfg(unix)]
        {
            let dir = entry_dir(&root);
            std::fs::remove_file(dir.join("libx-abc.rlib")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", dir.join("libx-abc.rlib")).unwrap();
            assert!(pack_public(&dir, &root.join("link.gz")).is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_round_trip_keeps_bytes_and_modes() {
        let root = tmpdir("rt");
        let dir = entry_dir(&root);
        let b = root.join("b.gz");
        pack(&dir, &b).unwrap();
        let out = root.join("out");
        unpack(&b, &out).unwrap();
        for n in [
            "libx-abc.rlib",
            "build_script_build-abc",
            "x-abc.d",
            "meta.json",
        ] {
            assert_eq!(
                std::fs::read(out.join(n)).unwrap(),
                std::fs::read(dir.join(n)).unwrap(),
                "{n}"
            );
        }
        let p = out.join("build_script_build-abc");
        let mode = crate::sys::mode(&std::fs::metadata(&p).unwrap(), &p);
        #[cfg(unix)]
        assert_eq!(mode & 0o111, 0o111);
        let _ = mode;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unpack_rejects_tampered_or_hostile_bundles() {
        let root = tmpdir("bad");
        let dir = entry_dir(&root);
        // A binary whose content does not match meta.json.
        std::fs::write(dir.join("libx-abc.rlib"), b"evil").unwrap();
        let b = root.join("b.gz");
        pack(&dir, &b).unwrap();
        assert!(unpack(&b, &root.join("o1")).is_none());
        assert!(unpack_public(&b, &root.join("public-tampered")).is_none());

        // Path traversal in a file name.
        let write = |name: &str| {
            let p = root.join(format!("{}.gz", name.len()));
            let mut gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&p).unwrap(),
                flate2::Compression::fast(),
            );
            gz.write_all(MAGIC).unwrap();
            gz.write_all(&(name.len() as u32).to_le_bytes()).unwrap();
            gz.write_all(name.as_bytes()).unwrap();
            gz.write_all(&0o644u32.to_le_bytes()).unwrap();
            gz.write_all(&1u64.to_le_bytes()).unwrap();
            gz.write_all(b"x").unwrap();
            gz.finish().unwrap();
            p
        };
        assert!(unpack(&write("../escape"), &root.join("o2")).is_none());
        assert!(!root.join("escape").exists());
        assert!(unpack(&write(".hidden"), &root.join("o3")).is_none());
        for name in ["../escape", "..\\escape", "C:escape", ".hidden", "x\0y"] {
            assert!(unpack_public(&write(name), &root.join("public-path")).is_none());
        }
        // No meta.json at all.
        assert!(unpack(&write("lonely"), &root.join("o4")).is_none());
        // Not a bundle.
        std::fs::write(root.join("junk"), b"not gzip").unwrap();
        assert!(unpack(&root.join("junk"), &root.join("o5")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keys_are_lowercase_hex_128() {
        assert!(valid_key(&"a".repeat(32)));
        assert!(!valid_key(&"A".repeat(32)));
        assert!(!valid_key("../../etc/passwd0000000000000000"));
        assert!(!valid_key(&"a".repeat(31)));
    }

    #[test]
    fn breaker_trips_and_expires() {
        let root = tmpdir("br");
        assert!(!tripped(&root));
        trip(&root, "503");
        assert!(tripped(&root));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(700);
        std::fs::File::options()
            .write(true)
            .open(breaker(&root))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!tripped(&root));
        let _ = std::fs::remove_dir_all(&root);
    }
}
