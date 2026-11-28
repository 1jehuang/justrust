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
    /// Measured intervals on the client monotonic clock, not reconstructed phase totals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<(f64, f64, String)>,
    /// Reconstructed from older records (coarser, no client marks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
    /// Where a segment's time went, when its owner measured that. Parts lie
    /// inside their segment and are not additional wait: they never sum to
    /// more than it, and what they leave is the segment's unmeasured rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    /// Raw artifact observations. Host pack may overlap client work, and
    /// receive contains wire_read + unpack_exposed. Not additive waterfall parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_times: Option<ArtifactTimes>,
}

/// A measured piece of one segment.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Part {
    /// Key of the segment this lies inside (`sync.link`).
    pub of: String,
    /// What was measured (`ready`, `receive`).
    pub what: String,
    pub secs: f64,
    /// Payload bytes that crossed the link during this part, when counted.
    #[serde(default, skip_serializing_if = "is_zero_bytes")]
    pub bytes: u64,
}

fn is_zero_bytes(n: &u64) -> bool {
    *n == 0
}

/// Lives with the wire protocol (`Flushed`), which is also compiled alone.
pub use crate::remote_proto::SyncTiming;

struct Clock {
    origin: Instant,
    marks: Vec<(&'static str, f64)>,
    notes: Vec<(&'static str, f64)>,
}

fn clock() -> &'static Mutex<Clock> {
    static CLOCK: OnceLock<Mutex<Clock>> = OnceLock::new();
    CLOCK.get_or_init(|| {
        Mutex::new(Clock {
            origin: Instant::now(),
            marks: Vec::new(),
            notes: Vec::new(),
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

/// Add `secs` to a duration measured by whoever owns that part of the wait
/// (the sync daemon, the artifact stream). Repeated names accumulate.
pub fn note(name: &'static str, secs: f64) {
    if !secs.is_finite() || secs <= 0.0 {
        return;
    }
    if let Ok(mut c) = clock().lock() {
        match c.notes.iter_mut().find(|(n, _)| *n == name) {
            Some((_, total)) => *total += secs,
            None => c.notes.push((name, secs)),
        }
    }
}

/// Total noted under `name` so far, in seconds.
pub fn noted(name: &str) -> f64 {
    clock()
        .lock()
        .ok()
        .and_then(|c| c.notes.iter().find(|(n, _)| *n == name).map(|(_, s)| *s))
        .unwrap_or(0.0)
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
    parts: Vec<Part>,
}

impl Waterfall {
    fn new() -> Self {
        Self {
            at: 0.0,
            segments: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// Record where `of`'s time went. Parts are scaled down together if
    /// they claim more than the segment holds, so they stay inside it.
    fn parts(&mut self, of: &str, parts: &[(&str, f64)]) {
        let Some(total) = self.segments.iter().find(|s| s.key == of).map(|s| s.secs) else {
            return;
        };
        let clean = |s: f64| if s.is_finite() { s.max(0.0) } else { 0.0 };
        let sum: f64 = parts.iter().map(|(_, s)| clean(*s)).sum();
        let scale = if sum > total { total / sum } else { 1.0 };
        for (what, secs) in parts {
            let secs = clean(*secs) * scale;
            if secs > 0.0 {
                self.parts.push(Part {
                    of: of.to_owned(),
                    what: (*what).to_owned(),
                    secs,
                    bytes: 0,
                });
            }
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
            trace: Vec::new(),
            derived,
            parts: self.parts,
            artifact_times: None,
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
    // Selecting and leasing the compiler: waits for a toolchain lifecycle
    // change on this machine, or installs the pinned compiler.
    w.until("client.toolchain", m.get("toolchain"));
    w.until("client.setup", m.get("cargo_spawn"));
    split_owned(
        &mut w,
        m.get("cargo_done"),
        &phase_parts(phases, "build"),
        "build.other",
    );
    w.until("client.report", Some(now));
    let mut e = w.finish(now, false);
    let baseline = m.get("record_baseline");
    let done = m.get("cargo_done");
    e.interval("client.setup", Some(0.), baseline, now);
    e.interval("build.window", baseline, done, now);
    e.interval("client.report", done, Some(now), now);
    e
}

impl E2e {
    fn interval(&mut self, key: &str, start: Option<f64>, end: Option<f64>, wall: f64) {
        if let (Some(lo), Some(hi)) = (start, end)
            && 0. <= lo
            && lo < hi
            && hi <= wall
        {
            self.trace.push((lo, hi, key.into()));
        }
    }
}

/// What the remote path knows when it records a run.
#[derive(Debug, Clone, Default)]
pub struct RemoteTimes {
    pub sync: Option<SyncTiming>,
    pub run: Option<crate::remote_proto::RunTiming>,
    /// Host-recorded wall and phases, from the host's summary.
    pub host_wall: f64,
    /// Identical-request wait inside slots::acquire, before record_baseline.
    pub slot_wait: f64,
    /// Seconds the build daemon held the request back for a prebuild of its
    /// units (`Summary::prebuild`), before the host recorder started.
    pub prebuild_wait: f64,
    /// What the host's own recorder measured around cargo.
    pub host_own: HostOwn,
    pub host_phases: crate::summary::Phases,
    /// Seconds between flush and exit that the daemon's transport to the
    /// machine was down and being reattached (the build kept running).
    pub link_outage: f64,
    /// Where bringing artifacts back went, as the client measured it.
    pub artifacts: ArtifactTimes,
}

/// The artifact return as the client's stream reader saw it. Seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtifactTimes {
    /// Request sent until the machine answered: connection and its start.
    pub connect: f64,
    /// Client wall waiting for frame headers, not total host encoding work.
    pub header_wait: f64,
    /// Receiving and decoding payload bytes (they overlap: decode is streamed).
    pub receive: f64,
    /// Sum of host preparation times of completed, spooled payloads.
    pub pack: Option<f64>,
    /// Snapshot and encoder startup through first output, not full encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_startup: Option<f64>,
    #[serde(skip_serializing_if = "is_zero_bytes")]
    pub streamed_full_files: u64,
    /// Wall spent reading compressed payload from the client stream.
    pub wire_read: f64,
    /// Remaining receive wall outside stream reads, excluding verification.
    pub unpack_exposed: f64,
    /// Checking each decoded file and moving it into place.
    pub verify: f64,
    /// Encoded payload bytes received (what crossed the link).
    pub bytes: u64,
}

impl ArtifactTimes {
    /// What this process noted while fetching (`artifacts.*`).
    pub fn noted() -> Self {
        Self {
            connect: noted("artifacts.connect"),
            header_wait: noted("artifacts.encode"),
            receive: noted("artifacts.receive"),
            pack: Some(noted("artifacts.pack")).filter(|s| *s > 0.),
            stream_startup: Some(noted("artifacts.stream_startup")).filter(|s| *s > 0.),
            streamed_full_files: noted("artifacts.streamed_full_files") as u64,
            wire_read: noted("artifacts.wire_read"),
            unpack_exposed: noted("artifacts.unpack_exposed"),
            verify: noted("artifacts.verify"),
            bytes: noted("artifacts.bytes") as u64,
        }
    }
}

/// The host recorder's time outside cargo, from the waterfall it wrote for
/// its own process (`local`). All of it lies inside the agent's execute time
/// and outside the host wall.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HostOwn {
    /// Selecting and leasing the compiler.
    pub toolchain: f64,
    /// Routing, shim, scope and slot, including any identical-build wait.
    pub setup: f64,
    /// After cargo: collecting and writing the run.
    pub report: f64,
}

impl HostOwn {
    /// From the summary the host wrote, before its waterfall is replaced by
    /// the end-to-end one.
    pub fn of(host: Option<&E2e>) -> Self {
        let Some(e) = host.filter(|e| !e.derived) else {
            return Self::default();
        };
        let seg = |k: &str| {
            e.segments
                .iter()
                .filter(|s| s.key == k && s.secs.is_finite() && s.secs > 0.0)
                .map(|s| s.secs)
                .sum::<f64>()
        };
        Self {
            toolchain: seg("client.toolchain"),
            setup: seg("client.route") + seg("client.setup"),
            report: seg("client.report"),
        }
    }
}

/// A hosted or remote build. Marks: `routed`, `remote_roots`, `admitted`,
/// `daemon_connected`, `flushed`, `exit`, `artifacts`, now = recording.
pub fn remote(m: &Marks, now: f64, r: &RemoteTimes) -> E2e {
    let mut w = Waterfall::new();
    if let Some(preflight) = m.get("remote_preflight") {
        w.until("client.route", Some(preflight));
        w.until("remote.preflight", m.get("routed"));
    } else {
        w.until("client.route", m.get("routed"));
    }
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
    if let Some(s) = r.sync {
        w.parts(
            "sync.link",
            &[
                ("ready", s.link_ready_ms / 1000.0),
                ("install", s.link_install_ms / 1000.0),
                ("session", s.link_session_ms / 1000.0),
            ],
        );
    }
    let mut host = phase_parts(&r.host_phases, "host");
    let phase_sum: f64 = host.iter().map(|(_, v)| v).sum();
    // Phases sum to the host wall; anything recorded beyond them is "other".
    host.push(("host.other".into(), (r.host_wall - phase_sum).max(0.0)));
    if let Some(t) = r.run.filter(valid_run_timing) {
        host.insert(0, ("host.prepare".into(), t.prepare_ms / 1000.0));
        let residual = (t.execute_ms / 1000.0 - r.host_wall).max(0.0);
        let wait = if r.slot_wait.is_finite() {
            r.slot_wait.clamp(0.0, residual)
        } else {
            0.0
        };
        host.insert(1, ("host.slot_wait".into(), wait));
        // The daemon waited before it started the host recorder, so this is
        // in nobody's phases: it comes out of the same unmarked remainder.
        let prebuild_wait = if r.prebuild_wait.is_finite() {
            r.prebuild_wait.clamp(0.0, residual - wait)
        } else {
            0.0
        };
        host.insert(2, ("host.prebuild_wait".into(), prebuild_wait));
        // The host recorder measured its own time around cargo. It never
        // exceeds what the agent saw: scale it into what is left.
        let own = r.host_own;
        let setup = (own.setup - wait).max(0.0);
        let left = residual - wait - prebuild_wait;
        let known = own.toolchain + setup + own.report;
        let scale = if known > left && known > 0.0 {
            left / known
        } else {
            1.0
        };
        host.insert(3, ("host.toolchain".into(), own.toolchain * scale));
        host.insert(4, ("host.setup".into(), setup * scale));
        host.push(("host.report".into(), own.report * scale));
        // This remainder has no start/end marks of its own. Keep the historical
        // key for saved profiles, but do not attribute it to recorder work.
        host.push(("host.recorder".into(), (left - known * scale).max(0.0)));
        host.push(("host.finalize".into(), t.finalize_ms / 1000.0));
        host.push(("host.records".into(), t.records_ms / 1000.0));
    }
    // An outage is time the client waited with the build still running on
    // the host: it overlaps host work that finished meanwhile. Only what the
    // host's own measurements leave unexplained can be the outage.
    if let Some(exit) = m.get("exit") {
        let known: f64 = host.iter().map(|(_, v)| v.max(0.0)).sum();
        let unexplained = (exit - w.at - known).max(0.0);
        let outage = if r.link_outage.is_finite() {
            r.link_outage.clamp(0.0, unexplained)
        } else {
            0.0
        };
        host.push(("remote.link_outage".into(), outage));
    }
    split_owned(&mut w, m.get("exit"), &host, "remote.host_overhead");
    w.until("remote.artifacts", m.get("artifacts"));
    let split_stream = r.artifacts.wire_read > 0. || r.artifacts.unpack_exposed > 0.;
    w.parts(
        "remote.artifacts",
        &[
            ("connect", r.artifacts.connect),
            ("header_wait", r.artifacts.header_wait),
            (
                "receive",
                if split_stream {
                    0.
                } else {
                    r.artifacts.receive
                },
            ),
            ("wire_read", r.artifacts.wire_read),
            ("unpack_exposed", r.artifacts.unpack_exposed),
            ("verify", r.artifacts.verify),
        ],
    );
    if let Some(p) = w
        .parts
        .iter_mut()
        .find(|p| p.of == "remote.artifacts" && matches!(p.what.as_str(), "receive" | "wire_read"))
    {
        p.bytes = r.artifacts.bytes;
    }
    w.until("client.report", Some(now));
    let mut e = w.finish(now, false);
    if r.artifacts != ArtifactTimes::default() {
        e.artifact_times = Some(r.artifacts);
    }
    let mut start = Some(0.);
    for (key, end) in [
        ("client.route", m.get("routed")),
        ("client.roots", m.get("remote_roots")),
        ("remote.admission", m.get("admitted")),
        ("remote.daemon_connect", m.get("daemon_connected")),
        ("sync.total", m.get("flushed")),
    ] {
        e.interval(key, start, end, now);
        start = end;
    }
    // The execution host has a separate clock. Do not place its phases
    // between flush and exit by assuming a one-way network latency.
    e.interval("remote.artifacts", m.get("exit"), m.get("artifacts"), now);
    e.interval("client.report", m.get("artifacts"), Some(now), now);
    e
}

fn valid_run_timing(t: &crate::remote_proto::RunTiming) -> bool {
    [t.prepare_ms, t.execute_ms, t.finalize_ms, t.records_ms]
        .into_iter()
        .all(|ms| ms.is_finite() && ms >= 0.0)
}

/// Cargo phases for a reconstruction. Older recorders lost the test phase
/// when the runner ran a binary itself after a failed pool hand-off, so its
/// time landed in `tail`. libtest's own reported time moves back, bounded
/// by the tail (pooled binaries overlap, so their sum can exceed wall).
fn corrected_phases(s: &crate::summary::Summary) -> crate::summary::Phases {
    let mut p = s.phases.clone();
    // Recorders with coverage metadata already measured a union of test
    // intervals. A concurrent suite's summed libtest durations are not wall.
    if s.measurement_coverage.is_some() {
        return p;
    }
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
        "client.toolchain" | "host.toolchain" => "compiler selection or install",
        "client.setup" => "slot, shim, setup",
        "host.setup" => "host slot, shim, setup",
        "host.report" => "host collect, report",
        "client.report" => "collect, report",
        "remote.admission" => "hosted admission",
        "remote.preflight" => "hosted maintenance check",
        "remote.daemon_connect" => "sync daemon connect",
        "remote.host_overhead" => "host start, results back",
        "remote.link_outage" => "connection lost, reattach",
        "host.prepare" => "agent preflight, protection",
        "host.recorder" => "unattributed host execution",
        "host.slot_wait" => "wait for identical build",
        "host.prebuild_wait" => "wait for prebuild of these units",
        "host.finalize" => "activity finalization, GC",
        "host.records" => "pack, send run records",
        "remote.artifacts" => "artifacts back",
        "remote.other" => "not attributed",
        "sync.lock_wait" => "sync: queued behind",
        "sync.link" => "sync: machine link",
        "sync.push" => "sync: source push",
        "sync.barrier" => "sync: confirm",
        "sync.other" => "sync: relay",
        "sync.total" => "sync (total)",
        "build.cargo_startup" | "host.cargo_startup" => "cargo startup",
        "build.lock_wait" | "host.lock_wait" => "Cargo lock estimate",
        "build.compile" | "host.compile" => "compiling",
        "build.build_gaps" | "host.build_gaps" => "build gaps",
        "build.tests" | "host.tests" => "running tests",
        "build.tail" | "host.tail" => "after build",
        "build.other" | "host.other" => "other",
        k => k,
    }
}

/// Human label for a measured part of segment `of`.
pub fn part_label<'a>(of: &str, what: &'a str) -> &'a str {
    match (of, what) {
        ("sync.link", "ready") => "machine ready (lease, ssh)",
        ("sync.link", "install") => "host client, mirror path",
        ("sync.link", "session") => "session start, handshake",
        ("remote.artifacts", "connect") => "request, host answers",
        ("remote.artifacts", "encode" | "header_wait") => "wait for frame header",
        ("remote.artifacts", "wire_read") => "payload stream reads",
        ("remote.artifacts", "unpack_exposed") => "unpack outside reads",
        ("remote.artifacts", "receive") => "download, decode",
        ("remote.artifacts", "verify") => "verify, move into place",
        (_, what) => what,
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
        for p in e.parts.iter().filter(|p| p.of == s.key && p.secs >= 0.005) {
            // Stream read time excludes concurrent decode and buffered arrivals,
            // so bytes per read second is not a network throughput measurement.
            let size = if p.what == "wire_read" && p.bytes > 0 {
                format!("  {:.1} MB payload", p.bytes as f64 / 1e6)
            } else if p.bytes > 0 {
                let mb = p.bytes as f64 / 1e6;
                format!("  {mb:.1} MB, {:.1} MB/s", mb / p.secs)
            } else {
                String::new()
            };
            let _ = writeln!(
                o,
                "  {:<6}   {:<24} {:>7.2}s{size}",
                "",
                part_label(&p.of, &p.what),
                p.secs
            );
        }
    }
    if let Some(pack) = e.artifact_times.and_then(|a| a.pack) {
        let _ = writeln!(
            o,
            "  Host pack (spooled files): {pack:.3}s, may overlap client wait; not added to wall"
        );
    }
    if let Some(a) = e.artifact_times
        && a.streamed_full_files > 0
    {
        let _ = write!(o, "  Streamed full artifacts: {}", a.streamed_full_files);
        if let Some(s) = a.stream_startup {
            let _ = write!(
                o,
                ", host time to first payload: {s:.3}s (sum, not added to wall)"
            );
        }
        o.push('\n');
    }
    o
}

#[cfg(test)]
mod tests {
    #[test]
    fn aggregate_lock_segments_do_not_claim_target_directory_attribution() {
        for key in ["build.lock_wait", "host.lock_wait"] {
            assert_eq!(super::label(key), "Cargo lock estimate");
        }
    }
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
            ("record_baseline", 0.18),
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
        assert_eq!(
            e.trace,
            vec![
                (0., 0.18, "client.setup".into()),
                (0.18, 3.6, "build.window".into()),
                (3.6, 3.8, "client.report".into())
            ]
        );
        assert!((sum(&e) - 3.8).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "client.route") - 0.05).abs() < 1e-9);
        assert!((get(&e, "client.setup") - 0.15).abs() < 1e-9);
        assert!((get(&e, "build.compile") - 2.0).abs() < 1e-9);
        assert!((get(&e, "build.other") - 0.1).abs() < 1e-9);
        assert!((get(&e, "client.report") - 0.2).abs() < 1e-9);
    }

    #[test]
    fn a_reattached_transport_drop_is_its_own_segment_and_never_takes_host_time() {
        let m = Marks(vec![("flushed", 1.0), ("exit", 40.0)]);
        let mut r = RemoteTimes {
            host_wall: 18.0,
            host_phases: Phases {
                compile: 18.0,
                ..Default::default()
            },
            link_outage: 20.0,
            ..Default::default()
        };
        let e = remote(&m, 40.0, &r);
        assert!((sum(&e) - 40.0).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "host.compile") - 18.0).abs() < 1e-9);
        assert!((get(&e, "remote.link_outage") - 20.0).abs() < 1e-9);
        assert!((get(&e, "remote.host_overhead") - 1.0).abs() < 1e-9);
        // The build ran through most of the drop: only what the host cannot
        // explain is the outage.
        r.link_outage = 30.0;
        let e = remote(&m, 40.0, &r);
        assert!((get(&e, "host.compile") - 18.0).abs() < 1e-9);
        assert!((get(&e, "remote.link_outage") - 21.0).abs() < 1e-9);
        assert_eq!(get(&e, "remote.host_overhead"), 0.0);
        r.link_outage = f64::NAN;
        assert_eq!(get(&remote(&m, 40.0, &r), "remote.link_outage"), 0.0);
    }

