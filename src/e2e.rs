//! End-to-end wait attribution: everything the caller waited for, from the
//! justrust process starting to the report, split into non-overlapping
//! segments that sum to that wait.
//!
//! Cargo's own phases (`summary::Phases`) only cover cargo's lifetime on the
//! machine that ran it. A hosted build also spends time routing, being
//! admitted, syncing sources, starting on the host, and returning results;
//! a local build spends time choosing a toolchain and slot and writing the
//! report. This module records monotonic marks along that path and turns
//! them into one waterfall.
//!
//! Recording is observational only: no environment, argument, or file that
//! cargo or rustc can see changes, so artifacts and fingerprints are shared
//! with unprofiled builds.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// One non-overlapping piece of the end-to-end wait.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Segment {
    /// Stable machine-readable key, `area.what` (`client.route`, `sync.push`).
    pub key: String,
    pub secs: f64,
}

/// The whole wait, split into segments. `segments` sum to `wall`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct E2e {
    /// Seconds from the justrust process starting to the report.
    pub wall: f64,
    pub segments: Vec<Segment>,
    /// Reconstructed from older records (coarser, no client marks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
}

/// Lives with the wire protocol (`Flushed`), which is also compiled alone.
pub use crate::remote_proto::SyncTiming;

struct Clock {
    origin: Instant,
    marks: Vec<(&'static str, f64)>,
}

fn clock() -> &'static Mutex<Clock> {
    static CLOCK: OnceLock<Mutex<Clock>> = OnceLock::new();
    CLOCK.get_or_init(|| {
        Mutex::new(Clock {
            origin: Instant::now(),
            marks: Vec::new(),
        })
    })
}

/// Anchor the clock. Call first thing in `main`; later calls are no-ops.
pub fn init() {
    let _ = clock();
}

/// Record that the run reached `name` now. Repeated names keep the latest.
pub fn mark(name: &'static str) {
    if let Ok(mut c) = clock().lock() {
        let t = c.origin.elapsed().as_secs_f64();
        c.marks.retain(|(n, _)| *n != name);
        c.marks.push((name, t));
    }
}

/// Seconds since the clock was anchored.
pub fn elapsed() -> f64 {
    clock()
        .lock()
        .map(|c| c.origin.elapsed().as_secs_f64())
        .unwrap_or(0.0)
}

/// Marks so far, as seconds since the anchor.
pub fn marks() -> Marks {
    Marks(clock().lock().map(|c| c.marks.clone()).unwrap_or_default())
}

