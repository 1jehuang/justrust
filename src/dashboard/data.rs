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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub(crate) const BUCKETS: usize = 9;

/// Seconds per [`Bucket`], indexed by `Bucket as usize`. Sums to the wait.
pub(crate) type Times = [f64; BUCKETS];

/// Parsed timings by run ID. A finished run's summary never changes, so each
/// is read once. `None` records a summary that could not be read.
pub(crate) type TimeCache =
    std::collections::HashMap<String, (Option<Times>, Option<CompileTimes>)>;

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
fn compile_times(units: &[u8]) -> Option<CompileTimes> {
    let mut out = [0.0; PARTS];
    let mut any = false;
    for row in units.split(|byte| *byte == b'\n') {
        let Ok(unit) = serde_json::from_slice::<UnitTime>(row) else {
            continue;
        };
        if !unit.wall.is_finite() || unit.wall <= 0.0 {
            continue;
        }
        let part = if matches!(
            unit.depcache.as_str(),
            "hit" | "remote" | "peer" | "prebuilt" | "local"
        ) {
            Part::Restores
        } else if unit.build_script {
            Part::BuildScripts
        } else if unit.crate_types.iter().any(|t| t == "proc-macro") {
            Part::ProcMacros
        } else if unit.test {
            Part::Tests
        } else if unit.local {
            Part::Workspace
        } else {
            Part::Dependencies
        };
        out[part as usize] += unit.wall;
        any = true;
    }
    any.then_some(out)
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
            ("host", "prepare" | "recorder" | "finalize" | "records") => Bucket::Hosted,
            _ => Bucket::Other,
        }
    }
}

/// Only the timing fields of a run summary, so older or newer summaries with
/// other shapes elsewhere still parse.
#[derive(serde::Deserialize)]
struct Timing {
    #[serde(default)]
    wall: f64,
    #[serde(default)]
    phases: Option<crate::summary::Phases>,
    #[serde(default)]
    e2e: Option<crate::e2e::E2e>,
}

/// End-to-end segments when recorded, otherwise the coarser cargo phases with
/// the unattributed remainder of the wall time as `Other`.
fn times(summary: &[u8]) -> Option<Times> {
    let timing: Timing = serde_json::from_slice(summary).ok()?;
    let mut out = [0.0; BUCKETS];
    let mut add = |bucket: Bucket, secs: f64| {
        if secs.is_finite() && secs > 0.0 {
            out[bucket as usize] += secs;
        }
    };
    match timing.e2e.filter(|e2e| !e2e.segments.is_empty()) {
        Some(e2e) => {
            for segment in &e2e.segments {
                add(Bucket::of(&segment.key), segment.secs);
            }
        }
        None => {
            let p = timing.phases?;
            add(Bucket::Startup, p.startup);
            add(Bucket::Lock, p.lock_wait);
            add(Bucket::Compile, p.compile);
            add(Bucket::Tests, p.test_run);
            add(Bucket::Other, p.build_gaps + p.tail);
            let attributed =
                p.startup + p.lock_wait + p.compile + p.test_run + p.build_gaps + p.tail;
            add(Bucket::Other, timing.wall - attributed);
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

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RunRow {
    pub id: String,
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
    /// Where the wait went. Only for runs whose summary is on this client.
    pub time: Option<Times>,
    /// What the compiler's time went to. Only for runs with units recorded here.
    pub compile: Option<CompileTimes>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct HostedSnapshot {
    pub account: crate::remote_hosted::Me,
    pub runs: Vec<RunRow>,
    pub history_error: Option<String>,
}

/// An independent local update. The caller retains last-good hosted data.
pub(crate) fn local_snapshot(cache: &mut TimeCache) -> Snapshot {
    let mut snapshot = Snapshot::default();
    match crate::status::collect() {
        Ok(status) => snapshot.active = status.active,
        Err(error) => snapshot.local_error = Some(format!("Active builds: {error:#}")),
    }
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
            if let Ok(dir) = crate::paths::runs_dir() {
                for row in &mut rows {
                    (row.time, row.compile) = *cache.entry(row.id.clone()).or_insert_with(|| {
                        let run = dir.join(&row.id);
                        (
                            read_summary(&run.join("summary.json"))
                                .ok()
                                .and_then(|bytes| times(&bytes)),
                            read_summary(&run.join("units.jsonl"))
                                .ok()
                                .and_then(|bytes| compile_times(&bytes)),
                        )
                    });
                }
                cache.retain(|id, _| rows.iter().any(|row| &row.id == id));
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
    snapshot
}

/// At most two authenticated GETs, regardless of how many runs are listed.
/// Errors propagate so the caller can keep its previous successful snapshot.
pub(crate) fn fetch_hosted() -> Result<HostedSnapshot> {
    let credentials = crate::remote_hosted::creds()?;
    let account = crate::remote_hosted::me(&credentials)?;
    let history = (|| -> Result<Vec<RunRow>> {
        let records = get(&credentials, "build/records?kind=run&summary=1&limit=50")?;
        let rows = records["records"]
            .as_array()
            .context("hosted records response is missing records array")?;
        Ok(rows.iter().take(50).filter_map(hosted_row).collect())
    })();
    let (runs, history_error) = match history {
        Ok(runs) => (runs, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Ok(HostedSnapshot {
        account,
        runs,
        history_error,
    })
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
    tail.lines()
        .rev()
        .take(INDEX_ROWS)
        .filter_map(|line| serde_json::from_str::<crate::summary::IndexEntry>(line).ok())
        .filter(|entry| valid_id(&entry.id))
        .map(|entry| RunRow {
            id: entry.id,
            start: entry.start,
            cwd: entry.cwd,
            args: entry.args,
            exit: Some(entry.exit),
            wall: entry.wall,
            place: if entry.remote.is_some() {
                "remote"
            } else {
                "local"
            }
            .into(),
            machine: entry.remote,
            origin: "local".into(),
            failure: entry.failure,
            compile_failed: entry.compile_failed,
            time: None,
            compile: None,
        })
        .collect()
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
        compile_failed: summary["compile_failed"].as_bool().unwrap_or(false),
        time: None,
        compile: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
