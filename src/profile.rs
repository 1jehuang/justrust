//! `justrust profile`: where agents' end-to-end build wait went, across
//! recorded runs. Each run contributes its `e2e` waterfall (or a coarser one
//! reconstructed from older records), so totals cover client routing, hosted
//! admission, source sync, host or local cargo phases, artifact return, and
//! reporting in one table.

use crate::e2e::{self, E2e};
use crate::paths;
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub runs: usize,
    /// Runs with client marks (not reconstructed).
    pub measured: usize,
    pub derived: usize,
    /// Indexed runs without a readable summary.
    pub unavailable: usize,
    pub total_wait_secs: f64,
    pub wall_p50: f64,
    pub wall_p90: f64,
    pub wall_max: f64,
    /// Segment key -> seconds, sorted descending in output.
    pub segments: Vec<SegmentTotal>,
    /// Per-category totals when grouping was requested (`--by`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupTotal>,
    /// The slowest runs, for `justrust show`.
    pub slowest: Vec<SlowRun>,
}

/// A dimension `justrust profile --by` can split runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dim {
    /// Cargo subcommand: check, test, build, clippy, run.
    Command,
    /// Cargo profile: dev, release, or a named `--profile`.
    Profile,
    /// How much was rebuilt: no-op, local crates only, some deps, cold.
    Scope,
}

/// Wait for one category of runs. Overhead is everything that is neither
/// compiling nor running tests: routing, admission, sync, locks, cargo
/// startup, artifact return, reporting.
#[derive(Debug, Default, Serialize, Clone)]
pub struct GroupTotal {
    pub key: String,
    pub runs: usize,
    pub secs: f64,
    pub share: f64,
    pub wall_p50: f64,
    pub wall_p90: f64,
    pub compile_secs: f64,
    pub test_secs: f64,
    pub overhead_secs: f64,
    pub overhead_p50: f64,
    pub overhead_p90: f64,
    /// Largest overhead segment in this category.
    pub top_overhead: String,
    pub top_overhead_secs: f64,
}

/// Units compiled at or above this count as a cold or full build.
const COLD_UNITS: usize = 50;

/// Cargo profile a command line selects.
pub fn cargo_profile(subcommand: &str, args: &[String]) -> String {
    let mut it = args.iter().take_while(|a| a.as_str() != "--");
    while let Some(a) = it.next() {
        if a == "--release" || a == "-r" {
            return "release".into();
        }
        if a == "--profile" {
            if let Some(p) = it.next() {
                return p.clone();
            }
        } else if let Some(p) = a.strip_prefix("--profile=") {
            return p.to_owned();
        }
    }
    match subcommand {
        "bench" => "bench",
        "install" => "release",
        _ => "dev",
    }
    .into()
}

/// How much of the graph a run rebuilt.
pub fn scope(u: &crate::summary::UnitStats) -> &'static str {
    if u.compiled == 0 {
        "no-op"
    } else if u.compiled >= COLD_UNITS {
        "cold"
    } else if u.dependencies + u.build_scripts == 0 {
        "incremental"
    } else {
        "some-deps"
    }
}

