//! Who owned the time when no compiler was running.
//!
//! `Phases::startup` and `Phases::build_gaps` are wall time inside cargo's
//! build with no rustc unit running. This module splits that time by what
//! was running instead, from evidence the recorder already has:
//!
//! - `shim`: the justrust rustc shim was working before it started the
//!   compiler or after the compiler exited (cache keys, cache tiers that
//!   missed, storing outputs). Exact from `Unit::shim_pre_secs` and
//!   `shim_post_secs`. For units recorded by older clients, sampled `rustc`
//!   processes that were alive outside every unit interval count too.
//! - `build_scripts`: a build script process was running (sampled).
//! - `other_procs`: some other sampled child was running (a linker, a
//!   target binary).
//! - `cargo`: nothing but cargo itself was observed. Fingerprinting, the job
//!   queue, unit graph and dep-info work, and children shorter than a
//!   sampling interval land here.
//!
//! Each second is charged once, in that order. The same split is made for
//! the startup window, where it separates the first unit's shim work from
//! cargo's own resolve and fingerprint time.
//!
//! `shim_job_secs` is different: the sum of shim time over all units,
//! whether or not a compiler was running elsewhere. Cargo holds a job slot
//! for that time, so it delays the build even where it is not a gap.

use crate::record::ProcRecord;
use crate::shim::{ShimStage, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Split {
    pub shim: f64,
    pub build_scripts: f64,
    pub other_procs: f64,
    pub cargo: f64,
}

impl Split {
    #[cfg(test)]
    fn total(&self) -> f64 {
        self.shim + self.build_scripts + self.other_procs + self.cargo
    }

    fn rounded(mut self) -> Self {
        for v in [
            &mut self.shim,
            &mut self.build_scripts,
            &mut self.other_procs,
            &mut self.cargo,
        ] {
            *v = (*v * 1000.0).round() / 1000.0;
        }
        self
    }

    /// `shim 1.2s, build scripts 0.4s, cargo 0.3s`, largest first, parts of
    /// at least 50ms.
    pub fn describe(&self) -> String {
        let mut parts = [
            ("shim", self.shim),
            ("build scripts", self.build_scripts),
            ("other processes", self.other_procs),
            ("cargo", self.cargo),
        ];
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        parts
            .iter()
            .filter(|p| p.1 >= 0.05)
            .map(|(name, v)| format!("{name} {v:.1}s"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Gaps {
    /// Split of `Phases::startup + Phases::lock_wait`.
    pub startup: Split,
    /// Split of `Phases::build_gaps`.
    pub build: Split,
    /// Shim seconds summed over units (job-slot time), compiler running
    /// elsewhere or not.
    pub shim_job_secs: f64,
    /// How many units carried exact shim timings.
    pub shim_timed_units: usize,
    /// `shim_job_secs` by stage, largest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shim_stages: Vec<ShimStage>,
}

impl Gaps {
    /// The report's `idle` line: who owned the no-compiler time, when it is
    /// at least 0.3s of wall time or 1s of job-slot time.
    pub fn report_line(&self, phases: &crate::summary::Phases) -> Option<String> {
        let mut parts = Vec::new();
        if phases.build_gaps >= 0.3 {
            parts.push(format!("gaps: {}", self.build.describe()));
        }
        if phases.startup + phases.lock_wait >= 0.3 {
            parts.push(format!("startup: {}", self.startup.describe()));
        }
        if self.shim_job_secs >= 1.0 {
            let top: Vec<String> = self
                .shim_stages
                .iter()
                .take(3)
                .filter(|s| s.secs >= 0.05)
                .map(|s| format!("{} {:.1}s", s.name, s.secs))
                .collect();
            parts.push(format!(
                "shim {:.1}s of job time over {} units ({})",
                self.shim_job_secs,
                self.shim_timed_units,
                top.join(", ")
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" | "))
    }
}

type Spans = Vec<(f64, f64)>;

/// Sorted, non-overlapping.
fn union(mut spans: Spans) -> Spans {
    spans.retain(|s| s.0.is_finite() && s.1.is_finite() && s.1 > s.0);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Spans = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `base` minus `cut`. Both sorted and non-overlapping.
fn subtract(base: &Spans, cut: &Spans) -> Spans {
    let mut out = Vec::new();
    let mut j = 0;
    for &(a, b) in base {
        let mut cur = a;
        while j < cut.len() && cut[j].1 <= cur {
            j += 1;
        }
        let mut k = j;
        while k < cut.len() && cut[k].0 < b {
            if cut[k].0 > cur {
                out.push((cur, cut[k].0));
            }
            cur = cur.max(cut[k].1);
            k += 1;
        }
        if cur < b {
            out.push((cur, b));
        }
    }
    out
}

fn length(spans: &Spans) -> f64 {
    spans.iter().map(|s| s.1 - s.0).sum()
}

fn clip(spans: &Spans, lo: f64, hi: f64) -> Spans {
    spans
        .iter()
        .map(|&(a, b)| (a.max(lo), b.min(hi)))
        .filter(|s| s.1 > s.0)
        .collect()
}

/// Charge `window` minus `compile` to the first of `owners` covering it.
fn split(window: (f64, f64), compile: &Spans, owners: [&Spans; 3]) -> Split {
    let mut rest = subtract(&vec![window], compile);
    let mut got = [0.0; 3];
    for (slot, owner) in got.iter_mut().zip(owners) {
        let left = subtract(&rest, owner);
        *slot = length(&rest) - length(&left);
        rest = left;
    }
    Split {
        shim: got[0],
        build_scripts: got[1],
        other_procs: got[2],
        cargo: length(&rest),
    }
    .rounded()
}

/// A build script, wrapped (`.jr-real-build_script_*`, `src/buildscript.rs`)
/// or not, by sampled name or path.
pub(crate) fn is_build_script(comm: &str, exe: &str) -> bool {
    comm.starts_with("build-script")
        || comm.starts_with("build_script")
        || comm.starts_with(".jr-real-build")
        || crate::sys::has_dir(exe, "build")
            && (exe.contains("build-script") || exe.contains("build_script"))
}

/// `start` is the recorder's start time, `startup_end` and `build_end` are
/// seconds after it (`Phases::startup + lock_wait`, and the end of the build
/// window).
pub fn attribute(
    units: &[Unit],
    procs: &[ProcRecord],
    start: f64,
    startup_end: f64,
    build_end: f64,
) -> Option<Gaps> {
    if units.is_empty() || build_end <= 0.0 {
        return None;
    }
    let rel = |t: f64| t - start;
    let compile = union(units.iter().map(|u| (rel(u.start), rel(u.end))).collect());
    let mut shim: Spans = Vec::new();
    let mut job_secs = 0.0;
    let mut timed = 0;
    let mut stages: Vec<ShimStage> = Vec::new();
    for u in units {
        let (Some(pre), post) = (u.shim_pre_secs, u.shim_post_secs.unwrap_or(0.0)) else {
            continue;
        };
        timed += 1;
        job_secs += pre + post;
        shim.push((rel(u.start) - pre, rel(u.start)));
        shim.push((rel(u.end), rel(u.end) + post));
        for s in &u.shim_stages {
            match stages.iter_mut().find(|t| t.name == s.name) {
                Some(t) => t.secs += s.secs,
                None => stages.push(s.clone()),
            }
        }
    }
    let mut scripts: Spans = Vec::new();
    let mut others: Spans = Vec::new();
    for p in procs {
        let span = (rel(p.first_seen), rel(p.last_seen));
        let exe = p.cmdline.split_whitespace().next().unwrap_or("");
        if is_build_script(&p.comm, exe) || p.kind == "build-script" {
            scripts.push(span);
        } else if p.kind == "rustc" {
            // The shim and the compiler it runs are both sampled as rustc.
            // Outside every unit interval that is the shim (older records
            // have no exact timings), or a compiler that left no unit.
            shim.push(span);
        } else if p.kind == "cargo"
            || matches!(p.comm.as_str(), "cargo" | "jr-cargo" | "justrust")
            || crate::sys::program_name(exe.as_ref()) == "justrust"
        {
            // Cargo itself and the recorder: alive for the whole run.
        } else {
            others.push(span);
        }
    }
    let (shim, scripts, others) = (union(shim), union(scripts), union(others));
    let window = |lo: f64, hi: f64| {
        split(
            (lo, hi.max(lo)),
            &clip(&compile, lo, hi),
            [
                &clip(&shim, lo, hi),
                &clip(&scripts, lo, hi),
                &clip(&others, lo, hi),
            ],
        )
    };
    stages.sort_by(|a, b| b.secs.total_cmp(&a.secs));
    for s in &mut stages {
        s.secs = (s.secs * 1000.0).round() / 1000.0;
    }
    Some(Gaps {
        startup: window(0.0, startup_end),
        build: window(startup_end, build_end),
        shim_job_secs: (job_secs * 1000.0).round() / 1000.0,
        shim_timed_units: timed,
        shim_stages: stages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(start: f64, end: f64, pre: Option<f64>, post: Option<f64>) -> Unit {
        Unit {
            start,
            end,
            wall: end - start,
            shim_pre_secs: pre,
            shim_post_secs: post,
            shim_stages: pre
                .map(|p| {
                    vec![ShimStage {
                        name: "miss-remote".into(),
                        secs: p,
                    }]
                })
                .unwrap_or_default(),
            ..Unit::default()
        }
    }

    fn proc(kind: &str, comm: &str, cmdline: &str, a: f64, b: f64) -> ProcRecord {
        ProcRecord {
            pid: 1,
            comm: comm.into(),
            cmdline: cmdline.into(),
            kind: kind.into(),
            first_seen: a,
            last_seen: b,
            cpu_secs: 0.0,
            max_rss_mb: 0.0,
        }
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn span_arithmetic() {
        let u = union(vec![(3.0, 4.0), (0.0, 1.0), (0.5, 2.0), (2.0, 2.0)]);
        assert_eq!(u, vec![(0.0, 2.0), (3.0, 4.0)]);
        let s = subtract(&vec![(0.0, 10.0)], &u);
        assert_eq!(s, vec![(2.0, 3.0), (4.0, 10.0)]);
        // A cut spanning two base spans is applied to both.
        let s = subtract(&vec![(0.0, 2.0), (3.0, 5.0)], &vec![(1.0, 4.0)]);
        assert_eq!(s, vec![(0.0, 1.0), (4.0, 5.0)]);
        assert_eq!(subtract(&vec![(0.0, 1.0)], &Vec::new()), vec![(0.0, 1.0)]);
    }

    #[test]
    fn every_gap_second_is_charged_once() {
        // Recorder starts at 100. Startup ends at 2, build ends at 12.
        // Unit A compiles 2..4 after 1s of shim work (1..2, in startup).
        // Unit B compiles 6..8 after 1.5s of shim work (4.5..6) and stores
        // for 0.5s (8..8.5). A build script runs 8.2..10 (overlapping the
        // store), a linker 10..10.5, and nothing but cargo 10.5..12 and
        // 4..4.5.
        let units = [
            unit(102.0, 104.0, Some(1.0), Some(0.0)),
            unit(106.0, 108.0, Some(1.5), Some(0.5)),
        ];
        let procs = [
            proc("cargo", "jr-cargo", "/t/bin/jr-cargo check", 100.0, 112.0),
            proc(
                "target-binary",
                ".jr-real-build_",
                "/w/target/debug/build/x-1/.jr-real-build_script_build-1",
                108.2,
                110.0,
            ),
            proc("linker", "cc", "cc -o x", 110.0, 110.5),
        ];
        let g = attribute(&units, &procs, 100.0, 2.0, 12.0).unwrap();
        close(g.startup.shim, 1.0);
        close(g.startup.cargo, 1.0);
        close(g.build.shim, 2.0);
        close(g.build.build_scripts, 1.5);
        close(g.build.other_procs, 0.5);
        close(g.build.cargo, 2.0);
        // The window is 10s, 4s of it compile.
        close(g.build.total(), 6.0);
        close(g.startup.total(), 2.0);
        close(g.shim_job_secs, 3.0);
        assert_eq!(g.shim_timed_units, 2);
        assert_eq!(g.shim_stages[0].name, "miss-remote");
        close(g.shim_stages[0].secs, 2.5);
        assert_eq!(
            g.build.describe(),
            "shim 2.0s, cargo 2.0s, build scripts 1.5s, other processes 0.5s"
        );
    }

    #[test]
    fn shim_time_under_a_running_compiler_is_job_time_not_a_gap() {
        // B's 3s of shim work is entirely while A compiles.
        let units = [
            unit(101.0, 106.0, Some(0.0), Some(0.0)),
            unit(105.0, 106.0, Some(3.0), Some(0.0)),
        ];
        let g = attribute(&units, &[], 100.0, 1.0, 6.0).unwrap();
        close(g.build.total(), 0.0);
        close(g.shim_job_secs, 3.0);
    }

    #[test]
    fn older_records_fall_back_to_sampled_rustc_processes() {
        // No exact timings: the shim process was sampled from 1 to 4, its
        // unit ran 3..4.
        let units = [unit(103.0, 104.0, None, None)];
        let procs = [proc(
            "rustc",
            "rustc",
            "/h/.justrust/bin/shim/ab/rustc --crate-name x",
            101.0,
            104.0,
        )];
        let g = attribute(&units, &procs, 100.0, 1.0, 4.0).unwrap();
        close(g.build.shim, 2.0);
        close(g.build.cargo, 0.0);
        close(g.startup.cargo, 1.0);
        assert_eq!(g.shim_timed_units, 0);
        close(g.shim_job_secs, 0.0);
    }

    #[test]
    fn no_units_no_split() {
        assert!(attribute(&[], &[], 0.0, 1.0, 2.0).is_none());
    }

    #[test]
    fn build_scripts_by_name_and_path() {
        assert!(is_build_script("build-script-bu", ""));
        assert!(is_build_script(".jr-real-build_", ""));
        assert!(is_build_script(
            "x",
            "/t/debug/build/serde-1/build_script_build-9"
        ));
        assert!(!is_build_script(
            "serde",
            "/t/debug/deps/build_script_helper-1"
        ));
        assert!(!is_build_script("rustc", "/t/bin/rustc"));
    }
}
