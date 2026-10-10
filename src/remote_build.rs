//! Run a cargo command on the remote compile machine.
//!
//! `justrust remote check|test|build|clippy|run <cargo args>`:
//!
//! 1. Find every source root the build reads: the workspace plus each path
//!    dependency outside it (for Jcode Desktop, the sibling `~/jcode`), each
//!    widened to its git repository root. Cached per workspace and lockfile.
//! 2. rsync those roots to the *same absolute paths* on the machine, so
//!    relative path dependencies resolve and every diagnostic path matches
//!    the local tree. `.git`, `target/`, gitignored files, and files over
//!    8 MB (demo videos) are skipped. One persistent ssh connection is shared
//!    by every call.
//! 3. Keep justrust itself in sync on the machine and run
//!    `justrust <sub> <args>` there, streaming its compact output back.
//!
//! Target dirs and the cargo registry stay warm on the machine's disk
//! between builds and across stops.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::remote;

/// Where this binary's own source lives, so the machine can build the same
/// justrust. Compile-time path of the checkout `cargo install` used.
const JUSTRUST_SRC: &str = env!("CARGO_MANIFEST_DIR");
const MAX_FILE: &str = "8M";

/// Every directory rsync must mirror for a build started in `cwd`.
pub fn source_roots(cwd: &Path) -> Result<Vec<PathBuf>> {
    let cache_key = roots_cache_key(cwd);
    let cache = remote::dir()?.join("roots.json");
    if let Some(key) = &cache_key
        && let Ok(b) = std::fs::read(&cache)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b)
        && let Some(list) = v.get(key).and_then(|l| l.as_array())
    {
        let roots: Vec<PathBuf> = list
            .iter()
            .filter_map(|p| p.as_str().map(PathBuf::from))
            .collect();
        if !roots.is_empty() && roots.iter().all(|r| r.is_dir()) {
            return Ok(roots);
        }
    }
    let metadata = |offline: bool| {
        let mut c = Command::new("cargo");
        c.args(["metadata", "--format-version", "1"]);
        if offline {
            c.arg("--offline");
        }
        c.current_dir(cwd)
            .stderr(Stdio::piped())
            .output()
            .context("cargo metadata")
    };
    let mut out = metadata(true)?;
    if !out.status.success() {
        out = metadata(false)?;
    }
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let m: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut dirs = BTreeSet::new();
    if let Some(ws) = m["workspace_root"].as_str() {
        dirs.insert(repo_root(Path::new(ws)));
    }
    for p in m["packages"].as_array().into_iter().flatten() {
        if p["source"].is_null()
            && let Some(manifest) = p["manifest_path"].as_str()
            && let Some(d) = Path::new(manifest).parent()
        {
            dirs.insert(repo_root(d));
        }
    }
    let roots = outermost(dirs);
    if let Some(key) = cache_key {
        let mut v: serde_json::Value = std::fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        v[key] = serde_json::json!(roots);
        let _ = std::fs::write(&cache, v.to_string());
    }
    Ok(roots)
}

/// Workspace root + lockfile mtime: path dependencies only change when the
/// lockfile does.
fn roots_cache_key(cwd: &Path) -> Option<String> {
    let mut d = Some(cwd);
    let mut lock = None;
    while let Some(dir) = d {
        let l = dir.join("Cargo.lock");
        if l.exists() {
            lock = Some(l);
        }
        d = dir.parent();
    }
    let lock = lock?;
    let mtime = std::fs::metadata(&lock)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}@{mtime}", lock.display()))
}

fn repo_root(d: &Path) -> PathBuf {
    let mut cur = Some(d);
    while let Some(c) = cur {
        if c.join(".git").exists() {
            return c.to_path_buf();
        }
        cur = c.parent();
    }
    d.to_path_buf()
}

/// Drop roots nested inside another root.
fn outermost(dirs: BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.iter().any(|o| d.starts_with(o)) {
            out.retain(|o| !o.starts_with(&d));
            out.push(d);
        }
    }
    out
}

pub struct SyncResult {
    pub files: usize,
    pub bytes: u64,
    pub names: Vec<PathBuf>,
}