    #[test]
    fn parts_stay_inside_their_segment_and_leave_the_sum_alone() {
        let m = Marks(vec![("flushed", 4.0), ("exit", 5.0), ("artifacts", 9.0)]);
        let r = RemoteTimes {
            sync: Some(SyncTiming {
                link_ms: 3000.0,
                push_ms: 1000.0,
                link_ready_ms: 2000.0,
                link_install_ms: 300.0,
                link_session_ms: 500.0,
                ..Default::default()
            }),
            artifacts: ArtifactTimes {
                connect: 1.0,
                header_wait: 2.0,
                receive: 4.0,
                verify: 1.0,
                bytes: 6_000_000,
                ..Default::default()
            },
            ..Default::default()
        };
        let e = remote(&m, 9.0, &r);
        assert!((sum(&e) - 9.0).abs() < 1e-9, "{e:?}");
        let part = |of: &str, what: &str| {
            e.parts
                .iter()
                .find(|p| p.of == of && p.what == what)
                .map_or(0.0, |p| p.secs)
        };
        assert!((get(&e, "sync.link") - 3.0).abs() < 1e-9);
        assert!((part("sync.link", "ready") - 2.0).abs() < 1e-9);
        assert!((part("sync.link", "session") - 0.5).abs() < 1e-9);
        // 8s claimed inside a 4s segment: scaled together, never beyond it.
        assert!((get(&e, "remote.artifacts") - 4.0).abs() < 1e-9);
        assert!((part("remote.artifacts", "receive") - 2.0).abs() < 1e-9);
        let inside: f64 = e
            .parts
            .iter()
            .filter(|p| p.of == "remote.artifacts")
            .map(|p| p.secs)
            .sum();
        assert!((inside - 4.0).abs() < 1e-9);
        let shown = render(&e);
        assert!(shown.contains("machine ready (lease, ssh)"), "{shown}");
        assert!(shown.contains("download, decode"), "{shown}");
        // 6 MB in the 2s the scaled part holds.
        assert!(shown.contains("6.0 MB, 3.0 MB/s"), "{shown}");
        // Old records and runs that measured nothing carry no parts.
        let plain = remote(&m, 9.0, &RemoteTimes::default());
        assert!(plain.parts.is_empty());
        assert!(!serde_json::to_string(&plain).unwrap().contains("parts"));
    }

