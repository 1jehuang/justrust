//! Read-only, account-scoped dashboard data. Network work is explicitly separate
//! from local polling and record bodies are fetched only on selection.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL_BYTES: u64 = 1024 * 1024;
const INDEX_ROWS: usize = 200;
const LOG_LINES: usize = 2000;
const SUMMARY_BYTES: u64 = 8 * 1024 * 1024;

/// Where a run's wait went, grouped from the end-to-end segments
/// (`crate::e2e`) into the few buckets a person can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Bucket {
    Compile,
    Tests,
    Sync,
    Artifacts,
    Hosted,
    Startup,
    Lock,
    Client,
    Other,
}

/// Half-open measured intervals in seconds since the start of this elapsed axis.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Timeline {
    #[serde(default)]
    pub end_to_end: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<Box<Timeline>>,
    pub wall: f64,
    pub spans: Vec<(f64, f64, Bucket)>,
}

pub(crate) const BUCKETS: usize = 9;

/// Seconds per [`Bucket`], indexed by `Bucket as usize`. Sums to the wait.
pub(crate) type Times = [f64; BUCKETS];

/// Positive profiles are immutable. For other rows, throttle filesystem probes
/// and remember successful reads, including empty or malformed ledgers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Ledger {
    #[default]
    Unknown,
    Complete,
    Probe(std::time::Instant, Option<(u64, std::time::SystemTime)>),
}

pub(crate) fn cached_ledger(row: &RunRow) -> Ledger {
    if row.compile.is_some() || row.reuse.is_some() || row.opt_level.is_some() {
        Ledger::Complete
    } else {
        Ledger::Unknown
    }
}
pub(crate) type TimeCache = std::collections::HashMap<
    String,
    (
        Option<Times>,
        Option<CompileTimes>,
        Option<ReuseStats>,
        Option<Timeline>,
        Option<String>,
        Option<String>,
        Ledger,
        Option<(u64, u64)>,
        Option<f64>,
    ),
>;

/// What the compiler's time went to, from the units a run recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Workspace,
    Tests,
    Dependencies,
    ProcMacros,
    BuildScripts,
    Restores,
}

pub(crate) const PARTS: usize = 6;

/// Unit seconds per [`Part`], indexed by `Part as usize`. Units overlap, so
/// the sum is compiler time, not time waited.
pub(crate) type CompileTimes = [f64; PARTS];

#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReuseStats {
    pub cgu_reused: u64,
    pub cgu_total: u64,
    pub loaded: u64,
    pub ledgers: u64,
    #[serde(default)]
    pub query_observed: bool,
    #[serde(default)]
    pub cgu_observed: bool,
    pub same_secs: f64,
    pub green_secs: f64,
    pub changed_secs: f64,
    pub unknown_secs: f64,
}
impl ReuseStats {
    // Older account records lack coverage flags. Positive totals are evidence,
    // but all-zero totals cannot distinguish a measured zero from no rows.
    pub fn has_queries(self) -> bool {
        self.query_observed
            || self.loaded > 0
            || [
                self.same_secs,
                self.green_secs,
                self.changed_secs,
                self.unknown_secs,
            ]
            .iter()
            .any(|&v| v > 0.)
    }
    pub fn has_cgus(self) -> bool {
        self.cgu_observed || self.cgu_total > 0
    }
    pub fn measured(self) -> bool {
        self.has_queries() || self.has_cgus()
    }
    pub fn merge(&mut self, o: Self) {
        self.query_observed |= o.has_queries();
        self.cgu_observed |= o.has_cgus();
        self.cgu_reused += o.cgu_reused;
        self.cgu_total += o.cgu_total;
        self.loaded += o.loaded;
        self.ledgers += o.ledgers;
        self.same_secs += o.same_secs;
        self.green_secs += o.green_secs;
        self.changed_secs += o.changed_secs;
        self.unknown_secs += o.unknown_secs;
    }
    fn observe(&mut self, rows: &[crate::reuse::Row]) {
        if rows.is_empty() {
            return;
        }
        self.ledgers += 1;
        for r in rows {
            if r.query == "<cgu>" {
                self.cgu_observed = true;
                self.cgu_total += r.count;
                if r.outcome == "loaded" {
                    self.cgu_reused += r.count;
                }
            } else if !crate::reuse::is_evidence(r) && r.secs.is_finite() && r.secs >= 0. {
                self.query_observed = true;
                match crate::reuse::class(&r.outcome) {
                    crate::reuse::Class::Reused => self.loaded += r.count,
                    crate::reuse::Class::Avoidable if r.outcome == "green_recompute" => {
                        self.green_secs += r.secs
                    }
                    crate::reuse::Class::Avoidable => self.same_secs += r.secs,
                    crate::reuse::Class::Necessary => self.changed_secs += r.secs,
                    crate::reuse::Class::Opaque => self.unknown_secs += r.secs,
                }
            }
        }
    }
}
impl Part {
    pub(crate) const ALL: [Part; PARTS] = [
        Part::Workspace,
        Part::Tests,
        Part::Dependencies,
        Part::ProcMacros,
        Part::BuildScripts,
        Part::Restores,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Part::Workspace => "workspace crates",
            Part::Tests => "test targets",
            Part::Dependencies => "dependencies",
            Part::ProcMacros => "proc macros",
            Part::BuildScripts => "build scripts",
            Part::Restores => "cache restores",
        }
    }
}

/// Only what the compile profile needs from a recorded unit.
#[derive(serde::Deserialize)]
struct UnitTime {
    opt_level: Option<String>,
    start: Option<f64>,
    end: Option<f64>,
    #[serde(default)]
    reuse: Vec<crate::reuse::Row>,
    #[serde(default)]
    crate_types: Vec<String>,
    #[serde(default)]
    test: bool,
    #[serde(default)]
    build_script: bool,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    wall: f64,
    #[serde(default)]
    depcache: String,
}

/// Unit seconds by what was compiled. A unit restored from a cache counts as
/// a restore whatever it is. `None` when no unit has a usable time.
#[cfg(test)]
fn compile_times(units: &[u8]) -> Option<CompileTimes> {
    unit_profiles(units).0
}
#[cfg(test)]
fn unit_profiles(units: &[u8]) -> (Option<CompileTimes>, Option<ReuseStats>) {
    let (compile, reuse, _) = profile(units, |_, _| {});
    (compile, reuse)
}
fn profile(
    units: &[u8],
    mut interval: impl FnMut(f64, f64),
) -> (Option<CompileTimes>, Option<ReuseStats>, Option<String>) {
    let mut opt = None;
    let mut agg = DashboardProfile::default();
    for row in units.split(|byte| *byte == b'\n') {
        let Ok(unit) = serde_json::from_slice::<UnitTime>(row) else {
            continue;
        };
        if let (Some(start), Some(end)) = (unit.start, unit.end) {
            interval(start, end);
        }
        observe_level(&mut opt, unit.opt_level);
        agg.observe(
            part(
                &unit.depcache,
                unit.build_script,
                &unit.crate_types,
                unit.test,
                unit.local,
            ),
            unit.wall,
            &unit.reuse,
        );
    }
    (agg.compile, agg.reuse, opt)
}

fn observe_level(opt: &mut Option<String>, level: Option<String>) {
    if let Some(level) = level.filter(|s| crate::summary::valid_unit_opt_level(s)) {
        match opt.as_deref() {
            None => *opt = Some(level),
            Some(previous) if previous != "mixed" && previous != level => {
                *opt = Some("mixed".into())
            }
            _ => {}
        }
    }
}

fn part(cache: &str, script: bool, types: &[String], test: bool, local: bool) -> Part {
    if matches!(cache, "hit" | "remote" | "peer" | "prebuilt" | "local") {
        Part::Restores
    } else if script {
        Part::BuildScripts
    } else if types.iter().any(|t| t == "proc-macro") {
        Part::ProcMacros
    } else if test {
        Part::Tests
    } else if local {
        Part::Workspace
    } else {
        Part::Dependencies
    }
}

/// Fixed-size, non-identifying personal dashboard evidence for account records.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct DashboardProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overall: Option<Times>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overall_v2: Option<[f64; 13]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compile: Option<CompileTimes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reuse: Option<ReuseStats>,
}

impl DashboardProfile {
    fn observe(&mut self, part: Part, wall: f64, rows: &[crate::reuse::Row]) {
        if wall.is_finite() && wall > 0. {
            self.compile.get_or_insert([0.; PARTS])[part as usize] += wall;
        }
        if !rows.is_empty() {
            self.reuse
                .get_or_insert_with(Default::default)
                .observe(rows);
        }
    }
    pub(crate) fn recorded(units: &[crate::shim::Unit]) -> Self {
        let mut agg = Self::default();
        for u in units {
            agg.observe(
                part(&u.depcache, u.build_script, &u.crate_types, u.test, u.local),
                u.wall,
                &u.reuse,
            );
        }
        agg
    }
    /// Pre-aggregate recorders returned only JSONL. Fold one row at a time,
    /// recovering opt level in the same pass instead of rereading the sidecar.
    pub(crate) fn returned(dir: &Path) -> (Self, Option<String>) {
        File::open(dir.join("units.jsonl"))
            .map(|f| Self::read_returned(std::io::BufReader::new(f)))
            .unwrap_or_default()
    }
    fn read_returned(mut input: impl std::io::BufRead) -> (Self, Option<String>) {
        let mut agg = Self::default();
        let mut opt = None;
        let mut line = Vec::new();
        loop {
            line.clear();
            match input.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Err(_) => return (Self::default(), None),
                Ok(_) => {}
            }
            if let Ok(u) = serde_json::from_slice::<UnitTime>(&line) {
                observe_level(&mut opt, u.opt_level);
                agg.observe(
                    part(&u.depcache, u.build_script, &u.crate_types, u.test, u.local),
                    u.wall,
                    &u.reuse,
                );
            }
        }
        (agg, opt)
    }
    pub(crate) fn set_overall(&mut self, s: &crate::summary::Summary) {
        self.overall = measured_parts(s.wall, Some(&s.phases), s.e2e.as_ref());
        self.overall_v2 = Some(overall_v2(s));
    }
}

