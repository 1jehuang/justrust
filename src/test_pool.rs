//! Test binaries in parallel, likely failures first.
//!
//! `cargo test` runs its test binaries one after another and waits for each.
//! A workspace with eleven binaries (Jcode Desktop) spends ~5 s per run on
//! small binaries queued behind each other, `jcode -p jcode-sdk` 10-12 s
//! (FINDINGS section 28).
//!
//! In agent mode the recorder starts a pool on a unix socket in the run
//! directory, and the test runner (`justrust __test-runner --pool=<sock>`)
//! hands a libtest binary to it instead of running it: it sends the program,
//! arguments, working directory and environment, gets an ack, and exits 0 so
//! cargo moves on to the next binary at once. The pool runs up to
//! `JUSTRUST_TEST_JOBS` binaries at a time (binaries that failed last time
//! first, then the longest by recorded wall), captures each one's output,
//! and prints it as one contiguous block when that binary finishes, so
//! failures show up as soon as they are known rather than after every
//! earlier binary. The recorder waits for the pool before it exits and turns
//! any failed binary into a failing exit code.
//!
//! Within a binary, with recorded history, the selected tests run as up to
//! six processes side by side: tests that failed last time plus tests in
//! modules whose files changed since the binary last ran (what the agent
//! is waiting on, reported as soon as they finish), each test whose
//! recorded time alone would set the binary's wall time, and the rest. All
//! use exact names from the binary's own `--list` with the user's filters
//! applied, so together they run exactly the requested tests; libtest's
//! "filtered out" counts are adjusted to match one plain run.
//!
//! Faithfulness: failures observed while other test processes ran are
//! checked again under the shared gate, selectively for cheap failure sets
//! or as the original group for expensive ones. Repeated failures retain
//! their original diagnostics. If failures disappear (or the process crashed),
//! the process is checked again alone
//! and that result is reported, with a note. Anything unexpected (unknown
//! libtest flags, `--nocapture`, one test thread, doc tests, benchmarks, no
//! socket) runs the binary the way cargo would. `JUSTRUST_TEST_PARALLEL=0`
//! turns the pool off, `JUSTRUST_TEST_SPLIT=0` only the per-binary split.

use crate::paths;
use crate::sys::net::{UnixListener, UnixStream};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const SOCK: &str = "tests.sock";
/// Prefix of the runner argument that names the pool socket.
pub const POOL_ARG: &str = "--pool=";
/// `--pool=env`: the socket path is in this variable.
pub const POOL_FROM_ENV: &str = "env";
pub const POOL_ENV: &str = "JUSTRUST_TEST_POOL";

