//! Early diagnostics: the first type error before `cargo check` finishes.
//!
//! A warm rust-analyzer, from the managed toolchain (`rust-analyzer-preview`
//! component, added on first use), stays resident per workspace in a small
//! daemon (`justrust __early-daemon <root>`). When an agent build starts, the
//! recorder asks the daemon for errors in the files that changed since its
//! last query. The daemon pushes their text to rust-analyzer, pulls its
//! diagnostics, and returns the hard errors that carry a rustc error code.
//! The recorder prints them labelled as early while cargo keeps running.
//!
//! Cargo stays the authority: early errors are printed in addition to
//! cargo's, never instead, and the report says when rustc disagreed.
//!
//! ```text
//! ~/.justrust/early/<key>/      key = hash of the workspace root
//!   sock                         unix socket, one JSON line each way
//!   lock                         flock: one daemon per workspace
//!   daemon.log                   daemon and rust-analyzer stderr
//!   target/                      rust-analyzer's own target dir (build
//!                                scripts, proc macros), never the build's
//! ```
//!
//! Opt-in while accuracy is measured: `JUSTRUST_EARLY=1`, or `justrust
//! early start` (later runs in that workspace then use the running daemon).
//! Fail open: any problem means no early output, the build is untouched.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const DAEMON_ARG: &str = "__early-daemon";
const COMPONENT: &str = "rust-analyzer-preview";

/// One diagnostic, positions 1-based.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diag {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reply {
    /// False while rust-analyzer is still loading the workspace.
    pub ready: bool,
    /// Why there is no answer (loading, manifest changed...).
    #[serde(default)]
    pub note: String,
    /// Files whose diagnostics were pulled.
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub diags: Vec<Diag>,
    /// Time spent inside the daemon.
    #[serde(default)]
    pub ms: f64,
    /// Files queued for the `callers` op (second phase).
    #[serde(default)]
    pub callers: usize,
    /// Every rust-analyzer diagnostic, unfiltered (`pull` with `raw`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<Value>,
}

/// What the run records about early diagnostics (`summary.early`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct EarlyInfo {
    /// Seconds from the start of the run to the daemon's answer.
    pub secs: f64,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub diags: Vec<Diag>,
    /// Seconds from the start of the run to the second-phase answer
    /// (files using a changed signature), when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callers_secs: Option<f64>,
    /// Time the daemon spent answering.
    #[serde(default)]
    pub daemon_ms: f64,
    /// Whether the errors were shown before cargo finished.
    #[serde(default)]
    pub shown: bool,
    /// Filled in after the build: rustc errors at the same file and line.
    #[serde(default)]
    pub matched: usize,
    /// Early errors rustc did not report (false positives).
    #[serde(default)]
    pub early_only: usize,
    /// rustc errors in the queried files that early missed.
    #[serde(default)]
    pub rustc_only: usize,
}

/// `JUSTRUST_EARLY`: unset or `0` = off, `auto` = query a running daemon
/// and start one for workspaces with recent agent builds (`maybe_autostart`),
/// anything else = on (query, and start the daemon on first use).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Off,
    Auto,
    On,
}

pub fn mode() -> Mode {
    match std::env::var("JUSTRUST_EARLY").as_deref() {
        Ok("0") | Ok("off") | Ok("no") | Ok("false") | Err(_) => Mode::Off,
        Ok("auto") => Mode::Auto,
        Ok(_) => Mode::On,
    }
}

pub fn enabled() -> bool {
    mode() != Mode::Off
}

/// Auto mode starts a daemon after this many agent builds in the workspace
/// within `AUTO_WINDOW_SECS`, if `AUTO_MIN_AVAILABLE_MB` of memory is free
/// (the Desktop workspace's rust-analyzer holds 4.7 GB).
const AUTO_MIN_BUILDS: usize = 3;
const AUTO_WINDOW_SECS: f64 = 3600.0;
const AUTO_MIN_AVAILABLE_MB: u64 = 8 * 1024;

/// Recent agent builds per workspace root, from the tail of the run index.
fn recent_agent_builds(index: &str, root: &Path, now: f64) -> usize {
    index
        .lines()
        .rev()
        .take(400)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .take_while(|e| {
            e["start"]
                .as_f64()
                .is_some_and(|t| now - t < AUTO_WINDOW_SECS)
        })
        .filter(|e| !e["agent_session"].is_null())
        .filter(|e| {
            e["cwd"]
                .as_str()
                .and_then(|c| workspace_root(Path::new(c)))
                .is_some_and(|r| r == root)
        })
        .count()
}

/// Auto mode, after an agent build of `cwd` finished: start the workspace's
/// daemon in the background when it has had several agent builds lately.
/// Never blocks, never fails the build.
pub fn maybe_autostart(cwd: &Path, sub: &str) {
    if mode() != Mode::Auto || !wanted_for(sub) {
        return;
    }
    let Some(root) = workspace_root(cwd) else {
        return;
    };
    let Ok(dir) = state_dir(&root) else { return };
    if UnixStream::connect(dir.join("sock")).is_ok() {
        return;
    }
    if crate::procfs::meminfo().is_none_or(|m| m.available_mb < AUTO_MIN_AVAILABLE_MB) {
        return;
    }
    let Ok(index) = crate::paths::index_file().and_then(|p| Ok(std::fs::read_to_string(p)?)) else {
        return;
    };
    if recent_agent_builds(&index, &root, crate::paths::now()) >= AUTO_MIN_BUILDS {
        let _ = spawn_daemon(&root, &dir);
    }
}

fn wanted_for(sub: &str) -> bool {
    matches!(sub, "check" | "c" | "clippy" | "build" | "b" | "test" | "t")
}