fn rsync(ip: &str, root: &Path) -> Result<SyncResult> {
    let ssh = remote::ssh_command_line(ip)?;
    let src = format!("{}/", root.display());
    let dst = format!("{}@{ip}:{}/", remote::SSH_USER, root.display());
    let out = Command::new("rsync")
        .args([
            "-a",
            // Remove files deleted locally, but never the machine's own
            // build output (excluded paths are left alone without
            // --delete-excluded, and target dirs are also protected).
            "--delete",
            "-z",
            "--out-format=%i %l %n",
            &format!("--max-size={MAX_FILE}"),
            "--filter=P target/",
            "--filter=P .justrust/",
            "--exclude=.git",
            "--exclude=target/",
            "--exclude=.justrust/",
            "--filter=:- .gitignore",
            "-e",
            &ssh,
            &src,
            &dst,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("rsync (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "rsync {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut r = SyncResult {
        files: 0,
        bytes: 0,
        names: Vec::new(),
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(flags), Some(len), Some(name)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if flags.starts_with("<f") {
            r.files += 1;
            r.bytes += len.parse::<u64>().unwrap_or(0);
            r.names.push(root.join(name));
        }
    }
    Ok(r)
}

/// Make each root's parent exist and belong to the ssh user (once per root).
fn prepare_roots(ip: &str, roots: &[PathBuf]) -> Result<()> {
    let stamp = remote::dir()?.join("prepared.json");
    let mut done: BTreeSet<String> = std::fs::read(&stamp)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let instance = remote::instance_id().unwrap_or_default();
    let todo: Vec<String> = roots
        .iter()
        .map(|r| r.display().to_string())
        .filter(|r| !done.contains(&format!("{instance}:{r}")))
        .collect();
    if todo.is_empty() {
        return Ok(());
    }
    let mut script = String::new();
    for r in &todo {
        script.push_str(&format!(
            "sudo install -d -o {u} -g {u} {q} && ",
            u = remote::SSH_USER,
            q = shell_quote(r)
        ));
    }
    script.push_str("true");
    let st = remote::ssh_base(ip)?.arg(script).status()?;
    if !st.success() {
        bail!("could not create source dirs on the remote machine");
    }
    done.extend(todo.into_iter().map(|r| format!("{instance}:{r}")));
    std::fs::write(&stamp, serde_json::to_vec(&done)?)?;
    Ok(())
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-=:,+@%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Identity of the running binary: reinstall remotely only when it changes,
/// not whenever justrust's source tree (maybe the workspace being built) is
/// edited.
fn local_binary_id() -> String {
    std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}-{mtime}", m.len())
        })
        .unwrap_or_default()
}

/// Make sure the machine runs the same justrust as this one, building it
/// from the synced source when the local binary changed.
fn ensure_remote_justrust(ip: &str) -> Result<()> {
    let id = local_binary_id();
    let stamp = remote::dir()?.join("remote-justrust");
    let instance = remote::instance_id().unwrap_or_default();
    let want = format!("{instance} {id}");
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        return Ok(());
    }
    let src = PathBuf::from(JUSTRUST_SRC);
    if !src.join("Cargo.toml").exists() {
        bail!("justrust source not found at {JUSTRUST_SRC}");
    }
    prepare_roots(ip, std::slice::from_ref(&src))?;
    rsync(ip, &src)?;
    eprintln!("justrust remote: installing justrust on the machine...");
    let t = Instant::now();
    let cmd = format!(
        ". ~/.cargo/env; cd {q} && CARGO_TARGET_DIR=~/.justrust-install-target cargo install --path . --locked -q",
        q = shell_quote(JUSTRUST_SRC)
    );
    let st = remote::ssh_base(ip)?.arg(cmd).status()?;
    if !st.success() {
        bail!("installing justrust on the remote machine failed");
    }
    std::fs::write(&stamp, want)?;
    eprintln!(
        "justrust remote: installed in {:.1}s",
        t.elapsed().as_secs_f64()
    );
    Ok(())
}

pub fn run(sub: &str, args: Vec<String>) -> Result<()> {
    let total = Instant::now();
    let cwd = std::env::current_dir()?;
    let ip = remote::ensure_running()?;

    let t = Instant::now();
    let roots = source_roots(&cwd)?;
    if !roots.iter().any(|r| cwd.starts_with(r)) {
        bail!("{} is not inside a synced source root", cwd.display());
    }
    let t_roots = t.elapsed().as_secs_f64();

    let t = Instant::now();
    prepare_roots(&ip, &roots)?;
    // Sync every root in parallel, plus justrust itself.
    let results: Vec<Result<SyncResult>> = std::thread::scope(|s| {
        let handles: Vec<_> = roots
            .iter()
            .map(|r| {
                let ip = ip.clone();
                s.spawn(move || rsync(&ip, r))
            })
            .collect();
        let js = s.spawn(|| ensure_remote_justrust(&ip));
        let out = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| bail!("rsync thread panicked")))
            .collect();
        js.join()
            .unwrap_or_else(|_| bail!("install thread panicked"))
            .map(|_| out)
    })?;
    let mut files = 0;
    let mut bytes = 0;
    let mut names = Vec::new();
    for r in results {
        let r = r?;
        files += r.files;
        bytes += r.bytes;
        names.extend(r.names);
    }
    let t_sync = t.elapsed().as_secs_f64();
    let shown: Vec<String> = names
        .iter()
        .take(3)
        .map(|n| n.strip_prefix(&cwd).unwrap_or(n).display().to_string())
        .collect();
    eprintln!(
        "justrust remote: synced {} root{} ({files} file{} changed, {}{}) in {:.2}s{}",
        roots.len(),
        if roots.len() == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" },
        human_bytes(bytes),
        if shown.is_empty() {
            String::new()
        } else {
            format!(
                ": {}{}",
                shown.join(", "),
                if names.len() > 3 { ", ..." } else { "" }
            )
        },
        t_sync,
        if t_roots > 0.05 {
            format!(", found roots in {t_roots:.2}s")
        } else {
            String::new()
        }
    );

    let mut cmd = format!(
        ". ~/.cargo/env; cd {} && ",
        shell_quote(&cwd.to_string_lossy())
    );
    for var in ["JUSTRUST_MAX_WARNINGS", "RUST_BACKTRACE", "RUST_LOG"] {
        if let Ok(v) = std::env::var(var) {
            cmd.push_str(&format!("{var}={} ", shell_quote(&v)));
        }
    }
    cmd.push_str("exec justrust ");
    cmd.push_str(sub);
    for a in &args {
        cmd.push(' ');
        cmd.push_str(&shell_quote(a));
    }
    let t = Instant::now();
    // A tty makes Ctrl+C reach the remote cargo. Only when we have one.
    let interactive = unsafe { libc::isatty(1) == 1 && libc::isatty(0) == 1 };
    let mut c = remote::ssh_base(&ip)?;
    if interactive {
        c.arg("-t");
    }
    let mut child = c
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    // Stream line by line (agents read our stdout).
    if let Some(out) = child.stdout.take() {
        use std::io::Write;
        let mut so = std::io::stdout().lock();
        for line in BufReader::new(out).split(b'\n') {
            let mut line = line?;
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            so.write_all(&line)?;
            so.write_all(b"\n")?;
            so.flush()?;
        }
    }
    let st = child.wait()?;
    eprintln!(
        "justrust remote: {} on {} in {:.1}s (sync {:.2}s, remote {:.1}s)",
        if st.success() { "ok" } else { "failed" },
        remote::describe_short().unwrap_or_else(|| ip.clone()),
        total.elapsed().as_secs_f64(),
        t_sync + t_roots,
        t.elapsed().as_secs_f64()
    );
    std::process::exit(st.code().unwrap_or(1));
}

fn human_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{b} B")
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / 1048576.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outermost_drops_nested() {
        let d: BTreeSet<PathBuf> = ["/a/b", "/a", "/c/d", "/c/e"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(
            outermost(d),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/c/d"),
                PathBuf::from("/c/e")
            ]
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("-p"), "-p");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
