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
use std::path::Path;

/// How long a finished run stays visible in `--waybar` output.
const SHOW_DONE_SECS: f64 = 8.0;
/// Only this many of the newest run directories are checked for activity.
const SCAN_RUNS: usize = 64;

#[derive(Debug, Serialize, Clone)]
pub struct Active {
    pub id: String,
    pub label: String,
    pub cwd: String,
    pub args: Vec<String>,
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
    let boot = crate::procfs::boot_time().unwrap_or(0.0);
    let started = boot + st.start_ticks as f64 / crate::procfs::clk_tck();
    boot == 0.0 || started <= since + 2.0
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
    let ok = |r: &&IndexEntry| r.cwd == meta.cwd && !r.compile_failed && r.id != meta.id;
    let same_args: Vec<&IndexEntry> = index
        .iter()
        .filter(ok)
        .filter(|r| r.args == meta.args)
        .collect();
    let same_sub: Vec<&IndexEntry> = index
        .iter()
        .filter(ok)
        .filter(|r| crate::record::subcommand(&os(&r.args)).unwrap_or_default() == meta.subcommand)
        .collect();
    let pool = if same_args.is_empty() {
        same_sub
    } else {
        same_args
    };
    // Recent history only: the codebase and machine change.
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

fn estimate(
    now: f64,
    meta: &Meta,
    live: &LiveState,
    done_units: &[Unit],
    index: &[IndexEntry],
    runs: &Path,
) -> Option<Estimate> {
    let to_compile = (live.total > 0).then(|| live.total - live.first_done.unwrap_or(0));
    let r = pick_reference(index, meta, to_compile)?;
    let rdir = runs.join(&r.id);
    let summary = load_summary(&rdir)?;
    let elapsed = now - meta.start;
    let ref_build_end = build_end(&summary);
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
        let ref_units = crate::summary::load_units(&rdir);
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

pub fn collect() -> Result<Status> {
    let runs = paths::runs_dir()?;
    let now = paths::now();
    let mut ids: Vec<String> = std::fs::read_dir(&runs)
        .map(|rd| {
            rd.filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    let recent_ids: Vec<&String> = ids.iter().rev().take(SCAN_RUNS).collect();
    let mut status = Status::default();
    let mut index: Option<Vec<IndexEntry>> = None;
    for id in &recent_ids {
        let dir = runs.join(id);
        if dir.join("summary.json").exists() {
            continue;
        }
        let Some(meta) = std::fs::read(dir.join("meta.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
        else {
            continue;
        };
        if !pid_alive(meta.pid, meta.start) {
            continue;
        }
        let live = crate::live::load(&dir).unwrap_or_default();
        let done_units = crate::summary::load_units(&dir);
        let index = index.get_or_insert_with(crate::runs::load_index);
        let est = estimate(now, &meta, &live, &done_units, index, &runs);
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
        let index = index.unwrap_or_else(crate::runs::load_index);
        if let Some(r) = index.last() {
            let ago = now - (r.start + r.wall);
            if ago < SHOW_DONE_SECS && r.units_compiled > 0 {
                status.recent = Some(Finished {
                    id: r.id.clone(),
                    label: label(&r.cwd, &r.args),
                    wall: r.wall,
                    ok: r.exit == 0,
                    ago,
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
                    "{} {} in {}",
                    r.label,
                    if r.ok { "finished" } else { "failed" },
                    dur(r.wall)
                );
            }
            None => out.push_str("no builds running\n"),
        }
        return out;
    }
    for a in &s.active {
        let _ = write!(out, "{}  {}  {}", a.label, phase_word(a), dur(a.elapsed));
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
                let text = format!("{icon} {} {}", r.label, dur(r.wall));
                format!(
                    "{{\"text\": {}, \"tooltip\": {}, \"class\": \"{class}\"}}",
                    json_escape(&text),
                    json_escape(&format!("{} {}", r.id, if r.ok { "ok" } else { "failed" }))
                )
            }
            None => "{\"text\": \"\", \"class\": \"idle\"}".into(),
        };
    };
    let mut text = format!("\u{e7a8} {} {}", a.label, phase_word(a));
    let (class, pct) = match &a.estimate {
        Some(e) => {
            let _ = write!(text, " ~{} left", dur(e.remaining));
            let over = a.elapsed > e.reference_wall * 1.2 && e.remaining < 2.0;
            (
                if over { "long" } else { "building" },
                (e.fraction * 100.0).round(),
            )
        }
        None => {
            let _ = write!(text, " {}", dur(a.elapsed));
            ("starting", 0.0)
        }
    };
    if s.active.len() > 1 {
        let _ = write!(text, " +{}", s.active.len() - 1);
    }
    let mut tip = String::new();
    for a in &s.active {
        let _ = writeln!(tip, "{}  ({})", a.label, a.cwd);
        let _ = writeln!(tip, "  cargo {}", a.args.join(" "));
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
        "{{\"text\": {}, \"tooltip\": {}, \"class\": \"{class}\", \"percentage\": {pct}}}",
        json_escape(&text),
        json_escape(tip.trim_end())
    )
}

pub fn command(json: bool, waybar: bool, watch: Option<f64>) -> Result<()> {
    loop {
        let s = collect()?;
        let out = if waybar {
            render_waybar(&s)
        } else if json {
            serde_json::to_string(&s)?
        } else {
            render_text(&s)
        };
        println!("{}", out.trim_end());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        match watch {
            Some(secs) => std::thread::sleep(std::time::Duration::from_secs_f64(secs.max(0.2))),
            None => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn formats_durations() {
        assert_eq!(dur(3.25), "3.2s");
        assert_eq!(dur(42.0), "42s");
        assert_eq!(dur(125.0), "2m05s");
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