/// The workspace root for `cwd`: the nearest ancestor whose Cargo.toml has a
/// `[workspace]` table, else the nearest Cargo.toml.
pub fn workspace_root(cwd: &Path) -> Option<PathBuf> {
    let mut nearest = None;
    for d in cwd.ancestors() {
        let m = d.join("Cargo.toml");
        if let Ok(t) = std::fs::read_to_string(&m) {
            if t.lines().any(|l| l.trim() == "[workspace]") {
                return Some(d.to_path_buf());
            }
            nearest.get_or_insert_with(|| d.to_path_buf());
        }
    }
    nearest
}

fn fnv(s: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn state_dir(root: &Path) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Ok(crate::paths::home()?
        .join("early")
        .join(format!("{:016x}", fnv(root.as_os_str().as_bytes()))))
}

// ------------------------------------------------------------------ client

/// Ask the daemon of the workspace at `cwd`, or start one when opted in.
/// `None` when there is nothing to show (no daemon, not ready, timeout).
pub fn query(cwd: &Path, timeout: Duration) -> Option<Reply> {
    query_op(cwd, timeout, "diagnostics")
}

fn query_op(cwd: &Path, timeout: Duration, op: &str) -> Option<Reply> {
    let root = workspace_root(cwd)?;
    let dir = state_dir(&root).ok()?;
    let sock = dir.join("sock");
    let s = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(_) => {
            if op != "diagnostics" || mode() == Mode::Auto {
                return None;
            }
            if let Err(e) = spawn_daemon(&root, &dir) {
                eprintln!("justrust: early diagnostics unavailable: {e:#}");
            }
            return None;
        }
    };
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let mut w = s.try_clone().ok()?;
    writeln!(w, "{}", json!({ "op": op })).ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// Run alongside a recorded build. Prints early errors to stderr unless the
/// build already finished, and sends what to record. The recorder waits at
/// most a moment for it after the build ends: early never delays a build.
pub fn run_for_build(
    sub: &str,
    cwd: PathBuf,
    start: f64,
    build_done: Arc<std::sync::atomic::AtomicBool>,
) -> Option<std::sync::mpsc::Receiver<EarlyInfo>> {
    if !enabled() || !wanted_for(sub) {
        return None;
    }
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if let Some(i) = early_for_build(cwd, start, build_done) {
            let _ = tx.send(i);
        }
    });
    Some(rx)
}

fn early_for_build(
    cwd: PathBuf,
    start: f64,
    build_done: Arc<std::sync::atomic::AtomicBool>,
) -> Option<EarlyInfo> {
    {
        let r = query(&cwd, Duration::from_secs(5))?;
        let secs = crate::paths::now() - start;
        let mut info = EarlyInfo {
            secs,
            ready: r.ready,
            note: r.note,
            files: r.files,
            diags: r.diags,
            daemon_ms: r.ms,
            ..Default::default()
        };
        let done = || build_done.load(std::sync::atomic::Ordering::SeqCst);
        if info.ready && !info.diags.is_empty() && !done() {
            let text = format_block(&info.diags, &cwd, secs, "");
            // One write: lines from cargo's tee never land inside the block.
            let _ = std::io::stderr().lock().write_all(text.as_bytes());
            info.shown = true;
        }
        // Second phase: a signature changed, so files that use the item
        // may now be wrong. Pulled after the first answer is out, and only
        // when the edited files themselves are clean (otherwise the agent
        // already has an error to fix, and the pulls would hold the daemon
        // for seconds).
        if info.ready
            && r.callers > 0
            && info.diags.is_empty()
            && !done()
            && let Some(c) = query_op(&cwd, Duration::from_secs(5), "callers")
        {
            let secs = crate::paths::now() - start;
            info.callers_secs = Some(secs);
            info.files.extend(c.files);
            if !c.diags.is_empty() && !done() {
                let text =
                    format_block(&c.diags, &cwd, secs, " in files using a changed signature");
                let _ = std::io::stderr().lock().write_all(text.as_bytes());
                info.shown = true;
            }
            info.diags.extend(c.diags);
        }
        Some(info)
    }
}

fn rel(file: &str, cwd: &Path) -> String {
    Path::new(file)
        .strip_prefix(cwd)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| file.to_owned())
}

fn format_block(diags: &[Diag], cwd: &Path, secs: f64, what: &str) -> String {
    let mut o = format!(
        "justrust: early errors{what} from rust-analyzer after {secs:.2}s (cargo is still running and has the final word):\n"
    );
    for d in diags {
        let head = if d.code.is_empty() {
            "error".to_owned()
        } else {
            format!("error[{}]", d.code)
        };
        o.push_str(&format!(
            "{head}: {}\n  --> {}:{}:{}\n",
            d.message,
            rel(&d.file, cwd),
            d.line,
            d.col
        ));
    }
    o.push_str("justrust: end of early errors\n");
    o
}

/// Errors rustc printed, as (absolute file, line, code), from the build's
/// output lines (`error[E0308]: ...` then ` --> path:line:col`).
pub fn rustc_errors<'a>(
    lines: impl Iterator<Item = &'a str>,
    root: &Path,
) -> Vec<(String, u32, String)> {
    let mut out = Vec::new();
    let mut code: Option<String> = None;
    for l in lines {
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("error[") {
            code = rest.split(']').next().map(str::to_owned);
            continue;
        }
        if t.starts_with("error:") {
            code = Some(String::new());
            continue;
        }
        if t.starts_with("warning") {
            code = None;
            continue;
        }
        if let Some(loc) = t.strip_prefix("--> ")
            && let Some(c) = code.take()
        {
            let mut parts = loc.rsplitn(3, ':');
            let (_col, line, file) = (parts.next(), parts.next(), parts.next());
            if let (Some(line), Some(file)) = (line.and_then(|l| l.parse().ok()), file) {
                let abs = if Path::new(file).is_absolute() {
                    PathBuf::from(file)
                } else {
                    root.join(file)
                };
                out.push((normalize(&abs), line, c));
            }
        }
    }
    out
}

fn normalize(p: &Path) -> String {
    let mut v: Vec<std::path::Component> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                v.pop();
            }
            std::path::Component::CurDir => {}
            c => v.push(c),
        }
    }
    v.iter().collect::<PathBuf>().display().to_string()
}

