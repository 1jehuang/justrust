//! Mine Jcode session files for cargo invocations and their cost.
//!
//! A session file is JSON with `working_dir`, `id`, and `messages`. Each message
//! has a `timestamp` and `content` blocks. A `tool_use` block that runs cargo
//! (directly, through `selfdev`/`desktop_selfdev`, or inside a `batch`) is
//! paired with its `tool_result`. Wall time comes from the `[tool timing: ...]`
//! header when present, otherwise from the message timestamp difference.

use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize)]
pub struct Call {
    pub session: String,
    pub working_dir: String,
    pub commands: Vec<String>,
    pub kind: Kind,
    pub timestamp: f64,
    pub wall_secs: Option<f64>,
    pub background: bool,
    /// Seconds reported by cargo's `Finished ... in Xs` lines.
    pub cargo_finished_secs: Vec<f64>,
    /// Seconds reported by libtest `finished in Xs` lines.
    pub test_exec_secs: f64,
    pub tests_run: u64,
    /// Crates cargo reported as `Compiling` or `Checking`.
    pub rebuilt_crates: Vec<String>,
    pub lock_blocked: bool,
    pub compile_errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Kind {
    Test,
    Check,
    Build,
    Fmt,
    Other,
}

static FINISHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Finished `?\w+`? profile \[[^\]]*\] target\(s\) in (?:(\d+)m )?([\d.]+)(m?s)")
        .unwrap()
});
static TEST_RESULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"test result: \w+\. (\d+) passed; (\d+) failed;.*finished in ([\d.]+)s").unwrap()
});
static REBUILT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:Compiling|Checking) (\S+) v").unwrap());
static TIMING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"tool timing: start=\S+ finish=\S+ duration=([\d.]+)(ms|s)").unwrap()
});
static ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^error(\[E\d+\])?:").unwrap());
static CARGO_SUB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"cargo\s+(?:\+\S+\s+)?(?:-\S+\s+)*([a-z-]+)").unwrap());
static PKG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-p\s+(\S+)").unwrap());

fn classify(cmd: &str) -> Kind {
    let mut kind = Kind::Other;
    for cap in CARGO_SUB.captures_iter(cmd) {
        let k = match &cap[1] {
            "test" | "nextest" => Kind::Test,
            "check" | "clippy" => Kind::Check,
            "build" | "run" => Kind::Build,
            "fmt" => Kind::Fmt,
            _ => continue,
        };
        kind = kind.min(k);
    }
    if kind == Kind::Other && cmd.starts_with("<desktop_selfdev:test>") {
        kind = Kind::Test;
    }
    if kind == Kind::Other && cmd.starts_with("<desktop_selfdev:build") {
        kind = Kind::Build;
    }
    kind
}

/// A short, stable label for grouping similar invocations.
pub fn pattern(cmd: &str) -> String {
    let Some(cap) = CARGO_SUB.captures(cmd) else {
        return cmd.chars().take(40).collect();
    };
    let start = cap.get(0).unwrap().end();
    let rest = &cmd[start..];
    let rest = rest.split(['|', ';', '&', '>']).next().unwrap_or("");
    let mut pkgs: Vec<&str> = PKG
        .captures_iter(rest)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    pkgs.dedup();
    let mut out = format!("cargo {}", &cap[1]);
    for p in pkgs {
        out.push_str(" -p ");
        out.push_str(p);
    }
    for flag in [
        "--workspace",
        "--release",
        "--all-targets",
        "--lib",
        "--tests",
    ] {
        if rest.split_whitespace().any(|w| w == flag) {
            out.push(' ');
            out.push_str(flag);
        }
    }
    if rest.contains("--profile selfdev") {
        out.push_str(" --profile selfdev");
    }
    out
}

fn tool_command(name: &str, input: &Value) -> Option<String> {
    match name {
        "Bash" | "bash" => input.get("command")?.as_str().map(str::to_owned),
        "desktop_selfdev" | "selfdev" => {
            let action = input.get("action").and_then(Value::as_str).unwrap_or("");
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            Some(format!("<{name}:{action}> {cmd}"))
        }
        _ => None,
    }
}

fn parse_ts(v: Option<&Value>) -> Option<f64> {
    let s = v?.as_str()?;
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    Some(dt.timestamp_millis() as f64 / 1000.0)
}

fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

struct Pending {
    ts: Option<f64>,
    commands: Vec<String>,
    background: bool,
}

pub fn extract(dir: &Path) -> Result<Vec<Call>> {
    let mut calls = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("session_") && name.ends_with(".json")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        extract_session(&session, &mut calls);
    }
    calls.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));
    Ok(calls)
}