/// Where the pool prints and records a line (implemented by the recorder).
pub trait Sink: Send + Sync {
    /// Print and record lines as one uninterrupted block. `stream` is "o"
    /// (stdout) or "e" (stderr).
    fn emit_block(&self, lines: &[(f64, &'static str, Vec<u8>)]);
}

/// Whether to run test binaries through the pool for this run.
pub fn enabled(agent: bool) -> bool {
    match std::env::var("JUSTRUST_TEST_PARALLEL").as_deref() {
        Ok("0") | Ok("off") | Ok("false") => false,
        Ok("1") | Ok("on") | Ok("true") => true,
        _ => agent,
    }
}

fn max_jobs() -> usize {
    std::env::var("JUSTRUST_TEST_JOBS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n >= 1)
        .unwrap_or_else(|| (crate::procfs::ncpu() / 4).clamp(2, 8))
}

/// One binary handed over by the runner.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub bin: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

// ------------------------------------------------------------------ runner

/// Whether the user's libtest arguments allow running the binary in the
/// pool: plain runs only, never when output is wanted live or tests must not
/// overlap.
pub fn poolable_args(user: &[String]) -> bool {
    let env_set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if env_set("RUST_TEST_NOCAPTURE") || std::env::var("RUST_TEST_THREADS").as_deref() == Ok("1") {
        return false;
    }
    let mut prev_threads = false;
    for a in user {
        if prev_threads && a == "1" {
            return false;
        }
        prev_threads = a == "--test-threads";
        if matches!(
            a.as_str(),
            "--nocapture"
                | "--no-capture"
                | "--test-threads=1"
                | "--list"
                | "--bench"
                | "-h"
                | "--help"
        ) || a.starts_with("--format")
            || a.starts_with("--logfile")
        {
            return false;
        }
    }
    true
}

/// Hand `bin args` to the pool at `sock`. True when the pool took it (the
/// caller then exits 0), false to run it here.
pub fn submit(sock: &Path, bin: &str, args: &[String]) -> bool {
    let Ok(cwd) = std::env::current_dir() else {
        return false;
    };
    let mut env = Vec::new();
    for (k, v) in std::env::vars_os() {
        match (k.into_string(), v.into_string()) {
            (Ok(k), Ok(v)) => env.push((k, v)),
            _ => return false,
        }
    }
    let req = Request {
        bin: bin.to_owned(),
        args: args.to_vec(),
        cwd: cwd.to_string_lossy().into_owned(),
        env,
    };
    let Ok(mut s) = UnixStream::connect(sock) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(mut line) = serde_json::to_string(&req) else {
        return false;
    };
    line.push('\n');
    if s.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let mut ack = String::new();
    BufReader::new(s).read_line(&mut ack).is_ok() && ack.trim() == "ok"
}

// ------------------------------------------------------------------ pool

struct Job {
    id: usize,
    req: Request,
    stem: String,
    priority: (bool, f64),
    arrived: usize,
}

#[derive(Default)]
struct State {
    queue: Vec<Job>,
    running: HashSet<usize>,
    overlapped: HashSet<usize>,
    next_id: usize,
    results: Vec<Finished>,
    cpu_secs: f64,
    /// Fail-fast tripped: a binary failed, so queued and later binaries are
    /// not run (cargo's default stops at the first failing binary).
    stopped: bool,
    skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct TestOutcome {
    name: String,
    passed: bool,
    secs: Option<f64>,
}

struct Finished {
    bin: String,
    stem: String,
    exit: i32,
    start: f64,
    end: f64,
    lines: Vec<(f64, &'static str, Vec<u8>)>,
    /// Every `test x ... ok|FAILED [<1.2s>]` line.
    outcomes: Vec<TestOutcome>,
    /// Tests this process left out only because justrust split the binary:
    /// subtracted from libtest's "filtered out" so totals match one run.
    hide_filtered: Option<u64>,
}

/// One process of a binary: its libtest args.
struct Group {
    args: Vec<String>,
    hide_filtered: Option<u64>,
}

struct Inner {
    state: Mutex<State>,
    cv: Condvar,
    sink: Arc<dyn Sink>,
    me: String,
    jobs: usize,
    history: Mutex<History>,
    gate: Gate,
    fail_fast: bool,
}

pub struct Pool {
    sock: PathBuf,
    inner: Arc<Inner>,
    accept: Option<std::thread::JoinHandle<()>>,
}

/// Result of the whole pool, for the recorder.
pub struct Outcome {
    pub any_failed: bool,
    pub cpu_secs: f64,
}

static LIVE_PIDS: Mutex<Vec<i32>> = Mutex::new(Vec::new());
static CANCEL_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub(crate) fn cancellation_signal() -> i32 {
    CANCEL_SIGNAL.load(std::sync::atomic::Ordering::SeqCst)
}

/// The same pids, readable from a signal handler (no lock). macOS has no
/// `PR_SET_PDEATHSIG`: the recorder forwards a SIGTERM it gets to these.
/// A full table only means an extra process is not signalled.
static SIGNAL_PIDS: [std::sync::atomic::AtomicI32; 64] =
    [const { std::sync::atomic::AtomicI32::new(0) }; 64];

fn track_signal_pid(pid: i32, add: bool) {
    use std::sync::atomic::Ordering;
    let (from, to) = if add { (0, pid) } else { (pid, 0) };
    for slot in &SIGNAL_PIDS {
        if slot
            .compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

/// Send `sig` to every test process the pool is running. Async-signal-safe.
#[cfg(unix)]
pub fn signal_live(sig: libc::c_int) {
    CANCEL_SIGNAL.store(sig, std::sync::atomic::Ordering::SeqCst);
    for slot in &SIGNAL_PIDS {
        let pid = slot.load(std::sync::atomic::Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: kill is async-signal-safe.
            unsafe {
                libc::kill(pid, sig);
            }
        }
    }
}

/// Test processes the pool is running, for the recorder's sampler (they are
/// children of the recorder, not of cargo).
pub fn live_pids() -> Vec<i32> {
    LIVE_PIDS.lock().map(|v| v.clone()).unwrap_or_default()
}

impl Pool {
    /// Listen in `run_dir`. `None` when the socket cannot be made (fail open:
    /// binaries then run under cargo as usual).
    pub fn start(
        run_dir: &Path,
        cwd: &Path,
        me: &Path,
        sink: Arc<dyn Sink>,
        fail_fast: bool,
    ) -> Option<Pool> {
        let sock = run_dir.join(SOCK);
        // sockaddr_un holds 108 bytes.
        if sock.as_os_str().len() >= 100 {
            return None;
        }
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).ok()?;
        let inner = Arc::new(Inner {
            state: Mutex::default(),
            cv: Condvar::new(),
            sink,
            me: me.to_string_lossy().into_owned(),
            jobs: max_jobs(),
            history: Mutex::new(History::load(cwd)),
            gate: Gate::default(),
            fail_fast,
        });
        let acc = inner.clone();
        let accept = std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                // An empty connection from `finish` stops the loop.
                if !accept_one(&acc, conn) {
                    break;
                }
            }
        });
        Some(Pool {
            sock,
            inner,
            accept: Some(accept),
        })
    }

    /// The runner argument that points at this pool: a constant, with the
    /// socket in [`POOL_ENV`], so cargo's argv is the same every run (a
    /// jr-cargo daemon keys its parked state on argv).
    pub fn runner_arg(&self) -> String {
        format!("{POOL_ARG}{POOL_FROM_ENV}")
    }

    /// The socket, for [`POOL_ENV`].
    pub fn sock(&self) -> &Path {
        &self.sock
    }

    /// After cargo exited: stop accepting, wait for every binary, confirm
    /// failures that may come from running in parallel, save history.
    pub fn finish(mut self) -> Outcome {
        // Wake the accept loop with an empty connection.
        let _ = UnixStream::connect(&self.sock);
        if let Some(a) = self.accept.take() {
            let _ = a.join();
        }
        let _ = std::fs::remove_file(&self.sock);
        let inner = &self.inner;
        let mut st = inner.state.lock().unwrap();
        while !st.queue.is_empty() || !st.running.is_empty() {
            st = inner.cv.wait(st).unwrap();
        }
        let any_failed = st.results.iter().any(|r| r.exit != 0);
        if !st.skipped.is_empty() {
            let msg = format!(
                "justrust: not run after the first failing test binary (cargo's fail-fast; --no-fail-fast runs them all): {}",
                st.skipped.join(", ")
            );
            inner
                .sink
                .emit_block(&[(paths::now(), "e", msg.into_bytes())]);
        }
        let mut h = inner.history.lock().unwrap();
        h.update(&st.results);
        h.save();
        Outcome {
            any_failed,
            cpu_secs: st.cpu_secs,
        }
    }
}

fn accept_one(inner: &Arc<Inner>, mut conn: UnixStream) -> bool {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    let Ok(n) = BufReader::new(&conn).read_line(&mut line) else {
        return true;
    };
    if n == 0 {
        return false;
    }
    let Ok(req) = serde_json::from_str::<Request>(&line) else {
        let _ = conn.write_all(b"no\n");
        return true;
    };
    let stem = stem_of(&req.bin);
    let priority = inner.history.lock().unwrap().priority(&stem);
    {
        let mut st = inner.state.lock().unwrap();
        if st.stopped {
            st.skipped.push(stem);
            drop(st);
            let _ = conn.write_all(b"ok\n");
            return true;
        }
        let id = st.next_id;
        st.next_id += 1;
        st.queue.push(Job {
            id,
            req,
            stem,
            priority,
            arrived: id,
        });
    }
    // Ack only once queued, so `finish` cannot miss the job.
    let _ = conn.write_all(b"ok\n");
    dispatch(inner);
    true
}

/// Start queued jobs while there is room: failed last time first, then the
/// longest recorded wall, then arrival order.
fn dispatch(inner: &Arc<Inner>) {
    let mut st = inner.state.lock().unwrap();
    while st.running.len() < inner.jobs && !st.queue.is_empty() {
        let best = (0..st.queue.len())
            .max_by(|&a, &b| {
                let (ja, jb) = (&st.queue[a], &st.queue[b]);
                ja.priority
                    .0
                    .cmp(&jb.priority.0)
                    .then(ja.priority.1.total_cmp(&jb.priority.1))
                    .then(jb.arrived.cmp(&ja.arrived))
            })
            .unwrap();
        let job = st.queue.remove(best);
        if !st.running.is_empty() {
            let running: Vec<usize> = st.running.iter().copied().collect();
            st.overlapped.extend(running);
            st.overlapped.insert(job.id);
        }
        st.running.insert(job.id);
        let inner = inner.clone();
        std::thread::spawn(move || run_job(&inner, job));
    }
}

fn run_job(inner: &Arc<Inner>, job: Job) {
    let groups = (cancellation_signal() == 0)
        .then(|| plan(inner, &job))
        .flatten()
        .unwrap_or_else(|| {
            vec![Group {
                args: job.req.args.clone(),
                hide_filtered: None,
            }]
        });
    if groups.len() > 1 {
        // Processes of one binary overlap each other.
        inner.state.lock().unwrap().overlapped.insert(job.id);
    }
    std::thread::scope(|s| {
        for g in &groups {
            let (inner, job) = (&**inner, &job);
            s.spawn(move || {
                let mut fin = {
                    let _shared = inner.gate.shared();
                    execute(inner, &job.req, &g.args, &job.stem)
                };
                fin.hide_filtered = g.hide_filtered;
                let overlapped = inner.state.lock().unwrap().overlapped.contains(&job.id);
                if should_confirm(fin.exit, overlapped, cancellation_signal()) {
                    // It ran next to other test processes: confirm alone.
                    fin = confirm_alone(inner, &job.req, &job.stem, &g.args, fin);
                }
                report(inner, &fin);
                let mut st = inner.state.lock().unwrap();
                if fin.exit != 0 && inner.fail_fast {
                    st.stopped = true;
                    let queued: Vec<String> = st.queue.drain(..).map(|j| j.stem).collect();
                    st.skipped.extend(queued);
                }
                st.results.push(fin);
            });
        }
    });
    inner.state.lock().unwrap().running.remove(&job.id);
    dispatch(inner);
    inner.cv.notify_all();
}

/// Run one process of a test binary, capturing its output with timestamps.
fn execute(inner: &Inner, req: &Request, args: &[String], stem: &str) -> Finished {
    let start = paths::now();
    if let sig @ 1.. = cancellation_signal() {
        return Finished {
            bin: req.bin.clone(),
            stem: stem.to_owned(),
            exit: 128 + sig,
            start,
            end: start,
            lines: vec![(
                start,
                "e",
                format!("justrust: {stem}: not started after cancellation (signal {sig})")
                    .into_bytes(),
            )],
            outcomes: Vec::new(),
            hide_filtered: None,
        };
    }
    let mut cmd = Command::new(&req.bin);
    cmd.args(args)
        .current_dir(&req.cwd)
        .env_clear()
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: prctl is async-signal-safe. The test process dies with the
    // recorder (Ctrl-C, a killed agent) instead of running on unseen.
    // Elsewhere the recorder forwards its signals (`signal_live`).
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("justrust: failed to run {}: {e}", req.bin);
            return Finished {
                bin: req.bin.clone(),
                stem: stem.to_owned(),
                exit: 127,
                start,
                end: paths::now(),
                lines: vec![(start, "e", msg.into_bytes())],
                outcomes: Vec::new(),
                hide_filtered: None,
            };
        }
    };
    let job = crate::sys::Job::for_child(&child);
    let pid = child.id() as i32;
    LIVE_PIDS.lock().unwrap().push(pid);
    track_signal_pid(pid, true);
    // Close the race between the cancellation check and publishing this PID.
    #[cfg(unix)]
    if let sig @ 1.. = cancellation_signal() {
        // SAFETY: this is our still-owned child, not yet reaped.
        unsafe {
            libc::kill(pid, sig);
        }
    }
    inner.sink.emit_block(&[(
        start,
        "e",
        format!("justrust: test process started: {stem} (pid {pid})").into_bytes(),
    )]);
    let read = |r: Box<dyn Read + Send>, s: &'static str| {
        let sink = inner.sink.clone();
        let stem = stem.to_owned();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut r = BufReader::new(r);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let line = buf.strip_suffix(b"\n").unwrap_or(&buf).to_vec();
                let now = paths::now();
                if long_running_warning(&line) {
                    // These are diagnostics, not outcomes. Send them while the
                    // child is alive, including during confirmation retries.
                    // Do not replay them in the final authoritative block.
                    let mut diagnostic = format!("justrust: {stem} (pid {pid}): ").into_bytes();
                    diagnostic.extend_from_slice(&line);
                    sink.emit_block(&[(now, "e", diagnostic)]);
                } else {
                    out.push((now, s, line));
                }
            }
            out
        })
    };
    let to = read(Box::new(child.stdout.take().unwrap()), "o");
    let te = read(Box::new(child.stderr.take().unwrap()), "e");
    let u = crate::sys::wait_usage(&mut child, job.as_ref());
    let (exit, cpu) = (u.code, u.user + u.sys);
    let mut lines = to.join().unwrap_or_default();
    lines.extend(te.join().unwrap_or_default());
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    LIVE_PIDS.lock().unwrap().retain(|&p| p != pid);
    track_signal_pid(pid, false);
    inner.state.lock().unwrap().cpu_secs += cpu;
    let outcomes = lines
        .iter()
        .filter_map(|(_, _, l)| test_outcome(&String::from_utf8_lossy(l)))
        .collect();
    Finished {
        bin: req.bin.clone(),
        stem: stem.to_owned(),
        exit,
        start,
        end: paths::now(),
        lines,
        outcomes,
        hide_filtered: None,
    }
}