/// Compare early errors with rustc's, within the files early looked at.
pub fn score(info: &mut EarlyInfo, rustc: &[(String, u32, String)]) {
    let files: BTreeSet<&str> = info.files.iter().map(String::as_str).collect();
    let in_scope: Vec<_> = rustc
        .iter()
        .filter(|(f, _, _)| files.contains(f.as_str()))
        .collect();
    let hit = |d: &Diag| {
        in_scope
            .iter()
            .any(|(f, l, _)| *f == d.file && *l == d.line)
    };
    info.matched = info.diags.iter().filter(|d| hit(d)).count();
    info.early_only = info.diags.len() - info.matched;
    info.rustc_only = in_scope
        .iter()
        .filter(|(f, l, _)| !info.diags.iter().any(|d| d.file == *f && d.line == *l))
        .count();
}

/// Lines for the agent report.
pub fn report_line(e: &EarlyInfo, compile_failed: bool) -> Option<String> {
    if !e.ready {
        return Some(format!(
            "early: rust-analyzer not ready ({})",
            if e.note.is_empty() {
                "loading"
            } else {
                &e.note
            }
        ));
    }
    if e.diags.is_empty() {
        return None;
    }
    let mut s = format!(
        "early: {} error{} from rust-analyzer at {:.2}s, {} confirmed by rustc",
        e.diags.len(),
        if e.diags.len() == 1 { "" } else { "s" },
        e.secs,
        e.matched
    );
    if e.early_only > 0 {
        s.push_str(&format!(
            ", {} NOT reported by rustc{}",
            e.early_only,
            if compile_failed {
                ""
            } else {
                " (the build passed: ignore them)"
            }
        ));
    }
    Some(s)
}

fn spawn_daemon(root: &Path, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    // Do not even try again soon after a daemon failed to start.
    let failed = dir.join("failed");
    if let Ok(age) = std::fs::metadata(&failed)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default())
        && age < Duration::from_secs(600)
    {
        bail!(
            "the daemon failed {}s ago: {}",
            age.as_secs(),
            std::fs::read_to_string(&failed).unwrap_or_default().trim()
        );
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("daemon.log"))?;
    let mut c = Command::new(std::env::current_exe()?);
    c.arg(DAEMON_ARG)
        .arg(root)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("JUSTRUST_RUN") || k == "CARGO_TARGET_DIR" || k == "RUSTC_WRAPPER" {
            c.env_remove(k.as_ref());
        }
    }
    use std::os::unix::process::CommandExt;
    c.process_group(0);
    c.spawn().context("starting the early diagnostics daemon")?;
    Ok(())
}

/// `justrust early start|stop|status`.
pub fn command(action: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = workspace_root(&cwd).context("no Cargo.toml here")?;
    let dir = state_dir(&root)?;
    match action {
        "start" => {
            let _ = std::fs::remove_file(dir.join("failed"));
            let t = Instant::now();
            loop {
                match query(&cwd, Duration::from_secs(30)) {
                    Some(r) if r.ready => {
                        println!(
                            "early diagnostics ready for {} after {:.1}s",
                            root.display(),
                            t.elapsed().as_secs_f64()
                        );
                        return Ok(());
                    }
                    _ if t.elapsed() > Duration::from_secs(900) => {
                        bail!(
                            "not ready after 15 min, see {}",
                            dir.join("daemon.log").display()
                        )
                    }
                    _ => std::thread::sleep(Duration::from_millis(500)),
                }
                if let Ok(f) = std::fs::read_to_string(dir.join("failed")) {
                    bail!("daemon failed: {}", f.trim());
                }
            }
        }
        "stop" => {
            if let Ok(mut s) = UnixStream::connect(dir.join("sock")) {
                let _ = writeln!(s, "{}", json!({"op": "stop"}));
                let mut b = String::new();
                let _ = s.read_to_string(&mut b);
                println!("stopped");
            } else {
                println!("not running");
            }
        }
        _ => match UnixStream::connect(dir.join("sock")) {
            Ok(mut s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                writeln!(s, "{}", json!({"op": "status"}))?;
                let mut b = String::new();
                BufReader::new(s).read_line(&mut b)?;
                println!("{}", b.trim());
            }
            Err(_) => println!("not running for {}", root.display()),
        },
    }
    Ok(())
}

// ------------------------------------------------------------------ daemon

fn log(msg: &str) {
    eprintln!(
        "{} {msg}",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f")
    );
}

/// The toolchain whose rust-analyzer to run: the pinned one, else the newest
/// installed. Adds the component when missing.
fn analyzer(root: &Path) -> Result<(crate::toolchain::Toolchain, PathBuf)> {
    use crate::toolchain as tc;
    let pinned = tc::resolve(root)?.and_then(|p| tc::installed(&p.spec).ok().flatten());
    let t = match pinned {
        Some(t) => t,
        None => tc::list_installed()
            .into_iter()
            .last()
            .context("no justrust toolchain installed (`justrust toolchain install nightly`)")?,
    };
    let bin = t.bin("rust-analyzer");
    if !bin.is_file() {
        tc::add_component(&t, COMPONENT, &mut std::io::stderr())?;
    }
    if !bin.is_file() {
        bail!("{} has no rust-analyzer", t.id);
    }
    Ok((t, bin))
}

type Pending = Arc<Mutex<HashMap<u64, Sender<Value>>>>;

struct Lsp {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next: u64,
    /// Seen `quiescent: true` at least once.
    ready: Arc<Mutex<bool>>,
}

fn send(w: &Mutex<ChildStdin>, v: &Value) -> Result<()> {
    let b = serde_json::to_vec(v)?;
    let mut w = w.lock().unwrap();
    write!(w, "Content-Length: {}\r\n\r\n", b.len())?;
    w.write_all(&b)?;
    w.flush()?;
    Ok(())
}

fn read_msg(r: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            return None;
        }
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0; len?];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