// Wire slots 0..8 retain their legacy labels. Only claim known work for
// recognized keys: legacy Bucket::of deliberately accepts unknown prefixes.
fn overall_v2(s: &crate::summary::Summary) -> [f64; 13] {
    const UNATTRIBUTED: usize = 8;
    const GAPS: usize = 9;
    const AFTER: usize = 10;
    const CARGO_UNKNOWN: usize = 11;
    const REMOTE_UNKNOWN: usize = 12;
    let mut out = [0.; 13];
    let mut add = |slot: usize, secs: f64| {
        if secs.is_finite() && secs > 0. {
            out[slot] += secs;
        }
    };
    let (wall, residual) = if let Some(e) = s.e2e.as_ref().filter(|e| !e.segments.is_empty()) {
        for seg in &e.segments {
            let key = seg.key.as_str();
            let slot = match key.split_once('.') {
                Some(("build" | "host", "build_gaps")) => GAPS,
                Some(("build" | "host", "tail")) => AFTER,
                Some(("build" | "host", "other")) => CARGO_UNKNOWN,
                Some(("build" | "host", "compile" | "tests" | "cargo_startup" | "lock_wait"))
                | Some(("remote", "admission" | "preflight" | "daemon_connect" | "artifacts"))
                | Some((
                    "host",
                    "prepare" | "slot_wait" | "toolchain" | "setup" | "report" | "finalize"
                    | "records",
                ))
                | Some(("sync", "lock_wait" | "link" | "push" | "barrier" | "other" | "total"))
                | Some(("client", "setup" | "toolchain" | "route" | "roots" | "report")) => {
                    Bucket::of(key) as usize
                }
                Some(("remote" | "host", _)) => REMOTE_UNKNOWN,
                Some(("build", _)) => CARGO_UNKNOWN,
                _ => UNATTRIBUTED,
            };
            add(slot, seg.secs);
        }
        (e.wall, UNATTRIBUTED)
    } else {
        let p = &s.phases;
        for (slot, secs) in [
            (Bucket::Startup as usize, p.startup),
            (Bucket::Lock as usize, p.lock_wait),
            (Bucket::Compile as usize, p.compile),
            (Bucket::Tests as usize, p.test_run),
            (GAPS, p.build_gaps),
            (AFTER, p.tail),
        ] {
            add(slot, secs);
        }
        (s.wall, CARGO_UNKNOWN)
    };
    let total: f64 = out.iter().sum();
    if wall.is_finite() && wall > 0. {
        if total > wall {
            for value in &mut out {
                *value *= wall / total;
            }
        } else {
            out[residual] += wall - total;
        }
    }
    out
}

/// Summary and sidecars travel together from the recorder. Neither their
/// epoch nor their wall time is the hosted client's clock.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct TimelineInput {
    start: Option<f64>,
    wall: f64,
    tests: BinaryNames,
    e2e: Option<crate::e2e::E2e>,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct BinaryNames {
    binaries: Vec<BinaryName>,
}
#[derive(serde::Deserialize)]
struct BinaryName {
    name: String,
}

fn timeline_profiles(
    summary: &[u8],
    units: &[u8],
    local: bool,
    processes: impl FnOnce() -> Vec<u8>,
) -> (
    Option<CompileTimes>,
    Option<ReuseStats>,
    Option<Timeline>,
    Option<String>,
) {
    let input = serde_json::from_slice::<TimelineInput>(summary).unwrap_or_default();
    let e2e = input.e2e.as_ref().filter(|e| {
        !e.derived
            && e.wall.is_finite()
            && e.wall > 0.
            && e.trace
                .iter()
                .any(|(a, b, _)| *a >= 0. && a < b && *b <= e.wall)
    });
    let mut timeline = Timeline {
        wall: e2e.map_or(input.wall, |e| e.wall),
        spans: Vec::new(),
        end_to_end: e2e.is_some(),
        cargo: None,
    };
    let mut window = e2e.is_none().then_some((0., input.wall));
    if let Some(e) = e2e {
        for (lo, hi, key) in &e.trace {
            if !(0. <= *lo && lo < hi && *hi <= e.wall) {
                continue;
            }
            if key == "build.window" {
                if local {
                    window = Some((*lo, *hi));
                }
            } else {
                timeline.spans.push((*lo, *hi, Bucket::of(key)));
            }
        }
    }
    let separate = window.is_none();
    let mut cargo = Timeline {
        wall: input.wall,
        spans: Vec::new(),
        end_to_end: false,
        cargo: None,
    };
    let (offset, limit) = window.unwrap_or((0., input.wall));
    let target = if separate { &mut cargo } else { &mut timeline };
    let mut observe = |start: f64, end: f64, bucket| {
        if let Some(epoch) = input.start {
            let lo = (start - epoch).max(0.);
            let hi = (end - epoch).min(input.wall).min(limit - offset);
            if start.is_finite() && end.is_finite() && lo < hi {
                target.spans.push((offset + lo, offset + hi, bucket));
            }
        }
    };
    let (compile, reuse, opt) = profile(units, |s, e| observe(s, e, Bucket::Compile));
    // Summary binary ranges can enclose gaps between several invocations of
    // the same binary. Color only the lifetime actually observed for each PID.
    if !input.tests.binaries.is_empty() {
        let names: std::collections::HashSet<_> = input
            .tests
            .binaries
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        if let Ok(procs) = serde_json::from_slice::<Vec<crate::record::ProcRecord>>(&processes()) {
            for p in procs {
                if p.kind == "target-binary"
                    && p.cmdline
                        .split_whitespace()
                        .next()
                        .is_some_and(|exe| names.contains(crate::sys::last_component(exe)))
                {
                    observe(p.first_seen, p.last_seen, Bucket::Tests);
                }
            }
        }
    }
    if separate && cargo.wall.is_finite() && cargo.wall > 0. && !cargo.spans.is_empty() {
        cargo
            .spans
            .sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
        if timeline.spans.is_empty() {
            timeline = cargo;
        } else {
            timeline.cargo = Some(Box::new(cargo));
        }
    }
    timeline
        .spans
        .sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let measured = timeline.wall.is_finite() && timeline.wall > 0. && !timeline.spans.is_empty();
    (compile, reuse, measured.then_some(timeline), opt)
}

impl Bucket {
    pub(crate) const ALL: [Bucket; BUCKETS] = [
        Bucket::Compile,
        Bucket::Tests,
        Bucket::Sync,
        Bucket::Artifacts,
        Bucket::Hosted,
        Bucket::Startup,
        Bucket::Lock,
        Bucket::Client,
        Bucket::Other,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Bucket::Compile => "compiling",
            Bucket::Tests => "running tests",
            Bucket::Sync => "source sync",
            Bucket::Artifacts => "artifacts back",
            Bucket::Hosted => "hosted overhead",
            Bucket::Startup => "cargo startup",
            Bucket::Lock => "build dir lock",
            Bucket::Client => "justrust client",
            Bucket::Other => "other",
        }
    }

    /// Bucket for an `area.what` segment key. Unrecognized keys are `Other`.
    fn of(key: &str) -> Bucket {
        match key.split_once('.').unwrap_or(("", key)) {
            ("sync", _) => Bucket::Sync,
            ("client", _) => Bucket::Client,
            (_, "compile") => Bucket::Compile,
            (_, "tests") => Bucket::Tests,
            (_, "cargo_startup") => Bucket::Startup,
            (_, "lock_wait") => Bucket::Lock,
            ("remote", "artifacts") => Bucket::Artifacts,
            ("remote", "other") => Bucket::Other,
            ("remote", _) => Bucket::Hosted,
            (
                "host",
                "prepare" | "slot_wait" | "toolchain" | "setup" | "report" | "recorder"
                | "finalize" | "records",
            ) => Bucket::Hosted,
            _ => Bucket::Other,
        }
    }
}

/// Preserve explicit zero counts without treating missing counts as zero.
fn cargo_units(units: &Value) -> Option<(u64, u64)> {
    Some((units["fresh"].as_u64()?, units["compiled"].as_u64()?))
}

#[derive(serde::Deserialize)]
struct Timing {
    #[serde(default)]
    units: Value,
    toolchain: Option<String>,
    #[serde(default)]
    wall: f64,
    #[serde(default)]
    phases: Option<crate::summary::Phases>,
    #[serde(default)]
    e2e: Option<crate::e2e::E2e>,
}

/// End-to-end segments when recorded, otherwise the coarser cargo phases with
/// the unattributed remainder of the wall time as `Other`.
#[cfg(test)]
fn times(summary: &[u8]) -> Option<Times> {
    measured_times(serde_json::from_slice(summary).ok()?)
}

