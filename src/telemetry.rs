//! Privacy-safe usage telemetry (account-linked for scoped hosted events), and the settings file it shares with
//! auto-update (`update.rs`).
//!
//! What is sent: one event per recorded build (subcommand, flags without
//! their values, exit code, timings, unit and cache counts, test counts,
//! waste kinds, which coding agent ran it), one per other justrust command
//! (its name, success, time), auto-update outcomes, and panic locations.
//! Each carries a random install id (`~/.justrust/install_id`), the justrust
//! version, OS, and CPU architecture. Never paths, crate or package names,
//! argument values, hostnames, environment values, or source.
//!
//! Recording costs one appended line (`~/.justrust/telemetry/queue.jsonl`).
//! A detached background process (`justrust __bg`) sends the queue in
//! batches to `{build}/telemetry` at most every few minutes and never delays
//! or fails a build.
//!
//! Off: `justrust telemetry off`, `JUSTRUST_NO_TELEMETRY=1`, or
//! `DO_NOT_TRACK=1`. Turning it off deletes anything not yet sent.

use crate::paths;
use serde_json::{Value, json};
use std::path::PathBuf;

const SETTINGS: &str = "settings.json";
/// Send the queue at most this often (seconds), unless it grows past
/// `FLUSH_BYTES`. `JUSTRUST_TELEMETRY_FLUSH_SECS` overrides (tests).
const FLUSH_SECS: f64 = 300.0;
const FLUSH_BYTES: u64 = 64 * 1024;
/// Unsent data is dropped beyond this (offline machines).
const MAX_QUEUE_BYTES: u64 = 4 * 1024 * 1024;
/// Events per POST (the server takes at most 200).
const BATCH: usize = 150;
pub const BG_ARG: &str = "__bg";

// ------------------------------------------------------------- settings

fn settings_path() -> Option<PathBuf> {
    Some(paths::home().ok()?.join(SETTINGS))
}

fn settings() -> Value {
    settings_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

/// `true`/`false` from `~/.justrust/settings.json`, `None` when unset.
pub(crate) fn setting(key: &str) -> Option<bool> {
    settings()[key].as_bool()
}

pub(crate) fn set_setting(key: &str, value: bool) -> anyhow::Result<()> {
    let p = settings_path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let mut s = settings();
    if !s.is_object() {
        s = json!({});
    }
    s[key] = json!(value);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&s)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

fn env_on(k: &str) -> bool {
    std::env::var(k).is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    })
}

/// Why telemetry is off, or `None` when it is on.
pub fn disabled_reason() -> Option<&'static str> {
    if cfg!(test) {
        return Some("unit tests");
    }
    if env_on("JUSTRUST_NO_TELEMETRY") {
        return Some("JUSTRUST_NO_TELEMETRY is set");
    }
    if env_on("DO_NOT_TRACK") {
        return Some("DO_NOT_TRACK is set");
    }
    if matches!(
        std::env::var("JUSTRUST_TELEMETRY").ok().as_deref(),
        Some("0" | "off" | "false" | "no")
    ) {
        return Some("JUSTRUST_TELEMETRY=off");
    }
    // The hosted build machine runs builds on a client's behalf: the client
    // already reports them.
    if std::env::var_os(crate::route::ON_AGENT).is_some() {
        return Some("remote build agent");
    }
    if setting("telemetry") == Some(false) {
        return Some("turned off with `justrust telemetry off`");
    }
    None
}

pub fn enabled() -> bool {
    disabled_reason().is_none()
}

// ------------------------------------------------------------ install id

/// A random v4 uuid from the OS-seeded std hasher keys (no extra crates).
fn random_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut b = [0u8; 16];
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for (i, chunk) in b.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn is_uuid_v4(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            14 => c == '4',
            19 => matches!(c, '8' | '9' | 'a' | 'b'),
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

/// The installer's id, or a new one saved for next time.
fn install_id() -> Option<String> {
    let p = paths::home().ok()?.join("install_id");
    if let Ok(s) = std::fs::read_to_string(&p) {
        let s = s.trim().to_ascii_lowercase();
        if is_uuid_v4(&s) {
            return Some(s);
        }
    }
    let id = random_uuid();
    std::fs::create_dir_all(p.parent()?).ok()?;
    std::fs::write(&p, format!("{id}\n")).ok()?;
    Some(id)
}

// ----------------------------------------------------------------- queue

fn dir() -> Option<PathBuf> {
    Some(paths::home().ok()?.join("telemetry"))
}

fn queue_lock(d: &std::path::Path) -> Option<std::fs::File> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(d.join("queue.lock"))
        .ok()?;
    f.try_lock().ok()?;
    Some(f)
}