/// rust-analyzer's experimental diagnostics (unresolved methods, fields,
/// names, trait bounds...): they find 13 of 19 typical agent errors instead
/// of 7 (FINDINGS 28). `JUSTRUST_EARLY_EXPERIMENTAL=0` turns them off; read
/// by the daemon when it starts.
fn experimental() -> bool {
    !matches!(
        std::env::var("JUSTRUST_EARLY_EXPERIMENTAL").as_deref(),
        Ok("0") | Ok("off") | Ok("no") | Ok("false")
    )
}

fn ra_config(target: &Path) -> Value {
    json!({
        "checkOnSave": false,
        "check": {"workspace": false},
        "cargo": {
            "targetDir": target,
            "buildScripts": {"enable": true},
            "allTargets": true,
        },
        "procMacro": {"enable": true},
        "diagnostics": {"enable": true, "experimental": {"enable": experimental()}},
        "cachePriming": {"enable": true},
        "files": {"watcher": "server"},
        "lens": {"enable": false},
        "inlayHints": {"enable": false},
    })
}

impl Lsp {
    fn start(root: &Path, dir: &Path) -> Result<Lsp> {
        let (t, bin) = analyzer(root)?;
        log(&format!("rust-analyzer {} from {}", bin.display(), t.id));
        let mut cmd = Command::new(&bin);
        crate::record::apply_toolchain_env(&mut cmd, &t);
        cmd.env("CARGO", t.bin("cargo"))
            .env("RA_LOG", "error")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd.spawn().context("starting rust-analyzer")?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().context("stdin")?));
        let stdout = child.stdout.take().context("stdout")?;
        let pending: Pending = Arc::default();
        let ready = Arc::new(Mutex::new(false));
        let cfg = ra_config(&dir.join("target"));
        {
            let (pending, ready, stdin, cfg) =
                (pending.clone(), ready.clone(), stdin.clone(), cfg.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                while let Some(m) = read_msg(&mut r) {
                    let method = m.get("method").and_then(Value::as_str);
                    match (method, m.get("id")) {
                        (Some("experimental/serverStatus"), _) => {
                            let p = &m["params"];
                            if p["quiescent"].as_bool() == Some(true) {
                                let mut g = ready.lock().unwrap();
                                if !*g {
                                    log(&format!("quiescent, health {}", p["health"]));
                                }
                                *g = true;
                            }
                        }
                        (Some("workspace/configuration"), Some(id)) => {
                            let n = m["params"]["items"].as_array().map_or(1, Vec::len);
                            let _ = send(
                                &stdin,
                                &json!({"jsonrpc": "2.0", "id": id, "result": vec![cfg.clone(); n]}),
                            );
                        }
                        (Some(_), Some(id)) => {
                            let _ =
                                send(&stdin, &json!({"jsonrpc": "2.0", "id": id, "result": null}));
                        }
                        (Some(_), None) => {}
                        (None, Some(id)) => {
                            if let Some(tx) =
                                id.as_u64().and_then(|i| pending.lock().unwrap().remove(&i))
                            {
                                let _ = tx.send(m);
                            }
                        }
                        (None, None) => {}
                    }
                }
                log("rust-analyzer closed its output");
                std::process::exit(1);
            });
        }
        let mut lsp = Lsp {
            child,
            stdin,
            pending,
            next: 1,
            ready,
        };
        let uri = file_uri(root);
        lsp.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": uri,
                "workspaceFolders": [{"uri": uri, "name": "root"}],
                "capabilities": {
                    "textDocument": {"diagnostic": {"dynamicRegistration": false}, "publishDiagnostics": {}},
                    "window": {"workDoneProgress": true},
                    "workspace": {"configuration": true},
                    "experimental": {"serverStatusNotification": true},
                },
                "initializationOptions": cfg,
            }),
            Duration::from_secs(60),
        )?;
        lsp.notify("initialized", json!({}))?;
        Ok(lsp)
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        send(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }

    fn request_async(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<std::sync::mpsc::Receiver<Value>> {
        let id = self.next;
        self.next += 1;
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);
        send(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        Ok(rx)
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let rx = self.request_async(method, params)?;
        let m = rx
            .recv_timeout(timeout)
            .context("rust-analyzer did not answer")?;
        if let Some(e) = m.get("error") {
            bail!("{method}: {e}");
        }
        Ok(m["result"].clone())
    }
}

fn file_uri(p: &Path) -> String {
    let mut s = String::from("file://");
    for b in p.display().to_string().bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

/// Every `.rs` file and manifest under the source roots, with a cheap
/// change stamp.
fn scan(roots: &[PathBuf]) -> HashMap<PathBuf, (SystemTime, u64)> {
    let mut out = HashMap::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if n == "target" || n.starts_with('.') || n.starts_with("target-") {
                    continue;
                }
                stack.push(e.path());
            } else if (n.ends_with(".rs") || n == "Cargo.toml")
                && let Ok(m) = e.metadata()
            {
                out.insert(
                    e.path(),
                    (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()),
                );
            }
        }
    }
    out
}

/// Workspace members and path dependencies (outside the root too: Jcode
/// Desktop builds crates of the sibling Jcode repo).
fn source_roots(root: &Path, t: Option<&crate::toolchain::Toolchain>) -> Vec<PathBuf> {
    let cargo = t.map_or_else(|| PathBuf::from("cargo"), |t| t.bin("cargo"));
    let mut roots = vec![root.to_path_buf()];
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline"])
        .current_dir(root)
        .stderr(Stdio::null())
        .output();
    if let Ok(o) = out
        && let Ok(v) = serde_json::from_slice::<Value>(&o.stdout)
    {
        for p in v["packages"].as_array().into_iter().flatten() {
            if p["source"].is_null()
                && let Some(m) = p["manifest_path"].as_str()
                && let Some(d) = Path::new(m).parent()
            {
                roots.push(d.to_path_buf());
            }
        }
    }
    roots.sort();
    roots.dedup_by(|b, a| b.starts_with(&*a));
    roots
}

