//! Client side of remote builds: `justrust remote check|test|...` and the
//! automatic routing in `justrust check|test|...`.
//!
//! 1. Find every source root the build reads: the workspace plus each path
//!    dependency outside it (for Jcode Desktop, the sibling `~/jcode`), each
//!    widened to its git repository root. Cached per lockfile.
//! 2. Hand the run to the sync daemon (`remote_daemon`). It keeps those
//!    roots mirrored at the *same absolute paths* on the machine, pushing
//!    edits as they are saved, so by the time a build asks there is usually
//!    nothing left to send.
//! 3. The machine runs `justrust <sub> <args>` and its compact output
//!    streams back. The run is recorded locally too (`justrust runs`,
//!    `justrust show`), marked as remote, so routing can learn from it.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::remote_backend::{self, Target};
use crate::remote_proto::{Msg, read_frame, write_frame};

/// Every directory the machine must mirror for a build started in `cwd`.
pub fn source_roots(cwd: &Path) -> Result<Vec<PathBuf>> {
    let cache_key = roots_cache_key(cwd);
    let cache = crate::remote::dir()?.join("roots.json");
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
        let mut c = Command::new(crate::paths::real_cargo().unwrap_or_else(|_| "cargo".into()));
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

/// Lockfile path + mtime: path dependencies only change with the lockfile.
fn roots_cache_key(cwd: &Path) -> Option<String> {
    let lock = lockfile(cwd)?;
    let mtime = std::fs::metadata(&lock)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}@{mtime}", lock.display()))
}

/// The outermost Cargo.lock at or above `cwd` (the workspace's).
pub fn lockfile(cwd: &Path) -> Option<PathBuf> {
    let mut d = Some(cwd);
    let mut lock = None;
    while let Some(dir) = d {
        let l = dir.join("Cargo.lock");
        if l.exists() {
            lock = Some(l);
        }
        d = dir.parent();
    }
    lock
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

/// Make sure the machine runs this exact justrust binary. Same OS and
/// architecture (the common case): copy it. Otherwise build it there from
/// crates.io at this version.
pub fn ensure_remote_justrust(t: &Target) -> Result<()> {
    let id = crate::remote_daemon::build_id();
    let stamp = crate::remote::dir()?.join("remote-justrust");
    let want = format!("{} {id}", t.dest);
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        return Ok(());
    }
    let arch = t
        .command()
        .arg("uname -sm")
        .stdin(Stdio::null())
        .output()
        .context("ssh uname")?;
    let remote_arch = String::from_utf8_lossy(&arch.stdout).trim().to_string();
    let local_arch = format!(
        "{} {}",
        if cfg!(target_os = "linux") {
            "Linux"
        } else {
            std::env::consts::OS
        },
        std::env::consts::ARCH
    );
    let t0 = Instant::now();
    if remote_arch == local_arch {
        let exe = std::env::current_exe()?;
        let tmp = ".cargo/bin/.justrust.upload";
        let ok = Command::new("rsync")
            .args(["-z", "-e", &t.rsync_shell()])
            .arg(&exe)
            .arg(format!("{}:{tmp}", t.dest))
            .stdin(Stdio::null())
            .status()?
            .success()
            && t.command()
                .arg(format!(
                    "mkdir -p ~/.cargo/bin && chmod +x ~/{tmp} && mv ~/{tmp} ~/.cargo/bin/justrust"
                ))
                .stdin(Stdio::null())
                .status()?
                .success();
        if !ok {
            bail!("copying justrust to the remote machine failed");
        }
    } else {
        eprintln!("justrust remote: building justrust on the machine ({remote_arch})...");
        let st = t
            .command()
            .arg(format!(
                ". ~/.cargo/env; cargo install justrust --version {} --locked -q",
                env!("CARGO_PKG_VERSION")
            ))
            .status()?;
        if !st.success() {
            bail!("installing justrust on the remote machine failed");
        }
    }
    std::fs::write(&stamp, want)?;
    crate::remote::log_event(&format!(
        "installed justrust on {} in {:.1}s",
        t.dest,
        t0.elapsed().as_secs_f64()
    ));
    Ok(())
}

/// Why a remote attempt did not produce a build result. The router falls
/// back to a local build on any of these.
#[derive(Debug)]
pub struct Infra(pub String);

pub struct Outcome {
    pub code: i32,
    #[allow(dead_code)] // used by the router (in progress)
    pub run_id: String,
}

