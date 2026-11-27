//! `justrust status`: what is building right now and when it will be done.
//!
//! A run is active while its directory has no `summary.json` and the recording
//! process is alive. Progress comes from `live.json` (cargo's `N/M` units and
//! the crates rustc is on) and `units.jsonl` (units the shim saw finish).
//!
//! The estimate lines the run up with a similar finished run from the same
//! directory: same arguments, about as many units to compile, median wall time
//! of the closest few. The
//! latest point in that reference run where every unit finished so far had also
//! finished there is "where we are". What the reference still needed from that
//! point is the time left, counting down between unit completions. Test and
//! `run` time after the build comes from the reference too.

use crate::live::LiveState;
use crate::paths;
use crate::record::Meta;
use crate::shim::Unit;
use crate::summary::{IndexEntry, Summary};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How long a finished run stays visible in `--waybar` output.
const SHOW_DONE_SECS: f64 = 8.0;
/// Only this many of each run-ID family are checked for activity.
const SCAN_RUNS: usize = 64;

#[derive(Debug, Serialize, Clone)]
pub struct Active {
    pub id: String,
    pub label: String,
    pub cwd: String,
    pub args: Vec<String>,
    pub agent_session: Option<String>,
    /// Logical CPUs on the build machine, or 0 when not yet known.
    pub ncpu: usize,
    /// Measured process-tree CPU usage, in cores, or null if unavailable/stale.
    pub cpu_cores: Option<f64>,
    /// Unix timestamp of the CPU measurement (not the status poll).
    pub sampled_at: Option<f64>,
    /// "local", "remote" (own machine) or "hosted".
    pub place: String,
    /// Why the router kept this build local (`meta.route`), when it said.
    pub reason: Option<String>,
    pub elapsed: f64,
    pub live: LiveState,
    pub units_finished: usize,
    /// Rustc units still running, with seconds they have been running.
    pub running: Vec<(String, f64)>,
    pub estimate: Option<Estimate>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Estimate {
    /// Seconds left, whole run (build plus tests or `run`).
    pub remaining: f64,
    /// Fraction done by time, 0..1.
    pub fraction: f64,
    /// The finished run this one was lined up with.
    pub reference: String,
    pub reference_wall: f64,
    /// "aligned" (unit by unit), "elapsed" (whole-run time only).
    pub method: &'static str,
}

#[derive(Debug, Serialize, Clone)]
pub struct Finished {
    pub id: String,
    pub label: String,
    pub wall: f64,
    pub ok: bool,
    pub ago: f64,
    pub place: String,
    /// Why the router kept this build local (`meta.route`), when it said.
    pub reason: Option<String>,
    pub cwd: String,
    pub args: Vec<String>,
    pub agent_session: Option<String>,
}

/// Shared with live CPU snapshot producers. Legacy sample journals provide
/// the same time and CPU fields, with the machine size taken from metadata.
type CpuSample = crate::record::LiveCpu;

fn measured_cpu(dir: &Path, meta: &Meta, now: f64) -> (usize, Option<f64>, Option<f64>) {
    const MAX_SAMPLE_BYTES: u64 = 16 * 1024;
    let snapshot = std::fs::File::open(dir.join("cpu.json"))
        .ok()
        .and_then(|f| serde_json::from_reader(f.take(MAX_SAMPLE_BYTES)).ok());
    let sample: Option<CpuSample> = snapshot.or_else(|| {
        let mut f = std::fs::File::open(dir.join("samples.jsonl")).ok()?;
        let len = f.metadata().ok()?.len();
        let offset = len.saturating_sub(MAX_SAMPLE_BYTES);
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut tail = String::new();
        f.take(MAX_SAMPLE_BYTES).read_to_string(&mut tail).ok()?;
        // Discard a possibly truncated first line when reading the tail.
        let tail = if offset > 0 {
            tail.split_once('\n')?.1
        } else {
            &tail
        };
        tail.lines()
            .rev()
            .find_map(|line| serde_json::from_str(line).ok())
    });
    let Some(sample) = sample else {
        return (meta.ncpu, None, None);
    };
    let ncpu = if sample.ncpu > 0 {
        sample.ncpu
    } else {
        meta.ncpu
    };
    if !sample.t.is_finite()
        || !sample.build_cores.is_finite()
        || sample.t < meta.start
        || now - sample.t > 5.0
        || sample.t - now > 1.0
        || sample.build_cores < 0.0
        || ncpu == 0
        || sample.build_cores > ncpu as f64
    {
        return (ncpu, None, None);
    }
    (ncpu, Some(sample.build_cores), Some(sample.t))
}

/// Where a run ran: its `meta.json` says, else the index's remote marker.
fn place_of(meta: Option<&Meta>, remote: bool) -> String {
    match meta.and_then(|m| m.place.clone()) {
        Some(p) => p,
        None if remote => "remote".into(),
        None => "local".into(),
    }
}

/// The recorded reason a build stayed local, `None` for remote runs and for
/// runs where routing was not consulted.
fn reason_of(meta: Option<&Meta>, place: &str) -> Option<String> {
    if place != "local" {
        return None;
    }
    let why = meta?.route.as_deref()?.trim();
    (!why.is_empty()).then(|| why.to_string())
}

/// Longest local reason shown in the bar. The tooltip has it in full.
const BAR_REASON_CHARS: usize = 48;

/// `[local: why]` for the bar, `[hosted]` when there is nothing to explain.
fn place_tag(place: &str, reason: Option<&str>) -> String {
    let Some(why) = reason else {
        return format!("[{place}]");
    };
    // The reason given with `--local=` is stored behind how it was asked.
    let why = why.strip_prefix("JUSTRUST_REMOTE=off: ").unwrap_or(why);
    let why: String = why.split_whitespace().collect::<Vec<_>>().join(" ");
    if why.chars().count() <= BAR_REASON_CHARS {
        return format!("[{place}: {why}]");
    }
    let cut: String = why.chars().take(BAR_REASON_CHARS - 1).collect();
    format!("[{place}: {}\u{2026}]", cut.trim_end())
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()
}

#[derive(Debug, Serialize, Default)]
pub struct Status {
    pub active: Vec<Active>,
    pub recent: Option<Finished>,
}

fn pid_alive(pid: u32, since: f64) -> bool {
    let Some(st) = crate::procfs::read_stat(pid as i32) else {
        return false;
    };
    // Rule out a reused pid: the process must have started before the run.
    // Without a boot time the start cannot be placed: trust the pid.
    let Some(boot) = crate::procfs::boot_time() else {
        return true;
    };
    let started = boot + st.start_ticks as f64 / crate::procfs::clk_tck();
    started <= since + 2.0
}

/// Short name for a run: the `-p` package if there is exactly one, else the
/// directory, then the subcommand.
pub fn label(cwd: &str, args: &[String]) -> String {
    let mut pkgs = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-p" || a == "--package" {
            if let Some(p) = it.next() {
                pkgs.push(p.clone());
            }
        } else if let Some(p) = a.strip_prefix("--package=") {
            pkgs.push(p.to_owned());
        }
    }
    let name = if pkgs.len() == 1 {
        pkgs.remove(0)
    } else {
        Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let sub = crate::record::subcommand(
        &args
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    format!("{name} {sub}").trim().to_owned()
}

fn unit_key(u: &Unit) -> String {
    format!(
        "{}|{}|{}|{}",
        u.package, u.crate_name, u.test, u.build_script
    )
}

/// Reference run: same directory and arguments if possible, closest number
/// of compiled units, newest first among equals.
fn pick_reference<'a>(
    index: &'a [IndexEntry],
    meta: &Meta,
    to_compile: Option<usize>,
) -> Option<&'a IndexEntry> {
    // Remote runs line up with remote runs, local with local, when both exist.
    let remote = meta.place.as_deref().is_some_and(|p| p != "local");
    let same_place = index
        .iter()
        .rev()
        .any(|r| r.cwd == meta.cwd && r.remote.is_some() == remote);
    let ok = |r: &&IndexEntry| {
        r.cwd == meta.cwd
            && !r.compile_failed
            && r.id != meta.id
            && (!same_place || r.remote.is_some() == remote)
    };
    // Exact arguments win even if their only match is older than every
    // subcommand match. Search to exhaustion on a miss, but keep only the
    // newest 20 matches and never parse subcommands on an exact hit.
    let mut pool: Vec<&IndexEntry> = index
        .iter()
        .rev()
        .filter(ok)
        .filter(|r| r.args == meta.args)
        .take(20)
        .collect();
    if pool.is_empty() {
        pool = index
            .iter()
            .rev()
            .filter(ok)
            .filter(|r| {
                crate::record::subcommand(&os(&r.args)).unwrap_or_default() == meta.subcommand
            })
            .take(20)
            .collect();
    }
    let mut near: Vec<(usize, &IndexEntry)> = match to_compile {
        Some(n) => {
            let mut v: Vec<(usize, &IndexEntry)> = pool.into_iter().enumerate().collect();
            v.sort_by_key(|(i, r)| (r.units_compiled.abs_diff(n), *i));
            v
        }
        None => pool
            .into_iter()
            .filter(|r| r.units_compiled > 0)
            .enumerate()
            .collect(),
    };
    // One run can be an outlier (busy machine, cold cache): take the median
    // wall time of the few most similar in size.
    if let (Some(n), Some(best)) = (to_compile, near.first().map(|(_, r)| r.units_compiled)) {
        let slack = (best.abs_diff(n)).max(best / 4).max(2);
        near.retain(|(_, r)| r.units_compiled.abs_diff(best) <= slack);
    }
    near.truncate(5);
    near.sort_by(|a, b| a.1.wall.total_cmp(&b.1.wall));
    near.get(near.len() / 2).map(|(_, r)| *r)
}