/// Only libtest's standalone timeout warning is safe to publish before
/// confirmation. In particular, never expose a provisional FAILED outcome.
fn long_running_warning(line: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    let Some(rest) = text.trim_end_matches('\r').strip_prefix("test ") else {
        return false;
    };
    let Some((name, seconds)) = rest.rsplit_once(" has been running for over ") else {
        return false;
    };
    !name.is_empty()
        && seconds
            .strip_suffix(" seconds")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Print one finished process as a block: a `Running` header (the summary
/// attributes the lines below it to the binary), its output, and for a
/// failure the error line cargo would have printed.
fn report(inner: &Inner, f: &Finished) {
    let header = format!(
        "     Running `{} {} {}`",
        inner.me,
        crate::test_runner::SUBCOMMAND,
        f.bin
    );
    let mut block = vec![(f.start, "e", header.into_bytes())];
    for (t, s, l) in &f.lines {
        let l = match f.hide_filtered {
            Some(n) => adjust_filtered(l, n),
            None => l.clone(),
        };
        block.push((*t, *s, l));
    }
    if f.exit != 0 {
        let msg = format!(
            "error: test failed: {} exited with code {} (justrust ran test binaries in parallel; JUSTRUST_TEST_PARALLEL=0 runs them one at a time)",
            f.stem, f.exit
        );
        block.push((f.end, "e", msg.into_bytes()));
    }
    inner.sink.emit_block(&block);
}

/// Subtract `n` from the "N filtered out" count of a `test result:` line.
fn adjust_filtered(line: &[u8], n: u64) -> Vec<u8> {
    let text = String::from_utf8_lossy(line);
    if !text.trim_start().starts_with("test result: ") {
        return line.to_vec();
    }
    let Some(end) = text.find(" filtered out") else {
        return line.to_vec();
    };
    let start = text[..end].rfind(' ').map_or(0, |i| i + 1);
    let Ok(v) = text[start..end].parse::<u64>() else {
        return line.to_vec();
    };
    format!("{}{}{}", &text[..start], v.saturating_sub(n), &text[end..]).into_bytes()
}

/// Prefer selective serial retries only for a small, inexpensive failure set.
/// With many failures, serializing their execution can cost much more than
/// replaying the original group with its normal libtest thread count.
fn selective_retry_is_cheap(first: &Finished) -> bool {
    let failed: Vec<_> = first.outcomes.iter().filter(|o| !o.passed).collect();
    failed.len() <= 8
        && failed.iter().filter_map(|o| o.secs).sum::<f64>()
            <= (first.end - first.start).max(0.0) * 2.0
}

fn should_confirm(exit: i32, overlapped: bool, cancelled: i32) -> bool {
    exit != 0 && overlapped && cancelled == 0
}

/// A process failed while other test processes ran. Rerun its failed
/// tests by exact name, one at a time (some tests fail only next to another
/// test of the same process): if every one fails again, the first result
/// stands and is reported at once. If some pass, they are rerun once more
/// with no other test process running; the ones that still pass are
/// reported as failing only in parallel, the rest as failures. A
/// process that died without naming a failed test (a crash) is run again
/// whole and alone. Groups with many or expensive failures first replay
/// together under the shared gate, then alone only if a failure disappears: serial
/// confirmation of 120 failures added 84s to a 15s rewrite test group.
/// Small failure sets remain selective: rerunning a 2,500-test binary to
/// settle three flaky names doubled `test -p jcode-tui --lib` (FINDINGS 41).
fn confirm_alone(
    inner: &Inner,
    req: &Request,
    stem: &str,
    args: &[String],
    first: Finished,
) -> Finished {
    if !should_confirm(first.exit, true, cancellation_signal()) {
        return first;
    }
    let failed_owned: Vec<String> = failed_names(&first)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let failed: Vec<&str> = failed_owned.iter().map(String::as_str).collect();
    let rerun = |names: &[&str], exclusive: bool| -> Option<HashSet<String>> {
        let mut again = strip_filters(args)?;
        again.push("--exact".into());
        if !again.iter().any(|a| a.starts_with("--test-threads")) {
            again.push("--test-threads=1".into());
        }
        again.extend(names.iter().map(|s| s.to_string()));
        let _g = if exclusive {
            inner.gate.exclusive()
        } else {
            inner.gate.shared()
        };
        let r = execute(inner, req, &again, stem);
        if cancellation_signal() != 0 {
            return None;
        }
        Some(failed_names(&r).into_iter().map(str::to_owned).collect())
    };
    let selective = selective_retry_is_cheap(&first);
    if !failed.is_empty() && selective {
        // Quick check, next to whatever else runs: a real failure does not
        // wait for the pool to drain.
        if let Some(still) = rerun(&failed, false) {
            let passed: Vec<&str> = failed
                .iter()
                .copied()
                .filter(|f| !still.contains(*f))
                .collect();
            if passed.is_empty() {
                return first;
            }
            // Settle only the ones that passed, alone.
            if let Some(again) = rerun(&passed, true) {
                let flaky: Vec<&str> = passed
                    .iter()
                    .copied()
                    .filter(|f| !again.contains(*f))
                    .collect();
                let mut out = first;
                if !flaky.is_empty() {
                    out = drop_failures(out, &flaky);
                    out.lines.push((
                        paths::now(),
                        "e",
                        format!(
                            "justrust: {stem}: {} failed while other test processes ran in parallel and passed when rerun alone by name; not counted as failures",
                            flaky.join(", ")
                        )
                        .into_bytes(),
                    ));
                }
                return out;
            }
        }
    }
    if !failed.is_empty() && !selective {
        // A large failure set is not evidence of interference. First replay
        // the original group under the shared gate, retaining its libtest
        // parallelism and filters. Only failures that disappear need an
        // exclusive confirmation. Going straight to the exclusive gate made
        // hundreds of deterministic compiler failures serialize the pool.
        let repeated = {
            let _shared = inner.gate.shared();
            execute(inner, req, args, stem)
        };
        if cancellation_signal() != 0 {
            return first;
        }
        let still_failed: HashSet<&str> = failed_names(&repeated).into_iter().collect();
        if failed.iter().all(|name| still_failed.contains(name)) {
            return first;
        }
        let note = format!(
            "justrust: {stem}: confirming {} failures by replaying the original test group alone instead of retrying each test serially",
            failed.len()
        );
        inner
            .sink
            .emit_block(&[(paths::now(), "e", note.into_bytes())]);
    }
    if cancellation_signal() != 0 {
        return first;
    }
    let _alone = inner.gate.exclusive();
    let mut alone = execute(inner, req, args, stem);
    alone.hide_filtered = first.hide_filtered;
    let alone_passed: HashSet<&str> = alone
        .outcomes
        .iter()
        .filter(|o| o.passed)
        .map(|o| o.name.as_str())
        .collect();
    let flaky: Vec<&str> = failed
        .iter()
        .copied()
        .filter(|f| alone_passed.contains(f))
        .collect();
    if alone.exit == 0 || !flaky.is_empty() {
        let what = if failed.is_empty() {
            format!("exited with code {}", first.exit)
        } else {
            format!("failed ({})", flaky.join(", "))
        };
        let note = format!(
            "justrust: {stem} {what} while other test processes ran in parallel, and passed when run alone; reporting the run alone"
        );
        alone.lines.push((paths::now(), "e", note.into_bytes()));
    }
    alone
}

fn failed_names(f: &Finished) -> Vec<&str> {
    f.outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| o.name.as_str())
        .collect()
}

/// `f` with the tests in `flaky` counted as passed: their outcome, their
/// `test x ... FAILED` line, and the `test result:` counts. The process
/// exits 0 when nothing else failed. Their failure output stays (it is the
/// evidence), with the note the caller adds.
fn drop_failures(mut f: Finished, flaky: &[&str]) -> Finished {
    let flaky: HashSet<String> = flaky.iter().map(|s| s.to_string()).collect();
    for o in &mut f.outcomes {
        if flaky.contains(&o.name) {
            o.passed = true;
        }
    }
    let n = flaky.len() as u64;
    for (_, _, l) in &mut f.lines {
        let text = String::from_utf8_lossy(l).into_owned();
        if let Some((name, rest)) = text
            .strip_prefix("test ")
            .and_then(|r| r.rsplit_once(" ... "))
            && flaky.contains(name)
            && rest.starts_with("FAILED")
        {
            *l = format!("test {name} ... ok{}", &rest["FAILED".len()..]).into_bytes();
        } else if text.trim_start().starts_with("test result: ") {
            *l = adjust_result(&text, n).into_bytes();
        }
    }
    if failed_names(&f).is_empty() {
        f.exit = 0;
    }
    f
}

/// A `test result:` line with `n` failures moved to passed.
fn adjust_result(text: &str, n: u64) -> String {
    let num = |key: &str| -> Option<(usize, usize, u64)> {
        let end = text.find(key)?;
        let start = text[..end].rfind(' ').map_or(0, |i| i + 1);
        Some((start, end, text[start..end].parse().ok()?))
    };
    let (Some((ps, pe, p)), Some((fs, fe, fl))) = (num(" passed"), num(" failed")) else {
        return text.to_owned();
    };
    let fl2 = fl.saturating_sub(n);
    let p2 = p + (fl - fl2);
    let mut out = format!(
        "{}{}{}{}{}",
        &text[..ps],
        p2,
        &text[pe..fs],
        fl2,
        &text[fe..]
    );
    if fl2 == 0 {
        out = out.replacen("test result: FAILED.", "test result: ok.", 1);
    }
    out
}

/// Lets test processes run together (shared) or one alone (exclusive). A
/// waiting exclusive holder keeps new shared holders out.
#[derive(Default)]
struct Gate {
    state: Mutex<(usize, usize, bool)>,
    cv: Condvar,
}

struct GateGuard<'a> {
    gate: &'a Gate,
    exclusive: bool,
}