/// Run `justrust <sub> <args>` remotely. `Ok(Err(Infra))` means nothing was
/// built and the caller may build locally instead. Output is streamed to
/// our stdout/stderr as it arrives.
pub fn run_remote(
    sub: &str,
    args: &[String],
    start: bool,
) -> Result<std::result::Result<Outcome, Infra>> {
    let total = Instant::now();
    let cwd = std::env::current_dir()?;
    let infra = |m: String| Ok(Err(Infra(m)));
    let roots = match source_roots(&cwd) {
        Ok(r) => r,
        Err(e) => return infra(format!("{e:#}")),
    };
    if !roots.iter().any(|r| cwd.starts_with(r)) {
        return infra(format!("{} is not inside a synced root", cwd.display()));
    }
    let sock = match crate::remote_daemon::connect() {
        Ok(s) => s,
        Err(e) => return infra(format!("{e:#}")),
    };
    let run_id = crate::record::new_run_id();
    let mut env = Vec::new();
    for var in [
        "JUSTRUST_MAX_WARNINGS",
        "RUST_BACKTRACE",
        "RUST_LOG",
        "JCODE_SESSION_ID",
        "CARGO_TERM_COLOR",
    ] {
        if let Ok(v) = std::env::var(var) {
            env.push((var.to_string(), v));
        }
    }
    let mut w = BufWriter::new(sock.try_clone()?);
    let mut r = BufReader::new(sock);
    let mut full = vec![sub.to_string()];
    full.extend(args.iter().cloned());
    write_frame(
        &mut w,
        &Msg::Run {
            id: 0,
            cwd: cwd.display().to_string(),
            args: full,
            env,
            roots: roots.iter().map(|r| r.display().to_string()).collect(),
            start,
            run_id: run_id.clone(),
        },
        &[],
    )?;
    let mut sync = None;
    let mut so = std::io::stdout();
    let mut se = std::io::stderr();
    loop {
        let frame = match read_frame(&mut r) {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) if sync.is_none() => {
                return infra("sync daemon closed the connection".into());
            }
            Ok(None) | Err(_) => {
                // Output already started: a local rerun would duplicate it.
                bail!("lost the remote build mid-run (see ~/.justrust/remote/daemon.log)");
            }
        };
        match frame {
            (
                Msg::Flushed {
                    files,
                    ms,
                    machine,
                    note,
                    ..
                },
                _,
            ) => {
                if !note.is_empty() {
                    eprintln!("justrust remote: {note}");
                }
                sync = Some((files, ms, machine));
            }
            (Msg::Out { stream, .. }, data) => {
                if stream == 1 {
                    so.write_all(&data)?;
                    so.flush()?;
                } else {
                    se.write_all(&data)?;
                    se.flush()?;
                }
            }
            (Msg::Exit { code, .. }, summary) => {
                let (files, ms, machine) = sync.unwrap_or((0, 0.0, String::new()));
                record_remote(
                    &run_id,
                    &summary,
                    &machine,
                    ms,
                    total.elapsed().as_secs_f64(),
                );
                eprintln!(
                    "justrust remote: {} on {machine} in {:.1}s (sync {:.0} ms, {files} file{} at build time)",
                    if code == 0 { "ok" } else { "failed" },
                    total.elapsed().as_secs_f64(),
                    ms,
                    if files == 1 { "" } else { "s" },
                );
                return Ok(Ok(Outcome { code, run_id }));
            }
            (Msg::Error { msg, .. }, _) if sync.is_none() => return infra(msg),
            (Msg::Error { msg, .. }, _) => bail!("remote build failed mid-run: {msg}"),
            _ => {}
        }
    }
}

/// Keep a local copy of the remote run so `justrust runs/show` and the
/// router see it. Marked `remote` in the index.
fn record_remote(run_id: &str, summary: &[u8], machine: &str, sync_ms: f64, wall: f64) {
    let Ok(mut s) = serde_json::from_slice::<crate::summary::Summary>(summary) else {
        return;
    };
    let Ok(dir) = crate::paths::runs_dir().map(|d| d.join(run_id)) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    s.remote = Some(crate::summary::RemoteInfo {
        machine: machine.to_string(),
        sync_ms,
        client_wall: wall,
    });
    let _ = std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&s).unwrap_or_default(),
    );
    if let (Ok(idx), Ok(line)) = (
        crate::paths::index_file(),
        serde_json::to_string(&s.index_entry()),
    ) {
        let _ = crate::paths::append_line(&idx, &line);
    }
}

/// `justrust remote check|test|...`: always remote (starting the machine if
/// it is stopped). Infrastructure failures are errors here, not fallbacks.
pub fn run(sub: &str, args: Vec<String>) -> Result<()> {
    if remote_backend::load().is_none() {
        bail!(
            "no remote machine configured. Create one with `justrust remote up`, or use one you have: `justrust remote use ssh user@host`"
        );
    }
    match run_remote(sub, &args, true)? {
        Ok(o) => std::process::exit(o.code),
        Err(Infra(m)) => bail!("remote build unavailable: {m}"),
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