fn os(args: &[String]) -> Vec<std::ffi::OsString> {
    args.iter().map(std::ffi::OsString::from).collect()
}

fn load_summary(dir: &Path) -> Option<Summary> {
    serde_json::from_slice(&std::fs::read(dir.join("summary.json")).ok()?).ok()
}

/// Seconds from start to the end of the build part of a finished run.
fn build_end(s: &Summary) -> f64 {
    let p = &s.phases;
    (p.startup + p.lock_wait + p.compile + p.build_gaps).min(s.wall)
}

fn estimate<'a>(
    now: f64,
    meta: &Meta,
    live: &LiveState,
    done_units: &[Unit],
    index: impl FnOnce() -> &'a [IndexEntry],
    runs: &Path,
) -> Option<Estimate> {
    // A previous build's duration cannot predict provisioning, source sync,
    // or lock contention. Do not turn elapsed setup time into a countdown.
    if !matches!(
        live.phase.as_str(),
        "compiling" | "testing" | "running" | "finishing"
    ) {
        return None;
    }
    // Remote unit records arrive at completion. Until we have an anchor,
    // subtracting client elapsed time would count sync/lock waits as work.
    if meta.place.as_deref().is_some_and(|p| p != "local")
        && done_units.is_empty()
        && live.finished_at.is_none()
    {
        return None;
    }
    let to_compile = (live.total > 0).then(|| live.total - live.first_done.unwrap_or(0));
    let r = pick_reference(index(), meta, to_compile)?;
    let rdir = runs.join(&r.id);
    let (summary, ref_units) = reference_run(&rdir)?;
    let summary = &*summary;
    let elapsed = now - meta.start;
    let ref_build_end = build_end(summary);
    let post = (summary.wall - ref_build_end).max(0.0);

    let mut out = Estimate {
        remaining: 0.0,
        fraction: 0.0,
        reference: r.id.clone(),
        reference_wall: summary.wall,
        method: "elapsed",
    };
    let remaining = if let Some(fin) = live.finished_at {
        // Build done: tests or the program are running.
        out.method = "aligned";
        (post - (now - fin)).max(0.0)
    } else {
        let ref_end = |k: &str| {
            ref_units
                .iter()
                .filter(|u| unit_key(u) == k)
                .map(|u| u.end - summary.start)
                .fold(None, |a: Option<f64>, e| Some(a.map_or(e, |a| a.max(e))))
        };
        // Line up on the latest unit that finished in both runs.
        let mut anchor: Option<(f64, f64)> = None; // (reference offset, our time)
        let mut matched = 0usize;
        let mut seen = HashSet::new();
        for u in done_units {
            let k = unit_key(u);
            if !seen.insert(k.clone()) {
                continue;
            }
            if let Some(off) = ref_end(&k) {
                matched += 1;
                if anchor.is_none_or(|(o, _)| off > o) {
                    anchor = Some((off, u.end));
                }
            }
        }
        let ref_keys: HashSet<String> = ref_units.iter().map(unit_key).collect();
        let similar = !ref_keys.is_empty()
            && (done_units.is_empty() || matched * 2 >= seen.len().min(ref_keys.len()));
        match anchor {
            Some((off, at)) if similar => {
                out.method = "aligned";
                (ref_build_end - off - (now - at)).max(0.0) + post
            }
            _ => (summary.wall - elapsed).max(0.0),
        }
    };
    // Past the reference: we cannot know, so admit it with a small floor that
    // grows slowly instead of claiming "0s left".
    let remaining = if remaining <= 0.5 && elapsed > summary.wall {
        (elapsed - summary.wall).mul_add(0.1, 1.0)
    } else {
        remaining
    };
    out.remaining = remaining;
    out.fraction = (elapsed / (elapsed + remaining)).clamp(0.0, 1.0);
    Some(out)
}