fn measured_times(timing: Timing) -> Option<Times> {
    measured_parts(timing.wall, timing.phases.as_ref(), timing.e2e.as_ref())
}
fn measured_parts(
    wall: f64,
    phases: Option<&crate::summary::Phases>,
    e2e: Option<&crate::e2e::E2e>,
) -> Option<Times> {
    let mut out = [0.0; BUCKETS];
    let mut add = |bucket: Bucket, secs: f64| {
        if secs.is_finite() && secs > 0.0 {
            out[bucket as usize] += secs;
        }
    };
    match e2e.filter(|e2e| !e2e.segments.is_empty()) {
        Some(e2e) => {
            for segment in &e2e.segments {
                add(Bucket::of(&segment.key), segment.secs);
            }
        }
        None => {
            let p = phases?;
            add(Bucket::Startup, p.startup);
            add(Bucket::Lock, p.lock_wait);
            add(Bucket::Compile, p.compile);
            add(Bucket::Tests, p.test_run);
            add(Bucket::Other, p.build_gaps + p.tail);
            let attributed =
                p.startup + p.lock_wait + p.compile + p.test_run + p.build_gaps + p.tail;
            add(Bucket::Other, wall - attributed);
        }
    }
    Some(out)
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Snapshot {
    pub active: Vec<crate::status::Active>,
    pub runs: Vec<RunRow>,
    pub account: Option<crate::remote_hosted::Me>,
    pub hosted_error: Option<String>,
    pub local_error: Option<String>,
    pub hosted_at: Option<f64>,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RunRow {
    pub id: String,
    pub toolchain_version: Option<String>,
    pub client_version: Option<String>,
    pub start: f64,
    pub cwd: String,
    pub args: Vec<String>,
    pub exit: Option<i32>,
    pub wall: f64,
    pub place: String,
    pub machine: Option<String>,
    /// `local` means the record is on this client, not where compilation ran.
    pub origin: String,
    /// Who a failed run is on, when the recorder classified it.
    pub failure: Option<crate::failure::Class>,
    pub compile_failed: bool,
    /// Cargo artifact counts (fresh, compiled), only when both were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_units: Option<(u64, u64)>,
    /// Where the wait went. Only for runs whose summary is on this client.
    pub time: Option<Times>,
    /// What the compiler's time went to. Only for runs with units recorded here.
    pub compile: Option<CompileTimes>,
    /// Effective rustc optimization observed across recorded units, or `mixed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opt_level: Option<String>,
    pub reuse: Option<ReuseStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<Timeline>,
    /// First-principles theoretical optimal wall-time floor in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theoretical_floor_secs: Option<f64>,
    /// Optimistic realistic wall-time estimate in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimistic_est_secs: Option<f64>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct HostedSnapshot {
    pub account: crate::remote_hosted::Me,
    pub runs: Vec<RunRow>,
    pub history_error: Option<String>,
    #[serde(skip)]
    pub cache_scope: Option<String>,
}

/// An independent local update. The caller retains last-good hosted data.
pub(crate) fn local_snapshot(
    cache: &mut TimeCache,
    history_ready: impl FnOnce(&[RunRow]),
) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let rows = (|| -> Result<Vec<RunRow>> {
        let path = crate::paths::index_file()?;
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("open local run index"),
        };
        let size = file.metadata()?.len();
        Ok(index_rows(&read_tail(&mut file, size, TAIL_BYTES)?))
    })();
    match rows {
        Ok(mut rows) => {
            for row in &mut rows {
                if let Some((time, compile, reuse, timeline, opt, toolchain, _, units, floor)) =
                    cache.get(&row.id)
                {
                    row.cargo_units = *units;
                    row.time = *time;
                    row.compile = *compile;
                    row.reuse = *reuse;
                    row.timeline.clone_from(timeline);
                    row.opt_level.clone_from(opt);
                    if row.toolchain_version.is_none() {
                        row.toolchain_version.clone_from(toolchain);
                    }
                    if row.theoretical_floor_secs.is_none() {
                        row.theoretical_floor_secs = *floor;
                    }
                }
            }
            // The index is small. Never hold its rows behind a directory scan
            // or hundreds of summary/unit parses needed by the profile cards.
            history_ready(&rows);
            if let Ok(dir) = crate::paths::runs_dir() {
                enrich_rows(&mut rows, cache, &dir);
            }
            snapshot.runs = rows;
        }
        Err(error) => {
            let message = format!("Local history: {error:#}");
            snapshot.local_error = Some(match snapshot.local_error.take() {
                Some(previous) => format!("{previous}\n{message}"),
                None => message,
            });
        }
    }
    match crate::status::collect() {
        Ok(status) => snapshot.active = status.active,
        Err(error) => {
            let message = format!("Active builds: {error:#}");
            snapshot.local_error = Some(match snapshot.local_error.take() {
                Some(previous) => format!("{previous}\n{message}"),
                None => message,
            });
        }
    }
    snapshot
}

fn enrich_rows(rows: &mut [RunRow], cache: &mut TimeCache, dir: &Path) {
    enrich_rows_at(rows, cache, dir, std::time::Instant::now());
}

fn enrich_rows_at(rows: &mut [RunRow], cache: &mut TimeCache, dir: &Path, now: std::time::Instant) {
    let mut ledgers: Vec<_> = rows
        .iter()
        .map(|r| cache.get(&r.id).map_or(Ledger::Unknown, |v| v.6))
        .collect();
    let due = |state: &Ledger| match state {
        Ledger::Unknown => true,
        Ledger::Probe(next, _) => *next <= now,
        Ledger::Complete => false,
    };
    let missing = ledgers.iter().filter(|s| due(s)).count();
    if missing > 0 {
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(4)
            .min(missing);
        let chunk = rows.len().div_ceil(workers).max(1);
        std::thread::scope(|scope| {
            for (rows, states) in rows.chunks_mut(chunk).zip(ledgers.chunks_mut(chunk)) {
                scope.spawn(move || {
                    for (row, state) in rows.iter_mut().zip(states) {
                        if !due(state) {
                            continue;
                        }
                        let run = dir.join(&row.id);
                        let path = run.join("units.jsonl");
                        let metadata = std::fs::metadata(&path).ok();
                        let stamp = metadata
                            .as_ref()
                            .and_then(|m| m.modified().ok().map(|mtime| (m.len(), mtime)));
                        let previous = *state;
                        let next = now + std::time::Duration::from_secs(10);
                        *state = Ledger::Probe(next, stamp);
                        if let Ledger::Probe(_, old) = previous
                            && old == stamp
                            && (old.is_some() || metadata.is_none())
                        {
                            continue;
                        }
                        let oversized = metadata.is_some_and(|m| m.len() > SUMMARY_BYTES);
                        let units = if oversized {
                            None
                        } else {
                            read_summary(&path).ok()
                        };
                        if units.is_none() {
                            if !oversized {
                                *state = Ledger::Probe(next, None);
                            }
                            if previous != Ledger::Unknown {
                                continue;
                            }
                        }
                        let units = units.unwrap_or_default();
                        let summary = read_summary(&run.join("summary.json")).unwrap_or_default();
                        if let Ok(mut timing) = serde_json::from_slice::<Timing>(&summary) {
                            if row.toolchain_version.is_none() {
                                row.toolchain_version = timing
                                    .toolchain
                                    .take()
                                    .as_deref()
                                    .map(crate::summary::recorded_toolchain_label);
                            }
                            row.cargo_units = cargo_units(&timing.units);
                            row.time = measured_times(timing);
                        }
                        if (row.theoretical_floor_secs.is_none()
                            || row.optimistic_est_secs.is_none())
                            && !summary.is_empty()
                            && let Ok(full) =
                                serde_json::from_slice::<crate::summary::Summary>(&summary)
                        {
                            let computed = crate::theoretical::compute(
                                &full.subcommand,
                                &full.args,
                                &full.git,
                                &full.units,
                                &full.phases,
                                &full.tests,
                            );
                            if row.theoretical_floor_secs.is_none() {
                                row.theoretical_floor_secs = Some(
                                    full.theoretical_floor
                                        .as_ref()
                                        .map(|f| f.total_secs)
                                        .unwrap_or(computed.total_secs),
                                );
                            }
                            if row.optimistic_est_secs.is_none() {
                                row.optimistic_est_secs = Some(
                                    full.theoretical_floor
                                        .as_ref()
                                        .map(|f| f.optimistic_est_secs)
                                        .filter(|&v| v > 0.0)
                                        .unwrap_or(computed.optimistic_est_secs),
                                );
                            }
                        }
                        let (compile, reuse, timeline, opt) =
                            timeline_profiles(&summary, &units, row.place == "local", || {
                                read_summary(&run.join("processes.json")).unwrap_or_default()
                            });
                        row.compile = compile;
                        row.reuse = reuse;
                        row.opt_level = opt;
                        if timeline.is_some() {
                            row.timeline = timeline;
                        }
                        if cached_ledger(row) == Ledger::Complete {
                            *state = Ledger::Complete;
                        }
                    }
                });
            }
        });
    }
    for (row, ledger) in rows.iter().zip(ledgers) {
        cache.insert(
            row.id.clone(),
            (
                row.time,
                row.compile,
                row.reuse,
                row.timeline.clone(),
                row.opt_level.clone(),
                row.toolchain_version.clone(),
                ledger,
                row.cargo_units,
                row.theoretical_floor_secs,
            ),
        );
    }
    let ids: std::collections::HashSet<_> = rows.iter().map(|row| row.id.as_str()).collect();
    cache.retain(|id, _| ids.contains(id.as_str()));
}

/// At most two concurrent authenticated GETs, regardless of the run count.
/// Publish account information immediately, without waiting on run history.
pub(crate) fn fetch_hosted(
    account_ready: impl FnOnce(crate::remote_hosted::Me),
    history_ready: impl FnOnce(&[RunRow]) + Send,
) -> Result<HostedSnapshot> {
    let credentials = crate::remote_hosted::creds()?;
    let mut snapshot = fetch_hosted_with(
        || crate::remote_hosted::me(&credentials),
        || {
            let records = get(&credentials, "build/records?kind=run&summary=1&limit=50")?;
            let rows = records["records"]
                .as_array()
                .context("hosted records response is missing records array")?;
            let rows: Vec<_> = rows.iter().take(50).filter_map(hosted_row).collect();
            history_ready(&rows);
            Ok(rows)
        },
        account_ready,
    )?;
    snapshot.cache_scope = super::cache::scope_for(&credentials);
    Ok(snapshot)
}

fn fetch_hosted_with(
    account: impl FnOnce() -> Result<crate::remote_hosted::Me>,
    history: impl FnOnce() -> Result<Vec<RunRow>> + Send,
    account_ready: impl FnOnce(crate::remote_hosted::Me),
) -> Result<HostedSnapshot> {
    std::thread::scope(|scope| {
        let history = scope.spawn(history);
        let account = account()?;
        account_ready(account.clone());
        let (runs, history_error) = match history
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("dashboard history worker panicked")))
        {
            Ok(runs) => (runs, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        Ok(HostedSnapshot {
            account,
            runs,
            history_error,
            cache_scope: None,
        })
    })
}

#[test]
#[ignore = "uses local run history, hosted credentials and the real network"]
fn full_dashboard_data_latency_probe() {
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let snapshot = local_snapshot(&mut TimeCache::new(), |rows| {
                eprintln!(
                    "dashboard-full local_history_visible_ms={} rows={}",
                    started.elapsed().as_millis(),
                    rows.len()
                )
            });
            eprintln!(
                "dashboard-full local_history_profiles_ms={} rows={} profiled={}",
                started.elapsed().as_millis(),
                snapshot.runs.len(),
                snapshot.runs.iter().filter(|r| r.compile.is_some()).count()
            );
            assert!(snapshot.local_error.is_none(), "{:?}", snapshot.local_error);
        });
        scope.spawn(|| {
            let hosted = fetch_hosted(
                |_| {
                    eprintln!(
                        "dashboard-full account_ms={}",
                        started.elapsed().as_millis()
                    )
                },
                |_| {
                    eprintln!(
                        "dashboard-full hosted_history_visible_ms={}",
                        started.elapsed().as_millis()
                    )
                },
            )
            .expect("hosted data");
            eprintln!(
                "dashboard-full hosted_history_ms={} rows={}",
                started.elapsed().as_millis(),
                hosted.runs.len()
            );
            assert!(hosted.history_error.is_none(), "{:?}", hosted.history_error);
        });
        scope.spawn(|| {
            let versions = fetch_versions(&Versions::local(), |_| {
                eprintln!(
                    "dashboard-full service_versions_visible_ms={}",
                    started.elapsed().as_millis()
                )
            });
            eprintln!(
                "dashboard-full versions_ms={} service={} machine_components={} machine_note={:?}",
                started.elapsed().as_millis(),
                versions.live.is_some(),
                versions.machine.len(),
                versions.machine_note
            );
            assert!(versions.error.is_none(), "{:?}", versions.error);
        });
    });
    eprintln!(
        "dashboard-full all_complete_ms={}",
        started.elapsed().as_millis()
    );
}