/// Bounded, fail-open lifecycle observer. It never performs network I/O.
pub(crate) struct BuildLifecycle {
    shared: std::sync::Arc<(std::sync::Mutex<(bool, bool)>, std::sync::Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
    id: String,
    sub: String,
    route: &'static str,
    terminal: bool,
    scope: Option<String>,
    logs: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
const HEARTBEAT_S: u64 = 30;

fn lifecycle_data(
    id: &str,
    sub: &str,
    route: &str,
    state: &str,
    exit: Option<i32>,
    logs: bool,
) -> Option<Value> {
    if !valid_run_id(id)
        || !matches!(state, "queued" | "running" | "done" | "abandoned")
        || (state == "done" && exit.is_none())
    {
        return None;
    }
    let route = match route {
        "remote"
        | "local_routed"
        | "local_unrouted"
        | "remote_off"
        | "forced"
        | "recent_remote_failure"
        | "non_linux"
        | "not_configured"
        | "no_history"
        | "non_utf8_args"
        | "cost_comparison"
        | "remote_cost_comparison"
        | "explore_remote"
        | "local_only" => route,
        _ => "other",
    };
    let mut data = json!({"run_id": id, "sub": safe_subcommand(sub), "route": route,
        "state": state, "heartbeat_s": HEARTBEAT_S,
        "log_availability": if logs { "local_only" } else { "none" }});
    if let Some(exit) = exit {
        data["exit"] = json!(exit);
    }
    Some(data)
}
fn emit_lifecycle(
    id: &str,
    sub: &str,
    route: &str,
    state: &str,
    exit: Option<i32>,
    logs: bool,
    scope: Option<&str>,
) {
    let Some(scope) = scope else { return };
    if let Some(data) = lifecycle_data(id, sub, route, state, exit, logs) {
        record_with_scope("build_state", data, Some(scope));
        spawn_background(true);
    }
}
impl BuildLifecycle {
    pub(crate) fn start(id: &str, sub: &str, route: &'static str) -> Self {
        let mut observer = Self {
            shared: Default::default(),
            worker: None,
            id: id.into(),
            sub: sub.into(),
            route,
            terminal: false,
            scope: telemetry_creds().as_ref().map(auth_scope),
            logs: Default::default(),
        };
        if enabled() && observer.scope.is_some() && valid_run_id(id) {
            emit_lifecycle(
                id,
                sub,
                route,
                "queued",
                None,
                false,
                observer.scope.as_deref(),
            );
            let scope = observer.scope.clone();
            let shared = observer.shared.clone();
            let logs = observer.logs.clone();
            let id = id.to_owned();
            let sub = sub.to_owned();
            observer.worker = std::thread::Builder::new()
                .name("build-heartbeat".into())
                .spawn(move || {
                    let (lock, cv) = &*shared;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        let (next, timeout) = cv
                            .wait_timeout(state, std::time::Duration::from_secs(HEARTBEAT_S))
                            .unwrap_or_else(|e| e.into_inner());
                        state = next;
                        if state.1 {
                            break;
                        }
                        if timeout.timed_out() {
                            emit_lifecycle(
                                &id,
                                &sub,
                                route,
                                if state.0 { "running" } else { "queued" },
                                None,
                                logs.load(std::sync::atomic::Ordering::Relaxed),
                                scope.as_deref(),
                            );
                        }
                    }
                })
                .ok();
        }
        observer
    }
    pub(crate) fn log_availability(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.logs.clone()
    }
    pub(crate) fn running(&self) {
        self.shared.0.lock().unwrap_or_else(|e| e.into_inner()).0 = true;
        emit_lifecycle(
            &self.id,
            &self.sub,
            self.route,
            "running",
            None,
            self.logs.load(std::sync::atomic::Ordering::Relaxed),
            self.scope.as_deref(),
        );
    }
    fn stop(&mut self) {
        self.shared.0.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
        self.shared.1.notify_all();
        // Do not wait at build exit. The stopped worker owns only telemetry state.
        self.worker.take();
    }
    pub(crate) fn done(&mut self, exit: i32, logs: bool) {
        self.stop();
        self.terminal = true;
        emit_lifecycle(
            &self.id,
            &self.sub,
            self.route,
            "done",
            Some(exit),
            logs,
            self.scope.as_deref(),
        );
    }
}
impl Drop for BuildLifecycle {
    fn drop(&mut self) {
        self.stop();
        if !self.terminal {
            emit_lifecycle(
                &self.id,
                &self.sub,
                self.route,
                "abandoned",
                None,
                self.logs.load(std::sync::atomic::Ordering::Relaxed),
                self.scope.as_deref(),
            );
        }
    }
}

/// Append one event to the queue. Never fails, never blocks on the network.
pub fn record(kind: &str, data: Value) {
    record_with_scope(kind, data, None);
}

fn lifecycle_scope_matches(expected: Option<&str>, current: Option<&str>) -> bool {
    expected.is_some() && expected == current
}

fn record_with_scope(kind: &str, data: Value, expected_scope: Option<&str>) {
    if !enabled() {
        return;
    }
    let observed_at = paths::now() as i64;
    let creds = telemetry_creds();
    let scope = creds.as_ref().map(auth_scope);
    if kind == "build_state" && !lifecycle_scope_matches(expected_scope, scope.as_deref()) {
        return;
    }
    let Some(d) = dir() else { return };
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let Some(_guard) = queue_lock(&d) else { return };
    let q = d.join("queue.jsonl");
    if std::fs::metadata(&q).is_ok_and(|m| m.len() > MAX_QUEUE_BYTES) {
        return;
    }
    let mut line =
        json!({ "event_id": random_uuid(), "kind": kind, "at": observed_at, "data": data });
    if let Some(scope) = scope {
        line["_auth_scope"] = json!(scope);
    }
    let _ = paths::append_line(&q, &line.to_string());
    notice_once(&d);
}

/// One line, the first time anything is recorded on this machine.
fn notice_once(d: &std::path::Path) {
    let stamp = d.join("notice-shown");
    if stamp.exists() || std::fs::write(&stamp, b"").is_err() {
        return;
    }
    eprintln!(
        "justrust: sends usage stats (timings, counts, no code, paths, or names; linked to your account when signed in) \
         to improve it. Off: `justrust telemetry off` or JUSTRUST_NO_TELEMETRY=1."
    );
}

/// Which coding agent is driving this process, from its environment.
fn agent_name() -> &'static str {
    let has = |k: &str| std::env::var_os(k).is_some();
    if has("JCODE_SESSION_ID") {
        "jcode"
    } else if has("CLAUDECODE") {
        "claude"
    } else if has("CODEX_SANDBOX") || has("CODEX_SANDBOX_NETWORK_DISABLED") {
        "codex"
    } else if has("GEMINI_CLI") {
        "gemini"
    } else if has("CURSOR_AGENT") || has("CURSOR_TRACE_ID") {
        "cursor"
    } else {
        "none"
    }
}

