//! Anonymous usage telemetry, and the settings file it shares with
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

/// Append one event to the queue. Never fails, never blocks on the network.
pub fn record(kind: &str, data: Value) {
    if !enabled() {
        return;
    }
    let Some(d) = dir() else { return };
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let q = d.join("queue.jsonl");
    if std::fs::metadata(&q).is_ok_and(|m| m.len() > MAX_QUEUE_BYTES) {
        return;
    }
    let line = json!({ "kind": kind, "at": paths::now() as i64, "data": data });
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
        "justrust: sends anonymous usage stats (timings, counts, no code, paths, or names) \
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
        if ok && !out.contains(&name) {
            out.push(name);
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
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
    put("sub", json!(s.subcommand));
    put("flags", json!(flag_names(s.args.get(1..).unwrap_or(&[]))));
    put("agent_mode", json!(agent_mode));
    put("agent", json!(agent_name()));
    put("exit", json!(s.exit));
    put("wall_s", json!(r2(s.wall)));
    put("cpu_s", json!(r2(s.cpu_secs)));
    put("ncpu", json!(s.ncpu));
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
    if let Some(why) = &s.route {
        // The router's rule, without its measured numbers.
        let rule: String = why
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect();
        put("route", json!(rule));
    }
    Value::Object(d)
}

pub fn record_run(s: &crate::summary::Summary, agent_mode: bool) {
    if enabled() {
        record("run", run_data(s, agent_mode));
    }
}

/// Report panics (location only), then run the default hook.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if enabled() {
            let loc = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let argv1 = std::env::args().nth(1).unwrap_or_default();
            let mode = if argv1.starts_with("__") || argv1.is_empty() {
                argv1
            } else {
                "cli".into()
            };
            record("crash", json!({ "location": loc, "mode": mode }));
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

/// POST one batch. `Ok(true)`: done with it (accepted, or rejected as
/// invalid, which a retry would not fix). `Ok(false)`: retry later.
fn post(body: &str) -> bool {
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
            "--data-binary",
            "@-",
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
        return false;
    };
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(body.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else {
        return false;
    };
    let code: u16 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    (200..300).contains(&code) || (400..500).contains(&code) && code != 429 && code != 408
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
    if !sending.exists() && std::fs::rename(&q, &sending).is_err() {
        return;
    }
    let Some(id) = install_id() else { return };
    let text = std::fs::read_to_string(&sending).unwrap_or_default();
    let events: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    for chunk in events.chunks(BATCH) {
        let body = json!({
            "install_id": id,
            "client": concat!("justrust ", env!("CARGO_PKG_VERSION")),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "events": chunk,
        })
        .to_string();
        if !post(&body) {
            // Retry later, unless it has been failing for days.
            if mtime(&sending).is_some_and(|t| paths::now() - t > 7.0 * 86400.0) {
                let _ = std::fs::remove_file(&sending);
            }
            return;
        }
    }
    let _ = std::fs::remove_file(&sending);
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
    if cfg!(test) || std::env::var_os(crate::route::ON_AGENT).is_some() {
        return;
    }
    let flush = enabled() && flush_due();
    let update = crate::update::check_due();
    if !flush && !update {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use crate::sys::Detach;
    let _ = std::process::Command::new(exe)
        .arg(BG_ARG)
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
        if crate::update::check_due() {
            crate::update::background();
        }
        if enabled() && flush_due() {
            flush();
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
                    "telemetry: on (anonymous usage stats, `justrust telemetry off` to stop)"
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
            println!("  endpoint: {}", telemetry_url());
            println!(
                "  sends: per build the subcommand, flag names, exit code, timings, unit/cache/test \
                 counts, waste kinds, agent name; per command its name and time; update outcomes; \
                 panic locations. With a random install id, version, OS, CPU arch."
            );
            println!("  never: source, paths, crate or package names, argument values, hostnames.");
        }
        other => anyhow::bail!("unknown action `{other}`: use on, off, or status"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn disabled_in_unit_tests() {
        assert_eq!(disabled_reason(), Some("unit tests"));
    }
}