#[test]
#[ignore = "uses this machine's hosted account and real network requests"]
fn hosted_dashboard_latency_probe() {
    for sample in 1..=3 {
        let start = std::time::Instant::now();
        let snapshot = fetch_hosted(
            |_| {
                eprintln!(
                    "dashboard sample={sample} account_ms={}",
                    start.elapsed().as_millis()
                )
            },
            |_| {},
        )
        .expect("authenticated dashboard refresh");
        assert!(
            snapshot.history_error.is_none(),
            "{:?}",
            snapshot.history_error
        );
        eprintln!(
            "dashboard sample={sample} complete_ms={} runs={}",
            start.elapsed().as_millis(),
            snapshot.runs.len()
        );
    }
}

fn get(credentials: &crate::remote_hosted::Creds, path: &str) -> Result<Value> {
    let response = crate::remote_hosted::request(credentials, "GET", path, None)?;
    if !(200..300).contains(&response.code) {
        return Err(crate::remote_hosted::api_error(&response));
    }
    Ok(response.body)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && id != "."
        && id != ".."
}

fn number(value: &Value) -> f64 {
    value.as_f64().filter(|n| n.is_finite()).unwrap_or(0.0)
}

fn index_rows(tail: &str) -> Vec<RunRow> {
    let mut seen = std::collections::HashSet::new();
    tail.lines()
        .rev()
        .take(INDEX_ROWS)
        .filter_map(|line| serde_json::from_str::<crate::summary::IndexEntry>(line).ok())
        .filter(|entry| valid_id(&entry.id) && seen.insert(entry.id.clone()))
        .map(|entry| RunRow {
            id: entry.id,
            toolchain_version: entry.toolchain_version,
            client_version: entry.client_version,
            start: entry.start,
            cwd: entry.cwd,
            args: entry.args,
            exit: Some(entry.exit),
            wall: entry.wall,
            place: crate::summary::execution_place(entry.place.as_deref(), entry.remote.as_deref())
                .into(),
            machine: entry.remote,
            origin: "local".into(),
            failure: entry.failure,
            compile_failed: entry.compile_failed,
            cargo_units: None,
            time: None,
            compile: None,
            reuse: None,
            timeline: None,
            opt_level: None,
            theoretical_floor_secs: entry.theoretical_floor_secs,
            optimistic_est_secs: entry.optimistic_est_secs,
        })
        .collect()
}

fn text(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

// The account list carries only the fixed reuse projection, not raw ledgers.
fn hosted_reuse(value: &Value) -> Option<ReuseStats> {
    value.as_object()?;
    let stats: ReuseStats = serde::Deserialize::deserialize(value).ok()?;
    const MAX: u64 = 1_000_000_000_000;
    (stats.ledgers > 0
        && stats.cgu_reused <= stats.cgu_total
        && [stats.ledgers, stats.cgu_total, stats.loaded]
            .iter()
            .all(|&n| n <= MAX)
        && [
            stats.same_secs,
            stats.green_secs,
            stats.changed_secs,
            stats.unknown_secs,
        ]
        .iter()
        .all(|&n| (0.0..=MAX as f64).contains(&n))
        && stats.measured())
    .then_some(stats)
}

fn hosted_row(value: &Value) -> Option<RunRow> {
    let id = value["id"].as_str()?;
    if !valid_id(id) {
        return None;
    }
    // The list deliberately contains metadata only. Do not invent a command,
    // verdict, or execution location from the uploader machine label.
    let summary = &value["summary"];
    Some(RunRow {
        id: id.into(),
        toolchain_version: text(&summary["toolchain_version"])
            .or_else(|| text(&summary["toolchain_pin"])),
        client_version: text(&summary["client_version"]),
        start: summary["start"]
            .as_f64()
            .filter(|n| n.is_finite())
            .unwrap_or_else(|| number(&value["at"])),
        cwd: summary["cwd"].as_str().unwrap_or_default().into(),
        args: summary["args"]
            .as_array()
            .into_iter()
            .flatten()
            .take(128)
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        exit: summary["exit"].as_i64().and_then(|n| i32::try_from(n).ok()),
        wall: number(&summary["wall"]),
        machine: value["machine"].as_str().map(str::to_owned),
        place: summary["place"]
            .as_str()
            .filter(|p| matches!(*p, "local" | "remote" | "hosted"))
            .unwrap_or("unknown")
            .into(),
        origin: "hosted".into(),
        failure: serde_json::from_value(summary["failure"].clone()).ok(),
        cargo_units: cargo_units(&summary["units"]),
        compile_failed: summary["compile_failed"].as_bool().unwrap_or(false),
        time: None,
        compile: None,
        reuse: hosted_reuse(&summary["dashboard_profile"]["reuse"]),
        timeline: None,
        opt_level: summary["opt_level"]
            .as_str()
            .filter(|level| matches!(*level, "0" | "1" | "2" | "3" | "s" | "z" | "mixed"))
            .map(str::to_owned),
        theoretical_floor_secs: summary["theoretical_floor"]["total_secs"]
            .as_f64()
            .or_else(|| summary["theoretical_floor_secs"].as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0),
        optimistic_est_secs: summary["theoretical_floor"]["optimistic_est_secs"]
            .as_f64()
            .or_else(|| summary["optimistic_est_secs"].as_f64())
            .filter(|n| n.is_finite() && *n > 0.0),
    })
}

/// Read a bounded suffix, dropping a partial first JSONL record. Lossy UTF-8
/// decoding also makes a truncated or partially written log safe to display.
fn read_tail(reader: &mut (impl Read + Seek), size: u64, limit: u64) -> Result<String> {
    let offset = size.saturating_sub(limit);
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes)?;
    let start = if offset > 0 {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |i| i + 1)
    } else {
        0
    };
    Ok(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

fn read_summary(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(SUMMARY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > SUMMARY_BYTES {
        bail!("run summary exceeds the dashboard's 8 MiB limit");
    }
    Ok(bytes)
}

fn log_text(tail: &str) -> String {
    let mut lines: Vec<String> = tail
        .lines()
        .rev()
        .take(LOG_LINES)
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|value| value["l"].as_str().map(str::to_owned))
                .unwrap_or_else(|| line.to_owned())
        })
        .collect();
    lines.reverse();
    lines.join("\n")
}

/// Detail is a user-triggered operation. No directory scans or implicit network
/// fallback: a local read error remains visible and never changes its origin.
pub(crate) fn fetch_detail(row: &RunRow, logs: bool) -> Result<String> {
    if !valid_id(&row.id) {
        bail!("invalid run ID");
    }
    if row.origin == "hosted" {
        if logs {
            return Ok("Hosted records contain run summaries, not output logs. Logs are available on the machine that recorded the run.".into());
        }
        let credentials = crate::remote_hosted::creds()?;
        let record = get(&credentials, &format!("build/records/run/{}", row.id))?;
        let data = record
            .get("data")
            .context("hosted record is missing data")?;
        return Ok(serde_json::to_string_pretty(data)?);
    }
    if row.origin != "local" {
        bail!("unknown run origin: {}", row.origin);
    }
    let dir = crate::paths::runs_dir()?.join(&row.id);
    if logs {
        let mut file = File::open(dir.join("output.jsonl")).context("open run output")?;
        let size = file.metadata()?.len();
        let tail = read_tail(&mut file, size, TAIL_BYTES)?;
        return Ok(log_text(&tail));
    }
    let bytes = read_summary(&dir.join("summary.json")).context("read run summary")?;
    let summary: crate::summary::Summary = serde_json::from_slice(&bytes)?;
    Ok(crate::runs::render(&summary, &dir))
}

/// What the hosted service says is live (`GET <build base>/version`).
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Live {
    /// The release clients are told to run.
    pub release: Option<String>,
    /// The operator holds the rollout at `release` instead of the newest.
    pub held: bool,
    pub auto_update: bool,
    /// The control plane deployment that answered.
    pub deployment: Option<String>,
    pub deployed_at: Option<String>,
    pub deployment_tag: Option<String>,
    /// Immutable worker-side bundle used to provision build machines.
    pub pool_bundle: Option<String>,
    /// None when talking to an older service that does not report this flag.
    pub toolchain_dist: Option<bool>,
    /// The compiler hosted builds use when a project pins none.
    pub toolchain: Option<String>,
    pub toolchain_version: Option<String>,
    pub protocol: Option<u32>,
    pub published: bool,
}

/// Parse the service's answer. `None` when it is not the version document.
pub(crate) fn live(document: &str) -> Option<Live> {
    let v: Value = serde_json::from_str(document).ok()?;
    v.get("release")?.as_object()?;
    Some(Live {
        release: text(&v["release"]["version"]),
        held: v["release"]["held"].as_bool().unwrap_or(false),
        auto_update: v["release"]["auto_update"].as_bool().unwrap_or(true),
        deployment: text(&v["service"]["id"]),
        deployed_at: text(&v["service"]["deployed_at"]),
        deployment_tag: text(&v["service"]["tag"]),
        pool_bundle: text(&v["pool_bundle"]),
        toolchain_dist: v["toolchain_dist"].as_bool(),
        toolchain: text(&v["toolchain"]["toolchain"]),
        toolchain_version: text(&v["toolchain"]["version"]),
        protocol: v["toolchain"]["protocol"]
            .as_u64()
            .and_then(|p| p.try_into().ok()),
        published: v["toolchain"]["published"].as_bool().unwrap_or(false),
    })
}

/// How this client compares with the release the service names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Currency {
    Current,
    /// A newer release exists.
    Behind(String),
    /// Newer than anything published.
    Ahead,
    Unknown,
}