/// Flag names from cargo args, values dropped: `-p foo --features=x` ->
/// `["-p", "--features"]`. Stops at `--` (test binary args).
fn flag_names(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        if a == "--" {
            out.push("--".to_string());
            break;
        }
        let Some(rest) = a.strip_prefix('-') else {
            continue;
        };
        let name = if let Some(long) = rest.strip_prefix('-') {
            format!("--{}", long.split('=').next().unwrap_or(""))
        } else {
            // Short flags may carry their value (`-pfoo`, `-j8`).
            format!("-{}", rest.chars().next().unwrap_or('?'))
        };
        let ok = name.len() <= 32
            && name
                .trim_start_matches('-')
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && name.len() > 1;
        let known = matches!(
            name.as_str(),
            "-p" | "-j"
                | "-q"
                | "-v"
                | "-r"
                | "--package"
                | "--jobs"
                | "--quiet"
                | "--verbose"
                | "--release"
                | "--profile"
                | "--features"
                | "--all-features"
                | "--no-default-features"
                | "--target"
                | "--target-dir"
                | "--manifest-path"
                | "--workspace"
                | "--exclude"
                | "--lib"
                | "--bin"
                | "--bins"
                | "--test"
                | "--tests"
                | "--example"
                | "--examples"
                | "--bench"
                | "--benches"
                | "--all-targets"
                | "--locked"
                | "--offline"
                | "--frozen"
                | "--message-format"
                | "--keep-going"
                | "--no-run"
                | "--doc"
        );
        if ok && known && !out.contains(&name) {
            out.push(name);
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn valid_toolchain_pin(pin: &str) -> bool {
    pin.strip_prefix("jr-").is_some_and(|h| {
        h.len() == 12
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn valid_run_id(id: &str) -> bool {
    let parts: Vec<_> = id.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 8
        && parts[1].len() == 9
        && !parts[2].is_empty()
        && parts[2].len() <= 10
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

fn safe_subcommand(sub: &str) -> &str {
    match sub {
        "check" | "build" | "test" | "clippy" | "run" | "bench" | "doc" | "rustc" => sub,
        _ => "other",
    }
}

pub(crate) fn local_route_label() -> &'static str {
    route_reason_label(crate::route::local_reason().as_deref(), false)
}

fn route_label(s: &crate::summary::Summary) -> &'static str {
    route_reason_label(s.route.as_deref(), s.remote.is_some())
}

fn route_reason_label(reason: Option<&str>, remote: bool) -> &'static str {
    match reason {
        Some("JUSTRUST_REMOTE=off") => "remote_off",
        Some("JUSTRUST_REMOTE=force") => "forced",
        Some("remote failed in the last 10 minutes") => "recent_remote_failure",
        Some("remote machines build for Linux, this host is not Linux") => "non_linux",
        Some("no remote machine configured") => "not_configured",
        Some("no history for this command") => "no_history",
        Some("non-UTF-8 arguments") => "non_utf8_args",
        Some(r) if r.starts_with("local ~") => "cost_comparison",
        Some(r) if r.starts_with("remote ~") => "remote_cost_comparison",
        Some(r) if r.starts_with("trying remote: local ~") => "explore_remote",
        Some(r) if r.ends_with("always runs locally") => "local_only",
        Some(_) => "other",
        None if remote => "remote",
        None => "local_unrouted",
    }
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[derive(Clone, Copy)]
pub(crate) enum PersistenceFailure {
    Log,
    Summary,
}

/// Fixed-label persistence evidence only. Never accepts an error chain or path.
pub(crate) fn record_persistence_failure(id: &str, detail: PersistenceFailure, exit: i32) {
    record("build_failure", persistence_failure_data(id, detail, exit));
    maybe_spawn_background();
}

fn persistence_failure_data(id: &str, detail: PersistenceFailure, exit: i32) -> Value {
    let mut data = json!({"schema_version": 2, "stage": "recording_persist",
        "detail": match detail { PersistenceFailure::Log => "log", PersistenceFailure::Summary => "summary" },
        "exit": exit});
    if valid_run_id(id) {
        data["run_id"] = json!(id);
    }
    data
}

/// The run event for a finished build: numbers and fixed labels only.
pub fn run_data(s: &crate::summary::Summary, agent_mode: bool) -> Value {
    let findings = crate::findings::analyze(s);
    let u = &s.units;
    let p = &s.phases;
    let r = &s.resources;
    let t = &s.tests;
    let code = s.diagnostics.first_error.as_deref().and_then(|e| {
        let i = e.find("error[E")?;
        let c = &e[i + 6..];
        let c = &c[..c.find(']')?];
        (c.len() == 5 && c[1..].bytes().all(|b| b.is_ascii_digit())).then(|| c.to_string())
    });
    let toolchain = match s.toolchain.as_deref() {
        None => "system",
        Some(t) if t.starts_with("jr-") => "jr",
        Some(t) if t.starts_with("nightly") => "nightly",
        Some(t) if t.starts_with("beta") => "beta",
        Some(_) => "stable",
    };
    let mut d = serde_json::Map::new();
    let mut put = |k: &str, v: Value| {
        d.insert(k.to_string(), v);
    };
    put("schema_version", json!(2));
    put("release_build", json!(crate::update::release_build()));
    if valid_run_id(&s.id) {
        put("run_id", json!(s.id));
    }
    put("sub", json!(safe_subcommand(&s.subcommand)));
    put("flags", json!(flag_names(s.args.get(1..).unwrap_or(&[]))));
    put("agent_mode", json!(agent_mode));
    put("agent", json!(agent_name()));
    put("exit", json!(s.exit));
    put("wall_s", json!(r2(s.wall)));
    put("cpu_s", json!(r2(s.cpu_secs)));
    put("ncpu", json!(s.ncpu));
    if let Some(coverage) = &s.measurement_coverage {
        put("measurement_samples", json!(coverage.samples));
        put("measurement_processes", json!(coverage.processes));
        put("measurement_span_s", json!(r2(coverage.sample_span_secs)));
        put(
            "measurement_max_gap_s",
            json!(r2(coverage.max_sample_gap_secs)),
        );
    }
    put("build_gaps_s", json!(r2(p.build_gaps)));
    put(
        "phase_total_s",
        json!(r2(p.startup
            + p.lock_wait
            + p.compile
            + p.build_gaps
            + p.test_run
            + p.tail)),
    );
    put(
        "cpu_accounting",
        json!(match s.cpu_secs_note.as_deref() {
            None => "wait4",
            Some("sampled process lower bound") => "sampled_lower_bound",
            Some("compiler CPU lower bound") => "compiler_lower_bound",
            Some(_) => "unknown_lower_bound",
        }),
    );
    put("units_with_pass_timings", json!(u.with_pass_timings));
    put(
        "units_missing_pass_timings",
        json!(u.compiled.saturating_sub(u.with_pass_timings)),
    );
    put("rustc_wall_sum_s", json!(r2(u.rustc_wall_sum)));
    put("depcache_miss_s", json!(r2(u.depcache_miss_secs)));
    put("localcache_saved_s", json!(r2(u.localcache_saved_secs)));
    put("tests_filtered", json!(t.filtered_out));
    put("tests_timed", json!(t.timed));
    put("tests_timed_s", json!(r2(t.timed_secs)));
    put("tests_libtest_s", json!(r2(t.libtest_secs)));
    put("psi_unavailable", json!(r.psi_unavailable));
    put("psi_cpu_s", json!(r2(r.psi_cpu_secs)));
    put("psi_mem_s", json!(r2(r.psi_mem_secs)));
    put("psi_io_s", json!(r2(r.psi_io_secs)));
    put(
        "failure_category",
        json!(if s.exit == 0 {
            "none"
        } else if s.diagnostics.compile_failed {
            "compile"
        } else if t.failed > 0 {
            "test"
        } else {
            "other"
        }),
    );
    put("startup_s", json!(r2(p.startup)));
    put("lock_wait_s", json!(r2(p.lock_wait)));
    put("compile_s", json!(r2(p.compile)));
    put("test_run_s", json!(r2(p.test_run)));
    put("tail_s", json!(r2(p.tail)));
    put("units", json!(u.compiled));
    put("units_local", json!(u.local));
    put("units_deps", json!(u.dependencies));
    put("units_build_scripts", json!(u.build_scripts));
    put("units_fresh", json!(u.fresh));
    put("units_total", json!(s.all_units));
    put("rustc_cpu_s", json!(r2(u.rustc_cpu_secs)));
    put("depcache_hits", json!(u.depcache_hits));
    put("depcache_misses", json!(u.depcache_misses));
    put("depcache_saved_s", json!(r2(u.depcache_saved_secs)));
    put("depcache_remote_hits", json!(u.depcache_remote_hits));
    put("depcache_peer_hits", json!(u.depcache_peer_hits));
    put("depcache_prebuilt_hits", json!(u.depcache_prebuilt_hits));
    put("localcache_hits", json!(u.localcache_hits));
    put("localcache_misses", json!(u.localcache_misses));
    put("link_s", json!(r2(s.link.link_pass_secs)));
    put("tests_passed", json!(t.passed));
    put("tests_failed", json!(t.failed));
    put("tests_ignored", json!(t.ignored));
    put("test_binaries", json!(t.binaries.len()));
    put("compile_errors", json!(s.diagnostics.compile_errors));
    put("warnings", json!(s.diagnostics.warnings));
    put("compile_failed", json!(s.diagnostics.compile_failed));
    put("peak_rss_mb", json!(r.peak_build_rss_mb.round()));
    put("avg_cores", json!(r2(r.avg_build_cores)));
    put("min_mem_avail_mb", json!(r.min_mem_available_mb));
    put("remote", json!(s.remote.is_some()));
    if let Some(pin) = s.toolchain.as_deref().filter(|t| valid_toolchain_pin(t)) {
        put("toolchain_pin", json!(pin));
    }
    put("toolchain", json!(toolchain));
    put("slot", json!(s.slot.is_some()));
    put("early", json!(s.early.is_some()));
    put("rebuild_reasons", json!(s.rebuild_reasons.len()));
    put(
        "waste",
        json!(
            findings
                .iter()
                .take(16)
                .map(|f| f.kind.clone())
                .collect::<Vec<_>>()
        ),
    );
    put(
        "waste_top_s",
        json!(r2(findings.first().map(|f| f.cost_secs).unwrap_or(0.0))),
    );
    put("tty", json!(crate::sys::isatty(2)));
    put("ci", json!(std::env::var_os("CI").is_some()));
    put("signed_in", json!(crate::remote_hosted::signed_in()));
    if let Some(c) = code {
        put("error_code", json!(c));
    }
    if let Some(rm) = &s.remote {
        put("remote_sync_ms", json!(rm.sync_ms.round()));
        put("remote_client_wall_s", json!(r2(rm.client_wall)));
    }
    put("route", json!(route_label(s)));
    Value::Object(d)
}

pub fn record_run(s: &crate::summary::Summary, agent_mode: bool) {
    if enabled() {
        record("run", run_data(s, agent_mode));
    }
}

/// Report panics (location only), then run the default hook.
fn panic_module(file: &str) -> &str {
    let basename = file
        .rsplit([char::from(47), char::from(92)])
        .next()
        .unwrap_or("");
    match basename {
        "main.rs" | "record.rs" | "summary.rs" | "telemetry.rs" | "remote_hosted.rs"
        | "remote_build.rs" | "depcache.rs" | "shim.rs" | "route.rs" | "update.rs"
        | "procfs.rs" => basename,
        _ => "other",
    }
}

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if enabled() {
            let mode = match std::env::args().nth(1).as_deref() {
                Some(BG_ARG) => "background",
                _ => "cli",
            };
            record(
                "crash",
                json!({ "module": info.location().map(|l| panic_module(l.file())), "line": info.location().map(|l| l.line()), "mode": mode }),
            );
        }
        prev(info);
    }));
}