/// A finished reference run's summary and units. Finished runs never change,
/// and `status --watch` re-estimates every second in every watcher, so parse
/// each reference once per process instead of once per tick.
fn reference_run(dir: &Path) -> Option<(std::sync::Arc<Summary>, std::sync::Arc<Vec<Unit>>)> {
    use std::sync::{Arc, Mutex};
    type Entry = (std::path::PathBuf, Arc<Summary>, Arc<Vec<Unit>>);
    static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    if let Ok(cache) = CACHE.lock()
        && let Some((_, s, u)) = cache.iter().find(|(p, _, _)| p == dir)
    {
        return Some((s.clone(), u.clone()));
    }
    let summary = Arc::new(load_summary(dir)?);
    let units = Arc::new(crate::summary::load_units(dir));
    if let Ok(mut cache) = CACHE.lock() {
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((dir.to_path_buf(), summary.clone(), units.clone()));
    }
    Some((summary, units))
}

fn recent_run_ids(runs: &Path) -> Vec<String> {
    // `status --watch` calls this every second and the runs directory holds
    // tens of thousands of entries. A new run changes the directory mtime, so
    // rescan only then.
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::path::PathBuf, std::time::SystemTime, Vec<String>)>> =
        Mutex::new(None);
    let mtime = std::fs::metadata(runs).and_then(|m| m.modified()).ok();
    if let (Some(mtime), Ok(cache)) = (mtime, CACHE.lock())
        && let Some((p, t, ids)) = cache.as_ref()
        && p == runs
        && *t == mtime
    {
        return ids.clone();
    }
    let ids = scan_run_ids(runs);
    if let (Some(mtime), Ok(mut cache)) = (mtime, CACHE.lock()) {
        *cache = Some((runs.to_path_buf(), mtime, ids.clone()));
    }
    ids
}

/// Partition away historical entries before sorting the small visible suffix.
/// Both ID families have unique keys, so ties cannot change their ordering.
fn newest_run_entries<T: Ord>(mut entries: Vec<T>) -> Vec<T> {
    if entries.len() > SCAN_RUNS {
        let discard = entries.len() - SCAN_RUNS;
        entries.select_nth_unstable(discard);
        entries.drain(..discard);
    }
    entries.sort_unstable_by(|a, b| b.cmp(a));
    entries
}

fn scan_run_ids(runs: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    let mut overrides = Vec::new();
    if let Ok(entries) = std::fs::read_dir(runs) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            let Ok(id) = entry.file_name().into_string() else {
                continue;
            };
            let bytes = id.as_bytes();
            if bytes.len() > 8 && bytes[..8].iter().all(u8::is_ascii_digit) && bytes[8] == b'-' {
                ids.push(id);
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                overrides.push((modified, id));
            }
        }
    }
    // Normal IDs sort by date without a stat per historical run. Imports and
    // explicit ID overrides get their own bounded pool so neither family can
    // hide the other (for example old readme-* fixtures sorting above dates).
    newest_run_entries(ids)
        .into_iter()
        .chain(newest_run_entries(overrides).into_iter().map(|(_, id)| id))
        .collect()
}

/// The full index, re-parsed only when the file changes. `status --watch`
/// needs it for estimates every second while any build runs, and it is tens
/// of MB.
fn cached_index() -> std::sync::Arc<Vec<IndexEntry>> {
    use std::sync::Mutex;
    type Key = (u64, std::time::SystemTime);
    static CACHE: Mutex<Option<(Key, std::sync::Arc<Vec<IndexEntry>>)>> = Mutex::new(None);
    let key = paths::index_file()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| Some((m.len(), m.modified().ok()?)));
    let Some(key) = key else {
        return std::sync::Arc::new(crate::runs::load_index());
    };
    let mut cache = match CACHE.lock() {
        Ok(c) => c,
        Err(_) => return std::sync::Arc::new(crate::runs::load_index()),
    };
    if let Some((k, v)) = cache.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let v = std::sync::Arc::new(crate::runs::load_index());
    *cache = Some((key, v.clone()));
    v
}

pub fn collect() -> Result<Status> {
    let runs = paths::runs_dir()?;
    let now = paths::now();
    let recent_ids = recent_run_ids(&runs);
    let mut status = Status::default();
    let mut index: Option<std::sync::Arc<Vec<IndexEntry>>> = None;
    for id in &recent_ids {
        let dir = runs.join(id);
        if dir.join("summary.json").exists() {
            continue;
        }
        let Some(meta) = read_meta(&dir) else {
            continue;
        };
        if !pid_alive(meta.pid, meta.start) {
            continue;
        }
        let live = crate::live::load(&dir).unwrap_or_default();
        let (ncpu, cpu_cores, sampled_at) = measured_cpu(&dir, &meta, now);
        let done_units = crate::summary::load_units(&dir);
        // Setup and unanchored remote runs cannot have an ETA. Keep loading
        // the potentially large history behind estimate's eligibility checks.
        let est = estimate(
            now,
            &meta,
            &live,
            &done_units,
            || index.get_or_insert_with(cached_index).as_slice(),
            &runs,
        );
        let running = live
            .active
            .iter()
            .zip(live.active_since.iter().chain(std::iter::repeat(&now)))
            .map(|(n, s)| (n.clone(), (now - s).max(0.0)))
            .collect();
        status.active.push(Active {
            id: meta.id.clone(),
            label: label(&meta.cwd, &meta.args),
            cwd: meta.cwd.clone(),
            args: meta.args.clone(),
            agent_session: meta.agent_session.clone(),
            ncpu,
            cpu_cores,
            sampled_at,
            reason: reason_of(Some(&meta), &place_of(Some(&meta), false)),
            place: place_of(Some(&meta), false),
            elapsed: now - meta.start,
            units_finished: done_units.len(),
            running,
            live,
            estimate: est,
        });
    }
    status.active.sort_by(|a, b| {
        let rem = |x: &Active| x.estimate.as_ref().map_or(f64::MAX, |e| e.remaining);
        rem(b).total_cmp(&rem(a))
    });
    if status.active.is_empty() {
        let last = match &index {
            Some(index) => index.last().cloned(),
            None => crate::runs::last_index_entry(),
        };
        if let Some(r) = last.as_ref() {
            let ago = now - (r.start + r.wall);
            if ago < SHOW_DONE_SECS && r.units_compiled > 0 {
                let meta = read_meta(&runs.join(&r.id));
                let place: String = crate::summary::execution_place(
                    r.place
                        .as_deref()
                        .or_else(|| meta.as_ref().and_then(|m| m.place.as_deref())),
                    r.remote.as_deref(),
                )
                .into();
                status.recent = Some(Finished {
                    id: r.id.clone(),
                    label: label(&r.cwd, &r.args),
                    // A remote run's wall is the machine's: show what the
                    // caller waited (sync included).
                    wall: r
                        .remote
                        .as_ref()
                        .and_then(|_| load_summary(&runs.join(&r.id))?.remote)
                        .map_or(r.wall, |ri| ri.client_wall.max(r.wall)),
                    ok: r.exit == 0,
                    ago,
                    reason: reason_of(meta.as_ref(), &place),
                    place,
                    cwd: r.cwd.clone(),
                    args: r.args.clone(),
                    agent_session: r.agent_session.clone(),
                });
            }
        }
    }
    Ok(status)
}