struct State {
    lsp: Lsp,
    roots: Vec<PathBuf>,
    stamps: HashMap<PathBuf, (SystemTime, u64)>,
    versions: HashMap<PathBuf, i64>,
    /// Files whose last pull had errors: pulled again on every query, so a
    /// repeated check still shows them early.
    erroring: BTreeSet<PathBuf>,
    manifest_changed: bool,
    queries: u64,
    /// Last seen text of every source file, to tell signature edits from
    /// body edits and to find the files that mention a changed item.
    texts: HashMap<PathBuf, String>,
    /// Files that mention an item whose signature changed in the last
    /// sync: pulled by the `callers` op, after the first answer.
    callers: Vec<PathBuf>,
}

impl State {
    fn sync(&mut self) -> Vec<PathBuf> {
        let now = scan(&self.roots);
        let mut changed = Vec::new();
        for (p, st) in &now {
            if self.stamps.get(p) != Some(st) {
                changed.push(p.clone());
            }
        }
        let gone: Vec<PathBuf> = self
            .stamps
            .keys()
            .filter(|p| !now.contains_key(*p))
            .cloned()
            .collect();
        self.stamps = now;
        for p in gone {
            if self.versions.remove(&p).is_some() {
                let _ = self.lsp.notify(
                    "textDocument/didClose",
                    json!({"textDocument": {"uri": file_uri(&p)}}),
                );
            }
            self.erroring.remove(&p);
        }
        let mut rs = Vec::new();
        let mut sig_names: BTreeSet<String> = BTreeSet::new();
        for p in changed {
            if p.file_name().is_some_and(|n| n == "Cargo.toml") {
                // rust-analyzer reloads the workspace on its own; until it
                // settles, its answers could be stale.
                self.manifest_changed = true;
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            if let Some(old) = self.texts.get(&p) {
                sig_names.extend(changed_signatures(old, &text));
            }
            self.texts.insert(p.clone(), text.clone());
            let uri = file_uri(&p);
            let r = match self.versions.get_mut(&p) {
                Some(v) => {
                    *v += 1;
                    self.lsp.notify(
                        "textDocument/didChange",
                        json!({"textDocument": {"uri": uri, "version": *v},
                               "contentChanges": [{"text": text}]}),
                    )
                }
                None => {
                    self.versions.insert(p.clone(), 1);
                    self.lsp.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": uri, "languageId": "rust",
                               "version": 1, "text": text}}),
                    )
                }
            };
            if r.is_ok() {
                rs.push(p);
            }
        }
        self.callers = mentioning(&self.texts, &sig_names, &rs, MAX_CALLER_FILES);
        rs
    }

    /// `extra`: files to pull besides the changed ones (`pull` op, used by
    /// the accuracy sweep to ask about every file of a crate).
    fn diagnostics(&mut self, extra: &[PathBuf], timeout: Duration, raw: bool) -> Reply {
        let t = Instant::now();
        self.queries += 1;
        let changed = self.sync();
        if !*self.lsp.ready.lock().unwrap() {
            return Reply {
                note: "loading the workspace".into(),
                ..Default::default()
            };
        }
        if self.manifest_changed {
            return Reply {
                note: "a Cargo.toml changed since the daemon started".into(),
                ..Default::default()
            };
        }
        let mut files: BTreeSet<PathBuf> = changed.into_iter().collect();
        files.extend(self.erroring.iter().cloned());
        files.extend(extra.iter().cloned());
        let files: Vec<PathBuf> = files.into_iter().collect();
        let mut r = self.pull(&files, timeout, raw);
        r.ms = t.elapsed().as_secs_f64() * 1000.0;
        r.callers = self.callers.len();
        r
    }

    fn pull(&mut self, files: &[PathBuf], timeout: Duration, raw: bool) -> Reply {
        let t = Instant::now();
        let mut rxs = Vec::new();
        for f in files {
            if let Ok(rx) = self.lsp.request_async(
                "textDocument/diagnostic",
                json!({"textDocument": {"uri": file_uri(f)}}),
            ) {
                rxs.push((f.clone(), rx));
            }
        }
        let deadline = Instant::now() + timeout;
        let mut diags = Vec::new();
        let mut pulled = Vec::new();
        let mut raws = Vec::new();
        for (f, rx) in rxs {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(m) = rx.recv_timeout(left) else {
                continue;
            };
            let items = m["result"]["items"].as_array().cloned().unwrap_or_default();
            pulled.push(f.display().to_string());
            if raw {
                raws.extend(items.iter().map(|d| {
                    json!({"file": f, "severity": d["severity"], "code": d["code"],
                           "line": d["range"]["start"]["line"].as_u64().map(|l| l + 1),
                           "message": d["message"]})
                }));
            }
            let mut errs: Vec<Diag> = items.iter().filter_map(|d| to_diag(&f, d)).collect();
            let mut seen = BTreeSet::new();
            errs.retain(|d| !d.code.is_empty() || seen.insert(d.line));
            // A file that does not parse has unreliable type errors.
            if errs.iter().any(|d| d.code.is_empty()) {
                errs.retain(|d| d.code.is_empty());
            }
            if errs.is_empty() {
                self.erroring.remove(&f);
            } else {
                self.erroring.insert(f.clone());
            }
            diags.extend(errs);
        }
        Reply {
            ready: true,
            note: String::new(),
            files: pulled,
            diags,
            ms: t.elapsed().as_secs_f64() * 1000.0,
            raw: raws,
            callers: 0,
        }
    }

    /// Second phase: diagnostics of the files that mention an item whose
    /// signature changed (callers of a changed fn, users of a changed
    /// struct). Does not sync again.
    /// Nearest files first, a few at a time, stopping at the first batch
    /// with errors: rust-analyzer checks a big file in 0.1-0.5 s, and only
    /// the first error has to beat cargo. The rest stay queued.
    fn callers(&mut self, timeout: Duration, raw: bool) -> Reply {
        let t = Instant::now();
        let mut out = Reply {
            ready: true,
            ..Default::default()
        };
        if !*self.lsp.ready.lock().unwrap() {
            return out;
        }
        while !self.callers.is_empty() && t.elapsed() < timeout {
            let n = self.callers.len().min(CALLER_BATCH);
            let batch: Vec<PathBuf> = self.callers.drain(..n).collect();
            let r = self.pull(&batch, timeout.saturating_sub(t.elapsed()), raw);
            out.files.extend(r.files);
            out.raw.extend(r.raw);
            let found = !r.diags.is_empty();
            out.diags.extend(r.diags);
            if found {
                break;
            }
        }
        out.callers = self.callers.len();
        out.ms = t.elapsed().as_secs_f64() * 1000.0;
        out
    }
}