// ----------------------------------------------------------------- flush

fn flush_secs() -> f64 {
    std::env::var("JUSTRUST_TELEMETRY_FLUSH_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FLUSH_SECS)
}

fn mtime(p: &std::path::Path) -> Option<f64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64())
}

/// Is a flush due? Two `stat`s.
pub(crate) fn flush_due() -> bool {
    let Some(d) = dir() else { return false };
    if d.join("sending.jsonl").exists() {
        return mtime(&d.join("last-flush")).is_none_or(|t| paths::now() - t >= flush_secs());
    }
    let q = d.join("queue.jsonl");
    let Ok(m) = std::fs::metadata(&q) else {
        return d.join("sending.jsonl").exists();
    };
    if m.len() == 0 {
        return false;
    }
    m.len() > FLUSH_BYTES
        || mtime(&d.join("last-flush")).is_none_or(|t| paths::now() - t >= flush_secs())
}

fn telemetry_url() -> String {
    std::env::var("JUSTRUST_TELEMETRY_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{}/telemetry", crate::remote_hosted::build_base()))
}

/// Bounded delivery outcomes. Configuration/authentication failures retain data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Accepted,
    Retry,
    Auth,
    Rejected,
}

fn delivery(code: u16) -> Delivery {
    match code {
        200..=299 => Delivery::Accepted,
        401 | 403 => Delivery::Auth,
        400 | 422 => Delivery::Rejected,
        _ => Delivery::Retry,
    }
}

