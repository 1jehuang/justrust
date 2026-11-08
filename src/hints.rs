//! Recurring-cost hints: what keeps costing this workspace time across runs,
//! not just this one. Read at the end of an agent-mode run from the tail of
//! the run index and a bounded number of recent summaries in the same
//! workspace, and shown only for costs the current run paid again (a slow
//! test it ran, a crate it rebuilt), so the line is always about the thing
//! the agent is iterating on.

use crate::summary::{IndexEntry, Summary};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How far back history counts.
const WINDOW_SECS: f64 = 7.0 * 86400.0;
/// Bytes read from the end of the index (about 1,500 runs).
const INDEX_TAIL_BYTES: u64 = 1 << 20;
/// Summaries read per report. Each is a few KB.
const MAX_SUMMARIES: usize = 60;
/// Hints shown per report.
const MAX_HINTS: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Hint {
    /// Seconds this cost added up to over the window, including this run.
    pub total_secs: f64,
    pub runs: usize,
    pub message: String,
}

/// Hints for `s` from recorded history. Empty when disabled
/// (`JUSTRUST_HINTS=0`), on a remote agent (the client reports instead), or
/// when nothing recurs.
pub fn for_run(s: &Summary) -> Vec<Hint> {
    if std::env::var_os("JUSTRUST_HINTS").is_some_and(|v| v == "0")
        || std::env::var_os(crate::route::ON_AGENT).is_some()
    {
        return Vec::new();
    }
    let Ok(runs) = crate::paths::runs_dir() else {
        return Vec::new();
    };
    let Ok(index) = crate::paths::index_file() else {
        return Vec::new();
    };
    let root = workspace(s);
    // Only a run that paid a candidate cost itself can get a hint: quick runs
    // skip the history read (about 40 ms) entirely.
    let candidate = s.tests.slowest.iter().any(|t| t.secs >= 2.0)
        || s.top_units.iter().any(|u| u.local && u.wall >= 10.0);
    if root.is_empty() || !candidate {
        return Vec::new();
    }
    let history: Vec<Summary> = recent_ids(&index, &root, (s.start - WINDOW_SECS, s.start), &s.id)
        .into_iter()
        .filter_map(|id| {
            let b = std::fs::read(runs.join(&id).join("summary.json")).ok()?;
            serde_json::from_slice(&b).ok()
        })
        .collect();
    analyze(s, &history)
}

fn workspace(s: &Summary) -> String {
    s.git.root.clone().unwrap_or_else(|| {
        crate::early::workspace_root(Path::new(&s.cwd))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.cwd.clone())
    })
}

/// Ids of the newest runs in `root` started in `since..=until`, newest first, at most
/// `MAX_SUMMARIES`. Reads only the tail of the index.
fn recent_ids(index: &Path, root: &str, (since, until): (f64, f64), exclude: &str) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(index) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(INDEX_TAIL_BYTES);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    let mut lines = buf.lines();
    if from > 0 {
        lines.next(); // partial line
    }
    let in_root = |cwd: &str| Path::new(cwd).starts_with(root);
    let mut ids: Vec<String> = lines
        .rev()
        .filter(|l| l.contains("\"cwd\""))
        .filter_map(|l| serde_json::from_str::<IndexEntry>(l).ok())
        .filter(|e| e.start >= since && e.start <= until && e.id != exclude && in_root(&e.cwd))
        .map(|e| e.id)
        .take(MAX_SUMMARIES * 2)
        .collect();
    ids.dedup();
    ids.truncate(MAX_SUMMARIES);
    ids
}

/// `name-0123456789abcdef` (cargo's test binary file name) to `name`.
fn binary_name(b: &str) -> &str {
    match b.rsplit_once('-') {
        Some((n, h)) if h.len() == 16 && h.bytes().all(|c| c.is_ascii_hexdigit()) => n,
        _ => b,
    }
}

#[derive(Default)]
struct Tally {
    secs: f64,
    runs: usize,
}

