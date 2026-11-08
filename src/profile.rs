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
    /// The slowest runs, for `justrust show`.
    pub slowest: Vec<SlowRun>,
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
    json: bool,
) -> Result<()> {
    let root = paths::runs_dir()?;
    let cwd = std::env::current_dir()?;
    let cutoff = days.map(|d| paths::now() - d * 86400.0);
    let mut seen = HashSet::new();
    let mut runs = Vec::new();
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
        runs.push((entry.id, format!("cargo {}", s.args.join(" ")), e));
    }
    let mut report = aggregate(&runs, 10);
    report.unavailable = unavailable;
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
    fn empty_history_renders() {
        let r = aggregate(&[], 5);
        assert_eq!(r.runs, 0);
        assert!(render(&r, "").contains("across 0 runs"));
    }
}
