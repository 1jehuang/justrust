//! `justrust __agent`: the remote half of the sync daemon.
//!
//! Started by the local daemon over one long-lived ssh session. Reads frames
//! from stdin and writes frames to stdout:
//!
//! - `Write`/`Delete`/`Mkdir` are applied in order (atomic rename for
//!   writes), so a `Barrier` acknowledged after them means the tree matches.
//! - `Run` starts `justrust <args>` in `cwd` and streams its output back as
//!   `Out` frames, then `Exit` with the run's summary.json as payload.
//!   Several runs may be in flight (parallel agents).
//! - `Cancel` sends SIGINT to a run's process group.
//!
//! Paths arrive absolute. With a non-empty prefix (a machine where the local
//! paths cannot be created) every path is rebased under it.

use anyhow::{Result, bail};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufWriter, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use crate::remote_proto::{MIN_VERSION, Msg, VERSION, read_frame, write_frame};

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// Process groups of runs in flight, for the signal handler: a killed
/// agent (`pkill`, a dropped session's SIGHUP) takes its builds with it
/// instead of leaving them to hold the cargo lock against the next agent.
static RUN_PGIDS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

fn track_pgid(pid: i32) {
    for slot in &RUN_PGIDS {
        if slot
            .compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

fn untrack_pgid(pid: i32) {
    for slot in &RUN_PGIDS {
        let _ = slot.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

extern "C" fn on_fatal_signal(sig: libc::c_int) {
    for slot in &RUN_PGIDS {
        let pid = slot.load(Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: kill is async-signal-safe.
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }
    // SAFETY: _exit is async-signal-safe.
    unsafe { libc::_exit(128 + sig) }
}

fn install_signal_handlers() {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
        // SAFETY: the handler only calls async-signal-safe functions.
        unsafe {
            libc::signal(sig, on_fatal_signal as *const () as libc::sighandler_t);
        }
    }
}

fn send(out: &Out, m: &Msg, payload: &[u8]) {
    if let Ok(mut w) = out.lock() {
        let _ = write_frame(&mut *w, m, payload);
    }
}

fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.jr-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let res = std::fs::write(&tmp, data)
        .and_then(|()| {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode & 0o777))
        })
        .and_then(|()| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    }
}

pub fn main(prefix: Option<String>) -> ! {
    install_signal_handlers();
    let out: Box<dyn Write + Send> = Box::new(BufWriter::new(std::io::stdout()));
    let code = match serve(std::io::stdin().lock(), out, prefix.unwrap_or_default()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("justrust agent: {e:#}");
            1
        }
    };
    std::process::exit(code)
}