impl Gate {
    fn shared(&self) -> GateGuard<'_> {
        let mut st = self.state.lock().unwrap();
        // (active shared, waiting exclusive, exclusive held)
        while st.1 > 0 || st.2 {
            st = self.cv.wait(st).unwrap();
        }
        st.0 += 1;
        GateGuard {
            gate: self,
            exclusive: false,
        }
    }

    fn exclusive(&self) -> GateGuard<'_> {
        let mut st = self.state.lock().unwrap();
        st.1 += 1;
        while st.0 > 0 || st.2 {
            st = self.cv.wait(st).unwrap();
        }
        st.1 -= 1;
        st.2 = true;
        GateGuard {
            gate: self,
            exclusive: true,
        }
    }
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let mut st = self.gate.state.lock().unwrap();
        if self.exclusive {
            st.2 = false;
        } else {
            st.0 -= 1;
        }
        self.gate.cv.notify_all();
    }
}

/// `test name ... ok|FAILED [<1.2s>]`.
fn test_outcome(line: &str) -> Option<TestOutcome> {
    let rest = line.trim().strip_prefix("test ")?;
    let (name, status) = rest.rsplit_once(" ... ")?;
    let (status, time) = match status.split_once(" <") {
        Some((s, t)) => (s, t.strip_suffix("s>").and_then(|t| t.parse().ok())),
        None => (status, None),
    };
    let passed = match status {
        "ok" => true,
        "FAILED" => false,
        _ => return None,
    };
    Some(TestOutcome {
        name: name.to_owned(),
        passed,
        secs: time,
    })
}

/// History key of a test binary: its file name, `x-0123456789abcdef`.
fn stem_of(bin: &str) -> String {
    // Cargo's hash is stable across edits and tells apart a package's lib
    // and bin test binaries (both named after the package).
    bin.rsplit('/').next().unwrap_or(bin).to_owned()
}

// ------------------------------------------------------------------ split

/// libtest flags that take a value.
const VALUE_FLAGS: &[&str] = &[
    "--test-threads",
    "--color",
    "--format",
    "-Z",
    "--logfile",
    "--shuffle-seed",
    "--skip",
];
/// libtest flags without a value that keep their meaning when we add exact
/// test names.
const PLAIN_FLAGS: &[&str] = &[
    "-q",
    "--quiet",
    "--ignored",
    "--include-ignored",
    "--show-output",
    "--report-time",
    "--ensure-time",
    "--exact",
    "--test",
];