/// Every version that decides how a build behaves: this client, the release
/// and compiler the service names, and what the build machine really runs.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Versions {
    pub client: String,
    /// A published release artifact, not a source build of the same number.
    pub release_build: bool,
    pub service: String,
    pub live: Option<Live>,
    pub error: Option<String>,
    /// Named versions read from the build machine, in its display order.
    pub machine: Vec<(String, String)>,
    /// Why `machine` is empty.
    pub machine_note: Option<String>,
    /// When the service was last asked, `None` before the first answer.
    pub at: Option<f64>,
    #[serde(default)]
    pub cached: bool,
}

impl Versions {
    /// What is known without the network.
    pub(crate) fn local() -> Self {
        let base = crate::remote_hosted::build_base();
        let host = base.split("://").nth(1).unwrap_or(&base);
        Self {
            client: crate::update::VERSION.into(),
            release_build: crate::update::release_build(),
            service: host.split('/').next().unwrap_or(host).into(),
            ..Default::default()
        }
    }

    pub(crate) fn currency(&self) -> Currency {
        match self.live.as_ref().and_then(|l| l.release.as_deref()) {
            None => Currency::Unknown,
            Some(r) if crate::update::newer(r, &self.client) => Currency::Behind(r.into()),
            Some(r) if crate::update::newer(&self.client, r) => Currency::Ahead,
            Some(_) => Currency::Current,
        }
    }

    /// Whether this client speaks the hosted toolchain's protocol.
    pub(crate) fn drives_toolchain(&self) -> Option<bool> {
        let protocol = self.live.as_ref()?.protocol?;
        Some(crate::toolchain_policy::SUPPORTED.contains(&protocol))
    }