#[derive(Debug, Clone, Default)]
pub struct Marks(pub Vec<(&'static str, f64)>);

impl Marks {
    pub fn get(&self, name: &str) -> Option<f64> {
        self.0.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
    }
}

/// Builds a waterfall from consecutive boundaries. Out-of-order or missing
/// boundaries never produce negative time: each segment starts where the
/// previous ended.
struct Waterfall {
    at: f64,
    segments: Vec<Segment>,
}

impl Waterfall {
    fn new() -> Self {
        Self {
            at: 0.0,
            segments: Vec::new(),
        }
    }

    /// Everything from the current point to `until` goes to `key`.
    fn until(&mut self, key: &str, until: Option<f64>) {
        let Some(t) = until else { return };
        if t > self.at {
            self.push(key, t - self.at);
            self.at = t;
        }
    }

    fn push(&mut self, key: &str, secs: f64) {
        if secs <= 0.0 {
            return;
        }
        match self.segments.iter_mut().find(|s| s.key == key) {
            Some(s) => s.secs += secs,
            None => self.segments.push(Segment {
                key: key.to_owned(),
                secs,
            }),
        }
    }

    /// Split the span up to `until` into the given parts, scaled down if
    /// they overrun it. What is left goes to `rest`.
    fn split(&mut self, until: Option<f64>, parts: &[(&str, f64)], rest: &str) {
        let Some(t) = until else { return };
        let span = (t - self.at).max(0.0);
        let sum: f64 = parts.iter().map(|(_, s)| s.max(0.0)).sum();
        let scale = if sum > span && sum > 0.0 {
            span / sum
        } else {
            1.0
        };
        for (k, s) in parts {
            self.push(k, s.max(0.0) * scale);
        }
        self.push(rest, span - sum * scale);
        self.at = self.at.max(t);
    }

    fn finish(self, wall: f64, derived: bool) -> E2e {
        E2e {
            wall,
            segments: self.segments,
            derived,
        }
    }
}

fn phase_parts(p: &crate::summary::Phases, prefix: &str) -> Vec<(String, f64)> {
    [
        ("cargo_startup", p.startup),
        ("lock_wait", p.lock_wait),
        ("compile", p.compile),
        ("build_gaps", p.build_gaps),
        ("tests", p.test_run),
        ("tail", p.tail),
    ]
    .into_iter()
    .map(|(k, v)| (format!("{prefix}.{k}"), v))
    .collect()
}

fn split_owned(w: &mut Waterfall, until: Option<f64>, parts: &[(String, f64)], rest: &str) {
    let parts: Vec<(&str, f64)> = parts.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    w.split(until, &parts, rest);
}

/// A local build: marks from this process (`routed`, `cargo_spawn`,
/// `cargo_done` once cargo and pooled tests finished) and cargo's phases.
pub fn local(m: &Marks, now: f64, phases: &crate::summary::Phases) -> E2e {
    let mut w = Waterfall::new();
    w.until("client.route", m.get("routed"));
    w.until("client.setup", m.get("cargo_spawn"));
    split_owned(
        &mut w,
        m.get("cargo_done"),
        &phase_parts(phases, "build"),
        "build.other",
    );
    w.until("client.report", Some(now));
    w.finish(now, false)
}

/// What the remote path knows when it records a run.
#[derive(Debug, Clone, Default)]
pub struct RemoteTimes {
    pub sync: Option<SyncTiming>,
    /// Host-recorded wall and phases, from the host's summary.
    pub host_wall: f64,
    pub host_phases: crate::summary::Phases,
}

/// A hosted or remote build. Marks: `routed`, `remote_roots`, `admitted`,
/// `daemon_connected`, `flushed`, `exit`, `artifacts`, now = recording.
pub fn remote(m: &Marks, now: f64, r: &RemoteTimes) -> E2e {
    let mut w = Waterfall::new();
    w.until("client.route", m.get("routed"));
    w.until("client.roots", m.get("remote_roots"));
    w.until("remote.admission", m.get("admitted"));
    w.until("remote.daemon_connect", m.get("daemon_connected"));
    match r.sync {
        Some(s) => w.split(
            m.get("flushed"),
            &[
                ("sync.lock_wait", s.lock_ms / 1000.0),
                ("sync.link", s.link_ms / 1000.0),
                ("sync.push", s.push_ms / 1000.0),
                ("sync.barrier", s.barrier_ms / 1000.0),
            ],
            "sync.other",
        ),
        None => w.until("sync.total", m.get("flushed")),
    }
    let mut host = phase_parts(&r.host_phases, "host");
    let phase_sum: f64 = host.iter().map(|(_, v)| v).sum();
    // Phases sum to the host wall; anything recorded beyond them is "other".
    host.push(("host.other".into(), (r.host_wall - phase_sum).max(0.0)));
    split_owned(&mut w, m.get("exit"), &host, "remote.host_overhead");
    w.until("remote.artifacts", m.get("artifacts"));
    w.until("client.report", Some(now));
    w.finish(now, false)
}

/// Cargo phases for a reconstruction. Older recorders lost the test phase
/// when the runner ran a binary itself after a failed pool hand-off, so its
/// time landed in `tail`. libtest's own reported time moves back, bounded
/// by the tail (pooled binaries overlap, so their sum can exceed wall).
fn corrected_phases(s: &crate::summary::Summary) -> crate::summary::Phases {
    let mut p = s.phases.clone();
    let missing = (s.tests.libtest_secs - p.test_run).max(0.0).min(p.tail);
    p.test_run += missing;
    p.tail -= missing;
    p
}

/// Coarse reconstruction for runs recorded before marks existed. Remote:
/// the client wall, its sync, and the host phases. Local: cargo's phases
/// only (the client's own time before and after cargo was not recorded).
pub fn derive(s: &crate::summary::Summary) -> Option<E2e> {
    let phases = corrected_phases(s);
    let Some(r) = s.remote.as_ref().filter(|r| r.client_wall > 0.0) else {
        if s.remote.is_some() || s.wall <= 0.0 {
            return None;
        }
        let mut w = Waterfall::new();
        split_owned(
            &mut w,
            Some(s.wall),
            &phase_parts(&phases, "build"),
            "build.other",
        );
        return Some(w.finish(s.wall, true));
    };
    let mut w = Waterfall::new();
    w.push("sync.total", (r.sync_ms / 1000.0).min(r.client_wall));
    w.at = (r.sync_ms / 1000.0).min(r.client_wall);
    let mut host = phase_parts(&phases, "host");
    let phase_sum: f64 = host.iter().map(|(_, v)| v).sum();
    host.push(("host.other".into(), (s.wall - phase_sum).max(0.0)));
    split_owned(&mut w, Some(r.client_wall), &host, "remote.other");
    Some(w.finish(r.client_wall, true))
}

/// Human label for a segment key.
pub fn label(key: &str) -> &str {
    match key {
        "client.route" => "justrust start, routing",
        "client.roots" => "find source roots",
        "client.setup" => "toolchain, slot, setup",
        "client.report" => "collect, report",
        "remote.admission" => "hosted admission",
        "remote.daemon_connect" => "sync daemon connect",
        "remote.host_overhead" => "host start, results back",
        "remote.artifacts" => "artifacts back",
        "remote.other" => "not attributed",
        "sync.lock_wait" => "sync: queued behind",
        "sync.link" => "sync: machine link",
        "sync.push" => "sync: source push",
        "sync.barrier" => "sync: confirm",
        "sync.other" => "sync: relay",
        "sync.total" => "sync (total)",
        "build.cargo_startup" | "host.cargo_startup" => "cargo startup",
        "build.lock_wait" | "host.lock_wait" => "build dir lock",
        "build.compile" | "host.compile" => "compiling",
        "build.build_gaps" | "host.build_gaps" => "build gaps",
        "build.tests" | "host.tests" => "running tests",
        "build.tail" | "host.tail" => "after build",
        "build.other" | "host.other" => "other",
        k => k,
    }
}

/// Section heading for a key's area.
pub fn area(key: &str) -> &'static str {
    match key.split('.').next() {
        Some("client") => "client",
        Some("remote") => "remote",
        Some("sync") => "sync",
        Some("build") => "build",
        Some("host") => "host",
        _ => "other",
    }
}

pub fn render(e: &E2e) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "End to end: {:.2}s waited{}",
        e.wall,
        if e.derived {
            " (reconstructed from an older record, coarser)"
        } else {
            ""
        }
    );
    for s in e.segments.iter().filter(|s| s.secs >= 0.005) {
        let pct = if e.wall > 0.0 {
            100.0 * s.secs / e.wall
        } else {
            0.0
        };
        let bar_n = if e.wall > 0.0 {
            ((s.secs / e.wall) * 24.0).round() as usize
        } else {
            0
        };
        let _ = writeln!(
            o,
            "  {:<6} {:<26} {:>7.2}s {:>4.0}%  {}",
            area(&s.key),
            label(&s.key),
            s.secs,
            pct,
            "█".repeat(bar_n.min(24))
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::Phases;

    fn sum(e: &E2e) -> f64 {
        e.segments.iter().map(|s| s.secs).sum()
    }

    fn get(e: &E2e, k: &str) -> f64 {
        e.segments
            .iter()
            .find(|s| s.key == k)
            .map_or(0.0, |s| s.secs)
    }

    #[test]
    fn local_waterfall_sums_to_wall() {
        let m = Marks(vec![
            ("routed", 0.05),
            ("cargo_spawn", 0.20),
            ("cargo_done", 3.6),
        ]);
        let p = Phases {
            startup: 0.3,
            compile: 2.0,
            test_run: 1.0,
            ..Default::default()
        };
        let e = local(&m, 3.8, &p);
        assert!((sum(&e) - 3.8).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "client.route") - 0.05).abs() < 1e-9);
        assert!((get(&e, "client.setup") - 0.15).abs() < 1e-9);
        assert!((get(&e, "build.compile") - 2.0).abs() < 1e-9);
        assert!((get(&e, "build.other") - 0.1).abs() < 1e-9);
        assert!((get(&e, "client.report") - 0.2).abs() < 1e-9);
    }

    #[test]
    fn remote_waterfall_splits_sync_and_host() {
        let m = Marks(vec![
            ("routed", 0.1),
            ("remote_roots", 0.12),
            ("admitted", 0.5),
            ("daemon_connected", 0.52),
            ("flushed", 3.0),
            ("exit", 10.0),
            ("artifacts", 10.5),
        ]);
        let r = RemoteTimes {
            sync: Some(SyncTiming {
                lock_ms: 2000.0,
                link_ms: 0.0,
                push_ms: 300.0,
                barrier_ms: 70.0,
            }),
            host_wall: 6.0,
            host_phases: Phases {
                startup: 0.5,
                compile: 5.0,
                tail: 0.5,
                ..Default::default()
            },
        };
        let e = remote(&m, 10.6, &r);
        assert!((sum(&e) - 10.6).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "sync.lock_wait") - 2.0).abs() < 1e-9);
        assert!((get(&e, "sync.other") - 0.11).abs() < 1e-9);
        assert!((get(&e, "host.compile") - 5.0).abs() < 1e-9);
        assert!((get(&e, "remote.host_overhead") - 1.0).abs() < 1e-9);
        assert!((get(&e, "remote.artifacts") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn overrunning_parts_scale_instead_of_going_negative() {
        // Pipelined runs can start on the host before the client hears the
        // sync finished: the host wall may exceed what the client saw.
        let m = Marks(vec![("flushed", 1.0), ("exit", 3.0)]);
        let r = RemoteTimes {
            host_wall: 4.0,
            host_phases: Phases {
                compile: 4.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let e = remote(&m, 3.0, &r);
        assert!((sum(&e) - 3.0).abs() < 1e-9, "{e:?}");
        assert!(e.segments.iter().all(|s| s.secs >= 0.0));
        assert!((get(&e, "host.compile") - 2.0).abs() < 1e-9);
    }

    #[test]
    fn every_mark_permutation_sums_to_wall_without_negatives() {
        let names = [
            "routed",
            "remote_roots",
            "admitted",
            "daemon_connected",
            "flushed",
            "exit",
            "artifacts",
            "cargo_spawn",
            "cargo_done",
        ];
        let r = RemoteTimes {
            sync: Some(SyncTiming {
                lock_ms: 900.0,
                link_ms: 50.0,
                push_ms: 300.0,
                barrier_ms: 60.0,
            }),
            host_wall: 5.0,
            host_phases: Phases {
                compile: 4.0,
                test_run: 1.0,
                ..Default::default()
            },
        };
        // Pseudo-random times, some marks absent, in arbitrary order.
        let mut seed = 7u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        for _ in 0..500 {
            let mut v = Vec::new();
            for n in names {
                if next() % 8 != 0 {
                    v.push((n, (next() % 8000) as f64 / 1000.0));
                }
            }
            let now = 8.5;
            let e = remote(&Marks(v.clone()), now, &r);
            assert!((sum(&e) - now).abs() < 1e-9, "{v:?} {e:?}");
            assert!(e.segments.iter().all(|s| s.secs >= 0.0), "{e:?}");
            let l = local(&Marks(v), now, &r.host_phases);
            assert!((sum(&l) - now).abs() < 1e-9, "{l:?}");
            assert!(l.segments.iter().all(|s| s.secs >= 0.0), "{l:?}");
        }
    }

    #[test]
    fn missing_and_out_of_order_marks_never_lose_time() {
        let m = Marks(vec![("routed", 0.5), ("admitted", 0.2), ("exit", 2.0)]);
        let e = remote(&m, 2.5, &RemoteTimes::default());
        assert!((sum(&e) - 2.5).abs() < 1e-9, "{e:?}");
        assert!(e.segments.iter().all(|s| s.secs > 0.0));
    }

    #[test]
    fn derives_coarse_waterfall_for_old_remote_runs() {
        let s = crate::summary::Summary {
            wall: 38.45,
            phases: Phases {
                compile: 9.83,
                test_run: 28.5,
                startup: 0.12,
                ..Default::default()
            },
            remote: Some(crate::summary::RemoteInfo {
                machine: "hosted".into(),
                sync_ms: 78.3,
                client_wall: 39.6,
                ..Default::default()
            }),
            ..Default::default()
        };
        let e = derive(&s).unwrap();
        assert!(e.derived);
        assert!((sum(&e) - 39.6).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "sync.total") - 0.0783).abs() < 1e-9);
        assert!(get(&e, "remote.other") > 1.0);
    }

    #[test]
    fn derives_local_runs_from_cargo_phases() {
        let s = crate::summary::Summary {
            wall: 3.3,
            phases: Phases {
                startup: 0.3,
                compile: 3.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let e = derive(&s).unwrap();
        assert!((sum(&e) - 3.3).abs() < 1e-9, "{e:?}");
        assert_eq!(e.segments[0].key, "build.cargo_startup");
    }

    #[test]
    fn derive_moves_lost_libtest_time_out_of_the_tail() {
        let mut s = crate::summary::Summary {
            wall: 569.0,
            phases: Phases {
                compile: 0.7,
                tail: 567.1,
                ..Default::default()
            },
            ..Default::default()
        };
        s.tests.libtest_secs = 567.06;
        let e = derive(&s).unwrap();
        assert!((get(&e, "build.tests") - 567.06).abs() < 1e-6, "{e:?}");
        assert!(get(&e, "build.tail") < 0.1, "{e:?}");
        assert!((sum(&e) - 569.0).abs() < 1e-6);
    }
}