fn trusted_endpoint(url: &str, custom: bool) -> bool {
    !custom && url == "https://build.jcode.sh/v1/telemetry"
}

fn telemetry_creds() -> Option<crate::remote_hosted::Creds> {
    if !trusted_endpoint(
        &telemetry_url(),
        std::env::var_os("JUSTRUST_TELEMETRY_URL").is_some(),
    ) {
        return None;
    }
    let c = crate::remote_hosted::creds().ok()?;
    (c.base == crate::remote_hosted::DEFAULT_API_BASE
        && c.build_base == crate::remote_hosted::DEFAULT_BUILD_BASE)
        .then_some(c)
}

fn auth_scope(c: &crate::remote_hosted::Creds) -> String {
    // Local-only fingerprint, never wire data. Domain-separated 128-bit scope
    // comparison fails anonymous on credential rotation or binary hash changes.
    use std::hash::{Hash, Hasher};
    let config = c.curl_auth_config();
    (0..2u8)
        .map(|domain| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            domain.hash(&mut h);
            config.hash(&mut h);
            format!("{:016x}", h.finish())
        })
        .collect()
}

fn scoped_batch(events: &[Value], scope: Option<&str>) -> (Vec<Value>, bool) {
    let bound = events.first().and_then(|e| e["_auth_scope"].as_str());
    let auth = bound.is_some() && bound == scope;
    let wire = events
        .iter()
        .take(BATCH)
        .take_while(|e| e["_auth_scope"].as_str() == bound)
        .map(|e| {
            let mut e = e.clone();
            if let Some(o) = e.as_object_mut() {
                o.remove("_auth_scope");
            }
            e
        })
        .collect();
    (wire, auth)
}

fn delivery_events(events: &[Value], authenticate: bool) -> Vec<&Value> {
    events
        .iter()
        .filter(|e| authenticate || e["kind"] != "build_state")
        .collect()
}