pub fn dur(s: f64) -> String {
    let s = s.max(0.0);
    if s < 10.0 {
        format!("{s:.1}s")
    } else if s < 60.0 {
        format!("{s:.0}s")
    } else if s < 3600.0 {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!(
            "{}h{:02}m",
            (s / 3600.0) as u64,
            ((s % 3600.0) / 60.0) as u64
        )
    }
}

fn phase_word(a: &Active) -> String {
    let l = &a.live;
    match l.phase.as_str() {
        "testing" => "testing".into(),
        "running" => "running".into(),
        "waiting for lock" => "waiting for lock".into(),
        "finishing" => "linking".into(),
        "syncing" => "syncing".into(),
        "waiting for sync lock" | "connecting" | "applying sources" => l.phase.clone(),
        "building" if l.total == 0 => "building".into(),
        _ if l.total > 0 => format!("{}/{}", l.done, l.total),
        _ => "starting".into(),
    }
}

/// One line per active run, for terminals.
fn render_text(s: &Status) -> String {
    let mut out = String::new();
    if s.active.is_empty() {
        match &s.recent {
            Some(r) => {
                let _ = writeln!(
                    out,
                    "{} {} {} in {}",
                    r.label,
                    r.place,
                    if r.ok { "finished" } else { "failed" },
                    dur(r.wall)
                );
            }
            None => out.push_str("no builds running\n"),
        }
        return out;
    }
    for a in &s.active {
        let _ = write!(
            out,
            "{}  {}  {}  {}",
            a.label,
            a.place,
            phase_word(a),
            dur(a.elapsed)
        );
        if let Some(e) = &a.estimate {
            let _ = write!(
                out,
                "  ~{} left ({:.0}%, {})",
                dur(e.remaining),
                e.fraction * 100.0,
                e.method
            );
        }
        out.push('\n');
        for (name, secs) in &a.running {
            let _ = writeln!(out, "    {name} {}", dur(*secs));
        }
    }
    out
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Waybar custom-module JSON: one line with text, tooltip, class, percentage.
pub fn render_waybar(s: &Status) -> String {
    let Some(a) = s.active.first() else {
        return match &s.recent {
            Some(r) => {
                let (icon, class) = if r.ok {
                    ("\u{f00c}", "done")
                } else {
                    ("\u{f00d}", "failed")
                };
                let text = format!(
                    "{icon} {} {} {}",
                    r.label,
                    place_tag(&r.place, r.reason.as_deref()),
                    dur(r.wall)
                );
                let mut tip = format!(
                    "{} {} ({})",
                    r.id,
                    if r.ok { "ok" } else { "failed" },
                    r.place
                );
                if let Some(why) = &r.reason {
                    let _ = write!(tip, "\n  local because: {why}");
                }
                format!(
                    "{{\"text\": {}, \"tooltip\": {}, \"class\": [\"{class}\", \"{}\"]}}",
                    json_escape(&text),
                    json_escape(&tip),
                    r.place
                )
            }
            None => "{\"text\": \"\", \"class\": \"idle\"}".into(),
        };
    };
    // Bar shows time taken so far, then progress. The time-left estimate
    // stays in the tooltip.
    let mut text = format!(
        "\u{e7a8} {} {} {} {}",
        a.label,
        place_tag(&a.place, a.reason.as_deref()),
        dur(a.elapsed),
        phase_word(a)
    );
    let (class, pct) = match &a.estimate {
        Some(e) => {
            let over = a.elapsed > e.reference_wall * 1.2 && e.remaining < 2.0;
            (
                if over { "long" } else { "building" },
                (e.fraction * 100.0).round(),
            )
        }
        None => ("starting", 0.0),
    };
    if s.active.len() > 1 {
        let _ = write!(text, " +{}", s.active.len() - 1);
    }
    let mut tip = String::new();
    for a in &s.active {
        let _ = writeln!(tip, "{}  ({}, {})", a.label, a.place, a.cwd);
        let _ = writeln!(tip, "  cargo {}", a.args.join(" "));
        if let Some(why) = &a.reason {
            let _ = writeln!(tip, "  local because: {why}");
        }
        let _ = writeln!(tip, "  elapsed {}, {}", dur(a.elapsed), phase_word(a));
        if let Some(e) = &a.estimate {
            let _ = writeln!(
                tip,
                "  ~{} left, {:.0}% (vs run {} that took {}, {})",
                dur(e.remaining),
                e.fraction * 100.0,
                e.reference,
                dur(e.reference_wall),
                e.method
            );
        } else {
            let _ = writeln!(tip, "  no similar finished run yet");
        }
        if a.live.compiling > 0 || a.live.fresh > 0 {
            let _ = writeln!(
                tip,
                "  {} compiled, {} fresh, {} rustc units done",
                a.live.compiling, a.live.fresh, a.units_finished
            );
        }
        for (name, secs) in &a.running {
            let _ = writeln!(tip, "    {name} {}", dur(*secs));
        }
    }
    format!(
        "{{\"text\": {}, \"tooltip\": {}, \"class\": [\"{class}\", \"{}\"], \"percentage\": {pct}}}",
        json_escape(&text),
        json_escape(tip.trim_end()),
        a.place
    )
}

pub fn command(json: bool, waybar: bool, watch: Option<f64>) -> Result<()> {
    let mut last = String::new();
    let mut wake = Wake::new();
    loop {
        let s = collect()?;
        let out = if waybar {
            render_waybar(&s)
        } else if json {
            serde_json::to_string(&s)?
        } else {
            render_text(&s)
        };
        // Under --watch, repeat lines only make the reader (Waybar) redraw.
        if watch.is_none() || out != last {
            println!("{}", out.trim_end());
            use std::io::Write;
            if std::io::stdout().flush().is_err() && watch.is_some() {
                return Ok(()); // reader went away
            }
            last = out;
        }
        let Some(secs) = watch else { return Ok(()) };
        // Only a running build (elapsed time) or a just-finished run (shown
        // for SHOW_DONE_SECS) changes on its own. Otherwise nothing changes
        // until a run starts or finishes: sleep on inotify until then.
        let tick = secs.max(0.2);
        let timeout = if !s.active.is_empty() {
            Some(tick)
        } else {
            s.recent
                .as_ref()
                .map(|r| (SHOW_DONE_SECS - r.ago).clamp(0.05, SHOW_DONE_SECS) + 0.05)
        };
        if !wake.wait(timeout) {
            std::thread::sleep(std::time::Duration::from_secs_f64(timeout.unwrap_or(tick)));
        }
    }
}

/// Blocks until justrust's state may have changed: a run directory appears or
/// changes, or the index is appended. Falls back to plain sleeping (returns
/// false) where inotify is unavailable.
struct Wake {
    #[cfg(target_os = "linux")]
    fd: Option<std::os::fd::OwnedFd>,
    #[cfg(target_os = "linux")]
    home_watch: i32,
    #[cfg(target_os = "linux")]
    runs_watch: i32,
}

impl Wake {
    fn new() -> Self {
        Self::new_at(paths::home().ok(), paths::runs_dir().ok())
    }

    fn new_at(home: Option<std::path::PathBuf>, runs: Option<std::path::PathBuf>) -> Self {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd;
            use std::os::unix::ffi::OsStrExt;
            // SAFETY: plain syscall; the fd is owned below.
            let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
            if raw < 0 {
                return Wake {
                    fd: None,
                    home_watch: -1,
                    runs_watch: -1,
                };
            }
            // SAFETY: fresh fd we own.
            let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
            let topology = libc::IN_CREATE
                | libc::IN_MOVED_TO
                | libc::IN_MOVED_FROM
                | libc::IN_DELETE
                | libc::IN_DELETE_SELF
                | libc::IN_MOVE_SELF;
            let mut watches = [-1; 2];
            for (i, dir) in [home, runs].into_iter().enumerate() {
                let Some(dir) = dir else { continue };
                let _ = std::fs::create_dir_all(&dir);
                let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
                    continue;
                };
                let mask = topology
                    | if i == 0 {
                        libc::IN_MODIFY | libc::IN_CLOSE_WRITE
                    } else {
                        0
                    };
                // SAFETY: valid fd and NUL-terminated path.
                watches[i] = unsafe { libc::inotify_add_watch(raw, c.as_ptr(), mask) };
            }
            Wake {
                // Partial registration could miss a run or completion forever.
                fd: watches.iter().all(|w| *w >= 0).then_some(fd),
                home_watch: watches[0],
                runs_watch: watches[1],
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (home, runs);
            Wake {}
        }
    }

    /// Drain the nonblocking queue before deciding whether a snapshot is needed.
    #[cfg(target_os = "linux")]
    fn drain(&self) -> std::io::Result<bool> {
        use std::os::fd::AsRawFd;
        let mut relevant = false;
        let mut buf = [0u8; 16 << 10];
        loop {
            // SAFETY: owned buffer and live, nonblocking inotify descriptor.
            let n = unsafe {
                libc::read(
                    self.fd.as_ref().unwrap().as_raw_fd(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    return Ok(relevant);
                }
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if n == 0 {
                return Err(std::io::Error::other("inotify closed"));
            }
            let mut offset = 0;
            let size = std::mem::size_of::<libc::inotify_event>();
            while offset + size <= n as usize {
                // SAFETY: header fits, but the byte buffer need not be aligned.
                let event = unsafe {
                    std::ptr::read_unaligned(buf.as_ptr().add(offset).cast::<libc::inotify_event>())
                };
                let start = offset + size;
                let end = start + event.len as usize;
                if end > n as usize {
                    return Err(std::io::Error::other("short inotify event"));
                }
                if event.mask & (libc::IN_IGNORED | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0
                {
                    return Err(std::io::Error::other("status watch invalidated"));
                }
                let name = &buf[start..end];
                let name = name.split(|b| *b == 0).next().unwrap_or_default();
                relevant |= event.mask & libc::IN_Q_OVERFLOW != 0
                    || (event.wd == self.home_watch && name == b"index.jsonl")
                    || (event.wd == self.runs_watch && event.mask & libc::IN_ISDIR != 0);
                offset = end;
            }
        }
    }

    /// Waits up to `timeout` seconds (forever if None). Returns false if it
    /// could not wait, so the caller sleeps instead.
    fn wait(&mut self, timeout: Option<f64>) -> bool {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let Some(fd) = &self.fd else { return false };
            let raw = fd.as_raw_fd();
            // Also watch stdout: a closed reader (Waybar reloaded) raises
            // POLLERR on the pipe, so an idle watcher exits instead of
            // sleeping forever as an orphan.
            let mut pfds = [
                libc::pollfd {
                    fd: raw,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: libc::STDOUT_FILENO,
                    events: 0,
                    revents: 0,
                },
            ];
            let deadline =
                timeout.map(|t| std::time::Instant::now() + std::time::Duration::from_secs_f64(t));
            loop {
                let ms = deadline.map_or(-1, |end| {
                    (end.saturating_duration_since(std::time::Instant::now())
                        .as_secs_f64()
                        * 1000.0)
                        .ceil()
                        .min(i32::MAX as f64) as i32
                });
                for pfd in &mut pfds {
                    pfd.revents = 0;
                }
                // SAFETY: two valid pollfds.
                let n = unsafe { libc::poll(pfds.as_mut_ptr(), 2, ms) };
                if pfds[1].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                    std::process::exit(0);
                }
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return false;
                }
                if n == 0 {
                    return true;
                }
                if pfds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    self.fd = None;
                    return false;
                }
                match self.drain() {
                    Ok(true) => {
                        // Coalesce only relevant bursts, never telemetry noise.
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if self.drain().is_err() {
                            self.fd = None;
                        }
                        return true;
                    }
                    Ok(false) => {
                        if deadline.is_some_and(|end| std::time::Instant::now() >= end) {
                            return true;
                        }
                    }
                    Err(_) => {
                        self.fd = None;
                        return false;
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = timeout;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn wake_fixture() -> (std::path::PathBuf, Wake) {
        let dir = std::env::temp_dir().join(format!(
            "justrust-wake-{}-{}",
            std::process::id(),
            paths::now()
        ));
        let runs = dir.join("runs");
        let wake = Wake::new_at(Some(dir.clone()), Some(runs));
        assert!(wake.fd.is_some());
        (dir, wake)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_wake_filters_real_inotify_events() {
        use std::io::Write;
        let (dir, mut wake) = wake_fixture();
        // Home telemetry and non-directory run entries do not change status.
        for _ in 0..100 {
            std::fs::write(dir.join("telemetry.json"), "noise").unwrap();
            std::fs::write(dir.join("runs/upload.tmp"), "noise").unwrap();
        }
        assert!(!wake.drain().unwrap());
        let start = std::time::Instant::now();
        assert!(wake.wait(Some(0.03)));
        assert!(start.elapsed() >= std::time::Duration::from_millis(20));
        // Continuous unrelated writes must not postpone the active-run tick.
        let noise_dir = dir.clone();
        let writer = std::thread::spawn(move || {
            for _ in 0..20 {
                std::fs::write(noise_dir.join("telemetry.json"), "noise").unwrap();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let start = std::time::Instant::now();
        assert!(wake.wait(Some(0.03)));
        assert!(start.elapsed() >= std::time::Duration::from_millis(20));
        writer.join().unwrap();
        assert!(!wake.drain().unwrap());
        // New runs wake, even before they have metadata.
        std::fs::create_dir(dir.join("runs/new-run")).unwrap();
        assert!(wake.drain().unwrap());
        // Completion append must wake before the writer closes the index.
        let mut index = std::fs::File::create(dir.join("index.jsonl")).unwrap();
        wake.drain().unwrap();
        index.write_all(b"{}\n").unwrap();
        assert!(wake.drain().unwrap());
        drop(index);
        // IN_CLOSE_WRITE fires once the last descriptor on the open file closes;
        // a sibling test forking a child can hold the O_CLOEXEC fd until exec().
        let close_wait = std::time::Instant::now();
        while !wake.drain().unwrap() {
            assert!(
                close_wait.elapsed() < std::time::Duration::from_secs(5),
                "IN_CLOSE_WRITE did not arrive after closing index.jsonl"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::write(dir.join("index.tmp"), "replacement").unwrap();
        std::fs::rename(dir.join("index.tmp"), dir.join("index.jsonl")).unwrap();
        assert!(wake.drain().unwrap());
        std::fs::remove_dir(dir.join("runs/new-run")).unwrap();
        assert!(wake.drain().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        // Losing either directory switches permanently to timed fallback.
        // The kernel reports the loss once the last descriptor on the
        // directory closes, and a child another test is forking can hold the
        // one the removal used until it execs. Until then the deletions inside
        // it are ordinary wakes.
        let lost = std::time::Instant::now();
        while wake.wait(Some(0.01)) {
            assert!(lost.elapsed() < std::time::Duration::from_secs(5));
        }
        assert!(wake.fd.is_none());
        assert!(!wake.wait(Some(0.01)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_wake_output_reader_closure() {
        const READY: &str = "JUSTRUST_STATUS_WAKE_TEST_READY";
        if let Some(ready) = std::env::var_os(READY) {
            let ready = std::path::PathBuf::from(ready);
            let dir = ready.parent().unwrap();
            let mut wake = Wake::new_at(Some(dir.to_owned()), Some(dir.join("runs")));
            std::fs::write(&ready, b"ready").unwrap();
            wake.wait(None);
            panic!("closed stdout should exit the watcher");
        }
        let (dir, _wake) = wake_fixture();
        let ready = dir.join("ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "status::tests::status_wake_output_reader_closure",
                "--nocapture",
            ])
            .env(READY, &ready)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "child did not reach inotify wait");
        drop(child.stdout.take());
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("idle watcher did not exit after output reader closed");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_wake_partial_registration_falls_back() {
        let (dir, _wake) = wake_fixture();
        let mut partial = Wake::new_at(Some(dir.clone()), None);
        assert!(!partial.wait(Some(0.01)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cpu_samples_are_measured_fresh_and_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "justrust-status-{}-{}",
            std::process::id(),
            paths::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let meta = Meta {
            start: 100.0,
            ncpu: 8,
            ..Default::default()
        };
        assert_eq!(measured_cpu(&dir, &meta, 110.0), (8, None, None));
        // A journal with a partially written trailing line still exposes the
        // last complete sample. Counts of processes never enter this math.
        std::fs::write(
            dir.join("samples.jsonl"),
            "{\"t\":109.0,\"build_cores\":2.5,\"rustc_procs\":99}\n{\"t\":",
        )
        .unwrap();
        assert_eq!(
            measured_cpu(&dir, &meta, 110.0),
            (8, Some(2.5), Some(109.0))
        );
        for value in [
            "{\"t\":104.0,\"build_cores\":2.5}",
            "{\"t\":112.0,\"build_cores\":2.5}",
            "{\"t\":99.0,\"build_cores\":2.5}",
            "{\"t\":109.0,\"build_cores\":-1}",
            "{\"t\":109.0,\"build_cores\":9}",
        ] {
            std::fs::write(dir.join("cpu.json"), value).unwrap();
            assert_eq!(measured_cpu(&dir, &meta, 110.0), (8, None, None), "{value}");
        }
        // A remote snapshot supplies the remote CPU count, not this host's.
        std::fs::write(
            dir.join("cpu.json"),
            "{\"t\":110.0,\"build_cores\":12.5,\"ncpu\":32}",
        )
        .unwrap();
        assert_eq!(
            measured_cpu(&dir, &meta, 110.0),
            (32, Some(12.5), Some(110.0))
        );
        std::fs::write(dir.join("cpu.json"), "malformed").unwrap();
        assert_eq!(
            measured_cpu(&dir, &meta, 110.0),
            (8, Some(2.5), Some(109.0))
        );
        assert_eq!(measured_cpu(&dir, &meta, 120.0), (8, None, None));
        // Imported IDs must not crowd recent normal runs out of the scan.
        let runs = dir.join("runs");
        std::fs::create_dir_all(runs.join("20261025-215400-42")).unwrap();
        for n in 0..70 {
            std::fs::create_dir_all(runs.join(format!("readme-fixture-{n}"))).unwrap();
        }
        let ids = recent_run_ids(&runs);
        assert!(ids.iter().any(|id| id == "20261025-215400-42"));
        assert_eq!(ids.len(), 1 + SCAN_RUNS);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn labels_runs() {
        assert_eq!(
            label("/home/u/jcode", &s(&["check", "--workspace"])),
            "jcode check"
        );
        assert_eq!(
            label(
                "/home/u/jcode",
                &s(&["test", "-p", "jcode-desktop-ui", "--lib"])
            ),
            "jcode-desktop-ui test"
        );
        assert_eq!(
            label("/x/repo", &s(&["build", "-p", "a", "-p", "b"])),
            "repo build"
        );
    }

    #[test]
    fn waybar_shows_where_the_build_runs() {
        let a = Active {
            id: "r1".into(),
            label: "jcode check".into(),
            cwd: "/w".into(),
            args: s(&["check"]),
            agent_session: None,
            ncpu: 0,
            cpu_cores: None,
            sampled_at: None,
            place: "hosted".into(),
            reason: None,
            elapsed: 4.0,
            live: LiveState {
                phase: "syncing".into(),
                ..Default::default()
            },
            units_finished: 0,
            running: vec![],
            estimate: None,
        };
        let st = Status {
            active: vec![a],
            recent: None,
        };
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["active"][0]["ncpu"], 0);
        for field in ["agent_session", "cpu_cores", "sampled_at"] {
            assert!(json["active"][0].get(field).unwrap().is_null());
        }
        let v: serde_json::Value = serde_json::from_str(&render_waybar(&st)).unwrap();
        let text = v["text"].as_str().unwrap();
        assert!(
            text.contains("[hosted]") && text.contains("syncing"),
            "{text}"
        );
        assert_eq!(v["class"], serde_json::json!(["starting", "hosted"]));
        let done = Status {
            active: vec![],
            recent: Some(Finished {
                id: "r2".into(),
                label: "jcode test".into(),
                wall: 12.0,
                ok: true,
                ago: 1.0,
                place: "local".into(),
                reason: None,
                cwd: "/w".into(),
                args: s(&["test"]),
                agent_session: None,
            }),
        };
        let v: serde_json::Value = serde_json::from_str(&render_waybar(&done)).unwrap();
        assert!(v["text"].as_str().unwrap().contains("[local]"));
        assert_eq!(v["class"], serde_json::json!(["done", "local"]));
        // A local run says why, cut short in the bar and whole in the tooltip.
        let mut why = done;
        let long = "JUSTRUST_REMOTE=off: need local debug binary for the macOS VM end-to-end test";
        why.recent.as_mut().unwrap().reason = Some(long.into());
        let v: serde_json::Value = serde_json::from_str(&render_waybar(&why)).unwrap();
        let text = v["text"].as_str().unwrap();
        assert!(
            text.contains("[local: need local debug binary for the macOS VM end-to\u{2026}]"),
            "{text}"
        );
        assert!(v["tooltip"].as_str().unwrap().contains(long));
        assert_eq!(
            place_tag("local", Some("`-q` always runs locally")),
            "[local: `-q` always runs locally]"
        );
        assert_eq!(place_tag("hosted", None), "[hosted]");
        // Old runs without a place: local unless the index says remote.
        assert_eq!(place_of(None, false), "local");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(dur(3.25), "3.2s");
        assert_eq!(dur(42.0), "42s");
        assert_eq!(dur(125.0), "2m05s");
    }

    #[test]
    fn active_locations_use_recorded_provenance_not_current_backend() {
        for place in ["hosted", "remote", "local"] {
            let meta = Meta {
                place: Some(place.into()),
                ..Default::default()
            };
            assert_eq!(place_of(Some(&meta), false), place);
        }
        assert_eq!(place_of(None, true), "remote");
        assert_eq!(place_of(None, false), "local");
    }

    fn entry(id: &str, args: &[&str], units: usize) -> IndexEntry {
        IndexEntry {
            id: id.into(),
            cwd: "/w".into(),
            args: s(args),
            units_compiled: units,
            ..Default::default()
        }
    }

    #[test]
    fn setup_and_lock_waits_have_no_countdown_even_with_a_reference() {
        let dir = std::env::temp_dir().join(format!(
            "jr-status-eta-{}-{}",
            std::process::id(),
            paths::now()
        ));
        std::fs::create_dir_all(dir.join("reference")).unwrap();
        let summary = Summary {
            wall: 63.0,
            ..Default::default()
        };
        std::fs::write(
            dir.join("reference/summary.json"),
            serde_json::to_vec(&summary).unwrap(),
        )
        .unwrap();
        let meta = Meta {
            id: "current".into(),
            cwd: "/w".into(),
            args: s(&["check"]),
            subcommand: "check".into(),
            start: 100.0,
            ..Default::default()
        };
        let index = [entry("reference", &["check"], 1)];
        let mut live = LiveState {
            phase: "compiling".into(),
            ..Default::default()
        };
        assert_eq!(
            estimate(159.0, &meta, &live, &[], || &index, &dir)
                .unwrap()
                .remaining,
            4.0
        );
        for phase in [
            "starting",
            "syncing",
            "connecting",
            "applying sources",
            "waiting for sync lock",
            "waiting for lock",
            "building",
        ] {
            live.phase = phase.into();
            assert!(
                estimate(
                    159.0,
                    &meta,
                    &live,
                    &[],
                    || panic!("setup loaded history"),
                    &dir
                )
                .is_none(),
                "{phase}"
            );
        }
        live.phase = "compiling".into();
        let remote = Meta {
            place: Some("hosted".into()),
            ..meta
        };
        assert!(
            estimate(
                159.0,
                &remote,
                &live,
                &[],
                || panic!("unanchored remote loaded history"),
                &dir
            )
            .is_none()
        );
        live.phase = "testing".into();
        live.finished_at = Some(158.0);
        assert!(estimate(159.0, &remote, &live, &[], || &index, &dir).is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    // Frozen pre-optimization algorithm: deliberately builds both full pools.
    fn legacy_pick_reference<'a>(
        index: &'a [IndexEntry],
        meta: &Meta,
        to_compile: Option<usize>,
    ) -> Option<&'a IndexEntry> {
        let remote = meta.place.as_deref().is_some_and(|p| p != "local");
        let same_place = index
            .iter()
            .any(|r| r.cwd == meta.cwd && r.remote.is_some() == remote);
        let ok = |r: &&IndexEntry| {
            r.cwd == meta.cwd
                && !r.compile_failed
                && r.id != meta.id
                && (!same_place || r.remote.is_some() == remote)
        };
        let same_args: Vec<&IndexEntry> = index
            .iter()
            .filter(ok)
            .filter(|r| r.args == meta.args)
            .collect();
        let same_sub: Vec<&IndexEntry> = index
            .iter()
            .filter(ok)
            .filter(|r| {
                crate::record::subcommand(&os(&r.args)).unwrap_or_default() == meta.subcommand
            })
            .collect();
        let pool = if same_args.is_empty() {
            same_sub
        } else {
            same_args
        };
        let pool: Vec<&IndexEntry> = pool.into_iter().rev().take(20).collect();
        let mut near: Vec<(usize, &IndexEntry)> = match to_compile {
            Some(n) => {
                let mut v: Vec<(usize, &IndexEntry)> = pool.into_iter().enumerate().collect();
                v.sort_by_key(|(i, r)| (r.units_compiled.abs_diff(n), *i));
                v
            }
            None => pool
                .into_iter()
                .filter(|r| r.units_compiled > 0)
                .enumerate()
                .collect(),
        };
        if let (Some(n), Some(best)) = (to_compile, near.first().map(|(_, r)| r.units_compiled)) {
            let slack = (best.abs_diff(n)).max(best / 4).max(2);
            near.retain(|(_, r)| r.units_compiled.abs_diff(best) <= slack);
        }
        near.truncate(5);
        near.sort_by(|a, b| a.1.wall.total_cmp(&b.1.wall));
        near.get(near.len() / 2).map(|(_, r)| *r)
    }

    fn varied_history(len: usize, seed: u64) -> Vec<IndexEntry> {
        let mut state = seed;
        (0..len)
            .map(|i| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let mut r = entry(
                    &i.to_string(),
                    match (state >> 32) % 5 {
                        0 => &["check"],
                        1 => &["check", "-q"],
                        2 => &["test"],
                        3 => &["--release", "check"],
                        _ => &[],
                    },
                    (state >> 40) as usize % 80,
                );
                r.cwd = if state & 3 == 0 { "/other" } else { "/w" }.into();
                r.compile_failed = state & 7 == 0;
                r.remote = (state & 8 == 0).then(|| "hosted".into());
                r.wall = match (state >> 48) % 16 {
                    0 => f64::NAN,
                    1 => -0.0,
                    n => n as f64,
                };
                r
            })
            .collect()
    }

    #[test]
    fn reference_matches_fullscan_oracle() {
        for seed in 0..24 {
            let index = varied_history(160, seed);
            for len in [0, 1, 2, 19, 20, 21, 60, 160] {
                for args in [s(&["check"]), s(&["check", "--absent"]), s(&[])] {
                    for place in [None, Some("local".into()), Some("hosted".into())] {
                        let meta = Meta {
                            id: (seed % 160).to_string(),
                            cwd: "/w".into(),
                            subcommand: crate::record::subcommand(&os(&args)).unwrap_or_default(),
                            args: args.clone(),
                            place,
                            ..Default::default()
                        };
                        for n in [None, Some(0), Some(2), Some(40), Some(usize::MAX)] {
                            let actual = pick_reference(&index[..len], &meta, n).map(|r| &r.id);
                            let expected =
                                legacy_pick_reference(&index[..len], &meta, n).map(|r| &r.id);
                            assert_eq!(actual, expected, "seed={seed} len={len} n={n:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reference_preserves_old_exact_and_place_detection() {
        let meta = Meta {
            id: "self".into(),
            cwd: "/w".into(),
            args: s(&["check"]),
            subcommand: "check".into(),
            ..Default::default()
        };
        let mut index = vec![entry("old-exact", &["check"], 1)];
        index.extend((0..100).map(|i| entry(&i.to_string(), &["check", "-q"], 1)));
        assert_eq!(
            pick_reference(&index, &meta, Some(1)).unwrap().id,
            "old-exact"
        );
        // The old matching-place row may be failed or the current run itself.
        // It still prevents falling back to successful opposite-place rows.
        index[0].remote = Some("hosted".into());
        for r in &mut index[1..] {
            r.remote = Some("hosted".into());
        }
        for failed in [false, true] {
            let mut local = entry(if failed { "failed" } else { "self" }, &["check"], 1);
            local.compile_failed = failed;
            index.insert(0, local);
            assert!(pick_reference(&index, &meta, Some(1)).is_none());
            assert!(legacy_pick_reference(&index, &meta, Some(1)).is_none());
            index.remove(0);
        }
        assert_eq!(
            pick_reference(&index, &meta, Some(1)).unwrap().id,
            "old-exact"
        );

        // Truncation happens before zero-unit filtering, and equal walls keep
        // the size/recency order through the stable median sort.
        let mut index = vec![entry("too-old", &["check"], 1)];
        index.extend((0..20).map(|i| entry(&i.to_string(), &["check"], 0)));
        assert!(pick_reference(&index, &meta, None).is_none());
        for r in &mut index {
            r.units_compiled = 1;
            r.wall = 3.0;
        }
        assert_eq!(pick_reference(&index, &meta, None).unwrap().id, "17");
        assert_eq!(pick_reference(&index, &meta, Some(1)).unwrap().id, "17");
    }

    #[test]
    #[ignore = "focused selector benchmark, run with --ignored --nocapture"]
    fn benchmark_reference_selection() {
        use std::hint::black_box;
        let index = varied_history(40_000, 17);
        for args in [s(&["check"]), s(&["check", "--absent"])] {
            let meta = Meta {
                id: "current".into(),
                cwd: "/w".into(),
                args,
                subcommand: "check".into(),
                ..Default::default()
            };
            type Selector =
                for<'a> fn(&'a [IndexEntry], &Meta, Option<usize>) -> Option<&'a IndexEntry>;
            for (name, select) in [
                ("legacy", legacy_pick_reference as Selector),
                ("current", pick_reference as Selector),
            ] {
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    black_box(select(
                        black_box(&index),
                        black_box(&meta),
                        black_box(Some(40)),
                    ));
                }
                eprintln!(
                    "{name} args={:?}: {:?}/100 selections (40000 rows)",
                    meta.args,
                    start.elapsed()
                );
            }
        }
    }

    #[test]
    fn newest_run_entries_matches_full_sort() {
        for len in [0, 1, SCAN_RUNS - 1, SCAN_RUNS, SCAN_RUNS + 1, 1000, 53_000] {
            let entries: Vec<_> = (0..len)
                .map(|i| (i.wrapping_mul(48271) % 104729, format!("run-{i}")))
                .collect();
            let mut expected = entries.clone();
            expected.sort();
            expected.reverse();
            expected.truncate(SCAN_RUNS);
            assert_eq!(newest_run_entries(entries), expected, "length {len}");
        }
        assert_eq!(
            newest_run_entries(vec!["same"; 100]),
            vec!["same"; SCAN_RUNS]
        );
    }

    #[test]
    #[ignore = "focused directory-ID selection diagnostic"]
    fn newest_run_entries_diagnostic() {
        use std::hint::black_box;
        let entries: Vec<_> = (0u64..53_000)
            .map(|i| format!("20261116-{:09}-{i:06}", i.wrapping_mul(48271) % 104729))
            .collect();
        let mut expected = entries.clone();
        expected.sort();
        expected.reverse();
        expected.truncate(SCAN_RUNS);
        assert_eq!(newest_run_entries(entries.clone()), expected);
        let mut full = Vec::new();
        let mut bounded = Vec::new();
        for _ in 0..21 {
            // Cloning is intentionally outside each timed selection.
            let mut old = entries.clone();
            let new = entries.clone();
            let start = std::time::Instant::now();
            old.sort();
            let old: Vec<_> = old.into_iter().rev().take(SCAN_RUNS).collect();
            black_box(old);
            full.push(start.elapsed().as_nanos());
            let start = std::time::Instant::now();
            black_box(newest_run_entries(new));
            bounded.push(start.elapsed().as_nanos());
        }
        full.sort();
        bounded.sort();
        eprintln!(
            "newest 64 of 53000 IDs: full-sort median={}ns, partition median={}ns (21 samples)",
            full[10], bounded[10]
        );
    }

    #[test]
    fn picks_reference_with_same_args_and_similar_size() {
        let mut index = vec![
            entry("1", &["check"], 300),
            entry("2", &["check"], 3),
            entry("3", &["test"], 4),
            entry("4", &["check"], 0),
        ];
        let meta = Meta {
            id: "9".into(),
            cwd: "/w".into(),
            args: s(&["check"]),
            subcommand: "check".into(),
            ..Default::default()
        };
        assert_eq!(pick_reference(&index, &meta, Some(250)).unwrap().id, "1");
        let only = |n| {
            pick_reference(&index[..2], &meta, Some(n))
                .unwrap()
                .id
                .clone()
        };
        assert_eq!(only(4), "2");
        let other = Meta {
            args: s(&["check", "-q"]),
            ..meta.clone()
        };
        // No exact match: same subcommand.
        assert_eq!(pick_reference(&index, &other, Some(250)).unwrap().id, "1");
        // Median wall of the closest five, not the single closest.
        index.clear();
        for (id, wall) in [("a", 7.0), ("b", 30.0), ("c", 8.0), ("d", 7.5), ("e", 7.7)] {
            let mut e = entry(id, &["check"], 1);
            e.wall = wall;
            index.push(e);
        }
        assert_eq!(pick_reference(&index, &meta, Some(1)).unwrap().id, "e");
    }
}