/// The user's args with filters (`--skip X` and positional) and `--exact`
/// removed: the base for a run by exact test names. `None` for flags we do
/// not know.
fn strip_filters(args: &[String]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--skip" {
            i += 2;
            continue;
        }
        if a.starts_with("--skip=") || a == "--exact" {
            i += 1;
            continue;
        }
        if VALUE_FLAGS.contains(&a.as_str()) {
            out.push(a.clone());
            out.push(args.get(i + 1)?.clone());
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            let known = PLAIN_FLAGS.contains(&a.as_str())
                || VALUE_FLAGS.iter().any(|f| a.starts_with(&format!("{f}=")));
            if !known {
                return None;
            }
            out.push(a.clone());
        }
        // Positional: a filter, dropped.
        i += 1;
    }
    Some(out)
}

/// The user's selection args only (filters, skips, exact, ignored), for `--list`.
fn list_args(args: &[String]) -> Vec<String> {
    let mut out = vec!["--list".to_owned()];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--skip" {
            out.push(a.clone());
            if let Some(v) = args.get(i + 1) {
                out.push(v.clone());
            }
            i += 2;
            continue;
        }
        if VALUE_FLAGS.contains(&a.as_str()) {
            i += 2;
            continue;
        }
        if !a.starts_with('-')
            || a.starts_with("--skip=")
            || matches!(a.as_str(), "--exact" | "--ignored" | "--include-ignored")
        {
            out.push(a.clone());
        }
        i += 1;
    }
    out
}

/// Module path of a source file: `src/a/b.rs` and `src/a/b/mod.rs` -> `a::b`.
/// Crate roots and non-`src` files give `None`.
pub fn module_of(rel: &str) -> Option<String> {
    let p = rel.strip_prefix("src/")?.strip_suffix(".rs")?;
    let p = p.strip_suffix("/mod").unwrap_or(p);
    if matches!(p, "lib" | "main") || p.contains("bin/") {
        return None;
    }
    Some(p.replace('/', "::"))
}

/// Tests in `names` that live in one of `modules` (by path, or by the last
/// segment when the full path matches nothing, for `#[path]` modules).
pub fn affected<'a>(names: &'a [String], modules: &[String]) -> HashSet<&'a str> {
    let mut out = HashSet::new();
    for m in modules {
        let full = |n: &str| n.starts_with(&format!("{m}::")) || n.contains(&format!("::{m}::"));
        let before = out.len();
        out.extend(names.iter().filter(|n| full(n)).map(String::as_str));
        if out.len() == before
            && let Some(last) = m.rsplit("::").next()
            && last.len() >= 4
            && !matches!(last, "tests" | "test" | "util" | "utils")
        {
            let seg =
                |n: &str| n.starts_with(&format!("{last}::")) || n.contains(&format!("::{last}::"));
            out.extend(names.iter().filter(|n| seg(n)).map(String::as_str));
        }
    }
    out
}

/// Split one binary into processes, as exact-name lists drawn from its own
/// `--list` with the user's filters applied, so together they run exactly
/// the selected tests:
///
/// 1. tests that failed last time and tests in the edited modules (the ones
///    an agent is waiting on), reported as soon as they finish,
/// 2. each test whose recorded duration alone is a large part of the binary
///    (it would set the binary's wall time and hold a libtest thread),
/// 3. everything else.
///
/// `None` to run the binary whole, as cargo would.
fn plan(inner: &Inner, job: &Job) -> Option<Vec<Group>> {
    if std::env::var("JUSTRUST_TEST_SPLIT").as_deref() == Ok("0") {
        return None;
    }
    let (failed, durations, last_run) = {
        let h = inner.history.lock().unwrap();
        (
            h.failed(&job.stem),
            h.durations(&job.stem),
            h.last_run(&job.stem),
        )
    };
    // Files of this package edited since this binary last ran.
    let modules: Vec<String> = match (
        last_run,
        job.req.env.iter().find(|(k, _)| k == "CARGO_MANIFEST_DIR"),
    ) {
        (Some(since), Some((_, dir))) => edited_modules(Path::new(dir), since),
        _ => Vec::new(),
    };
    let slow_candidates = durations.values().any(|&d| d >= SLOW_TEST_SECS);
    if modules.is_empty() && failed.is_empty() && !slow_candidates {
        return None;
    }
    let base = strip_filters(&job.req.args)?;
    let out = Command::new(&job.req.bin)
        .args(list_args(&job.req.args))
        .current_dir(&job.req.cwd)
        .env_clear()
        .envs(job.req.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    // Benchmarks run once as tests in a test run; exact-name lists of tests
    // would leave them out. Never drop a selected item: run whole.
    if listing.lines().any(|l| l.ends_with(": benchmark")) {
        return None;
    }
    let names: Vec<String> = listing
        .lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .map(str::to_owned)
        .collect();
    let groups = partition(&names, &modules, &failed, &durations);
    if groups.len() < 2 {
        return None;
    }
    Some(
        groups
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                // The first process reports the user's own filtering, the
                // others none: together, the counts of one plain run.
                let hide = if i == 0 {
                    (names.len() - g.len()) as u64
                } else {
                    u64::MAX
                };
                let mut args = base.clone();
                args.push("--exact".into());
                args.extend(g.into_iter().map(str::to_owned));
                Group {
                    args,
                    hide_filtered: Some(hide),
                }
            })
            .collect(),
    )
}

/// Modules (`a::b`) of `.rs` files under `<pkg>/src` modified after `since`.
fn edited_modules(pkg: &Path, since: f64) -> Vec<String> {
    let src = pkg.join("src");
    let mut out = Vec::new();
    let mut stack = vec![src.clone()];
    let mut seen = 0usize;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > 20_000 {
                return out;
            }
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(m) = e.metadata()
                && let Ok(mt) = m.modified()
                && mt
                    .duration_since(std::time::UNIX_EPOCH)
                    .is_ok_and(|d| d.as_secs_f64() > since)
                && let Ok(rel) = p.strip_prefix(pkg)
                && let Some(m) = module_of(&rel.to_string_lossy())
            {
                out.push(m);
            }
        }
    }
    out
}

/// A test this slow (recorded) is worth its own process.
const SLOW_TEST_SECS: f64 = 1.0;
/// At most this many slow tests get their own process per binary.
const MAX_SLOW_SPLITS: usize = 4;

/// The groups of [`plan`], first group first. Every name lands in exactly one.
fn partition<'a>(
    names: &'a [String],
    modules: &[String],
    failed: &HashSet<String>,
    durations: &HashMap<String, f64>,
) -> Vec<Vec<&'a str>> {
    let mut first = affected(names, modules);
    first.extend(
        names
            .iter()
            .filter(|n| failed.contains(*n))
            .map(String::as_str),
    );
    let total: f64 = names.iter().filter_map(|n| durations.get(n)).sum();
    let mut slow: Vec<(&str, f64)> = names
        .iter()
        .map(String::as_str)
        .filter(|n| !first.contains(n))
        .filter_map(|n| durations.get(n).map(|&d| (n, d)))
        .filter(|&(_, d)| d >= SLOW_TEST_SECS && d >= 0.25 * total)
        .collect();
    slow.sort_by(|a, b| b.1.total_cmp(&a.1));
    slow.truncate(MAX_SLOW_SPLITS);
    let slow_set: HashSet<&str> = slow.iter().map(|s| s.0).collect();
    let mut first: Vec<&str> = first.into_iter().collect();
    first.sort_unstable();
    let rest: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| !first.contains(n) && !slow_set.contains(n))
        .collect();
    let mut groups = Vec::new();
    if !first.is_empty() {
        groups.push(first);
    }
    groups.extend(slow.into_iter().map(|s| vec![s.0]));
    if !rest.is_empty() {
        groups.push(rest);
    }
    groups
}