/// Serve one daemon connection until it closes. Builds still running when
/// it does are interrupted: nobody is left to read their output, and they
/// would hold the cargo lock against the next connection's builds.
pub fn serve<R: Read>(mut input: R, out: Box<dyn Write + Send>, prefix: String) -> Result<()> {
    let out: Out = Arc::new(Mutex::new(out));
    // Older daemons still speak v1, a subset of v2 (they never send
    // `Synced` and ignore the new fields): serve them in their version so a
    // machine shared by two justrust builds keeps working for both.
    let version = match read_frame(&mut input)? {
        Some((Msg::Hello { version }, _)) if (MIN_VERSION..=VERSION).contains(&version) => version,
        Some((Msg::Hello { version }, _)) => {
            send(
                &out,
                &Msg::Error {
                    id: 0,
                    msg: format!("protocol {version}, agent speaks {MIN_VERSION} to {VERSION}"),
                },
                &[],
            );
            bail!("protocol mismatch");
        }
        _ => bail!("expected Hello"),
    };
    send(&out, &Msg::Hello { version }, &[]);
    let at = |p: &str| -> PathBuf {
        if prefix.is_empty() {
            PathBuf::from(p)
        } else {
            Path::new(&prefix).join(p.trim_start_matches('/'))
        }
    };
    let pids: Arc<Mutex<HashMap<u64, i32>>> = Arc::default();
    // Paths whose latest Write or Delete failed: the tree differs from what
    // the daemon believes. Reported with every barrier until a later frame
    // for the same path succeeds, so no build runs on a stale file.
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let res = loop {
        let (msg, payload) = match read_frame(&mut input) {
            Ok(Some(f)) => f,
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        };
        match msg {
            Msg::Write { path, mode } => match write_atomic(&at(&path), &payload, mode) {
                Ok(()) => {
                    failed.remove(&path);
                }
                Err(e) => {
                    eprintln!("justrust agent: write {path}: {e}");
                    failed.insert(path);
                }
            },
            Msg::Delete { path } => match remove(&at(&path)) {
                Ok(()) => {
                    failed.remove(&path);
                }
                Err(e) => {
                    eprintln!("justrust agent: delete {path}: {e}");
                    failed.insert(path);
                }
            },
            Msg::Mkdir { path } => {
                let _ = std::fs::create_dir_all(at(&path));
            }
            Msg::Synced { path } => {
                let under = format!("{}/", path.trim_end_matches('/'));
                failed.retain(|p| !p.starts_with(&under));
            }
            Msg::Barrier { id } => send(
                &out,
                &Msg::BarrierAck {
                    id,
                    failed: failed.iter().cloned().collect(),
                },
                &[],
            ),
            Msg::Run {
                id,
                cwd,
                args,
                env,
                run_id,
                ..
            } => {
                let out = out.clone();
                let pids = pids.clone();
                let cwd = at(&cwd);
                std::thread::spawn(move || {
                    let code = match start_run(id, &cwd, &args, &env, &run_id, &out, &pids) {
                        Ok(c) => c,
                        Err(e) => {
                            send(
                                &out,
                                &Msg::Error {
                                    id,
                                    msg: format!("{e:#}"),
                                },
                                &[],
                            );
                            return;
                        }
                    };
                    let summary = summary_for(&run_id).unwrap_or_default();
                    send(&out, &Msg::Exit { id, code }, &summary);
                });
            }
            Msg::Cancel { id } => {
                if let Some(pid) = pids.lock().ok().and_then(|p| p.get(&id).copied()) {
                    // SAFETY: signalling our own child's process group.
                    unsafe {
                        libc::kill(-pid, libc::SIGINT);
                    }
                }
            }
            Msg::Ping => send(
                &out,
                &Msg::Pong {
                    roots: Vec::new(),
                    machine: String::new(),
                    pushed: 0,
                    connected: true,
                    build: String::new(),
                    active: pids.lock().map(|p| p.len() as u64).unwrap_or(0),
                    skipped: 0,
                },
                &[],
            ),
            _ => {}
        }
    };
    if let Ok(p) = pids.lock() {
        for pid in p.values() {
            // SAFETY: signalling our own child's process group.
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }
    res
}

fn summary_for(run_id: &str) -> Option<Vec<u8>> {
    if run_id.is_empty() {
        return None;
    }
    std::fs::read(
        crate::paths::runs_dir()
            .ok()?
            .join(run_id)
            .join("summary.json"),
    )
    .ok()
}

fn start_run(
    id: u64,
    cwd: &Path,
    args: &[String],
    env: &[(String, String)],
    run_id: &str,
    out: &Out,
    pids: &Arc<Mutex<HashMap<u64, i32>>>,
) -> Result<i32> {
    let me = std::env::current_exe()?;
    let mut c = Command::new(me);
    c.args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if !run_id.is_empty() {
        c.env("JUSTRUST_RUN_ID_OVERRIDE", run_id);
    }
    // cargo and rustup live in ~/.cargo/bin, which a non-login ssh shell
    // may not have on PATH.
    if let Some(home) = dirs::home_dir() {
        let path = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", format!("{}/.cargo/bin:{path}", home.display()));
    }
    let mut child = c.spawn()?;
    pids.lock().unwrap().insert(id, child.id() as i32);
    track_pgid(child.id() as i32);
    let mut pumps = Vec::new();
    for (stream, pipe) in [
        (
            1u8,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            2u8,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ] {
        let Some(mut pipe) = pipe else { continue };
        let out = out.clone();
        pumps.push(std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 << 10];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                send(&out, &Msg::Out { id, stream }, &buf[..n]);
            }
        }));
    }
    for p in pumps {
        let _ = p.join();
    }
    let st = child.wait()?;
    pids.lock().unwrap().remove(&id);
    untrack_pgid(child.id() as i32);
    use std::os::unix::process::ExitStatusExt;
    Ok(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)))
}