/// At most this many files are pulled for a changed signature, the ones
/// closest to the edited file first.
const MAX_CALLER_FILES: usize = 24;
/// Caller files pulled per round of the `callers` op.
const CALLER_BATCH: usize = 4;

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Item name declared on `line` (`fn x`, `struct X`, ...), if any.
fn declared_name(line: &str) -> Option<&str> {
    const KW: &[&str] = &[
        "fn", "struct", "enum", "trait", "type", "const", "static", "union",
    ];
    let mut words = line
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    while let Some(w) = words.next() {
        if KW.contains(&w) {
            let name = words.next()?;
            return (!name.as_bytes()[0].is_ascii_digit()).then_some(name);
        }
    }
    None
}

/// Names of items whose signature differs between `old` and `new`: a
/// changed line that declares an item, or a changed line inside a
/// multi-line `fn` signature (between `fn name(` and the line that opens
/// its body). Body edits return nothing, so they cost no extra pulls.
pub fn changed_signatures(old: &str, new: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (text, other) in [(new, old), (old, new)] {
        let others: std::collections::HashSet<&str> = other.lines().collect();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if others.contains(l) || l.trim().is_empty() {
                continue;
            }
            let t = l.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if let Some(n) = declared_name(t) {
                names.insert(n.to_owned());
                continue;
            }
            // Inside a multi-line signature? Walk back to the `fn` line; any
            // line opening a body or ending an item in between means no.
            for j in (i.saturating_sub(12)..i).rev() {
                let p = lines[j].trim_end();
                if let Some(n) = declared_name(lines[j].trim_start()) {
                    if lines[j].contains("fn ") && !p.ends_with('{') && !p.ends_with(';') {
                        names.insert(n.to_owned());
                    }
                    break;
                }
                if p.ends_with('{') || p.ends_with(';') || p.ends_with('}') {
                    break;
                }
            }
        }
    }
    names
}