// ------------------------------------------------------------------ history

/// Per workspace: per binary, the last wall time, the tests that failed and
/// have not passed since, and recorded per-test durations.
#[derive(Debug, Serialize, Deserialize, Default)]
struct History {
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(default)]
    binaries: HashMap<String, BinHistory>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct BinHistory {
    wall: f64,
    #[serde(default)]
    failed: Vec<String>,
    /// Only tests of at least `KEEP_TEST_SECS`, to keep the file small.
    #[serde(default)]
    slow: HashMap<String, f64>,
    updated: f64,
    /// Start of the last run: files modified after it are new edits.
    #[serde(default)]
    started: f64,
}

const KEEP_TEST_SECS: f64 = 0.2;

impl History {
    fn load(cwd: &Path) -> History {
        let Ok(home) = paths::home() else {
            return History::default();
        };
        let path = home.join("tests").join(format!(
            "{:016x}.json",
            fnv(cwd.as_os_str().as_encoded_bytes())
        ));
        let mut h: History = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        h.path = Some(path);
        h
    }

    /// (failed last time, recorded wall). Unknown binaries sort as long.
    fn priority(&self, stem: &str) -> (bool, f64) {
        match self.binaries.get(stem) {
            Some(b) => (!b.failed.is_empty(), b.wall),
            None => (false, f64::MAX),
        }
    }