    pub(crate) fn machine(&self, name: &str) -> Option<&str> {
        self.machine
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The whole picture as text, for the detail view.
    pub(crate) fn report(&self) -> String {
        use std::fmt::Write;
        let mut o = String::from("Justrust Personal Dashboard · Versions\n\n");
        if self.cached {
            let _ = writeln!(
                o,
                "Cached information from the last successful load; refreshing in the background.\n"
            );
        }
        let _ = writeln!(
            o,
            "Client\n  justrust {} ({})\n  {}",
            self.client,
            if self.release_build {
                "published release"
            } else {
                "source build, not a published artifact"
            },
            match self.currency() {
                Currency::Current => "up to date with the release the service names".to_owned(),
                Currency::Behind(v) => format!("{v} is available: run `justrust update`"),
                Currency::Ahead => "newer than the release the service names".to_owned(),
                Currency::Unknown => "latest release not known yet".to_owned(),
            }
        );
        let _ = writeln!(o, "\nService ({})", self.service);
        match &self.live {
            Some(l) => {
                let or = |v: &Option<String>| v.clone().unwrap_or_else(|| "not reported".into());
                let _ = writeln!(o, "  deployment  {}", or(&l.deployment));
                let _ = writeln!(o, "  deployed    {}", or(&l.deployed_at));
                let _ = writeln!(o, "  build tag   {}", or(&l.deployment_tag));
                let _ = writeln!(o, "  host bundle {}", or(&l.pool_bundle));
                let _ = writeln!(
                    o,
                    "  distribution {}",
                    match l.toolchain_dist {
                        Some(true) => "enabled",
                        Some(false) => "disabled",
                        None => "not reported",
                    }
                );
                let _ = writeln!(
                    o,
                    "  release     {}{}",
                    or(&l.release),
                    if l.held { " (rollout held here)" } else { "" }
                );
                let _ = writeln!(
                    o,
                    "  auto-update {}",
                    if l.auto_update {
                        "on"
                    } else {
                        "off for every client"
                    }
                );
                let _ = writeln!(
                    o,
                    "  toolchain   {}{}",
                    or(&l.toolchain),
                    if l.toolchain.is_some() && !l.published {
                        " (not on the toolchain store)"
                    } else {
                        ""
                    }
                );
                if let Some(p) = l.protocol {
                    let _ = writeln!(
                        o,
                        "  protocol    {p} ({})",
                        match self.drives_toolchain() {
                            Some(false) => "this client cannot drive it: run `justrust update`",
                            _ => "supported by this client",
                        }
                    );
                }
            }
            None => {
                let _ = writeln!(o, "  not reached yet");
            }
        }
        if let Some(e) = &self.error {
            let _ = writeln!(o, "  error: {e}");
        }
        let _ = writeln!(
            o,
            "\nBuild machine (your configured machine, not the whole fleet)"
        );
        if self.machine.is_empty() && self.machine_note.is_none() {
            let _ = writeln!(o, "  component versions have not been reported yet");
        }
        for (name, value) in &self.machine {
            let _ = writeln!(o, "  {name:<11} {value}");
        }
        if let Some(note) = &self.machine_note {
            let _ = writeln!(o, "  {note}");
        }
        o
    }
}

fn fetch_live() -> Result<Live> {
    let url = format!("{}/version", crate::remote_hosted::build_base());
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--proto", "=https,http"])
        .args(["--connect-timeout", "5", "--max-time", "10"])
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .output()
        .context("running curl")?;
    if !out.status.success() {
        let error = String::from_utf8_lossy(&out.stderr);
        bail!(
            "{}",
            error.lines().last().unwrap_or("request failed").trim()
        );
    }
    live(&String::from_utf8_lossy(&out.stdout)).context("the service returned no version document")
}

/// What the build machine runs, read from the machine over the open session.
fn machine_versions() -> (Vec<(String, String)>, Option<String>) {
    let status = crate::remote::collect(true);
    if !status.configured {
        return (
            Vec::new(),
            Some("no remote build machine configured".into()),
        );
    }
    let Some(probe) = status.probe else {
        return (
            Vec::new(),
            Some(
                status
                    .error
                    .unwrap_or_else(|| "no session open to the build machine".into()),
            ),
        );
    };
    let mut versions: Vec<_> = probe
        .versions
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect();
    if versions.is_empty()
        && let Some(rustc) = probe.rustc
    {
        versions.push(("rustc".into(), rustc));
    }
    let note = (!probe.ok).then(|| {
        probe
            .error
            .unwrap_or_else(|| "the build machine did not answer".into())
    });
    (versions, note)
}

/// Network: the service's version document and the build machine's probe.
/// `last` keeps the previous answers visible when one of them fails.
pub(crate) fn fetch_versions(last: &Versions, service_ready: impl FnOnce(Versions)) -> Versions {
    fetch_versions_with(last, fetch_live, machine_versions, service_ready)
}

fn fetch_versions_with(
    last: &Versions,
    service: impl FnOnce() -> Result<Live>,
    machine: impl FnOnce() -> (Vec<(String, String)>, Option<String>) + Send,
    service_ready: impl FnOnce(Versions),
) -> Versions {
    std::thread::scope(|scope| {
        let machine = scope.spawn(machine);
        let mut versions = Versions::local();
        match service() {
            Ok(live) => versions.live = Some(live),
            Err(error) => {
                versions.live = last.live.clone();
                versions.error = Some(error.to_string());
            }
        }
        versions.machine = last.machine.clone();
        versions.machine_note = last.machine_note.clone();
        versions.cached = versions.error.is_some() && last.live.is_some();
        versions.at = if versions.error.is_none() {
            Some(crate::paths::now())
        } else {
            last.at
        };
        service_ready(versions.clone());
        (versions.machine, versions.machine_note) = machine
            .join()
            .unwrap_or_else(|_| (Vec::new(), Some("machine inventory worker failed".into())));
        if versions.machine.is_empty() && !last.machine.is_empty() {
            versions.machine = last.machine.clone();
        }
        versions
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn service_versions_publish_while_machine_inventory_is_blocked() {
        let (started, wait_started) = std::sync::mpsc::channel();
        let (published, wait_published) = std::sync::mpsc::channel();
        let timeout = std::time::Duration::from_secs(5);
        let versions = fetch_versions_with(
            &Versions::local(),
            || {
                wait_started
                    .recv_timeout(timeout)
                    .expect("machine runs concurrently");
                Ok(Live {
                    release: Some("1.2.3".into()),
                    ..Default::default()
                })
            },
            move || {
                started.send(()).unwrap();
                wait_published
                    .recv_timeout(timeout)
                    .expect("service visible before machine finishes");
                (vec![("rustc".into(), "machine-rustc".into())], None)
            },
            |versions| {
                assert_eq!(versions.live.unwrap().release.as_deref(), Some("1.2.3"));
                published.send(()).unwrap();
            },
        );
        assert_eq!(versions.machine("rustc"), Some("machine-rustc"));
    }

    #[test]
    fn failed_version_refresh_keeps_original_timestamp_and_cached_indicator() {
        let last = Versions {
            live: Some(Live::default()),
            at: Some(123.0),
            ..Default::default()
        };
        let versions = fetch_versions_with(
            &last,
            || bail!("offline"),
            || (Vec::new(), None),
            |partial| {
                assert!(partial.cached);
                assert_eq!(partial.at, Some(123.0));
            },
        );
        assert!(versions.cached);
        assert_eq!(versions.at, Some(123.0));
        assert_eq!(versions.error.as_deref(), Some("offline"));
    }

    #[test]
    fn reuse_ledger_is_independent_of_wall_and_excludes_credit_sidecars() {
        let rows = crate::reuse::parse(
            "loaded\t<cgu>\t0\t255\nchanged\t<cgu>\t0\t1\nloaded\tquery\t0.1\t17\neval_always_same\tquery\t1.5\t2\ngreen_recompute\tquery\t2\t1\nnew\tquery\t3\t1\nnohash\tquery\t4\t1\nloaded_avoided\tquery\t10\t1",
        );
        let units = serde_json::to_vec(&serde_json::json!({"reuse":rows})).unwrap();
        let (compile, stats) = unit_profiles(&units);
        assert!(compile.is_none());
        assert_eq!(
            stats.unwrap(),
            ReuseStats {
                ledgers: 1,
                query_observed: true,
                cgu_observed: true,
                cgu_reused: 255,
                cgu_total: 256,
                loaded: 17,
                same_secs: 1.5,
                green_secs: 2.,
                changed_secs: 3.,
                unknown_secs: 4.
            }
        );
        assert!(unit_profiles(b"{\"wall\":1}").1.is_none());
    }

    #[test]
    fn reuse_coverage_requires_rows_not_markers_or_credit_sidecars() {
        let profile = |text| {
            let mut stats = ReuseStats::default();
            stats.observe(&crate::reuse::parse(text));
            stats
        };
        let mut all = profile("first\t<session>\t0\t1\nloaded_avoided\tquery\t1\t1");
        assert!(!all.measured());
        let cgu = profile("new\t<cgu>\t0\t0");
        assert!(cgu.has_cgus() && !cgu.has_queries());
        all.merge(cgu);
        let query = profile("new\tquery\t0\t1");
        assert!(query.has_queries() && !query.has_cgus());
        all.merge(query);
        assert!(all.has_queries() && all.has_cgus());
        let mut legacy = serde_json::json!(all);
        let obj = legacy.as_object_mut().unwrap();
        obj.remove("query_observed");
        obj.remove("cgu_observed");
        let read = |v| serde_json::from_value::<ReuseStats>(v).unwrap();
        assert!(!read(legacy.clone()).measured());
        legacy["cgu_total"] = 9.into();
        let stats = read(legacy.clone());
        assert!(stats.has_cgus() && !stats.has_queries());
        legacy["changed_secs"] = 0.000_042.into();
        assert!(read(legacy).has_queries());
    }

    #[test]
    fn overall_v2_preserves_clock_domain_and_marks_unknown_causes() {
        let keys = [
            "host.compile",
            "build.tests",
            "sync.push",
            "remote.artifacts",
            "host.prepare",
            "host.cargo_startup",
            "host.lock_wait",
            "client.setup",
            "future.compile",
            "host.build_gaps",
            "build.tail",
            "host.other",
            "host.recorder",
            "remote.host_overhead",
            "remote.other",
        ];
        let mut s = crate::summary::Summary {
            wall: 70.,
            e2e: Some(crate::e2e::E2e {
                wall: 16.,
                segments: keys
                    .into_iter()
                    .map(|k| crate::e2e::Segment {
                        key: k.into(),
                        secs: 1.,
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            overall_v2(&s),
            [1., 1., 1., 1., 1., 1., 1., 1., 2., 1., 1., 1., 3.]
        );
        s.e2e = None;
        s.phases.compile = 60.;
        s.phases.build_gaps = 3.;
        s.phases.tail = 2.;
        assert_eq!(
            overall_v2(&s),
            [60., 0., 0., 0., 0., 0., 0., 0., 0., 3., 2., 5., 0.]
        );
        s.wall = 32.5;
        assert_eq!(overall_v2(&s).iter().sum::<f64>(), 32.5);
    }

    #[test]
    fn private_profile_matches_legacy_and_current_recorders() {
        let u = crate::shim::Unit {
            local: true,
            wall: 1.125,
            opt_level: "2".into(),
            reuse: crate::reuse::parse("green_recompute\tprivate_query\t0.125\t1"),
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&u).unwrap();
        let (mut old, level) = DashboardProfile::read_returned(bytes.as_slice());
        assert_eq!(level.as_deref(), Some("2"));
        let new = DashboardProfile::recorded(&[u]);
        assert_eq!(serde_json::json!(old), serde_json::json!(new));
        let s = crate::summary::Summary {
            wall: 3.,
            ..Default::default()
        };
        old.set_overall(&s);
        let json = serde_json::json!(old);
        assert_eq!(
            json["overall"],
            serde_json::json!([0., 0., 0., 0., 0., 0., 0., 0., 3.])
        );
        assert_eq!(
            json["overall_v2"],
            serde_json::json!([0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 3., 0.])
        );
        assert_eq!(
            json["compile"],
            serde_json::json!([1.125, 0., 0., 0., 0., 0.])
        );
        assert_eq!(json["reuse"]["green_secs"], 0.125);
        assert!(!json.to_string().contains("private_query"));
        assert_eq!(
            serde_json::json!(DashboardProfile::default()),
            serde_json::json!({})
        );
    }

    #[test]
    fn timeline_aligns_only_in_the_recorders_clock_domain() {
        let summary = br#"{"start":100,"wall":6,"tests":{"binaries":[{"name":"suite","start":0,"wall":6}]},"e2e":{"wall":12,"segments":[],"trace":[[0,2,"client.setup"],[2,8,"build.window"],[10,12,"client.report"]]}}"#;
        let units = br#"{"start":99,"end":103}
{"wall":10}
{"start":107,"end":108}"#;
        let procs = br#"[{"pid":1,"comm":"suite","cmdline":"target/debug/deps/suite --exact foo","kind":"target-binary","first_seen":102,"last_seen":104,"cpu_secs":0,"max_rss_mb":0}]"#;
        let local = timeline_profiles(summary, units, true, || procs.to_vec())
            .2
            .unwrap();
        assert!(local.end_to_end);
        assert_eq!(local.wall, 12.);
        assert_eq!(
            local.spans,
            [
                (0., 2., Bucket::Client),
                (2., 5., Bucket::Compile),
                (4., 6., Bucket::Tests),
                (10., 12., Bucket::Client)
            ]
        );
        let hosted = timeline_profiles(summary, units, false, || procs.to_vec())
            .2
            .unwrap();
        assert_eq!(
            hosted.cargo.unwrap().spans,
            [(0., 3., Bucket::Compile), (2., 4., Bucket::Tests)]
        );
        assert_eq!(
            hosted.spans,
            [(0., 2., Bucket::Client), (10., 12., Bucket::Client)]
        );
    }

    #[test]
    fn legacy_timeline_uses_cargo_wall_not_reconstructed_segments() {
        let summary = br#"{"start":100,"wall":5,"e2e":{"wall":20,"segments":[{"key":"sync.push","secs":15}]}}"#;
        let t = timeline_profiles(summary, br#"{"start":103,"end":110}"#, false, Vec::new)
            .2
            .unwrap();
        assert!(!t.end_to_end);
        assert_eq!((t.wall, t.spans), (5., vec![(3., 5., Bucket::Compile)]));
        assert!(
            timeline_profiles(summary, br#"{"wall":4}"#, false, Vec::new)
                .2
                .is_none()
        );
    }

    #[test]
    fn opt_level_uses_observed_units_not_profile_defaults() {
        let measured = |units: &[u8]| profile(units, |_, _| {}).2;
        assert_eq!(measured(br#"{"wall":3}"#), None);
        assert_eq!(measured(br#"{"opt_level":"unknown"}"#), None);
        for level in ["0", "1", "2", "3", "s", "z"] {
            let unit = serde_json::json!({"opt_level": level});
            assert_eq!(
                measured(format!("{unit}\n{unit}").as_bytes()).as_deref(),
                Some(level)
            );
        }
        assert_eq!(
            measured(b"{\"opt_level\":\"0\"}\n{\"opt_level\":\"z\"}\n{\"opt_level\":\"0\"}")
                .as_deref(),
            Some("mixed")
        );
    }

    #[test]
    fn cached_profile_rows_need_no_summary_or_unit_files() {
        let time = Some([1.0; BUCKETS]);
        let compile = Some([2.0; PARTS]);
        let mut rows = vec![RunRow {
            id: "saved".into(),
            time,
            compile,
            opt_level: Some("s".into()),
            cargo_units: Some((4, 2)),
            ..Default::default()
        }];
        let mut cache = TimeCache::from([
            (
                "saved".into(),
                (
                    time,
                    compile,
                    None,
                    None,
                    Some("s".into()),
                    None,
                    Ledger::Complete,
                    Some((4, 2)),
                    None,
                ),
            ),
            (
                "expired".into(),
                (
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Ledger::Unknown,
                    None,
                    None,
                ),
            ),
        ]);
        enrich_rows(
            &mut rows,
            &mut cache,
            Path::new("a-dashboard-fixture-with-no-run-files"),
        );
        assert_eq!(rows[0].time, time);
        assert_eq!(rows[0].compile, compile);
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache["saved"],
            (
                time,
                compile,
                None,
                None,
                Some("s".into()),
                None,
                Ledger::Complete,
                Some((4, 2)),
                None,
            )
        );
    }

    #[test]
    fn delayed_ledger_recovers_after_restore_without_rereading_unchanged_files() {
        use std::{
            fs,
            time::{Duration, Instant},
        };
        let dir = std::env::temp_dir().join(format!(
            "jr-ledger-fixture-{}-{}",
            std::process::id(),
            crate::paths::now()
        ));
        let run = dir.join("saved");
        fs::create_dir_all(&run).unwrap();
        let units = run.join("units.jsonl");
        let now = Instant::now();
        let mut rows = [RunRow {
            id: "saved".into(),
            toolchain_version: Some("new".into()),
            ..Default::default()
        }];
        let mut cache = TimeCache::from([(
            "saved".into(),
            (
                None,
                None,
                None,
                None,
                None,
                Some("old".into()),
                cached_ledger(&rows[0]),
                None,
                None,
            ),
        )]);
        let mut refresh = |second, rows: &mut [RunRow]| {
            enrich_rows_at(rows, &mut cache, &dir, now + Duration::from_secs(second));
            cache["saved"].6
        };
        assert!(matches!(refresh(0, &mut rows), Ledger::Probe(_, None)));
        fs::write(&units, b"").unwrap();
        assert!(matches!(refresh(2, &mut rows), Ledger::Probe(_, None)));
        assert!(matches!(
            refresh(10, &mut rows),
            Ledger::Probe(_, Some((0, _)))
        ));
        fs::write(&units, b"invalid").unwrap();
        fs::write(
            run.join("summary.json"),
            br#"{"toolchain":"old","units":{"fresh":3,"compiled":0}}"#,
        )
        .unwrap();
        assert!(matches!(
            refresh(20, &mut rows),
            Ledger::Probe(_, Some((7, _)))
        ));
        assert!(matches!(
            refresh(30, &mut rows),
            Ledger::Probe(_, Some((7, _)))
        ));
        fs::remove_file(&units).unwrap();
        assert!(matches!(refresh(40, &mut rows), Ledger::Probe(_, None)));
        // A directory at the ledger path has metadata but cannot be read as JSONL.
        fs::create_dir(&units).unwrap();
        assert!(matches!(refresh(50, &mut rows), Ledger::Probe(_, None)));
        fs::remove_dir(&units).unwrap();
        fs::write(&units, br#"{"crate_types":["lib"],"wall":2.0}"#).unwrap();
        assert_eq!(refresh(60, &mut rows), Ledger::Complete);
        assert!(rows[0].compile.is_some());
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(refresh(70, &mut rows), Ledger::Complete);
        assert_eq!(rows[0].cargo_units, Some((3, 0)));
        assert_eq!(rows[0].toolchain_version.as_deref(), Some("new"));
    }

    #[test]
    fn hosted_requests_overlap_and_account_is_published_before_history() {
        let (started, wait_started) = std::sync::mpsc::channel();
        let (ready, wait_ready) = std::sync::mpsc::channel();
        let timeout = std::time::Duration::from_secs(5);
        let snapshot = fetch_hosted_with(
            || {
                wait_started
                    .recv_timeout(timeout)
                    .expect("history starts before account completes");
                Ok(crate::remote_hosted::Me {
                    email: Some("account".into()),
                    ..Default::default()
                })
            },
            move || {
                started.send(()).unwrap();
                wait_ready
                    .recv_timeout(timeout)
                    .expect("account is visible before history completes");
                Ok(vec![RunRow {
                    id: "history".into(),
                    ..Default::default()
                }])
            },
            |account| {
                assert_eq!(account.email.as_deref(), Some("account"));
                ready.send(()).unwrap();
            },
        )
        .unwrap();
        assert_eq!(snapshot.runs[0].id, "history");
        assert!(snapshot.history_error.is_none());
    }

    #[test]
    fn hosted_history_failure_still_publishes_account() {
        let mut published = false;
        let snapshot = fetch_hosted_with(
            || Ok(Default::default()),
            || bail!("history offline"),
            |_| published = true,
        )
        .unwrap();
        assert!(published);
        assert!(snapshot.runs.is_empty());
        assert_eq!(snapshot.history_error.as_deref(), Some("history offline"));
    }

    #[test]
    fn hosted_account_failure_does_not_publish_unvalidated_account() {
        let result = fetch_hosted_with(
            || bail!("account offline"),
            || Ok(Vec::new()),
            |_| panic!("failed account must not be published"),
        );
        assert_eq!(result.unwrap_err().to_string(), "account offline");
    }

    #[test]
    fn versions_compare_the_client_with_what_the_service_names() {
        let doc = r#"{"service":{"id":"f0daab20-248d","deployed_at":"2026-11-13T21:31:40Z"},
            "release":{"version":"0.1.0-alpha.26","held":true,"auto_update":false},
            "toolchain":{"toolchain":"jr-ac20de5e7921","protocol":1,"published":true}}"#;
        let l = live(doc).unwrap();
        assert_eq!(l.release.as_deref(), Some("0.1.0-alpha.26"));
        assert!(l.held && !l.auto_update && l.published);
        assert_eq!(l.toolchain.as_deref(), Some("jr-ac20de5e7921"));
        assert_eq!(l.protocol, Some(1));
        // Missing parts are unknown, not an error. Another document is.
        let bare =
            live(r#"{"service":{"id":null},"release":{"version":null},"toolchain":null}"#).unwrap();
        assert_eq!(
            (bare.release, bare.toolchain, bare.protocol),
            (None, None, None)
        );
        assert!(bare.auto_update);
        assert!(live(r#"{"error":{"code":"not_found"}}"#).is_none());
        assert!(live("<html>").is_none());

        let mut v = Versions {
            client: "0.1.0-alpha.26".into(),
            release_build: true,
            service: "build.jcode.sh".into(),
            ..Default::default()
        };
        assert_eq!(v.currency(), Currency::Unknown);
        assert_eq!(v.drives_toolchain(), None);
        v.live = Some(l);
        assert_eq!(v.currency(), Currency::Current);
        assert_eq!(v.drives_toolchain(), Some(true));
        v.client = "0.1.0-alpha.9".into();
        assert_eq!(v.currency(), Currency::Behind("0.1.0-alpha.26".into()));
        v.client = "0.1.0".into();
        assert_eq!(v.currency(), Currency::Ahead);
        v.machine = vec![("rustc".into(), "rustc 1.99.1".into())];
        v.machine_note = Some("no session open".into());
        let report = v.report();
        for want in [
            "justrust 0.1.0 (published release)",
            "newer than the release",
            "Service (build.jcode.sh)",
            "0.1.0-alpha.26 (rollout held here)",
            "off for every client",
            "protocol    1 (supported by this client)",
            "rustc       rustc 1.99.1",
            "no session open",
        ] {
            assert!(report.contains(want), "missing {want:?} in\n{report}");
        }
        let local = Versions::local();
        assert_eq!(local.client, crate::update::VERSION);
        assert!(!local.service.contains('/'), "{}", local.service);
    }

    #[test]
    fn personal_inventory_includes_control_plane_and_every_machine_component() {
        let metadata = live(r#"{"service":{"id":"deploy-future","tag":"build-future"},"release":{"version":"9.0.0"},"pool_bundle":"sha256-future","toolchain_dist":true}"#).unwrap();
        assert_eq!(metadata.deployment_tag.as_deref(), Some("build-future"));
        assert_eq!(metadata.pool_bundle.as_deref(), Some("sha256-future"));
        assert_eq!(metadata.toolchain_dist, Some(true));
        let inventory = Versions {
            live: Some(metadata),
            machine: [
                "rustc",
                "llvm",
                "cargo",
                "jr-cargo",
                "wild",
                "agent",
                "cc",
                "clang",
                "mold",
                "ld.lld",
                "ld",
                "os",
                "kernel",
                "future-component",
            ]
            .into_iter()
            .map(|name| (name.into(), format!("{name}-version-future")))
            .collect(),
            ..Default::default()
        };
        let report = inventory.report();
        assert!(report.starts_with("Justrust Personal Dashboard · Versions"));
        for expected in [
            "build-future",
            "sha256-future",
            "distribution enabled",
            "not the whole fleet",
        ] {
            assert!(report.contains(expected), "{report}");
        }
        for (_, version) in &inventory.machine {
            assert!(report.contains(version), "missing {version} in {report}");
        }
        let old = live(r#"{"service":{},"release":{},"toolchain":null}"#).unwrap();
        assert_eq!(old.pool_bundle, None);
        assert_eq!(old.toolchain_dist, None);
        assert!(
            Versions::default()
                .report()
                .contains("not been reported yet")
        );
    }

    #[test]
    fn defaults_are_simple_empty_snapshots() {
        let snapshot = Snapshot::default();
        assert!(snapshot.active.is_empty() && snapshot.runs.is_empty());
        assert!(snapshot.account.is_none() && snapshot.hosted_error.is_none());
        assert!(snapshot.local_error.is_none() && snapshot.hosted_at.is_none());
        assert!(HostedSnapshot::default().runs.is_empty());
        assert_eq!(RunRow::default().exit, None);
    }

    #[test]
    fn segments_group_into_buckets_that_sum_to_the_wait() {
        for (key, bucket) in [
            ("host.compile", Bucket::Compile),
            ("build.compile", Bucket::Compile),
            ("host.tests", Bucket::Tests),
            ("sync.push", Bucket::Sync),
            ("sync.lock_wait", Bucket::Sync),
            ("host.lock_wait", Bucket::Lock),
            ("remote.artifacts", Bucket::Artifacts),
            ("remote.admission", Bucket::Hosted),
            ("host.records", Bucket::Hosted),
            ("build.cargo_startup", Bucket::Startup),
            ("client.route", Bucket::Client),
            ("remote.other", Bucket::Other),
            ("host.build_gaps", Bucket::Other),
            ("new.unknown", Bucket::Other),
            ("nodot", Bucket::Other),
        ] {
            assert_eq!(Bucket::of(key), bucket, "{key}");
        }
        assert_eq!(Bucket::ALL.map(|b| b as usize), [0, 1, 2, 3, 4, 5, 6, 7, 8]);

        let e2e = serde_json::json!({"wall": 99.0, "e2e": {"wall": 10.0, "segments": [
            {"key": "client.route", "secs": 1.0},
            {"key": "sync.push", "secs": 2.0},
            {"key": "sync.link", "secs": 0.5},
            {"key": "host.compile", "secs": 6.0},
            {"key": "host.tests", "secs": 0.5},
            {"key": "host.tail", "secs": -3.0}
        ]}});
        let t = times(e2e.to_string().as_bytes()).unwrap();
        assert_eq!(t[Bucket::Sync as usize], 2.5);
        assert_eq!(t[Bucket::Compile as usize], 6.0);
        assert_eq!(t.iter().sum::<f64>(), 10.0);

        // Without end-to-end segments, cargo phases plus the unattributed rest.
        let phases = serde_json::json!({"wall": 10.0, "phases": {"startup": 1.0,
            "lock_wait": 0.0, "compile": 6.0, "build_gaps": 0.5, "test_run": 1.0, "tail": 0.5}});
        let t = times(phases.to_string().as_bytes()).unwrap();
        assert_eq!(t[Bucket::Startup as usize], 1.0);
        assert_eq!(t[Bucket::Other as usize], 2.0);
        assert_eq!(t.iter().sum::<f64>(), 10.0);
        assert!(times(b"{\"wall\": 3}").is_none());
        assert!(times(b"not json").is_none());
    }

    #[test]
    fn compile_times_group_units_by_what_was_compiled() {
        let units = [
            r#"{"crate_types":["lib"],"local":true,"wall":4.0,"depcache":"local-miss"}"#,
            r#"{"crate_types":["bin"],"local":true,"test":true,"wall":3.0}"#,
            r#"{"crate_types":["lib"],"wall":2.0,"depcache":"miss"}"#,
            r#"{"crate_types":["proc-macro"],"wall":1.5}"#,
            r#"{"crate_types":["bin"],"build_script":true,"wall":0.5}"#,
            r#"{"crate_types":["lib"],"wall":0.25,"depcache":"prebuilt"}"#,
            r#"{"crate_types":["lib"],"local":true,"wall":0.25,"depcache":"local"}"#,
            r#"{"crate_types":["lib"],"wall":-1.0}"#,
            "torn row",
        ]
        .join("\n");
        let t = compile_times(units.as_bytes()).unwrap();
        assert_eq!(t[Part::Workspace as usize], 4.0);
        assert_eq!(t[Part::Tests as usize], 3.0);
        assert_eq!(t[Part::Dependencies as usize], 2.0);
        assert_eq!(t[Part::ProcMacros as usize], 1.5);
        assert_eq!(t[Part::BuildScripts as usize], 0.5);
        assert_eq!(t[Part::Restores as usize], 0.5);
        assert_eq!(Part::ALL.map(|p| p as usize), [0, 1, 2, 3, 4, 5]);
        assert!(compile_times(b"").is_none());
        assert!(compile_times(b"{}\nnot json").is_none());
    }

    #[test]
    fn index_and_hosted_history_preserve_each_runs_versions() {
        let entry = crate::summary::IndexEntry {
            id: "versioned".into(),
            toolchain_version: Some("0.4.2".into()),
            client_version: Some("0.2.7".into()),
            ..Default::default()
        };
        let summary = serde_json::to_value(entry).unwrap();
        let local = index_rows(&summary.to_string()).pop().unwrap();
        let hosted = hosted_row(&serde_json::json!({"id":"versioned", "summary":summary})).unwrap();
        for row in [local, hosted] {
            assert_eq!(row.toolchain_version.as_deref(), Some("0.4.2"));
            assert_eq!(row.client_version.as_deref(), Some("0.2.7"));
        }
        let old = hosted_row(&serde_json::json!({"id":"old"})).unwrap();
        assert_eq!((old.toolchain_version, old.client_version), (None, None));
    }

    #[test]
    fn failure_class_comes_only_from_the_record() {
        let entry = crate::summary::IndexEntry {
            id: "run-1".into(),
            exit: 101,
            compile_failed: true,
            failure: Some(crate::failure::Class::UserCode),
            ..Default::default()
        };
        let rows = index_rows(&(serde_json::to_string(&entry).unwrap() + "\n"));
        assert_eq!(rows[0].failure, Some(crate::failure::Class::UserCode));
        assert!(rows[0].compile_failed && rows[0].time.is_none());
        let hosted =
            hosted_row(&serde_json::json!({"id":"c", "summary":{"exit":1, "failure":"infra"}}))
                .unwrap();
        assert_eq!(hosted.failure, Some(crate::failure::Class::Infra));
        let unknown =
            hosted_row(&serde_json::json!({"id":"c", "summary":{"exit":1, "failure":"new_class"}}))
                .unwrap();
        assert_eq!(unknown.failure, None);
        assert!(!unknown.compile_failed);
    }

    #[test]
    fn completed_locations_preserve_hosted_remote_local_and_legacy_provenance() {
        for (place, machine, expected) in [
            (Some("hosted"), Some("fleet-1"), "hosted"),
            (Some("remote"), Some("ssh:personal"), "remote"),
            (Some("local"), None, "local"),
            (None, Some("hosted"), "hosted"),
            (None, Some("ssh:hosted"), "remote"),
            (None, None, "local"),
        ] {
            let entry = crate::summary::IndexEntry {
                id: "provenance-run".into(),
                place: place.map(str::to_owned),
                remote: machine.map(str::to_owned),
                ..Default::default()
            };
            let rows = index_rows(&serde_json::to_string(&entry).unwrap());
            assert_eq!(rows[0].place, expected);
            assert_eq!(
                rows[0].origin, "local",
                "record storage is not execution location"
            );
            let row = hosted_row(&serde_json::json!({"id":"cloud-run", "machine":"client-laptop", "summary":{"place":expected}})).unwrap();
            assert_eq!(row.place, expected);
            assert_eq!(row.origin, "hosted");
        }
    }

    #[test]
    fn index_tail_is_newest_first_and_bounded() {
        let mut tail = String::new();
        for i in 0..250 {
            let entry = crate::summary::IndexEntry {
                id: format!("run-{i}"),
                ..Default::default()
            };
            tail.push_str(&serde_json::to_string(&entry).unwrap());
            tail.push('\n');
        }
        let rows = index_rows(&tail);
        assert_eq!(rows.len(), INDEX_ROWS);
        assert_eq!(rows[0].id, "run-249");
        assert_eq!(rows[199].id, "run-50");
        assert_eq!(rows[0].origin, "local");
        assert_eq!(rows[0].exit, Some(0));
        assert!(index_rows("broken\n{\"id\":").is_empty());
    }

    #[test]
    fn bounded_tail_discards_partial_record() {
        let bytes = b"first\nsecond\nthird\n";
        let tail = read_tail(&mut Cursor::new(bytes), bytes.len() as u64, 10).unwrap();
        assert_eq!(tail, "third\n");
        assert_eq!(
            read_tail(&mut Cursor::new(bytes), bytes.len() as u64, 2).unwrap(),
            ""
        );
        assert_eq!(
            read_tail(&mut Cursor::new(bytes), bytes.len() as u64, 100).unwrap(),
            String::from_utf8_lossy(bytes)
        );
    }

    #[test]
    fn metadata_does_not_fabricate_summary_fields() {
        let row = hosted_row(
            &serde_json::json!({"id":"run-1", "at":123, "machine":"laptop", "bytes":100}),
        )
        .unwrap();
        assert_eq!(row.start, 123.0);
        assert_eq!(row.machine.as_deref(), Some("laptop"));
        assert!(row.args.is_empty() && row.cwd.is_empty());
        assert_eq!(row.exit, None);
        assert_eq!(row.place, "unknown");
        assert_eq!(row.origin, "hosted");
    }

    #[test]
    fn account_list_reuse_counts_only_measured_rows_after_local_dedup() {
        use serde_json::json;
        let base = json!({"ledgers":1,"loaded":0,"cgu_total":0,"cgu_reused":0,
            "same_secs":0,"green_secs":0,"changed_secs":0,"unknown_secs":0});
        let record = |id, reuse| {
            hosted_row(&json!({"id":id,"at":123,"summary":{
            "args":["test","--lib"],"exit":0,"wall":3.5,"place":"hosted",
            "dashboard_profile":{"reuse":reuse}}}))
            .unwrap()
        };
        let mut cgu = base.clone();
        cgu["cgu_total"] = json!(8);
        cgu["cgu_reused"] = json!(7);
        let mut zero_query = base.clone();
        zero_query["query_observed"] = json!(true);
        zero_query["cgu_observed"] = json!(false);
        let mut state = super::super::State {
            hosted_runs: vec![
                record("cgu", cgu.clone()),
                record("query", zero_query),
                record("legacy-zero", base),
                record("missing", Value::Null),
                record("both", cgu),
            ],
            ..Default::default()
        };
        let mut local = state.hosted_runs.last().unwrap().clone();
        local.origin = "local".into();
        local.reuse = Some(ReuseStats {
            loaded: 13,
            ledgers: 1,
            ..Default::default()
        });
        state.snapshot.runs.push(local);
        let rows = state.rows();
        let measured: Vec<_> = rows.iter().filter_map(|r| r.reuse).collect();
        assert_eq!((rows.len(), measured.len()), (5, 3));
        assert_eq!(measured.iter().filter(|r| r.has_queries()).count(), 2);
        assert_eq!(measured.iter().filter(|r| r.has_cgus()).count(), 1);
        assert_eq!(measured.iter().map(|r| r.loaded).sum::<u64>(), 13);
        assert_eq!(measured.iter().map(|r| r.cgu_reused).sum::<u64>(), 7);
    }

    #[test]
    fn account_list_rejects_malformed_reuse_without_fabricating_zeros() {
        use serde_json::json;
        let base = json!({"ledgers":1,"loaded":1,"cgu_total":2,"cgu_reused":1,
            "same_secs":0,"green_secs":0.25,"changed_secs":0,"unknown_secs":0});
        for (key, bad) in [
            ("ledgers", json!(0)),
            ("loaded", json!(0.5)),
            ("cgu_reused", json!(3)),
            ("same_secs", json!(-1)),
            ("changed_secs", json!(1.1e12)),
            ("unknown_secs", json!("0")),
            ("loaded", json!(1_000_000_000_001u64)),
            ("cgu_observed", json!(1)),
            ("query_observed", Value::Null),
        ] {
            let mut invalid = base.clone();
            invalid[key] = bad;
            assert_eq!(hosted_reuse(&invalid), None, "{key}");
        }
        let mut partial = base.clone();
        partial.as_object_mut().unwrap().remove("loaded");
        assert_eq!(hosted_reuse(&partial), None);
        assert!(
            hosted_row(&json!({"id":"legacy","summary":{"reuse":base}}))
                .unwrap()
                .reuse
                .is_none()
        );
    }

    #[test]
    fn hosted_cargo_counts_require_two_explicit_unsigned_integers() {
        use serde_json::json;
        for (units, expected) in [
            (json!({"fresh":0,"compiled":0}), Some((0, 0))),
            (json!({"fresh":6,"compiled":2}), Some((6, 2))),
            (json!({"compiled":2}), None),
            (json!({"fresh":3,"compiled":-1}), None),
            (json!({"fresh":"3","compiled":2}), None),
            (Value::Null, None),
        ] {
            let row = hosted_row(&json!({"id":"cloud", "summary":{"units":units}})).unwrap();
            assert_eq!(row.cargo_units, expected);
        }
    }

    #[test]
    fn compact_hosted_summary_reports_only_explicit_values() {
        let row = hosted_row(&serde_json::json!({"id":"cloud-1", "at":999, "summary": {"start":123, "cwd":"/work/project", "args":["test","--lib"], "exit":101, "wall":4.5, "place":"hosted"}})).unwrap();
        assert_eq!(row.start, 123.0);
        assert_eq!(row.exit, Some(101));
        assert_eq!(row.args, ["test", "--lib"]);
        assert_eq!(row.wall, 4.5);
        assert_eq!(row.place, "hosted");
        let unknown = hosted_row(&serde_json::json!({"id":"cloud-2", "at":999, "summary":{"exit":null, "wall":null, "place":null}})).unwrap();
        assert_eq!(unknown.exit, None);
        assert_eq!(unknown.place, "unknown");
    }

    #[test]
    fn unsafe_ids_never_reach_files_or_api_paths() {
        for id in [
            "",
            ".",
            "..",
            "../secret",
            "foo/bar",
            "foo?key=x",
            "foo\\bar",
            "foo\n",
        ] {
            assert!(!valid_id(id), "{id:?}");
        }
        assert!(valid_id("20261111-1234.remote_1"));
        assert!(hosted_row(&serde_json::json!({"id":"../secret"})).is_none());
    }

    #[test]
    fn log_jsonl_extracts_text_and_preserves_non_json() {
        assert_eq!(
            log_text("{\"t\":1,\"s\":\"e\",\"l\":\"error: example\"}\nraw line\n"),
            "error: example\nraw line"
        );
        let input = "line\n".repeat(LOG_LINES + 10);
        assert_eq!(log_text(&input).lines().count(), LOG_LINES);
    }
}