/// Category label of a run for the requested dimensions.
pub fn group_key(s: &crate::summary::Summary, dims: &[Dim]) -> String {
    dims.iter()
        .map(|d| match d {
            Dim::Command => s.subcommand.clone(),
            Dim::Profile => cargo_profile(&s.subcommand, &s.args),
            Dim::Scope => scope(&s.units).to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Per-category totals, largest total wait first.
pub fn group(runs: &[(String, &E2e)]) -> Vec<GroupTotal> {
    struct Acc {
        g: GroupTotal,
        walls: Vec<f64>,
        overheads: Vec<f64>,
        segs: BTreeMap<String, f64>,
    }
    let mut accs: BTreeMap<&str, Acc> = BTreeMap::new();
    let mut total = 0.0;
    for (key, e) in runs {
        let a = accs.entry(key).or_insert_with(|| Acc {
            g: GroupTotal {
                key: key.clone(),
                ..Default::default()
            },
            walls: Vec::new(),
            overheads: Vec::new(),
            segs: BTreeMap::new(),
        });
        let (mut compile, mut tests) = (0.0, 0.0);
        for s in &e.segments {
            if s.key.ends_with(".compile") {
                compile += s.secs;
            } else if s.key.ends_with(".tests") {
                tests += s.secs;
            } else {
                *a.segs.entry(s.key.clone()).or_default() += s.secs;
            }
        }
        let overhead = (e.wall - compile - tests).max(0.0);
        total += e.wall;
        a.g.runs += 1;
        a.g.secs += e.wall;
        a.g.compile_secs += compile;
        a.g.test_secs += tests;
        a.g.overhead_secs += overhead;
        a.walls.push(e.wall);
        a.overheads.push(overhead);
    }
    let mut out: Vec<GroupTotal> = accs
        .into_values()
        .map(|mut a| {
            a.walls.sort_by(f64::total_cmp);
            a.overheads.sort_by(f64::total_cmp);
            a.g.wall_p50 = percentile(&a.walls, 0.5);
            a.g.wall_p90 = percentile(&a.walls, 0.9);
            a.g.overhead_p50 = percentile(&a.overheads, 0.5);
            a.g.overhead_p90 = percentile(&a.overheads, 0.9);
            a.g.share = if total > 0.0 { a.g.secs / total } else { 0.0 };
            if let Some((k, v)) = a.segs.into_iter().max_by(|x, y| x.1.total_cmp(&y.1)) {
                a.g.top_overhead = k;
                a.g.top_overhead_secs = v;
            }
            a.g
        })
        .collect();
    out.sort_by(|a, b| b.secs.total_cmp(&a.secs));
    out
}

fn render_groups(o: &mut String, groups: &[GroupTotal]) {
    use std::fmt::Write;
    let width = groups
        .iter()
        .map(|g| g.key.chars().count())
        .max()
        .unwrap_or(0)
        .max(8);
    let _ = writeln!(
        o,
        "  {:<width$} {:>5} {:>9} {:>6} {:>7} {:>7} {:>8} {:>6} {:>9} {:>8} {:>8}  TOP OVERHEAD",
        "CATEGORY",
        "RUNS",
        "TOTAL",
        "SHARE",
        "P50",
        "P90",
        "COMPILE",
        "TESTS",
        "OVERHEAD",
        "OVH P50",
        "OVH P90"
    );
    for g in groups {
        let pct = |v: f64| {
            if g.secs > 0.0 {
                100.0 * v / g.secs
            } else {
                0.0
            }
        };
        let _ = writeln!(
            o,
            "  {:<width$} {:>5} {:>8.1}s {:>5.1}% {:>6.1}s {:>6.1}s {:>7.0}% {:>5.0}% {:>8.0}% {:>7.2}s {:>7.2}s  {} {:.0}%",
            g.key,
            g.runs,
            g.secs,
            100.0 * g.share,
            g.wall_p50,
            g.wall_p90,
            pct(g.compile_secs),
            pct(g.test_secs),
            pct(g.overhead_secs),
            g.overhead_p50,
            g.overhead_p90,
            e2e::label(&g.top_overhead),
            pct(g.top_overhead_secs)
        );
    }
    let _ = writeln!(
        o,
        "  Overhead is wait that is neither compiling nor running tests. Scope: no-op compiled nothing, incremental only workspace crates, some-deps under {COLD_UNITS} units including dependencies, cold {COLD_UNITS} or more.\n"
    );
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct SegmentTotal {
    pub key: String,
    pub secs: f64,
    pub share: f64,
    /// Runs where this segment took at least a second.
    pub runs_over_1s: usize,
    pub max_secs: f64,
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct SlowRun {
    pub id: String,
    pub wall: f64,
    pub top_segment: String,
    pub top_secs: f64,
    pub command: String,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Aggregate waterfalls. `slowest` keeps the top `keep` runs by wall.
pub fn aggregate(runs: &[(String, String, E2e)], keep: usize) -> Report {
    let mut r = Report::default();
    let mut totals: BTreeMap<String, SegmentTotal> = BTreeMap::new();
    let mut walls = Vec::new();
    for (id, command, e) in runs {
        r.runs += 1;
        if e.derived {
            r.derived += 1;
        } else {
            r.measured += 1;
        }
        r.total_wait_secs += e.wall;
        walls.push(e.wall);
        for s in &e.segments {
            let t = totals.entry(s.key.clone()).or_insert_with(|| SegmentTotal {
                key: s.key.clone(),
                ..Default::default()
            });
            t.secs += s.secs;
            t.max_secs = t.max_secs.max(s.secs);
            if s.secs >= 1.0 {
                t.runs_over_1s += 1;
            }
        }
        let top = e
            .segments
            .iter()
            .max_by(|a, b| a.secs.total_cmp(&b.secs))
            .cloned()
            .unwrap_or_default();
        r.slowest.push(SlowRun {
            id: id.clone(),
            wall: e.wall,
            top_segment: top.key,
            top_secs: top.secs,
            command: command.clone(),
        });
    }
    walls.sort_by(f64::total_cmp);
    r.wall_p50 = percentile(&walls, 0.5);
    r.wall_p90 = percentile(&walls, 0.9);
    r.wall_max = walls.last().copied().unwrap_or(0.0);
    let mut segs: Vec<SegmentTotal> = totals.into_values().collect();
    for s in &mut segs {
        s.share = if r.total_wait_secs > 0.0 {
            s.secs / r.total_wait_secs
        } else {
            0.0
        };
    }
    segs.sort_by(|a, b| b.secs.total_cmp(&a.secs));
    r.segments = segs;
    r.slowest.sort_by(|a, b| b.wall.total_cmp(&a.wall));
    r.slowest.truncate(keep);
    r
}

pub fn render(r: &Report, scope: &str) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "End-to-end wait across {} runs{scope}: {:.1}s total, p50 {:.2}s, p90 {:.2}s, max {:.1}s",
        r.runs, r.total_wait_secs, r.wall_p50, r.wall_p90, r.wall_max
    );
    let _ = writeln!(
        o,
        "  {} with full client marks, {} reconstructed from older records (coarser), {} without a summary\n",
        r.measured, r.derived, r.unavailable
    );
    if !r.groups.is_empty() {
        render_groups(&mut o, &r.groups);
    }
    let _ = writeln!(
        o,
        "  {:<6} {:<26} {:>9} {:>6} {:>9} {:>8} {:>8}",
        "AREA", "SEGMENT", "TOTAL", "SHARE", "PER RUN", ">=1s", "MAX"
    );
    for s in r.segments.iter().filter(|s| s.secs >= 0.01) {
        let _ = writeln!(
            o,
            "  {:<6} {:<26} {:>8.1}s {:>5.1}% {:>8.2}s {:>8} {:>7.1}s",
            e2e::area(&s.key),
            e2e::label(&s.key),
            s.secs,
            100.0 * s.share,
            if r.runs > 0 {
                s.secs / r.runs as f64
            } else {
                0.0
            },
            s.runs_over_1s,
            s.max_secs
        );
    }
    if !r.slowest.is_empty() {
        let _ = writeln!(o, "\nSlowest runs (`justrust show ID` for the waterfall)");
        for s in &r.slowest {
            let _ = writeln!(
                o,
                "  {:<28} {:>7.1}s  {:<22} {:>6.1}s  {}",
                s.id,
                s.wall,
                e2e::label(&s.top_segment),
                s.top_secs,
                trunc(&s.command, 60)
            );
        }
    }
    let _ = writeln!(
        o,
        "\nSegments are non-overlapping and sum to each run's wait. Concurrent runs overlap in real time, so totals are agent wait, not elapsed time."
    );
    o
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

pub fn command(
    days: Option<f64>,
    here: bool,
    remote_only: bool,
    local_only: bool,
    measured_only: bool,
    by: &[Dim],
    json: bool,
) -> Result<()> {
    let root = paths::runs_dir()?;
    let cwd = std::env::current_dir()?;
    let cutoff = days.map(|d| paths::now() - d * 86400.0);
    let mut seen = HashSet::new();
    let mut runs = Vec::new();
    let mut keys = Vec::new();
    let mut unavailable = 0;
    for entry in crate::runs::load_index() {
        if cutoff.is_some_and(|c| entry.start < c)
            || (here && !Path::new(&entry.cwd).starts_with(&cwd))
            || (remote_only && entry.remote.is_none())
            || (local_only && entry.remote.is_some())
            || !seen.insert(entry.id.clone())
        {
            continue;
        }
        let Some(s) = std::fs::read(root.join(&entry.id).join("summary.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<crate::summary::Summary>(&b).ok())
        else {
            unavailable += 1;
            continue;
        };
        let Some(e) = s.e2e.clone().or_else(|| e2e::derive(&s)) else {
            unavailable += 1;
            continue;
        };
        if measured_only && e.derived {
            continue;
        }
        if !by.is_empty() {
            keys.push(group_key(&s, by));
        }
        runs.push((entry.id, format!("cargo {}", s.args.join(" ")), e));
    }
    let mut report = aggregate(&runs, 10);
    report.unavailable = unavailable;
    let keyed: Vec<(String, &E2e)> = keys.into_iter().zip(runs.iter().map(|r| &r.2)).collect();
    report.groups = group(&keyed);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let mut scope = String::new();
        if let Some(d) = days {
            scope += &format!(" in the last {d} days");
        }
        if here {
            scope += " under this directory";
        }
        if remote_only {
            scope += " (remote only)";
        }
        if local_only {
            scope += " (local only)";
        }
        print!("{}", render(&report, &scope));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::Segment;

    fn run(id: &str, segs: &[(&str, f64)], derived: bool) -> (String, String, E2e) {
        (
            id.into(),
            "cargo check".into(),
            E2e {
                wall: segs.iter().map(|(_, s)| s).sum(),
                segments: segs
                    .iter()
                    .map(|(k, s)| Segment {
                        key: (*k).into(),
                        secs: *s,
                    })
                    .collect(),
                derived,
            },
        )
    }

    #[test]
    fn aggregates_segments_shares_and_slowest() {
        let runs = vec![
            run("a", &[("sync.push", 100.0), ("host.compile", 3.0)], false),
            run("b", &[("sync.push", 0.1), ("host.compile", 4.0)], true),
            run("c", &[("build.compile", 1.0)], false),
        ];
        let r = aggregate(&runs, 2);
        assert_eq!(r.runs, 3);
        assert_eq!((r.measured, r.derived), (2, 1));
        assert!((r.total_wait_secs - 108.1).abs() < 1e-9);
        assert_eq!(r.segments[0].key, "sync.push");
        assert_eq!(r.segments[0].runs_over_1s, 1);
        assert!((r.segments[0].max_secs - 100.0).abs() < 1e-9);
        let share: f64 = r.segments.iter().map(|s| s.share).sum();
        assert!((share - 1.0).abs() < 1e-9);
        assert_eq!(r.slowest.len(), 2);
        assert_eq!(r.slowest[0].id, "a");
        assert_eq!(r.slowest[0].top_segment, "sync.push");
        assert!((r.wall_max - 103.0).abs() < 1e-9);
        let text = render(&r, "");
        assert!(text.contains("sync: source push"), "{text}");
    }

    #[test]
    fn groups_split_overhead_from_compile_and_tests() {
        let a = run("a", &[("host.compile", 6.0), ("sync.push", 2.0)], false).2;
        let b = run(
            "b",
            &[
                ("host.compile", 2.0),
                ("host.tests", 1.0),
                ("remote.admission", 1.0),
            ],
            false,
        )
        .2;
        let c = run("c", &[("remote.admission", 1.0)], false).2;
        let g = group(&[
            ("test dev incremental".into(), &a),
            ("test dev incremental".into(), &b),
            ("check dev no-op".into(), &c),
        ]);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].key.as_str(), g[0].runs), ("test dev incremental", 2));
        assert!((g[0].secs - 12.0).abs() < 1e-9);
        assert!((g[0].compile_secs - 8.0).abs() < 1e-9);
        assert!((g[0].test_secs - 1.0).abs() < 1e-9);
        assert!((g[0].overhead_secs - 3.0).abs() < 1e-9);
        assert_eq!(g[0].top_overhead, "sync.push");
        assert!((g[1].overhead_secs - g[1].secs).abs() < 1e-9);
        let share: f64 = g.iter().map(|g| g.share).sum();
        assert!((share - 1.0).abs() < 1e-9);
        let mut text = String::new();
        render_groups(&mut text, &g);
        assert!(text.contains("check dev no-op"), "{text}");
    }

    #[test]
    fn cargo_profile_and_scope_classify_runs() {
        let args = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(cargo_profile("test", &args("test -p x")), "dev");
        assert_eq!(cargo_profile("build", &args("build --release")), "release");
        assert_eq!(
            cargo_profile("build", &args("build --profile selfdev")),
            "selfdev"
        );
        assert_eq!(
            cargo_profile("build", &args("build --profile=fast")),
            "fast"
        );
        assert_eq!(cargo_profile("test", &args("test -- --release")), "dev");
        let units = |compiled, dependencies| crate::summary::UnitStats {
            compiled,
            dependencies,
            ..Default::default()
        };
        assert_eq!(scope(&units(0, 0)), "no-op");
        assert_eq!(scope(&units(3, 0)), "incremental");
        assert_eq!(scope(&units(5, 2)), "some-deps");
        assert_eq!(scope(&units(200, 150)), "cold");
    }

    #[test]
    fn empty_history_renders() {
        let r = aggregate(&[], 5);
        assert_eq!(r.runs, 0);
        assert!(render(&r, "").contains("across 0 runs"));
    }
}