fn post(body: &str, auth: Option<&crate::remote_hosted::Creds>) -> Delivery {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("curl")
        .args([
            "-sS",
            "--proto",
            "=https,http",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--config",
            "-",
            "-o",
            crate::sys::DEV_NULL,
            "-w",
            "%{http_code}",
        ])
        .arg(telemetry_url())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Delivery::Retry;
    };
    if let Some(mut i) = child.stdin.take() {
        let mut config = auth.map(|c| c.curl_auth_config()).unwrap_or_default();
        let escaped = body
            .replace(
                char::from(92),
                &String::from_iter([char::from(92), char::from(92)]),
            )
            .replace('"', &String::from_iter([char::from(92), '"']));
        config.push_str(&format!("data-binary = \"{escaped}\"\n"));
        let _ = i.write_all(config.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else {
        return Delivery::Retry;
    };
    if !out.status.success() {
        return Delivery::Retry;
    }
    let code: u16 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    delivery(code)
}

/// Send everything queued. Runs in the background process.
pub(crate) fn flush() {
    let Some(d) = dir() else { return };
    let _ = std::fs::write(d.join("last-flush"), b"");
    if !enabled() {
        discard();
        return;
    }
    let sending = d.join("sending.jsonl");
    let q = d.join("queue.jsonl");
    // A previous flush that failed left `sending.jsonl`: send it first.
    // Appenders keep writing to a fresh queue while this one is sent.
    {
        let Some(_guard) = queue_lock(&d) else { return };
        if !sending.exists() && std::fs::rename(&q, &sending).is_err() {
            return;
        }
    }
    let Some(id) = install_id() else { return };
    let Ok(text) = std::fs::read_to_string(&sending) else {
        return;
    };
    let events: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let creds = telemetry_creds();
    let scope = creds.as_ref().map(auth_scope);
    let mut offset = 0;
    while offset < events.len() {
        let (chunk, authenticate) = scoped_batch(&events[offset..], scope.as_deref());
        if !enabled() {
            discard();
            return;
        }
        let wire = delivery_events(&chunk, authenticate);
        if wire.is_empty() {
            offset += chunk.len();
            if checkpoint(&sending, &events[offset..]).is_err() {
                return;
            }
            continue;
        }
        let body = json!({
            "install_id": id,
            "client": concat!("justrust ", env!("CARGO_PKG_VERSION")),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "events": wire,
        })
        .to_string();
        let outcome = post(&body, if authenticate { creds.as_ref() } else { None });
        if !enabled() {
            discard();
            return;
        }
        let label = match outcome {
            Delivery::Accepted => "accepted",
            Delivery::Retry => "retry",
            Delivery::Auth => "auth_required",
            Delivery::Rejected => "invalid_batch",
        };
        let _ = std::fs::write(
            d.join("delivery.json"),
            json!({"status": label, "at": paths::now() as i64}).to_string(),
        );
        if matches!(outcome, Delivery::Retry | Delivery::Auth) {
            // Retry later, unless it has been failing for days.
            if mtime(&sending).is_some_and(|t| paths::now() - t > 7.0 * 86400.0) {
                let _ = std::fs::remove_file(&sending);
            }
            return;
        }
        // Persist only the unsent suffix after every acknowledged batch.
        // A crash between the POST acknowledgement and this rename can still
        // duplicate that batch: exact-once requires server idempotency.
        offset += chunk.len();
        let remaining = &events[offset..];
        if checkpoint(&sending, remaining).is_err() {
            return;
        }
    }
    let _ = std::fs::remove_file(&sending);
}

fn checkpoint(path: &std::path::Path, events: &[Value]) -> std::io::Result<()> {
    let tmp = path.with_extension("checkpoint");
    let mut text = String::new();
    for event in events {
        text.push_str(&event.to_string());
        text.push('\n');
    }
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// Delete everything not yet sent.
fn discard() {
    if let Some(d) = dir() {
        let _ = std::fs::remove_file(d.join("queue.jsonl"));
        let _ = std::fs::remove_file(d.join("sending.jsonl"));
    }
}

// ------------------------------------------------------- background work

/// After a run or command: if a telemetry flush or an update check is due,
/// start one detached `justrust __bg` to do it. Costs a few `stat`s.
pub fn maybe_spawn_background() {
    spawn_background(false);
}

fn spawn_background(lifecycle: bool) {
    if cfg!(test) || std::env::var_os(crate::route::ON_AGENT).is_some() {
        return;
    }
    let flush = enabled() && (lifecycle || flush_due());
    let update = crate::update::check_due();
    if !flush && !update {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use crate::sys::Detach;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(BG_ARG);
    if lifecycle {
        cmd.arg("--telemetry-now");
    }
    let _ = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .detached()
        .spawn();
}

/// `justrust __bg`: one at a time, lowest priority.
pub fn background_main() -> ! {
    crate::sys::lower_priority();
    let lock = paths::home().ok().and_then(|h| {
        std::fs::create_dir_all(&h).ok()?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(h.join("bg.lock"))
            .ok()?;
        f.try_lock().is_ok().then_some(f)
    });
    if lock.is_some() {
        let lifecycle = std::env::args().any(|a| a == "--telemetry-now");
        if enabled() && (flush_due() || lifecycle) {
            // Drain events appended during the first POST too. Otherwise a short
            // build's terminal event can be stranded behind the active sender.
            // Bounded even with concurrent writers and an offline endpoint.
            for _ in 0..if lifecycle { 4 } else { 1 } {
                flush();
                let Some(d) = dir() else { break };
                if d.join("sending.jsonl").exists()
                    || !std::fs::metadata(d.join("queue.jsonl")).is_ok_and(|m| m.len() > 0)
                {
                    break;
                }
            }
        }
        if !lifecycle && crate::update::check_due() {
            crate::update::background();
        }
    }
    std::process::exit(0)
}

// --------------------------------------------------------------- command

/// `justrust telemetry [on|off|status]`.
pub fn command(action: &str) -> anyhow::Result<()> {
    match action {
        "off" | "disable" => {
            set_setting("telemetry", false)?;
            discard();
            println!("telemetry off. Nothing more is sent, and unsent data was deleted.");
        }
        "on" | "enable" => {
            set_setting("telemetry", true)?;
            match disabled_reason() {
                None => {
                    record("telemetry", json!({ "enabled": true }));
                    println!("telemetry on. Thank you: it shows where builds are slow.");
                }
                Some(why) => println!("telemetry setting on, but still off here: {why}"),
            }
        }
        "status" => {
            match disabled_reason() {
                None => println!(
                    "telemetry: on (usage stats, account-linked when signed in; `justrust telemetry off` to stop)"
                ),
                Some(why) => println!("telemetry: off ({why})"),
            }
            if let Some(d) = dir() {
                let n = std::fs::read_to_string(d.join("queue.jsonl"))
                    .map(|t| t.lines().count())
                    .unwrap_or(0)
                    + std::fs::read_to_string(d.join("sending.jsonl"))
                        .map(|t| t.lines().count())
                        .unwrap_or(0);
                println!("  queued, not yet sent: {n} events ({})", d.display());
            }
            if let Some(d) = dir() {
                let health = std::fs::read(d.join("delivery.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                let status = match health.as_ref().and_then(|v| v["status"].as_str()) {
                    Some("accepted") => "accepted",
                    Some("retry") => "retry pending",
                    Some("auth_required") => "authentication rejected, retained for retry",
                    Some("invalid_batch") => "invalid batch discarded",
                    _ => "not attempted",
                };
                println!("  delivery: {status}");
            }
            println!("  endpoint: {}", telemetry_url());
            println!(
                "  sends: per build the subcommand, flag names, exit code, timings, unit/cache/test \
                 counts, waste kinds, agent name; per command its name and time; update outcomes; \
                 panic line numbers. With a random install id, version, OS, CPU arch."
            );
            println!("  never: source, paths, crate or package names, argument values, hostnames.");
        }
        other => anyhow::bail!("unknown action `{other}`: use on, off, or status"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn observation_quality_is_numeric_and_absence_is_not_zero() {
        let mut s = crate::summary::Summary::default();
        assert!(run_data(&s, false).get("measurement_samples").is_none());
        s.measurement_coverage = Some(crate::summary::MeasurementCoverage {
            samples: 8,
            processes: 5,
            sample_span_secs: 1.401,
            max_sample_gap_secs: 0.407,
        });
        let data = run_data(&s, false);
        assert_eq!(data["release_build"], crate::update::release_build());
        assert_eq!(data["measurement_samples"], 8);
        assert_eq!(data["measurement_processes"], 5);
        assert_eq!(data["measurement_span_s"], 1.4);
        assert_eq!(data["measurement_max_gap_s"], 0.41);
        assert!(data.as_object().unwrap().len() <= 80);
    }
    use super::*;

    #[test]
    fn persistence_failures_are_bounded_and_validate_run_ids() {
        for (detail, expected) in [
            (PersistenceFailure::Log, "log"),
            (PersistenceFailure::Summary, "summary"),
        ] {
            let data = persistence_failure_data("20261014-112403320-1234", detail, 101);
            assert_eq!(data["stage"], "recording_persist");
            assert_eq!(data["detail"], expected);
            assert_eq!(data["exit"], 101);
            assert_eq!(data["schema_version"], 2);
            assert_eq!(data["run_id"], "20261014-112403320-1234");
            let invalid = persistence_failure_data("/private/path?token=secret", detail, 0);
            assert!(invalid.get("run_id").is_none());
            assert!(!invalid.to_string().contains("secret"));
        }
    }

    #[test]
    fn lifecycle_contract_is_closed_and_private() {
        let id = "20261014-112403320-1234";
        let data =
            lifecycle_data(id, "private-sub", "/private/path", "done", Some(101), true).unwrap();
        assert_eq!(data["sub"], "other");
        assert_eq!(data["route"], "other");
        assert_eq!(data["exit"], 101);
        assert_eq!(data["log_availability"], "local_only");
        assert!((1..=300).contains(&data["heartbeat_s"].as_u64().unwrap()));
        assert!(!data.to_string().contains("private"));
        assert!(lifecycle_data(id, "test", "remote", "done", None, false).is_none());
        assert!(lifecycle_data(id, "test", "remote", "unknown", None, false).is_none());
        assert!(lifecycle_data("private-id", "test", "remote", "queued", None, false).is_none());
        let running = lifecycle_data(id, "test", "remote", "running", None, false).unwrap();
        assert_eq!(running["log_availability"], "none");
        assert!(running.get("exit").is_none());
    }

    #[test]
    fn lifecycle_never_delivers_in_anonymous_or_rotated_scope() {
        assert!(lifecycle_scope_matches(Some("old"), Some("old")));
        assert!(!lifecycle_scope_matches(Some("old"), Some("new")));
        assert!(!lifecycle_scope_matches(None, None));
        assert!(!lifecycle_scope_matches(Some("old"), None));
        let events = vec![
            json!({"kind":"build_state", "_auth_scope":"old", "event_id":"stable"}),
            json!({"kind":"run", "_auth_scope":"old"}),
        ];
        let (chunk, auth) = scoped_batch(&events, Some("new"));
        assert!(!auth);
        let wire = delivery_events(&chunk, auth);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["kind"], "run");
        let (chunk, auth) = scoped_batch(&events, Some("old"));
        assert!(auth);
        let wire = delivery_events(&chunk, auth);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["event_id"], "stable");
        assert!(wire[0].get("_auth_scope").is_none());
    }

    #[test]
    fn uuids_are_v4_and_distinct() {
        let a = random_uuid();
        let b = random_uuid();
        assert!(is_uuid_v4(&a), "{a}");
        assert!(is_uuid_v4(&b), "{b}");
        assert_ne!(a, b);
        assert!(!is_uuid_v4("0b7c3c1e-1111-1abc-8def-0123456789ab"));
    }

    #[test]
    fn flags_drop_values_and_names() {
        let a: Vec<String> = [
            "-p",
            "secret-crate",
            "--features=private",
            "-j8",
            "--lib",
            "my_test_filter",
            "--",
            "--nocapture",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(flag_names(&a), ["-p", "--features", "-j", "--lib", "--"]);
        let weird: Vec<String> = vec!["--x/../../etc".into(), "-".into()];
        assert!(flag_names(&weird).is_empty());
    }

    #[test]
    fn hostile_summary_never_exports_text() {
        let secret = "PRIVATE-secret-path-token";
        let mut s = crate::summary::Summary {
            id: secret.into(),
            subcommand: secret.into(),
            cwd: secret.into(),
            route: Some(secret.into()),
            toolchain: Some(secret.into()),
            cpu_secs_note: Some(secret.into()),
            agent_session: Some(secret.into()),
            args: vec![
                "test".into(),
                format!("--{secret}"),
                "--features=PRIVATE-secret-path-token".into(),
            ],
            ..Default::default()
        };
        s.diagnostics.first_error = Some(secret.into());
        s.units.depcache_miss_env = vec![secret.into()];
        s.remote = Some(crate::summary::RemoteInfo {
            machine: secret.into(),
            ..Default::default()
        });
        let d = run_data(&s, true);
        assert!(!d.to_string().contains(secret), "{d}");
        assert!(d.get("run_id").is_none());
        assert_eq!(d["route"], "other");
        assert_eq!(d["sub"], "other");
        assert_eq!(d["cpu_accounting"], "unknown_lower_bound");
    }

    #[test]
    fn structured_diagnostics_cover_summary_fields() {
        let mut s = crate::summary::Summary {
            id: "20261014-104956974-1234".into(),
            subcommand: "test".into(),
            ..Default::default()
        };
        s.phases.build_gaps = 1.25;
        s.units.compiled = 5;
        s.units.with_pass_timings = 2;
        s.units.depcache_miss_secs = 4.5;
        s.units.localcache_saved_secs = 3.25;
        s.tests.filtered_out = 11;
        s.tests.timed = 2;
        s.tests.timed_secs = 0.75;
        let d = run_data(&s, false);
        for (key, value) in [
            ("schema_version", json!(2)),
            ("run_id", json!(s.id)),
            ("build_gaps_s", json!(1.25)),
            ("phase_total_s", json!(1.25)),
            ("units_missing_pass_timings", json!(3)),
            ("depcache_miss_s", json!(4.5)),
            ("localcache_saved_s", json!(3.25)),
            ("tests_filtered", json!(11)),
            ("tests_timed", json!(2)),
            ("tests_timed_s", json!(0.75)),
        ] {
            assert_eq!(d[key], value, "{key}");
        }
        assert!(!valid_run_id("20261014-104956974-private"));
    }

    #[test]
    fn delivery_preserves_auth_and_configuration_failures() {
        for code in [401, 403] {
            assert_eq!(delivery(code), Delivery::Auth);
        }
        for code in [0, 301, 404, 408, 413, 429, 500, 503] {
            assert_eq!(delivery(code), Delivery::Retry);
        }
        assert_eq!(delivery(200), Delivery::Accepted);
        assert_eq!(delivery(400), Delivery::Rejected);
    }

    #[test]
    fn checkpoint_does_not_replay_accepted_prefix() {
        let d = std::env::temp_dir().join(format!("justrust-telemetry-{}", random_uuid()));
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("sending.jsonl");
        let events: Vec<_> = (0..BATCH + 2).map(|i| json!({"n": i})).collect();
        checkpoint(&path, &events).unwrap();
        checkpoint(&path, &events[BATCH..]).unwrap();
        let remaining: Vec<Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(remaining, events[BATCH..]);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn queue_identity_crossing_never_reattributes() {
        let a = crate::remote_hosted::Creds::new(
            crate::remote_hosted::DEFAULT_API_BASE.into(),
            "token-a".into(),
        )
        .unwrap();
        let b = crate::remote_hosted::Creds::new(
            crate::remote_hosted::DEFAULT_API_BASE.into(),
            "token-b".into(),
        )
        .unwrap();
        let sa = auth_scope(&a);
        let sb = auth_scope(&b);
        assert_ne!(sa, sb);
        let events = vec![
            json!({"kind": "run"}),
            json!({"kind": "run", "_auth_scope": sa}),
            json!({"kind": "run", "_auth_scope": sb}),
        ];
        let (legacy, auth) = scoped_batch(&events, Some(&sb));
        assert_eq!(legacy.len(), 1);
        assert!(!auth);
        let (old, auth) = scoped_batch(&events[1..], Some(&sb));
        assert_eq!(old.len(), 1);
        assert!(!auth);
        assert!(old[0].get("_auth_scope").is_none());
        let (current, auth) = scoped_batch(&events[2..], Some(&sb));
        assert!(auth);
        assert!(!current[0].to_string().contains(&sb));
        assert!(!scoped_batch(&events[2..], None).1);
    }

    #[test]
    fn only_default_hosted_endpoint_may_receive_credentials() {
        assert_eq!(
            panic_module("/private/workspace/src/telemetry.rs"),
            "telemetry.rs"
        );
        assert_eq!(panic_module("/private/source/secret.rs"), "other");
        assert!(trusted_endpoint(
            "https://build.jcode.sh/v1/telemetry",
            false
        ));
        for url in [
            "http://build.jcode.sh/v1/telemetry",
            "https://build.jcode.sh.evil/v1/telemetry",
            "https://build.jcode.sh/v1/telemetry?x=1",
            "https://custom.test/telemetry",
        ] {
            assert!(!trusted_endpoint(url, false));
        }
        assert!(!trusted_endpoint(
            "https://build.jcode.sh/v1/telemetry",
            true
        ));
    }

    #[test]
    fn disabled_in_unit_tests() {
        assert!(valid_toolchain_pin("jr-ac20de5e7921"));
        assert!(!valid_toolchain_pin("jr-private-package"));
        assert!(!valid_toolchain_pin("jr-AC20DE5E7921"));
        assert_eq!(disabled_reason(), Some("unit tests"));
    }
}