/// Files (other than `changed`) whose text mentions one of `names` as a
/// whole word, nearest to the changed files first (longest shared path).
fn mentioning(
    texts: &HashMap<PathBuf, String>,
    names: &BTreeSet<String>,
    changed: &[PathBuf],
    max: usize,
) -> Vec<PathBuf> {
    if names.is_empty() {
        return Vec::new();
    }
    // Module names of the changed files (`src/effort.rs` -> `effort`,
    // `src/foo/mod.rs` -> `foo`, `jcode-core/src/lib.rs` -> `jcode_core`).
    let modules: Vec<String> = changed
        .iter()
        .filter_map(|c| {
            let stem = c.file_stem()?.to_str()?;
            Some(match stem {
                "mod" => c.parent()?.file_name()?.to_str()?.to_owned(),
                "lib" | "main" => c
                    .parent()?
                    .parent()?
                    .file_name()?
                    .to_str()?
                    .replace('-', "_"),
                s => s.to_owned(),
            })
        })
        .collect();
    let mut hits: Vec<(u8, usize, &PathBuf)> = Vec::new();
    for (p, text) in texts {
        if changed.contains(p) {
            continue;
        }
        let b = text.as_bytes();
        // Best evidence of a real use: the name behind the module of a
        // changed file (`effort::label`), then any whole-word mention.
        // Common names (`label`, `new`) appear in most files, so the
        // qualified ones must be pulled first.
        let mut rank = 0;
        for n in names {
            for i in memchr::memmem::find_iter(b, n.as_bytes()) {
                let whole = (i == 0 || !is_ident(b[i - 1]))
                    && b.get(i + n.len()).is_none_or(|c| !is_ident(*c));
                if !whole {
                    continue;
                }
                rank = rank.max(1);
                let before = &text[..i];
                if modules.iter().any(|m| before.ends_with(&format!("{m}::"))) {
                    rank = 2;
                    break;
                }
            }
        }
        if rank > 0 {
            let near = changed
                .iter()
                .map(|c| {
                    c.components()
                        .zip(p.components())
                        .take_while(|(a, b)| a == b)
                        .count()
                })
                .max()
                .unwrap_or(0);
            hits.push((rank, near, p));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
    hits.into_iter()
        .take(max)
        .map(|(_, _, p)| p.clone())
        .collect()
}

/// Hard errors with a rustc error code only. rust-analyzer's own
/// diagnostics (unresolved proc macro, inactive code, unlinked file) are
/// about its limits, not the program.
fn to_diag(file: &Path, d: &Value) -> Option<Diag> {
    if d["severity"].as_u64() != Some(1) {
        return None;
    }
    let code = d["code"].as_str()?;
    let message = d["message"].as_str().unwrap_or_default();
    // Parser errors have no rustc code. rustc rejects the same file, so
    // report them uncoded (rustc prints them as plain `error:`).
    let (code, message) = if code == "syntax-error" {
        (
            "",
            message.strip_prefix("Syntax Error: ").unwrap_or(message),
        )
    } else {
        (code, message)
    };
    let b = code.as_bytes();
    if !code.is_empty() && !(b.len() == 5 && b[0] == b'E' && b[1..].iter().all(u8::is_ascii_digit))
    {
        return None;
    }
    // Where rust-analyzer's own inference gave up, not the program's:
    // "type annotations needed" (E0282) and anything mentioning a type it
    // could not compute. On the Desktop snapshot these were the only
    // errors it reported on sources rustc accepts (FINDINGS 28).
    if code == "E0282" || message.contains("{unknown}") {
        return None;
    }
    // A fn item passed where a closure is expected: rust-analyzer says
    // `fn x(..): FnOnce<..>` is not satisfied (E0277) where rustc reports
    // E0593 one line later. Wrong code and line: not shown.
    if code == "E0277" && message.contains("the trait bound `fn ") && message.contains(": Fn") {
        return None;
    }
    // rust-analyzer's parser recovers differently than rustc's and reports
    // several errors per mistake. Keep only the first one per line.
    let start = &d["range"]["start"];
    Some(Diag {
        file: file.display().to_string(),
        line: start["line"].as_u64()? as u32 + 1,
        col: start["character"].as_u64()? as u32 + 1,
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

pub fn daemon_main(root: PathBuf) -> ! {
    let code = match daemon(&root) {
        Ok(()) => 0,
        Err(e) => {
            log(&format!("daemon failed: {e:#}"));
            if let Ok(d) = state_dir(&root) {
                let _ = std::fs::write(d.join("failed"), format!("{e:#}"));
            }
            1
        }
    };
    std::process::exit(code)
}

fn daemon(root: &Path) -> Result<()> {
    let dir = state_dir(root)?;
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("lock"))?;
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let sock = dir.join("sock");
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    log(&format!("daemon up for {}", root.display()));
    let started = Instant::now();
    let tc = analyzer(root).ok().map(|x| x.0);
    let roots = source_roots(root, tc.as_ref());
    // Baseline before rust-analyzer reads the disk: anything edited after
    // this is pushed on the first query.
    let stamps = scan(&roots);
    let texts: HashMap<PathBuf, String> = stamps
        .keys()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .filter_map(|p| Some((p.clone(), std::fs::read_to_string(p).ok()?)))
        .collect();
    log(&format!(
        "{} source roots, {} files",
        roots.len(),
        stamps.len()
    ));
    let lsp = Lsp::start(root, &dir)?;
    let state = Arc::new(Mutex::new(State {
        lsp,
        roots,
        stamps,
        versions: HashMap::new(),
        erroring: BTreeSet::new(),
        manifest_changed: false,
        queries: 0,
        texts,
        callers: Vec::new(),
    }));
    let idle_secs: u64 = std::env::var("JUSTRUST_EARLY_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3 * 3600);
    let last = Arc::new(Mutex::new(Instant::now()));
    {
        let (last, state, sock) = (last.clone(), state.clone(), sock.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                if last.lock().unwrap().elapsed() > Duration::from_secs(idle_secs) {
                    log("idle, exiting");
                    let _ = std::fs::remove_file(&sock);
                    let _ = state.lock().map(|mut s| s.lsp.child.kill());
                    std::process::exit(0);
                }
            }
        });
    }
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        *last.lock().unwrap() = Instant::now();
        let state = state.clone();
        let sock = sock.clone();
        // A query holds the state lock; status answers without waiting
        // behind a slow pull only if it gets the lock (try_lock).
        std::thread::spawn(move || {
            let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = String::new();
            let mut w = match conn.try_clone() {
                Ok(w) => w,
                Err(_) => return,
            };
            if BufReader::new(conn).read_line(&mut line).is_err() {
                return;
            }
            let req: Value = serde_json::from_str(&line).unwrap_or_default();
            match req["op"].as_str() {
                Some("diagnostics") => {
                    let r = state.lock().unwrap().diagnostics(
                        &[],
                        Duration::from_secs(4),
                        req["raw"].as_bool() == Some(true),
                    );
                    let _ = writeln!(w, "{}", serde_json::to_string(&r).unwrap_or_default());
                }
                Some("callers") => {
                    let r = state
                        .lock()
                        .unwrap()
                        .callers(Duration::from_secs(4), req["raw"].as_bool() == Some(true));
                    let _ = writeln!(w, "{}", serde_json::to_string(&r).unwrap_or_default());
                }
                Some("pull") => {
                    let files: Vec<PathBuf> = req["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f.as_str().map(PathBuf::from))
                        .collect();
                    let r = state.lock().unwrap().diagnostics(
                        &files,
                        Duration::from_secs(300),
                        req["raw"].as_bool() == Some(true),
                    );
                    let _ = writeln!(w, "{}", serde_json::to_string(&r).unwrap_or_default());
                }
                Some("stop") => {
                    log("stop requested");
                    let _ = std::fs::remove_file(&sock);
                    if let Ok(mut s) = state.lock() {
                        let _ = s.lsp.child.kill();
                    }
                    let _ = writeln!(w, "{{}}");
                    std::process::exit(0);
                }
                _ => {
                    let v = match state.try_lock() {
                        Ok(s) => json!({
                            "ready": *s.lsp.ready.lock().unwrap(),
                            "queries": s.queries,
                            "files": s.stamps.len(),
                            "erroring": s.erroring.len(),
                            "rss_mb": crate::procfs::read_stat(s.lsp.child.id() as i32)
                                .map_or(0, |p| p.rss_pages * crate::procfs::page_size() / (1 << 20)),
                            "uptime_secs": started.elapsed().as_secs(),
                        }),
                        Err(_) => json!({"busy": true}),
                    };
                    let _ = writeln!(w, "{v}");
                }
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rustc_errors() {
        let lines = [
            "error[E0308]: mismatched types",
            "  --> crates/a/src/x.rs:20:22",
            "   |",
            "warning: unused variable",
            " --> crates/a/src/y.rs:3:1",
            "error: cannot find macro `foo` in this scope",
            " --> ../jcode/crates/b/src/z.rs:7:5",
        ];
        let got = rustc_errors(lines.iter().copied(), Path::new("/w/desk"));
        assert_eq!(
            got,
            vec![
                ("/w/desk/crates/a/src/x.rs".into(), 20, "E0308".into()),
                ("/w/jcode/crates/b/src/z.rs".into(), 7, "".into()),
            ]
        );
    }

    #[test]
    fn scores_against_rustc() {
        let d = |l| Diag {
            file: "/w/a.rs".into(),
            line: l,
            col: 1,
            code: "E0308".into(),
            message: String::new(),
        };
        let mut e = EarlyInfo {
            ready: true,
            files: vec!["/w/a.rs".into()],
            diags: vec![d(3), d(9)],
            ..Default::default()
        };
        let rustc = vec![
            ("/w/a.rs".to_string(), 3, "E0308".to_string()),
            ("/w/a.rs".to_string(), 5, "E0599".to_string()),
            ("/w/other.rs".to_string(), 1, "E0308".to_string()),
        ];
        score(&mut e, &rustc);
        assert_eq!((e.matched, e.early_only, e.rustc_only), (1, 1, 1));
        let line = report_line(&e, false).unwrap();
        assert!(line.contains("NOT reported by rustc"), "{line}");
        assert!(line.contains("ignore them"), "{line}");
    }

    #[test]
    fn keeps_only_rustc_coded_errors() {
        let f = Path::new("/w/a.rs");
        let mk = |sev: u64, code: &str| {
            json!({"severity": sev, "code": code, "message": "m",
                   "range": {"start": {"line": 4, "character": 2}}})
        };
        assert_eq!(to_diag(f, &mk(1, "E0308")).unwrap().line, 5);
        assert!(to_diag(f, &mk(1, "unresolved-proc-macro")).is_none());
        assert!(to_diag(f, &mk(2, "E0308")).is_none());
        assert!(to_diag(f, &mk(1, "E0282")).is_none());
        let syn = json!({"severity": 1, "code": "syntax-error", "message": "Syntax Error: expected expression",
                         "range": {"start": {"line": 0, "character": 0}}});
        let d = to_diag(f, &syn).unwrap();
        assert_eq!(
            (d.code.as_str(), d.message.as_str()),
            ("", "expected expression")
        );
        let unknown = json!({"severity": 1, "code": "E0308", "message": "expected {unknown}",
                             "range": {"start": {"line": 1, "character": 1}}});
        assert!(to_diag(f, &unknown).is_none());
        let fnptr = json!({"severity": 1, "code": "E0277",
                           "message": "the trait bound `fn label(&str, u8) -> &str: FnOnce<(&str,)>` is not satisfied",
                           "range": {"start": {"line": 1, "character": 1}}});
        assert!(to_diag(f, &fnptr).is_none());
    }

    #[test]
    fn workspace_root_prefers_workspace_table() {
        let d = std::env::temp_dir().join(format!("jr-early-{}", std::process::id()));
        let inner = d.join("crates/a");
        std::fs::create_dir_all(inner.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        std::fs::write(inner.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
        assert_eq!(workspace_root(&inner.join("src")), Some(d.clone()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn signature_changes_but_not_body_edits() {
        let old = "pub fn label(effort: &str) -> &str {\n    effort\n}\nfn two(\n    a: u8,\n) -> u8 {\n    a\n}\n";
        let body = old.replace("    effort\n", "    let _ = 1;\n    effort\n");
        assert!(changed_signatures(old, &body).is_empty());
        let sig = old.replace("(effort: &str)", "(effort: &str, x: u8)");
        assert_eq!(
            changed_signatures(old, &sig),
            BTreeSet::from(["label".to_owned()])
        );
        let multi = old.replace("    a: u8,\n", "    a: u16,\n");
        assert_eq!(
            changed_signatures(old, &multi),
            BTreeSet::from(["two".to_owned()])
        );
        let removed = old.replace("fn two(\n    a: u8,\n) -> u8 {\n    a\n}\n", "");
        assert!(changed_signatures(old, &removed).contains("two"));
    }

    #[test]
    fn finds_files_mentioning_a_name() {
        let texts: HashMap<PathBuf, String> = [
            ("/w/a/x.rs", "fn f() { label(1) }"),
            ("/w/a/y.rs", "fn f() { labels(1) }"),
            ("/w/b/z.rs", "use crate::edited::label;"),
            ("/w/a/edited.rs", "fn label() {}"),
        ]
        .into_iter()
        .map(|(p, t)| (PathBuf::from(p), t.to_owned()))
        .collect();
        let names = BTreeSet::from(["label".to_owned()]);
        let got = mentioning(&texts, &names, &[PathBuf::from("/w/a/edited.rs")], 10);
        assert_eq!(
            got,
            // z.rs names `edited::label`, a real use: first.
            vec![PathBuf::from("/w/b/z.rs"), PathBuf::from("/w/a/x.rs")]
        );
        assert_eq!(mentioning(&texts, &names, &[], 1).len(), 1);
    }

    #[test]
    fn counts_recent_agent_builds_of_a_workspace() {
        let d = std::env::temp_dir().join(format!("jr-early-auto-{}", std::process::id()));
        std::fs::create_dir_all(d.join("ws/src")).unwrap();
        std::fs::write(d.join("ws/Cargo.toml"), "[workspace]\n").unwrap();
        let ws = d.join("ws");
        let line = |t: f64, cwd: &Path, agent: bool| {
            json!({"start": t, "cwd": cwd, "agent_session": if agent { json!("s") } else { Value::Null }})
                .to_string()
        };
        let index = [
            line(100.0, &ws, true), // too old
            line(5000.0, &ws, true),
            line(5100.0, &d, true), // other dir
            line(5200.0, &ws.join("src"), true),
            line(5300.0, &ws, false), // not an agent
        ]
        .join("\n");
        assert_eq!(recent_agent_builds(&index, &ws, 5400.0), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn uri_escapes() {
        assert_eq!(file_uri(Path::new("/a b/c.rs")), "file:///a%20b/c.rs");
    }
}