/// The hints for `s` given earlier runs of the same workspace.
pub fn analyze(s: &Summary, history: &[Summary]) -> Vec<Hint> {
    let mut out = Vec::new();

    // Slow tests this run ran, that were slow before too.
    let mut tests: HashMap<&str, Tally> = HashMap::new();
    for h in history.iter().chain(std::iter::once(s)) {
        let mut seen = std::collections::HashSet::new();
        for t in &h.tests.slowest {
            if t.secs >= 2.0 && seen.insert(t.name.as_str()) {
                let e = tests.entry(t.name.as_str()).or_default();
                e.secs += t.secs;
                e.runs += 1;
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for t in &s.tests.slowest {
        if t.secs < 2.0 || !seen.insert(t.name.as_str()) {
            continue;
        }
        let Some(e) = tests.get(t.name.as_str()) else {
            continue;
        };
        if e.runs < 3 || e.secs < 60.0 {
            continue;
        }
        let bin = binary_name(&t.binary);
        let filter = if bin.is_empty() {
            String::new()
        } else {
            format!(" (`--test {bin}`)")
        };
        out.push(Hint {
            total_secs: e.secs,
            runs: e.runs,
            message: format!(
                "test `{}`{filter} has cost {:.0}s over {} runs this week (~{:.0}s each). \
                 If it is not what you are changing, skip it while iterating \
                 (`-- --skip {}`) and run it once before finishing; or mark it \
                 #[ignore] and run it with `-- --ignored` in CI",
                t.name,
                e.secs,
                e.runs,
                e.secs / e.runs as f64,
                t.name
            ),
        });
    }

    // Local crates this run rebuilt that keep taking long to rebuild.
    let mut units: HashMap<(&str, &str), Tally> = HashMap::new();
    for h in history.iter().chain(std::iter::once(s)) {
        let mut seen = std::collections::HashSet::new();
        for u in h.top_units.iter().filter(|u| u.local) {
            if seen.insert((u.name.as_str(), u.kind.as_str())) {
                let e = units.entry((u.name.as_str(), u.kind.as_str())).or_default();
                e.secs += u.wall;
                e.runs += 1;
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for u in s.top_units.iter().filter(|u| u.local && u.wall >= 10.0) {
        if !seen.insert((u.name.as_str(), u.kind.as_str())) {
            continue;
        }
        let Some(e) = units.get(&(u.name.as_str(), u.kind.as_str())) else {
            continue;
        };
        let avg = e.secs / e.runs as f64;
        if e.runs < 3 || e.secs < 120.0 || avg < 10.0 {
            continue;
        }
        let what = if u.kind == "test" {
            "test harness build"
        } else {
            "rebuild"
        };
        out.push(Hint {
            total_secs: e.secs,
            runs: e.runs,
            message: format!(
                "`{}` {what} has cost {:.0}s over {} runs this week (~{avg:.0}s each). \
                 Every edit recompiles the whole crate: `justrust split` shows where to cut it \
                 so edits rebuild a smaller piece{}",
                u.name,
                e.secs,
                e.runs,
                if u.kind == "test" {
                    "; prefer `check` until tests are needed"
                } else {
                    ""
                }
            ),
        });
    }

    out.sort_by(|a, b| b.total_secs.total_cmp(&a.total_secs));
    out.truncate(MAX_HINTS);
    out
}

/// Footer lines for `hints`, empty when there are none.
pub fn render(hints: &[Hint]) -> String {
    let mut o = String::new();
    for (i, h) in hints.iter().enumerate() {
        o.push_str(if i == 0 { "  habit   " } else { "          " });
        o.push_str(&h.message);
        o.push('\n');
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::{TestTime, UnitBreakdown};

    fn run(id: &str, tests: &[(&str, f64)], units: &[(&str, f64)]) -> Summary {
        let mut s = Summary {
            id: id.into(),
            ..Default::default()
        };
        s.tests.slowest = tests
            .iter()
            .map(|(n, secs)| TestTime {
                name: (*n).into(),
                binary: "oracle-0123456789abcdef".into(),
                secs: *secs,
                passed: true,
            })
            .collect();
        s.top_units = units
            .iter()
            .map(|(n, w)| UnitBreakdown {
                name: (*n).into(),
                kind: "lib".into(),
                local: true,
                wall: *w,
                ..Default::default()
            })
            .collect();
        s
    }

    #[test]
    fn recurring_slow_test_run_again_is_hinted() {
        let hist: Vec<_> = (0..4)
            .map(|i| run(&i.to_string(), &[("big_oracle", 40.0)], &[]))
            .collect();
        let now = run("now", &[("big_oracle", 41.0), ("tiny", 0.5)], &[]);
        let h = analyze(&now, &hist);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].runs, 5);
        assert!((h[0].total_secs - 201.0).abs() < 1e-9);
        assert!(h[0].message.contains("--skip big_oracle"));
        assert!(h[0].message.contains("--test oracle`"));
    }

    #[test]
    fn nothing_when_this_run_did_not_pay_it_or_it_is_rare() {
        let hist: Vec<_> = (0..4)
            .map(|i| run(&i.to_string(), &[("big_oracle", 40.0)], &[]))
            .collect();
        assert!(analyze(&run("now", &[("other", 30.0)], &[]), &hist).is_empty());
        // Two runs are not a pattern.
        assert!(analyze(&run("now", &[("big_oracle", 40.0)], &[]), &hist[..1]).is_empty());
    }

    #[test]
    fn recurring_heavy_rebuild_points_at_split() {
        let hist: Vec<_> = (0..3)
            .map(|i| run(&i.to_string(), &[], &[("engine", 50.0)]))
            .collect();
        let h = analyze(&run("now", &[], &[("engine", 55.0)]), &hist);
        assert_eq!(h.len(), 1);
        assert!(h[0].message.contains("justrust split"));
        // A fast rebuild this time is not worth a hint.
        assert!(analyze(&run("now", &[], &[("engine", 2.0)]), &hist).is_empty());
    }

    #[test]
    fn binary_hash_is_stripped_only_when_it_is_one() {
        assert_eq!(
            binary_name("graph_integrity-05e372fa25b7dad3"),
            "graph_integrity"
        );
        assert_eq!(binary_name("my-crate"), "my-crate");
        assert_eq!(binary_name(""), "");
    }

    #[test]
    fn index_tail_filters_by_workspace_and_window() {
        let d = std::env::temp_dir().join(format!("jr-hints-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let idx = d.join("index.jsonl");
        let mut body = String::new();
        for (id, start, cwd) in [
            ("old", 10.0, "/w"),
            ("a", 100.0, "/w/sub"),
            ("other", 100.0, "/x"),
            ("b", 200.0, "/w"),
            ("me", 300.0, "/w"),
        ] {
            let e = IndexEntry {
                id: id.into(),
                start,
                cwd: cwd.into(),
                ..Default::default()
            };
            body.push_str(&serde_json::to_string(&e).unwrap());
            body.push('\n');
        }
        body.push_str("garbage\n");
        std::fs::write(&idx, body).unwrap();
        let got = recent_ids(&idx, "/w", (50.0, 250.0), "me");
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got, vec!["b", "a"]);
    }

    #[test]
    fn render_marks_first_line() {
        let h = Hint {
            total_secs: 1.0,
            runs: 3,
            message: "m".into(),
        };
        assert_eq!(render(&[h.clone(), h]), "  habit   m\n          m\n");
    }
}
