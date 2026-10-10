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
use std::collections::HashMap;
use std::io::{BufWriter, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::remote_proto::{Msg, VERSION, read_frame, write_frame};

type Out = Arc<Mutex<BufWriter<std::io::Stdout>>>;

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
    std::fs::write(&tmp, data)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode & 0o777))?;
    std::fs::rename(&tmp, path)
}

pub fn main(prefix: Option<String>) -> ! {
    let code = match run(prefix.unwrap_or_default()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("justrust agent: {e:#}");
            1
        }
    };
    std::process::exit(code)
}

fn run(prefix: String) -> Result<()> {
    let out: Out = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let mut input = std::io::stdin().lock();
    match read_frame(&mut input)? {
        Some((Msg::Hello { version }, _)) if version == VERSION => {}
        Some((Msg::Hello { version }, _)) => {
            send(
                &out,
                &Msg::Error {
                    id: 0,
                    msg: format!("protocol {version}, agent speaks {VERSION}"),
                },
                &[],
            );
            bail!("protocol mismatch");
        }
        _ => bail!("expected Hello"),
    }
    send(&out, &Msg::Hello { version: VERSION }, &[]);
    let at = |p: &str| -> PathBuf {
        if prefix.is_empty() {
            PathBuf::from(p)
        } else {
            Path::new(&prefix).join(p.trim_start_matches('/'))
        }
    };
    let pids: Arc<Mutex<HashMap<u64, i32>>> = Arc::default();
    while let Some((msg, payload)) = read_frame(&mut input)? {
        match msg {
            Msg::Write { path, mode } => {
                if let Err(e) = write_atomic(&at(&path), &payload, mode) {
                    send(
                        &out,
                        &Msg::Error {
                            id: 0,
                            msg: format!("write {path}: {e}"),
                        },
                        &[],
                    );
                }
            }
            Msg::Delete { path } => {
                let p = at(&path);
                let _ = std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir_all(&p));
            }
            Msg::Mkdir { path } => {
                let _ = std::fs::create_dir_all(at(&path));
            }
            Msg::Barrier { id } => send(&out, &Msg::BarrierAck { id }, &[]),
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
                },
                &[],
            ),
            _ => {}
        }
    }
    Ok(())
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
    use std::os::unix::process::ExitStatusExt;
    Ok(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)))
}