pub fn extract_session(session: &Value, calls: &mut Vec<Call>) {
    let working_dir = session
        .get("working_dir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let id = session
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let Some(messages) = session.get("messages").and_then(Value::as_array) else {
        return;
    };
    for message in messages {
        let ts = parse_ts(message.get("timestamp"));
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    let mut commands = Vec::new();
                    if name == "batch" {
                        for sub in input
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            let tool = sub.get("tool").and_then(Value::as_str).unwrap_or("");
                            commands.extend(tool_command(tool, sub));
                        }
                    } else {
                        commands.extend(tool_command(name, &input));
                    }
                    commands.retain(|c| c.contains("cargo") || c.starts_with('<'));
                    if commands.is_empty() {
                        continue;
                    }
                    let background = input
                        .get("run_in_background")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    pending.insert(
                        id,
                        Pending {
                            ts,
                            commands,
                            background,
                        },
                    );
                }
                Some("tool_result") => {
                    let tid = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let Some(p) = pending.remove(tid) else {
                        continue;
                    };
                    let text = result_text(block.get("content"));
                    calls.push(build_call(&id, &working_dir, p, ts, &text));
                }
                _ => {}
            }
        }
    }
}

fn build_call(session: &str, wd: &str, p: Pending, end_ts: Option<f64>, text: &str) -> Call {
    let mut wall = match (p.ts, end_ts) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    if let Some(c) = TIMING.captures(text) {
        let v: f64 = c[1].parse().unwrap_or(0.0);
        wall = Some(if &c[2] == "ms" { v / 1000.0 } else { v });
    }
    let cargo_finished_secs = FINISHED
        .captures_iter(text)
        .map(|c| {
            let mins: f64 = c.get(1).map_or(0.0, |m| m.as_str().parse().unwrap_or(0.0));
            let v: f64 = c[2].parse().unwrap_or(0.0);
            let v = if &c[3] == "ms" { v / 1000.0 } else { v };
            mins * 60.0 + v
        })
        .collect();
    let mut test_exec_secs = 0.0;
    let mut tests_run = 0;
    for c in TEST_RESULT.captures_iter(text) {
        tests_run += c[1].parse::<u64>().unwrap_or(0) + c[2].parse::<u64>().unwrap_or(0);
        test_exec_secs += c[3].parse::<f64>().unwrap_or(0.0);
    }
    let kind = p
        .commands
        .iter()
        .map(|c| classify(c))
        .min()
        .unwrap_or(Kind::Other);
    Call {
        session: session.to_owned(),
        working_dir: wd.to_owned(),
        kind,
        timestamp: p.ts.unwrap_or(0.0),
        wall_secs: wall,
        background: p.background,
        cargo_finished_secs,
        test_exec_secs,
        tests_run,
        rebuilt_crates: REBUILT
            .captures_iter(text)
            .map(|c| c[1].to_owned())
            .collect(),
        lock_blocked: text.contains("Blocking waiting for file lock"),
        compile_errors: ERROR.find_iter(text).count(),
        commands: p.commands,
    }
}

pub fn dump(calls: &[Call], path: &Path) -> Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for c in calls {
        serde_json::to_writer(&mut f, c)?;
        f.write_all(b"\n")?;
    }
    Ok(())
}

#[derive(Default)]
struct Dist(Vec<f64>);

impl Dist {
    fn push(&mut self, v: f64) {
        self.0.push(v);
    }
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.0.clone();
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            return 0.0;
        }
        v[((p / 100.0) * v.len() as f64) as usize].min(v[v.len() - 1])
    }
    fn sum(&self) -> f64 {
        self.0.iter().sum()
    }
}

impl fmt::Display for Dist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return write!(f, "n=0");
        }
        write!(
            f,
            "n={:<5} total={:>6.1}h  p50={:>5.1}s  p90={:>6.1}s  p99={:>6.1}s",
            self.0.len(),
            self.sum() / 3600.0,
            self.pct(50.0),
            self.pct(90.0),
            self.pct(99.0)
        )
    }
}

pub struct Report {
    text: String,
}