    fn failed(&self, stem: &str) -> HashSet<String> {
        self.binaries
            .get(stem)
            .map(|b| b.failed.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// When this binary's last recorded run started.
    fn last_run(&self, stem: &str) -> Option<f64> {
        self.binaries
            .get(stem)
            .map(|b| b.started)
            .filter(|&t| t > 0.0)
    }

    fn durations(&self, stem: &str) -> HashMap<String, f64> {
        self.binaries
            .get(stem)
            .map(|b| b.slow.clone())
            .unwrap_or_default()
    }

    /// Fold in every process of one run.
    fn update(&mut self, results: &[Finished]) {
        let mut wall: HashMap<&str, (f64, f64)> = HashMap::new();
        for f in results {
            let w = wall.entry(&f.stem).or_insert((f.start, f.end));
            w.0 = w.0.min(f.start);
            w.1 = w.1.max(f.end);
            let b = self.binaries.entry(f.stem.clone()).or_default();
            for o in &f.outcomes {
                b.failed.retain(|n| *n != o.name);
                if !o.passed {
                    b.failed.push(o.name.clone());
                }
                match o.secs {
                    Some(s) if s >= KEEP_TEST_SECS => {
                        b.slow.insert(o.name.clone(), s);
                    }
                    Some(_) => {
                        b.slow.remove(&o.name);
                    }
                    None => {}
                }
            }
            b.failed.sort();
            b.failed.dedup();
            b.updated = f.end;
        }
        for (stem, (s, e)) in wall {
            if let Some(b) = self.binaries.get_mut(stem) {
                // A filtered run is short; decay rather than forget the full wall.
                b.wall = (e - s).max(b.wall * 0.5);
                b.started = s;
            }
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if let Ok(b) = serde_json::to_vec(self)
            && std::fs::write(&tmp, b).is_ok()
        {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn fnv(s: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    struct TmpDir(PathBuf);
    #[cfg(unix)]
    impl TmpDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    #[cfg(unix)]
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn drop_failures_counts_flaky_tests_as_passed() {
        let line = |t: &str| (0.0, "o", t.as_bytes().to_vec());
        let out = |name: &str, passed| TestOutcome {
            name: name.into(),
            passed,
            secs: None,
        };
        let f = Finished {
            bin: "b".into(),
            stem: "b".into(),
            exit: 101,
            start: 0.0,
            end: 1.0,
            lines: vec![
                line("test a::x ... FAILED <0.057s>"),
                line("test a::y ... FAILED"),
                line("test a::z ... ok"),
                line(
                    "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s",
                ),
            ],
            outcomes: vec![out("a::x", false), out("a::y", false), out("a::z", true)],
            hide_filtered: None,
        };
        let g = drop_failures(f, &["a::x"]);
        let text: Vec<String> = g
            .lines
            .iter()
            .map(|l| String::from_utf8_lossy(&l.2).into_owned())
            .collect();
        assert_eq!(text[0], "test a::x ... ok <0.057s>");
        assert_eq!(text[1], "test a::y ... FAILED");
        assert!(
            text[3].starts_with("test result: FAILED. 2 passed; 1 failed;"),
            "{}",
            text[3]
        );
        assert_eq!(g.exit, 101);
        let h = drop_failures(g, &["a::y"]);
        let last = String::from_utf8_lossy(&h.lines[3].2).into_owned();
        assert!(
            last.starts_with("test result: ok. 3 passed; 0 failed;"),
            "{last}"
        );
        assert_eq!(h.exit, 0);
        assert!(failed_names(&h).is_empty());
    }

    #[test]
    fn binaries_are_keyed_by_file_name() {
        assert_eq!(
            stem_of("/t/debug/deps/jcode_desktop_ui-945cf00a983ec5be"),
            "jcode_desktop_ui-945cf00a983ec5be"
        );
    }

    #[test]
    fn maps_files_to_modules() {
        assert_eq!(module_of("src/a/b.rs").as_deref(), Some("a::b"));
        assert_eq!(module_of("src/a/b/mod.rs").as_deref(), Some("a::b"));
        assert_eq!(module_of("src/lib.rs"), None);
        assert_eq!(module_of("tests/x.rs"), None);
    }

    #[test]
    fn finds_tests_in_edited_modules() {
        let names = s(&[
            "slots::tests::refresh",
            "slots_extra::tests::x",
            "panel::panel_login::tests::y",
            "other::z",
        ]);
        let got = affected(&names, &s(&["slots"]));
        assert_eq!(got, HashSet::from(["slots::tests::refresh"]));
        // `#[path]`-style: file panel_login.rs, module panel::panel_login.
        let got = affected(&names, &s(&["panel_login"]));
        assert_eq!(got, HashSet::from(["panel::panel_login::tests::y"]));
    }

    #[test]
    fn strips_filters_and_keeps_known_flags() {
        assert_eq!(
            strip_filters(&s(&[
                "-Z",
                "unstable-options",
                "--report-time",
                "fps",
                "--skip",
                "x",
                "-q"
            ])),
            Some(s(&["-Z", "unstable-options", "--report-time", "-q"]))
        );
        assert_eq!(strip_filters(&s(&["--weird"])), None);
        assert_eq!(
            list_args(&s(&[
                "-Z",
                "unstable-options",
                "fps",
                "--skip",
                "x",
                "--exact"
            ])),
            s(&["--list", "fps", "--skip", "x", "--exact"])
        );
    }

    #[test]
    fn pool_refuses_live_output_and_one_thread() {
        assert!(poolable_args(&s(&["fps"])));
        assert!(!poolable_args(&s(&["--nocapture"])));
        assert!(!poolable_args(&s(&["--test-threads", "1"])));
        assert!(!poolable_args(&s(&["--test-threads=1"])));
        assert!(poolable_args(&s(&["--test-threads", "4"])));
        assert!(!poolable_args(&s(&["--list"])));
        assert!(!poolable_args(&s(&[
            "-Z",
            "unstable-options",
            "--format",
            "json"
        ])));
    }

    #[test]
    fn parses_test_outcomes() {
        let o = |n: &str, p, secs| TestOutcome {
            name: n.into(),
            passed: p,
            secs,
        };
        assert_eq!(
            test_outcome("test a::b ... ok <0.100s>"),
            Some(o("a::b", true, Some(0.1)))
        );
        assert_eq!(test_outcome("test a ... FAILED"), Some(o("a", false, None)));
        assert_eq!(test_outcome("test a ... ignored"), None);
    }

    fn fin(outcomes: &[(&str, bool, f64)]) -> Finished {
        Finished {
            bin: "/t/deps/b-0123456789abcdef".into(),
            stem: "b".into(),
            exit: 0,
            start: 0.0,
            end: 2.0,
            lines: Vec::new(),
            hide_filtered: None,
            outcomes: outcomes
                .iter()
                .map(|&(n, p, s)| TestOutcome {
                    name: n.into(),
                    passed: p,
                    secs: Some(s),
                })
                .collect(),
        }
    }

    #[test]
    fn retry_policy_keeps_small_failures_selective() {
        assert!(selective_retry_is_cheap(&fin(&[("a", false, 0.1)])));
        assert!(selective_retry_is_cheap(&fin(&[("a", false, 4.0)])));
        assert!(!selective_retry_is_cheap(&fin(&[("a", false, 4.1)])));
        let mut many = fin(&[]);
        many.outcomes = (0..8)
            .map(|n| TestOutcome {
                name: format!("test_{n}"),
                passed: false,
                secs: None,
            })
            .collect();
        assert!(selective_retry_is_cheap(&many));
        many.outcomes.push(TestOutcome {
            name: "ninth".into(),
            passed: false,
            secs: None,
        });
        assert!(!selective_retry_is_cheap(&many));
    }

    #[cfg(unix)]
    #[test]
    fn many_failures_replay_original_group_and_preserve_exit() {
        let dir = TmpDir(std::env::temp_dir().join(format!("jr-retry-{}", std::process::id())));
        std::fs::create_dir_all(dir.path()).unwrap();
        let bin = dir.path().join("retry");
        let calls = dir.path().join("calls");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\necho 'test t0 ... FAILED'\necho 'test t1 ... ok'\nexit 101\n",
                calls.display()
            ),
        )
        .unwrap();
        crate::sys::set_mode(&bin, 0o755).unwrap();
        let sink = Arc::new(VecSink(Mutex::new(Vec::new())));
        let pool = Pool::start(dir.path(), dir.path(), Path::new("jr"), sink, true).unwrap();
        let req = Request {
            bin: bin.to_string_lossy().into_owned(),
            args: s(&["group", "--skip", "excluded"]),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Vec::new(),
        };
        let mut first = fin(&[]);
        first.exit = 101;
        first.hide_filtered = Some(7);
        first.outcomes = (0..120)
            .map(|n| TestOutcome {
                name: format!("t{n}"),
                passed: false,
                secs: Some(0.1),
            })
            .collect();
        let replay = confirm_alone(&pool.inner, &req, "retry", &req.args, first);
        assert_eq!(replay.exit, 101);
        assert_eq!(replay.hide_filtered, Some(7));
        assert_eq!(failed_names(&replay), ["t0"]);
        assert_eq!(
            std::fs::read_to_string(calls).unwrap(),
            "group --skip excluded\ngroup --skip excluded\n"
        );
        pool.finish();
    }

    /// Repeatable expensive failures must not wait for an unrelated shared
    /// holder to drain. Release it on timeout too so regressions cannot hang.
    #[cfg(unix)]
    #[test]
    fn expensive_repeatable_failure_keeps_shared_gate() {
        let dir =
            TmpDir(std::env::temp_dir().join(format!("jr-shared-retry-{}", std::process::id())));
        std::fs::create_dir_all(dir.path()).unwrap();
        let bin = dir.path().join("retry");
        std::fs::write(&bin, "#!/bin/sh\necho 'test slow ... FAILED'\nexit 101\n").unwrap();
        crate::sys::set_mode(&bin, 0o755).unwrap();
        let sink = Arc::new(VecSink(Mutex::new(Vec::new())));
        let pool = Pool::start(dir.path(), dir.path(), Path::new("jr"), sink, true).unwrap();
        let req = Request {
            bin: bin.to_string_lossy().into_owned(),
            args: s(&["group", "--skip", "excluded"]),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Vec::new(),
        };
        let mut first = fin(&[]);
        first.exit = 101;
        first.hide_filtered = Some(7);
        first.outcomes = vec![TestOutcome {
            name: "slow".into(),
            passed: false,
            secs: Some(100.0),
        }];
        let shared = pool.inner.gate.shared();
        let inner = pool.inner.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = confirm_alone(&inner, &req, "retry", &req.args, first);
            tx.send(result).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(shared);
        worker.join().unwrap();
        let result = result.expect("repeatable failure waited for the exclusive gate");
        assert_eq!(result.exit, 101);
        assert_eq!(result.hide_filtered, Some(7));
        assert_eq!(failed_names(&result), ["slow"]);
        pool.finish();
    }

    #[cfg(unix)]
    struct VecSink(Mutex<Vec<String>>);
    #[cfg(unix)]
    impl Sink for VecSink {
        fn emit_block(&self, lines: &[(f64, &'static str, Vec<u8>)]) {
            let mut v = self.0.lock().unwrap();
            v.extend(
                lines
                    .iter()
                    .map(|l| String::from_utf8_lossy(&l.2).into_owned()),
            );
        }
    }

    #[test]
    fn timeout_warning_is_not_a_test_outcome() {
        assert!(long_running_warning(
            b"test module::slow has been running for over 60 seconds"
        ));
        assert!(long_running_warning(
            b"test module::slow has been running for over 120 seconds\r"
        ));
        for line in [
            &b"test module::slow ... FAILED"[..],
            b"test module::slow ... ok",
            b"test result: FAILED. 0 passed; 1 failed",
            b"test x has been running for over many seconds",
            b"test x has been running for over 60 seconds trailing output",
            b"\xfftest x has been running for over 60 seconds",
        ] {
            assert!(!long_running_warning(line));
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_does_not_spawn_or_confirm_tests() {
        const FLAG: &str = "JUSTRUST_TEST_CANCEL_FIXTURE";
        if std::env::var_os(FLAG).is_some() {
            let dir =
                TmpDir(std::env::temp_dir().join(format!("jr-cancel-{}", std::process::id())));
            std::fs::create_dir_all(dir.path()).unwrap();
            let sink = Arc::new(VecSink(Mutex::new(Vec::new())));
            let pool = Pool::start(dir.path(), dir.path(), Path::new("jr"), sink, false).unwrap();
            let req = Request {
                bin: "/bin/sh".into(),
                args: s(&["-c", "echo unexpected > spawned"]),
                cwd: dir.path().to_string_lossy().into_owned(),
                env: vec![],
            };
            let ready = dir.path().join("ready");
            let cancel = std::thread::spawn(move || {
                for _ in 0..250 {
                    if ready.exists() {
                        signal_live(libc::SIGTERM);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                panic!("test process did not become ready");
            });
            let first = execute(
                &pool.inner,
                &req,
                &s(&[
                    "-c",
                    "echo 'test failed ... FAILED'; echo ready > ready; exec /bin/sleep 5",
                ]),
                "cancelled",
            );
            cancel.join().unwrap();
            assert_eq!(first.exit, 128 + libc::SIGTERM);
            let result = confirm_alone(&pool.inner, &req, "cancelled", &req.args, first);
            assert_eq!(failed_names(&result), ["failed"]);
            assert_ne!(result.exit, 0);
            let pending = execute(&pool.inner, &req, &req.args, "pending");
            assert_eq!(pending.exit, 128 + libc::SIGTERM);
            assert!(!dir.path().join("spawned").exists());
            pool.finish();
            return;
        }
        assert!(should_confirm(101, true, 0));
        assert!(
            should_confirm(134, true, 0),
            "crashes still need confirmation"
        );
        assert!(!should_confirm(101, true, libc::SIGTERM));
        assert!(!should_confirm(101, true, libc::SIGINT));
        assert!(!should_confirm(0, true, 0));
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "test_pool::tests::cancellation_does_not_spawn_or_confirm_tests",
                "--nocapture",
            ])
            .env(FLAG, "1")
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// A child waits for the sink to acknowledge its warning. Buffering until
    /// exit cannot pass this test, even on a fast machine.
    #[cfg(unix)]
    #[test]
    fn timeout_warning_is_live_and_not_replayed() {
        struct AcknowledgeSink {
            lines: Mutex<Vec<String>>,
            release: PathBuf,
        }
        impl Sink for AcknowledgeSink {
            fn emit_block(&self, lines: &[(f64, &'static str, Vec<u8>)]) {
                for (_, _, raw) in lines {
                    let text = String::from_utf8_lossy(raw).into_owned();
                    if text.contains("has been running for over") {
                        std::fs::write(&self.release, "ack").unwrap();
                    }
                    self.lines.lock().unwrap().push(text);
                }
            }
        }
        let dir = TmpDir(std::env::temp_dir().join(format!("jr-live-{}", std::process::id())));
        std::fs::create_dir_all(dir.path()).unwrap();
        let sink = Arc::new(AcknowledgeSink {
            lines: Mutex::new(Vec::new()),
            release: dir.path().join("release"),
        });
        let pool =
            Pool::start(dir.path(), dir.path(), Path::new("jr"), sink.clone(), false).unwrap();
        let req = Request {
            bin: "/bin/sh".into(),
            args: vec![],
            cwd: dir.path().to_string_lossy().into_owned(),
            env: vec![],
        };
        for stream in ["", " >&2"] {
            let _ = std::fs::remove_file(&sink.release);
            let script = format!(
                "echo 'test module::slow has been running for over 60 seconds'{stream}; \
                 i=0; while [ ! -f release ] && [ $i -lt 100 ]; do /bin/sleep 0.02; i=$((i+1)); done; \
                 [ -f release ] || exit 99; \
                 echo 'test module::slow ... FAILED'; echo 'failure details' >&2; exit 101"
            );
            let result = execute(&pool.inner, &req, &["-c".into(), script], "fixture");
            assert_eq!(result.exit, 101, "warning was not delivered before exit");
            assert_eq!(failed_names(&result), ["module::slow"]);
            assert!(
                result
                    .lines
                    .iter()
                    .any(|(_, s, l)| *s == "e" && l == b"failure details")
            );
            assert!(!result.lines.iter().any(|(_, _, l)| long_running_warning(l)));
            report(&pool.inner, &result);
        }
        let lines = sink.lines.lock().unwrap();
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.contains("has been running for over"))
                .count(),
            2
        );
        assert_eq!(
            lines
                .iter()
                .filter(|l| *l == "test module::slow ... FAILED")
                .count(),
            2
        );
        assert!(
            lines
                .iter()
                .filter(|l| l.contains("has been running for over"))
                .all(|l| l.starts_with("justrust: fixture (pid "))
        );
        drop(lines);
        pool.finish();
    }

    /// Cargo stops at the first failing test binary unless --no-fail-fast.
    /// Running every binary anyway made `test --workspace` on Jcode run
    /// 8,911 tests against stock's 1,862 (FINDINGS 41).
    #[cfg(unix)]
    fn pool_run(fail_fast: bool) -> (bool, Vec<String>, Vec<String>) {
        // Short: the pool's socket path must fit in sockaddr_un.
        let dir =
            TmpDir(std::env::temp_dir().join(format!("jr-ff-{}-{fail_fast}", std::process::id())));
        let _ = std::fs::remove_dir_all(dir.path());
        std::fs::create_dir_all(dir.path()).unwrap();
        let mk = |name: &str, body: &str| {
            let p = dir.path().join(name);
            std::fs::write(
                &p,
                format!(
                    "#!/bin/sh\necho ran >> {}/ran-{name}\n{body}\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            crate::sys::set_mode(&p, 0o755).unwrap();
            p.to_string_lossy().into_owned()
        };
        let bad = mk("bad", "echo 'test t::a ... FAILED'; exit 101");
        let good = mk("good", "echo 'test t::b ... ok'; exit 0");
        let sink = Arc::new(VecSink(Mutex::new(Vec::new())));
        // One job at a time, so `good` is queued or arrives after `bad` failed.
        unsafe { std::env::set_var("JUSTRUST_TEST_JOBS", "1") };
        let pool = Pool::start(
            dir.path(),
            dir.path(),
            Path::new("jr"),
            sink.clone(),
            fail_fast,
        )
        .unwrap();
        assert!(submit(&pool.sock, &bad, &[]));
        // Let `bad` finish before `good` arrives (cargo runs binaries in order).
        let t0 = std::time::Instant::now();
        while pool.inner.state.lock().unwrap().results.is_empty() && t0.elapsed().as_secs() < 10 {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(submit(&pool.sock, &good, &[]));
        let o = pool.finish();
        let ran: Vec<String> = ["bad", "good"]
            .into_iter()
            .filter(|n| dir.path().join(format!("ran-{n}")).exists())
            .map(String::from)
            .collect();
        let out = sink.0.lock().unwrap().clone();
        (o.any_failed, ran, out)
    }

    // Runs `sh` scripts.
    #[cfg(unix)]
    #[test]
    fn pool_stops_after_a_failing_binary_like_cargo() {
        let (failed, ran, out) = pool_run(true);
        assert!(failed);
        assert_eq!(ran, ["bad"]);
        assert!(
            out.iter().any(
                |l| l.contains("not run after the first failing test binary") && l.contains("good")
            ),
            "{out:?}"
        );
        let (failed, ran, out) = pool_run(false);
        assert!(failed);
        assert_eq!(ran, ["bad", "good"]);
        assert!(!out.iter().any(|l| l.contains("not run after")), "{out:?}");
    }

    #[test]
    fn history_tracks_failures_until_they_pass() {
        let mut h = History::default();
        h.update(&[fin(&[("x", false, 0.1), ("y", true, 3.0)])]);
        assert_eq!(h.priority("b"), (true, 2.0));
        assert!(h.failed("b").contains("x"));
        assert_eq!(h.durations("b").get("y"), Some(&3.0));
        h.update(&[fin(&[("y", true, 0.01)])]);
        assert!(h.failed("b").contains("x"), "x did not run, still suspect");
        assert!(h.durations("b").is_empty(), "y got fast");
        h.update(&[fin(&[("x", true, 0.1), ("y", true, 0.1)])]);
        assert!(h.failed("b").is_empty());
        assert_eq!(h.priority("nope"), (false, f64::MAX));
    }

    #[test]
    fn partition_runs_every_selected_test_exactly_once() {
        let names = s(&["a::t1", "a::t2", "b::slow", "b::fast", "c::flaky", "c::x"]);
        let failed = HashSet::from(["c::flaky".to_owned()]);
        let dur = HashMap::from([("b::slow".to_owned(), 5.0), ("b::fast".to_owned(), 0.3)]);
        let g = partition(&names, &s(&["a"]), &failed, &dur);
        assert_eq!(
            g,
            vec![
                vec!["a::t1", "a::t2", "c::flaky"],
                vec!["b::slow"],
                vec!["b::fast", "c::x"]
            ]
        );
        let mut all: Vec<&str> = g.concat();
        all.sort_unstable();
        let mut want: Vec<&str> = names.iter().map(String::as_str).collect();
        want.sort_unstable();
        assert_eq!(all, want);
        // Nothing known: one group (the caller then runs the binary whole).
        assert_eq!(
            partition(&names, &[], &HashSet::new(), &HashMap::new()).len(),
            1
        );
    }
}