    #[test]
    fn artifact_observations_round_trip_without_adding_overlapping_work() {
        let marks = Marks(vec![("exit", 1.), ("artifacts", 5.)]);
        let raw = ArtifactTimes {
            header_wait: 0.5,
            pack: Some(3.75),
            stream_startup: Some(0.125),
            streamed_full_files: 2,
            receive: 3.,
            wire_read: 2.,
            unpack_exposed: 1.,
            verify: 0.5,
            bytes: 2_000_000,
            ..Default::default()
        };
        let e = remote(
            &marks,
            5.,
            &RemoteTimes {
                artifacts: raw,
                ..Default::default()
            },
        );
        let saved = serde_json::to_string(&e).unwrap();
        let restored: E2e = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.artifact_times, Some(raw));
        assert_eq!(
            restored
                .parts
                .iter()
                .filter(|p| p.of == "remote.artifacts")
                .map(|p| p.secs)
                .sum::<f64>(),
            4.
        );
        let shown = render(&restored);
        for label in [
            "wait for frame header",
            "payload stream reads",
            "unpack outside reads",
            "2.0 MB payload",
            "3.750s, may overlap",
            "Streamed full artifacts: 2, host time to first payload: 0.125s",
        ] {
            assert!(shown.contains(label), "{shown}");
        }
        assert!(!shown.contains("download, decode") && !shown.contains("MB/s"));
        let old: E2e = serde_json::from_str(r#"{"wall":1,"segments":[{"key":"remote.artifacts","secs":1}],"parts":[{"of":"remote.artifacts","what":"encode","secs":1}]}"#).unwrap();
        assert!(old.artifact_times.is_none());
        assert!(render(&old).contains("wait for frame header"));
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
            run: None,
            sync: Some(SyncTiming {
                lock_ms: 2000.0,
                link_ms: 0.0,
                push_ms: 300.0,
                barrier_ms: 70.0,
                ..Default::default()
            }),
            host_wall: 6.0,
            slot_wait: 0.0,
            host_own: HostOwn::default(),
            host_phases: Phases {
                startup: 0.5,
                compile: 5.0,
                tail: 0.5,
                ..Default::default()
            },
            ..Default::default()
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
    fn host_agent_timings_explain_recorder_and_maintenance_without_double_counting() {
        let m = Marks(vec![("flushed", 1.0), ("exit", 11.0)]);
        let r = RemoteTimes {
            run: Some(crate::remote_proto::RunTiming {
                prepare_ms: 200.0,
                execute_ms: 6500.0,
                finalize_ms: 400.0,
                records_ms: 300.0,
            }),
            host_wall: 5.0,
            host_phases: Phases {
                compile: 5.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let e = remote(&m, 11.0, &r);
        assert!((sum(&e) - 11.0).abs() < 1e-9, "{e:?}");
        for (key, expected) in [
            ("host.prepare", 0.2),
            ("host.compile", 5.0),
            ("host.recorder", 1.5),
            ("host.finalize", 0.4),
            ("host.records", 0.3),
            ("remote.host_overhead", 2.6),
        ] {
            assert!((get(&e, key) - expected).abs() < 1e-9, "{key}: {e:?}");
        }
    }

    #[test]
    fn host_recorder_time_around_cargo_is_named_not_left_unattributed() {
        // Replay of 20261125-234725124-1994670-pre: the build waited 11.59s
        // for a toolchain promotion on the host before selecting its compiler.
        let marks = Marks(vec![("flushed", 0.2), ("exit", 11.85)]);
        let host = local(
            &Marks(vec![
                ("routed", 0.004),
                ("toolchain", 11.574),
                ("record_baseline", 11.58),
                ("cargo_spawn", 11.582),
                ("cargo_done", 11.592),
            ]),
            11.598,
            &Phases {
                startup: 0.009,
                ..Default::default()
            },
        );
        assert!((get(&host, "client.toolchain") - 11.57).abs() < 1e-9);
        assert!((get(&host, "client.setup") - 0.008).abs() < 1e-9);
        assert!((sum(&host) - 11.598).abs() < 1e-9);
        let mut times = RemoteTimes {
            host_wall: 0.012,
            host_own: HostOwn::of(Some(&host)),
            run: Some(crate::remote_proto::RunTiming {
                execute_ms: 11600.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        let e = remote(&marks, 11.86, &times);
        assert!((get(&e, "host.toolchain") - 11.57).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "host.setup") - 0.012).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "host.report") - 0.006).abs() < 1e-9, "{e:?}");
        assert!(get(&e, "host.recorder") < 0.001, "{e:?}");
        assert!(render(&e).contains("compiler selection or install"));
        let without = remote(
            &marks,
            11.86,
            &RemoteTimes {
                host_own: HostOwn::default(),
                ..times.clone()
            },
        );
        assert!((get(&without, "host.recorder") - 11.588).abs() < 1e-9);
        assert!((sum(&e) - sum(&without)).abs() < 1e-9);

        // The identical-build wait happens inside the host's setup: it is not
        // counted twice.
        times.slot_wait = 0.005;
        let e = remote(&marks, 11.86, &times);
        assert!((get(&e, "host.slot_wait") - 0.005).abs() < 1e-9);
        assert!((get(&e, "host.setup") - 0.007).abs() < 1e-9);

        // The daemon's wait for a prebuild of these units is named, out of
        // the remainder nobody marked: the whole stays the same, and it
        // never exceeds what is left.
        let mut waited = RemoteTimes {
            host_own: HostOwn::default(),
            prebuild_wait: 3.53,
            ..times.clone()
        };
        let e = remote(&marks, 11.86, &waited);
        assert!((get(&e, "host.prebuild_wait") - 3.53).abs() < 1e-9);
        assert!((get(&e, "host.slot_wait") - 0.005).abs() < 1e-9);
        assert!((get(&e, "host.recorder") - (11.588 - 0.005 - 3.53)).abs() < 1e-9);
        assert!((sum(&e) - sum(&without)).abs() < 1e-9);
        assert!(render(&e).contains("wait for prebuild of these units"));
        for (said, shown) in [(1e9, 11.583), (f64::NAN, 0.0), (-1.0, 0.0)] {
            waited.prebuild_wait = said;
            let e = remote(&marks, 11.86, &waited);
            assert!(
                (get(&e, "host.prebuild_wait") - shown).abs() < 1e-9,
                "{said}"
            );
            assert!((sum(&e) - sum(&without)).abs() < 1e-9);
        }
        // It happened before the host recorder started, so the recorder's
        // own measurements share only what it leaves.
        waited = RemoteTimes {
            prebuild_wait: 3.53,
            ..times.clone()
        };
        let e = remote(&marks, 11.86, &waited);
        assert!((get(&e, "host.prebuild_wait") - 3.53).abs() < 1e-9);
        assert!(get(&e, "host.toolchain") < 11.588 - 3.53);
        assert!((sum(&e) - sum(&without)).abs() < 1e-9);

        // A host that reports more than the agent measured is scaled down,
        // never past the residual.
        times.slot_wait = 0.0;
        times.host_own.toolchain = 100.0;
        let e = remote(&marks, 11.86, &times);
        assert!((sum(&e) - sum(&without)).abs() < 1e-9);
        assert!(get(&e, "host.recorder") < 1e-9);
        assert!((get(&e, "host.toolchain") - 11.588 * 100.0 / 100.018).abs() < 1e-9);

        // A reconstructed waterfall has no marks to trust.
        let mut derived = host.clone();
        derived.derived = true;
        assert_eq!(HostOwn::of(Some(&derived)), HostOwn::default());
        assert_eq!(HostOwn::of(None), HostOwn::default());
    }

    #[test]
    fn identical_build_wait_only_consumes_host_execution_residual() {
        // Replay execution measurements from 20261118-053339515-2563372-pre.
        let marks = Marks(vec![("flushed", 0.0), ("exit", 245.0)]);
        let mut times = RemoteTimes {
            host_wall: 101.45868349075317,
            run: Some(crate::remote_proto::RunTiming {
                execute_ms: 244922.168591,
                ..Default::default()
            }),
            ..Default::default()
        };
        let old = remote(&marks, 245.0, &times);
        let residual = get(&old, "host.recorder");
        assert!((residual - 143.46348510024683).abs() < 1e-9);
        for (measured, expected) in [
            (143.332422317, 143.332422317),
            (300.0, residual),
            (-1.0, 0.0),
            (f64::INFINITY, 0.0),
            (f64::NAN, 0.0),
        ] {
            times.slot_wait = measured;
            let e = remote(&marks, 245.0, &times);
            assert!((get(&e, "host.slot_wait") - expected).abs() < 1e-9);
            assert!((get(&e, "host.recorder") - (residual - expected)).abs() < 1e-9);
            assert!((sum(&e) - sum(&old)).abs() < 1e-9);
            assert_eq!(
                render(&e).contains("wait for identical build"),
                expected > 0.0
            );
        }
        times.slot_wait = 143.332422317;
        times.run = None;
        assert_eq!(get(&remote(&marks, 245.0, &times), "host.slot_wait"), 0.0);
    }

    #[test]
    fn remote_preflight_is_separate_from_client_routing() {
        let m = Marks(vec![
            ("remote_preflight", 0.1),
            ("routed", 0.8),
            ("exit", 1.0),
        ]);
        let e = remote(&m, 1.0, &RemoteTimes::default());
        assert!((get(&e, "client.route") - 0.1).abs() < 1e-9);
        assert!((get(&e, "remote.preflight") - 0.7).abs() < 1e-9);
        assert!((sum(&e) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pipelined_agent_timings_are_scaled_to_the_client_span() {
        let m = Marks(vec![("flushed", 1.0), ("exit", 3.0)]);
        let r = RemoteTimes {
            run: Some(crate::remote_proto::RunTiming {
                prepare_ms: 100.0,
                execute_ms: 4500.0,
                finalize_ms: 200.0,
                records_ms: 200.0,
            }),
            host_wall: 4.0,
            host_phases: Phases {
                compile: 4.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let e = remote(&m, 3.0, &r);
        assert!((sum(&e) - 3.0).abs() < 1e-9, "{e:?}");
        assert!((get(&e, "host.compile") - 1.6).abs() < 1e-9, "{e:?}");
        assert_eq!(get(&e, "remote.host_overhead"), 0.0);
        assert!(
            e.segments
                .iter()
                .all(|s| s.secs.is_finite() && s.secs >= 0.0)
        );
    }

    #[test]
    fn invalid_agent_timing_is_ignored_and_pipeline_overlap_remains_bounded() {
        let m = Marks(vec![("flushed", 1.0), ("exit", 3.0)]);
        for invalid in [f64::NAN, f64::INFINITY, -1.0] {
            let r = RemoteTimes {
                run: Some(crate::remote_proto::RunTiming {
                    prepare_ms: invalid,
                    execute_ms: 5000.0,
                    ..Default::default()
                }),
                host_wall: 4.0,
                host_phases: Phases {
                    compile: 4.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let e = remote(&m, 3.0, &r);
            assert!((sum(&e) - 3.0).abs() < 1e-9, "{e:?}");
            assert!(
                e.segments
                    .iter()
                    .all(|s| s.secs.is_finite() && s.secs >= 0.0)
            );
            assert_eq!(get(&e, "host.prepare"), 0.0);
            assert_eq!(get(&e, "host.compile"), 2.0);
        }
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
            run: None,
            sync: Some(SyncTiming {
                lock_ms: 900.0,
                link_ms: 50.0,
                push_ms: 300.0,
                barrier_ms: 60.0,
                ..Default::default()
            }),
            slot_wait: 0.0,
            host_own: HostOwn::default(),
            host_wall: 5.0,
            host_phases: Phases {
                compile: 4.0,
                test_run: 1.0,
                ..Default::default()
            },
            ..Default::default()
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
        s.measurement_coverage = Some(Default::default());
        let e = derive(&s).unwrap();
        assert_eq!(get(&e, "build.tests"), 0.0);
        assert!((get(&e, "build.tail") - 567.1).abs() < 1e-6);
    }
}