impl Report {
    pub fn build(calls: &[Call], repo: Option<&str>, max_wall: f64, top: usize) -> Report {
        use std::fmt::Write as _;
        let mut out = String::new();
        let selected: Vec<&Call> = calls
            .iter()
            .filter(|c| repo.is_none_or(|r| c.working_dir.contains(r)))
            .filter(|c| c.kind != Kind::Other)
            .collect();
        let fg: Vec<&Call> = selected
            .iter()
            .copied()
            .filter(|c| !c.background)
            .filter(|c| c.wall_secs.is_some_and(|w| w > 0.0 && w < max_wall))
            .collect();
        let hung = selected
            .iter()
            .filter(|c| c.wall_secs.is_some_and(|w| w >= max_wall))
            .count();
        let _ = writeln!(
            out,
            "cargo calls: {} ({} foreground, {} background, {} over {:.0}s excluded)\n",
            selected.len(),
            fg.len(),
            selected.iter().filter(|c| c.background).count(),
            hung,
            max_wall
        );

        let _ = writeln!(out, "Agent-blocking wall time by kind");
        let mut by_kind: BTreeMap<Kind, Dist> = BTreeMap::new();
        let mut all = Dist::default();
        for c in &fg {
            let w = c.wall_secs.unwrap();
            by_kind.entry(c.kind).or_default().push(w);
            all.push(w);
        }
        let _ = writeln!(out, "  {:<6} {all}", "all");
        for (k, d) in &by_kind {
            let _ = writeln!(out, "  {:<6} {d}", format!("{k:?}").to_lowercase());
        }

        let tests: Vec<&&Call> = fg
            .iter()
            .filter(|c| c.kind == Kind::Test && c.tests_run > 0)
            .collect();
        let total: f64 = tests.iter().map(|c| c.wall_secs.unwrap()).sum();
        let exec: f64 = tests.iter().map(|c| c.test_exec_secs).sum();
        let mut ran = Dist::default();
        for c in &tests {
            ran.push(c.tests_run as f64);
        }
        let _ = writeln!(
            out,
            "\nTest calls with results: {}  wall={:.1}h  test execution={:.1}h ({:.0}%)  compile+overhead={:.1}h ({:.0}%)",
            tests.len(),
            total / 3600.0,
            exec / 3600.0,
            100.0 * exec / total.max(1.0),
            (total - exec) / 3600.0,
            100.0 * (total - exec) / total.max(1.0)
        );
        let _ = writeln!(
            out,
            "  tests run per call: p50={:.0} p90={:.0}",
            ran.pct(50.0),
            ran.pct(90.0)
        );

        let failed: Dist = Dist(
            fg.iter()
                .filter(|c| c.compile_errors > 0)
                .map(|c| c.wall_secs.unwrap())
                .collect(),
        );
        let locked: Dist = Dist(
            fg.iter()
                .filter(|c| c.lock_blocked)
                .map(|c| c.wall_secs.unwrap())
                .collect(),
        );
        let _ = writeln!(out, "\nCalls that ended in compile errors: {failed}");
        let _ = writeln!(out, "Calls blocked on the cargo lock:     {locked}");

        let mut patterns: HashMap<String, Dist> = HashMap::new();
        for c in &fg {
            let share = c.wall_secs.unwrap() / c.commands.len() as f64;
            for cmd in &c.commands {
                patterns.entry(pattern(cmd)).or_default().push(share);
            }
        }
        let mut patterns: Vec<_> = patterns.into_iter().collect();
        patterns.sort_by(|a, b| b.1.sum().total_cmp(&a.1.sum()));
        let _ = writeln!(out, "\nTop command patterns by total wall time");
        for (p, d) in patterns.iter().take(top) {
            let _ = writeln!(
                out,
                "  {:>5.1}h  n={:<4} p50={:>5.1}s  {}",
                d.sum() / 3600.0,
                d.0.len(),
                d.pct(50.0),
                p
            );
        }

        let mut crates: HashMap<&str, usize> = HashMap::new();
        for c in &fg {
            for k in &c.rebuilt_crates {
                *crates.entry(k).or_default() += 1;
            }
        }
        let mut crates: Vec<_> = crates.into_iter().collect();
        crates.sort_by(|a, b| b.1.cmp(&a.1));
        let _ = writeln!(
            out,
            "\nMost frequently rebuilt crates (where output was visible)"
        );
        for (k, n) in crates.iter().take(top) {
            let _ = writeln!(out, "  {n:>4}  {k}");
        }
        Report { text: out }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_cargo_subcommands() {
        assert_eq!(classify("cd x && cargo test -p a --lib"), Kind::Test);
        assert_eq!(classify("cargo +nightly check"), Kind::Check);
        assert_eq!(classify("cargo fmt && cargo build"), Kind::Build);
        assert_eq!(classify("cargo fmt && cargo test"), Kind::Test);
        assert_eq!(classify("ls"), Kind::Other);
    }

    #[test]
    fn normalizes_patterns() {
        assert_eq!(
            pattern("cd /x && cargo test -p jcode-desktop-ui --lib voice 2>&1 | tail"),
            "cargo test -p jcode-desktop-ui --lib"
        );
    }

    #[test]
    fn extracts_call_with_timing_and_results() {
        let session = json!({
            "id": "s1",
            "working_dir": "/home/u/proj",
            "messages": [
                {"timestamp": "2026-10-01T00:00:00Z", "content": [
                    {"type": "tool_use", "id": "t1", "name": "bash", "input": {"command": "cargo test -p a"}}
                ]},
                {"timestamp": "2026-10-01T00:01:00Z", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content":
                        "[tool timing: start=a finish=b duration=42.5s]\n   Compiling a v0.1.0\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 30s\ntest result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.50s"}
                ]}
            ]
        });
        let mut calls = Vec::new();
        extract_session(&session, &mut calls);
        assert_eq!(calls.len(), 1);
        let c = &calls[0];
        assert_eq!(c.kind, Kind::Test);
        assert_eq!(c.wall_secs, Some(42.5));
        assert_eq!(c.cargo_finished_secs, vec![90.0]);
        assert_eq!(c.tests_run, 7);
        assert_eq!(c.test_exec_secs, 2.5);
        assert_eq!(c.rebuilt_crates, vec!["a".to_owned()]);
    }
}
